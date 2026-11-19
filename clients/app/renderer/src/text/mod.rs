//! Text, the same on every platform and on the web: Parley (Linebender's) shapes it with HarfRust, breaks it into lines
//! as Unicode says (UAX #14, by ICU4X), reorders each line as Unicode's bidi algorithm says (UAX #9) and runs a text's
//! styled runs together, fontique finds the fonts (Allo, then the system's for what Allo hasn't, emoji too), and swash
//! rasterizes each glyph into the atlas (`colr`, skrifa, a COLRv1 emoji, which swash doesn't). This module lays a text
//! out to a width, clamps it with an ellipsis, measures it for layout and paints its glyphs from the atlas. The glyph
//! and sprite types are GPUI's (crates/gpui/src/text_system/line_layout.rs and platform.rs), as its renderer draws them.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontData, FontFamily, FontFeature, FontFeatures, FontVariation, FontVariations, FontWeight,
    Layout, LayoutContext, StyleProperty, WhiteSpaceCollapse, fontique, setting::Tag,
};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

use crate::geometry::{Bounds, ContentMask, DevicePixels, Hsla, Pixels, Point, ScaledPixels, Size, point, size};
use crate::render::Atlas;
use crate::render::atlas::AtlasKey;
use crate::render::scene::{MonochromeSprite, PolychromeSprite, Scene, TransformationMatrix};

mod colr;

/// How many positions between device pixels a glyph is rasterized at, along the line; across it, one, as text runs
/// along whole pixels.
pub(crate) const SUBPIXEL_VARIANTS_X: u8 = 4;

/// A face as a run was set in: its font, where it is on its variation axes, and whether it's emboldened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FontId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(pub u32);

/// A single glyph, ready to paint.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    pub id: GlyphId,
    /// The position of this glyph in its containing line.
    pub position: Point<Pixels>,
    /// The index of this glyph in the original text.
    pub index: usize,
    /// Where the characters it draws end in the text, and how far it advances.
    pub end: usize,
    pub advance: f32,
    /// It's in a right-to-left run, so its first character's caret is at its right.
    pub rtl: bool,
    pub is_emoji: bool,
}

/// What the atlas keys a glyph's image by: its face, size, scale and where it falls between device pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderGlyphParams {
    pub font_id: FontId,
    pub glyph_id: GlyphId,
    font_size: u32,
    pub subpixel_variant: Point<u8>,
    scale_factor: u32,
    pub is_emoji: bool,
}

impl RenderGlyphParams {
    pub fn font_size(&self) -> f32 {
        f32::from_bits(self.font_size)
    }
    pub fn scale_factor(&self) -> f32 {
        f32::from_bits(self.scale_factor)
    }
}

/// How text is set: its size and line height in pixels, its weight on the font's `wght` axis, its tracking in em,
/// and whether its figures are tabular.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: u16,
    pub tracking: f32,
    pub line_height: f32,
    pub tabular: bool,
}

impl TextStyle {
    fn key(&self) -> [u32; 5] {
        [self.size.to_bits(), u32::from(self.weight), self.tracking.to_bits(), self.line_height.to_bits(), u32::from(self.tabular)]
    }
}

/// A run of a paragraph's text set one way, as GPUI's `TextRun` (crates/gpui/src/text_system.rs): how many bytes it
/// is, its type style, its colour, what's behind it and whether it's underlined, as a text inside a text sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRun {
    pub len: usize,
    pub style: TextStyle,
    pub color: Hsla,
    pub background_color: Option<Hsla>,
    pub underline: bool,
}

/// One line of a paragraph as it's drawn: its glyphs, each with its face and size, from its own left edge.
#[derive(Debug, Default, Clone)]
pub struct Line {
    pub glyphs: Vec<(FontId, f32, ShapedGlyph)>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    /// Where it is in the paragraph's text, in bytes.
    pub start: usize,
    pub end: usize,
    /// Its paragraph runs right to left.
    pub rtl: bool,
}

/// Text wrapped to a width, line by line.
#[derive(Debug, Default, Clone)]
pub struct Paragraph {
    pub lines: Vec<Line>,
    pub line_height: f32,
}

impl Paragraph {
    pub fn size(&self) -> Size<f32> {
        Size { width: self.lines.iter().map(|line| line.width).fold(0.0, f32::max), height: self.lines.len().max(1) as f32 * self.line_height }
    }

    /// The line byte `index` is on: the last that starts at or before it, so a caret where a line wraps goes to the
    /// start of the next.
    pub fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|line| line.start <= index).unwrap_or(0)
    }

    /// Where the caret goes before byte `index`: its line, and how far along it, in the line's own drawing. Where
    /// the text runs right to left the caret before a character is at its glyph's right.
    pub fn caret(&self, index: usize) -> (usize, f32) {
        let at = self.line_of(index);
        let Some(line) = self.lines.get(at) else { return (0, 0.0) };
        let x = match line.glyphs.iter().find(|(_, _, glyph)| glyph.index <= index && index < glyph.end.max(glyph.index + 1)) {
            Some((_, _, glyph)) if glyph.rtl => glyph.position.x.0 + glyph.advance,
            Some((_, _, glyph)) => glyph.position.x.0,
            // Past the line's last character: after the glyph that ends there, on the side its run goes.
            None => match line.glyphs.iter().filter(|(_, _, glyph)| glyph.index < index).max_by_key(|(_, _, glyph)| glyph.index) {
                Some((_, _, glyph)) if glyph.rtl => glyph.position.x.0,
                Some((_, _, glyph)) => glyph.position.x.0 + glyph.advance,
                None if line.rtl => line.width,
                None => 0.0,
            },
        };
        (at, x)
    }

    /// The byte nearest to `x` along line `at`, or `None` if it has no such line: the side of the glyph under it
    /// nearer to `x`, which in a right-to-left run is its end on its left.
    pub fn index_on(&self, at: usize, x: f32) -> Option<usize> {
        let line = self.lines.get(at)?;
        // A line that wraps ends before the spaces it hangs, so a caret at its end is still on it.
        let last = if at + 1 < self.lines.len() { line.end.saturating_sub(1).max(line.start) } else { line.end };
        let sides = |glyph: &ShapedGlyph| if glyph.rtl { (glyph.end, glyph.index) } else { (glyph.index, glyph.end) };
        let mut nearest = (f32::INFINITY, if line.rtl { line.start } else { last });
        for (_, _, glyph) in &line.glyphs {
            let (left, right) = sides(glyph);
            let (x0, x1) = (glyph.position.x.0, glyph.position.x.0 + glyph.advance);
            if x >= x0 && x < x1 {
                return Some((if x < (x0 + x1) / 2.0 { left } else { right }).min(last));
            }
            for (edge, index) in [(x0, left), (x1, right)] {
                if (x - edge).abs() < nearest.0 {
                    nearest = ((x - edge).abs(), index);
                }
            }
        }
        Some(nearest.1.min(last))
    }

    /// The byte nearest to a point inside the paragraph.
    pub fn index_at(&self, point: Point<f32>) -> usize {
        let at = ((point.y / self.line_height).floor().max(0.0) as usize).min(self.lines.len().saturating_sub(1));
        self.index_on(at, point.x).unwrap_or(0)
    }

    /// Where bytes `start..end` are drawn on each line, as `spans` says, without the spaces a line hangs past its
    /// end, as CSS draws an inline's background, underline and where it's pressed.
    pub fn inked(&self, start: usize, end: usize) -> Vec<(usize, f32, f32)> {
        self.spans(start, end)
            .into_iter()
            .filter_map(|(at, x0, x1)| {
                let line = &self.lines[at];
                let x1 = if line.rtl { x1 } else { x1.min(line.width) };
                (x1 > x0).then_some((at, x0, x1))
            })
            .collect()
    }

    /// Where bytes `start..end` are drawn on each line: its line and the stretches along it, left to right.
    pub fn spans(&self, start: usize, end: usize) -> Vec<(usize, f32, f32)> {
        let mut spans: Vec<(usize, f32, f32)> = Vec::new();
        for (at, line) in self.lines.iter().enumerate() {
            let mut covered: Vec<(f32, f32)> = line
                .glyphs
                .iter()
                .filter(|(_, _, glyph)| glyph.index < end && glyph.end > start)
                .map(|(_, _, glyph)| (glyph.position.x.0, glyph.position.x.0 + glyph.advance))
                .collect();
            covered.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (x0, x1) in covered {
                match spans.last_mut() {
                    Some((line, _, right)) if *line == at && x0 <= *right + 0.5 => *right = right.max(x1),
                    _ => spans.push((at, x0, x1)),
                }
            }
        }
        spans
    }
}

/// A face glyphs are drawn from: a font, at the place on its variation axes its run was set at, emboldened where the
/// font hasn't the weight asked for, and whether it's an emoji font's, whose glyphs are drawn in colour.
struct Face {
    font: FontData,
    coords: Vec<i16>,
    embolden: bool,
    emoji: bool,
}

/// A text shaped in its type styles, by the hash of both, with them to tell it from another of the same hash.
struct Shaped {
    text: String,
    style: [u32; 5],
    fonts: Vec<(Range<usize>, [u32; 5])>,
    layout: Layout<()>,
}

/// Text's fonts and what's been laid out with them: each text, by its type styles, shaped once and broken into lines
/// again for each width it's asked at.
pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    family: String,
    shaped: HashMap<u64, Shaped>,
    faces: Vec<Face>,
    scaler: ScaleContext,
    raster_bounds: HashMap<RenderGlyphParams, Bounds<DevicePixels>>,
    /// A glyph's image, made to measure it, until it's put in the atlas.
    pending: HashMap<RenderGlyphParams, swash::scale::image::Image>,
}

impl TextSystem {
    /// Text in `family`, from `fonts`, and from the system's fonts where those lack a character, if `system_fonts`.
    pub fn new(family: &str, fonts: Vec<Vec<u8>>, system_fonts: bool) -> TextSystem {
        let mut collection = fontique::Collection::new(fontique::CollectionOptions { shared: false, system_fonts });
        for font in fonts {
            collection.register_fonts(fontique::Blob::new(Arc::new(font)), None);
        }
        TextSystem {
            fonts: FontContext { collection, source_cache: fontique::SourceCache::default() },
            layouts: LayoutContext::new(),
            family: family.to_owned(),
            shaped: HashMap::new(),
            faces: Vec::new(),
            scaler: ScaleContext::new(),
            raster_bounds: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// `text` set in `style`, or in `runs` where it's given them (a text with texts in it), wrapped to `width` (none:
    /// as wide as it is) and cut short with an ellipsis after `max_lines`. A line ends at a newline, or where Unicode's
    /// line breaking (UAX #14) allows before it'd be wider than `width`; a word wider than `width` alone overflows it,
    /// as CSS's `overflow-wrap: normal` has it. Its lines are as tall as its tallest run's.
    pub fn paragraph(&mut self, text: &str, style: &TextStyle, runs: &[TextRun], width: Option<f32>, max_lines: Option<usize>) -> Paragraph {
        let fonts = fonts(text, style, runs);
        // A run's line is as tall for its size as the text's is for its own, as an inline inherits CSS's unitless
        // `line-height`: a larger run makes its lines taller.
        let line_height = fonts.iter().map(|(_, run)| run.size * style.line_height / style.size).fold(style.line_height, f32::max);
        let mut lines = self.lines(text, style, &fonts, width, 0);
        if let Some(max) = max_lines.filter(|max| *max > 0 && lines.len() > *max) {
            lines.truncate(max);
            let last = lines.pop().expect("max > 0");
            let end = text[last.start..].find('\n').map_or(text.len(), |end| last.start + end);
            let tail = style_at(&fonts, end.saturating_sub(1).max(last.start)).unwrap_or(*style);
            let ellipsis = self.lines("…", &tail, &[], None, 0).pop().map_or(0.0, |line| line.width);
            let room = width.unwrap_or(f32::INFINITY) - ellipsis;
            let whole = self.lines(&text[last.start..end], style, &within(&fonts, last.start..end), None, last.start).pop().unwrap_or_default();
            let cut = whole.glyphs.iter().find(|(_, _, glyph)| glyph.position.x.0 + glyph.advance > room).map_or(end, |(_, _, glyph)| glyph.index);
            let kept = last.start + text[last.start..cut].trim_end().len();
            let tail = style_at(&fonts, kept.saturating_sub(1).max(last.start)).unwrap_or(*style);
            let cut_text = format!("{}…", &text[last.start..kept]);
            let mut cut_fonts = within(&fonts, last.start..kept);
            cut_fonts.push((kept - last.start..cut_text.len(), tail));
            lines.extend(self.lines(&cut_text, style, &cut_fonts, None, last.start));
        }
        Paragraph { lines, line_height }
    }

    /// `text` set in `style`, each of `fonts` its range in its own, broken into lines no wider than `width` and each
    /// reordered as it's drawn; its glyphs' and lines' indices are moved on by `offset`.
    fn lines(&mut self, text: &str, style: &TextStyle, fonts: &[(Range<usize>, TextStyle)], width: Option<f32>, offset: usize) -> Vec<Line> {
        let mut hasher = std::hash::DefaultHasher::new();
        (text, style.key()).hash(&mut hasher);
        for (range, style) in fonts {
            (range, style.key()).hash(&mut hasher);
        }
        let key = hasher.finish();
        let same = |shaped: &Shaped| shaped.text == text && shaped.style == style.key() && shaped.fonts.iter().map(|(range, key)| (range, *key)).eq(fonts.iter().map(|(range, style)| (range, style.key())));
        if !self.shaped.get(&key).is_some_and(same) {
            if self.shaped.len() > 4096 {
                self.shaped.clear();
            }
            let layout = self.shape(text, style, fonts);
            let fonts = fonts.iter().map(|(range, style)| (range.clone(), style.key())).collect();
            self.shaped.insert(key, Shaped { text: text.to_owned(), style: style.key(), fonts, layout });
        }
        let layout = &mut self.shaped.get_mut(&key).expect("shaped above").layout;
        layout.break_all_lines(width);
        drawn(layout, text, offset, &mut self.faces)
    }

    /// `text` shaped in `style`, each of `fonts` in its own: Allo at its weight and its size on its optical size axis,
    /// as CSS's `font-optical-sizing: auto` sets it, with its letter spacing and figures, and its white space kept, as
    /// an input's is; a line's start is its left, whichever way it runs, as where it goes along its box is ours. Its
    /// lines are at least as tall as `style`'s font, as CSS's strut makes them.
    fn shape(&mut self, text: &str, style: &TextStyle, fonts: &[(Range<usize>, TextStyle)]) -> Layout<()> {
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(&self.family)));
        builder.push_default(StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve));
        for property in properties(style) {
            builder.push_default(property);
        }
        for (range, style) in fonts {
            for property in properties(style) {
                builder.push(property, range.clone());
            }
        }
        let mut layout = builder.build(text);
        layout.set_alignment(Alignment::Left, AlignmentOptions::default());
        layout
    }

    /// Where glyph `params` is drawn from its pen, in device pixels, which is nothing for a space.
    fn glyph_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let image = self.render_glyph(params)?;
        let bounds = Bounds {
            origin: point(image.placement.left.into(), (-image.placement.top).into()),
            size: size(image.placement.width.into(), image.placement.height.into()),
        };
        if !bounds.is_zero() {
            self.pending.insert(params.clone(), image);
        }
        Ok(bounds)
    }

    /// Glyph `params`'s pixels, `bounds` in size: coverage for a glyph, BGRA for an emoji.
    fn rasterize_glyph(&mut self, params: &RenderGlyphParams, bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        let mut image = match self.pending.remove(params) {
            Some(image) => image,
            None => self.render_glyph(params)?,
        };
        if params.is_emoji {
            for pixel in image.data.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        Ok((bounds.size, image.data))
    }

    // From Zed's GPUI (crates/gpui_wgpu/src/cosmic_text_system.rs at 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a), changed:
    // swash rasterizes a glyph at the place on its font's axes Parley shaped it at, a COLRv1 emoji by skrifa (`colr`).
    // Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
    // (clients/app/renderer/LICENSE-APACHE).
    fn render_glyph(&mut self, params: &RenderGlyphParams) -> Result<swash::scale::image::Image> {
        let face = &self.faces[params.font_id.0];
        let (data, index) = (face.font.data.data(), face.font.index);
        let ppem = params.font_size() * params.scale_factor();
        if params.is_emoji {
            let coords: Vec<skrifa::instance::NormalizedCoord> = face.coords.iter().map(|coord| skrifa::instance::NormalizedCoord::from_bits(*coord)).collect();
            let colr = skrifa::FontRef::from_index(data, index).ok().and_then(|font| colr::render(&font, params.glyph_id.0.try_into().ok()?, ppem, &coords));
            if let Some((left, top, width, height, rgba)) = colr {
                let mut image = swash::scale::image::Image::new();
                image.content = swash::scale::image::Content::Color;
                image.placement = swash::zeno::Placement { left, top, width, height };
                image.data = rgba;
                return Ok(image);
            }
        }
        let font = swash::FontRef::from_index(data, index as usize).context("the font isn't one swash reads")?;
        let mut scaler = self.scaler.builder(font).size(ppem).hint(true).normalized_coords(&face.coords).build();
        let sources: &[Source] = if params.is_emoji {
            &[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };
        let offset = Vector::new(params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor(), 0.0);
        let mut render = Render::new(sources);
        render.format(Format::Alpha).offset(offset);
        if face.embolden {
            render.embolden(ppem / 48.0);
        }
        let glyph: u16 = params.glyph_id.0.try_into()?;
        render.render(&mut scaler, glyph).with_context(|| format!("swash didn't draw {params:?}"))
    }

    /// Paints `paragraph` with its top left at `origin`, each line placed along `width` by `align` (0 start, 0.5
    /// centre, 1 end), as the shaders draw glyphs: from the atlas, each at the nearest of a few positions between
    /// device pixels, rasterized at the window's scale, then turned as `transformation` says.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        paragraph: &Paragraph,
        origin: Point<Pixels>,
        width: f32,
        align: f32,
        color: &dyn Fn(usize) -> Hsla,
        scale_factor: f32,
        content_mask: ContentMask<ScaledPixels>,
        transformation: TransformationMatrix,
        atlas: &Atlas,
        scene: &mut Scene,
    ) -> Result<()> {
        for (at, line) in paragraph.lines.iter().enumerate() {
            // The line's baseline, with half its leading above, as CSS sets a line box.
            let leading = paragraph.line_height - (line.ascent + line.descent);
            let baseline = origin.y.0 + at as f32 * paragraph.line_height + leading / 2.0 + line.ascent;
            let left = origin.x.0 + (width - line.width).max(0.0) * align;
            for (font_id, size, glyph) in &line.glyphs {
                let point = Point { x: Pixels(left + glyph.position.x.0), y: Pixels(baseline + glyph.position.y.0) };
                self.paint_glyph(point, *font_id, *size, glyph, color(glyph.index), scale_factor, content_mask, transformation, atlas, scene)?;
            }
        }
        Ok(())
    }

    // From GPUI's `Window::paint_glyph` and `paint_emoji` (crates/gpui/src/window.rs at 69e21302).
    #[allow(clippy::too_many_arguments)]
    fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        font_size: f32,
        glyph: &ShapedGlyph,
        color: Hsla,
        scale_factor: f32,
        content_mask: ContentMask<ScaledPixels>,
        transformation: TransformationMatrix,
        atlas: &Atlas,
        scene: &mut Scene,
    ) -> Result<()> {
        let glyph_origin = origin.scale(scale_factor);
        let subpixel_variant = Point {
            x: (glyph_origin.x.0.fract() * SUBPIXEL_VARIANTS_X as f32).floor() as u8,
            y: 0,
        };
        let params = RenderGlyphParams {
            font_id,
            glyph_id: glyph.id,
            font_size: font_size.to_bits(),
            subpixel_variant,
            scale_factor: scale_factor.to_bits(),
            is_emoji: glyph.is_emoji,
        };
        let raster_bounds = match self.raster_bounds.get(&params) {
            Some(bounds) => *bounds,
            None => {
                let bounds = self.glyph_bounds(&params)?;
                self.raster_bounds.insert(params.clone(), bounds);
                bounds
            }
        };
        if raster_bounds.is_zero() {
            return Ok(());
        }
        let tile = atlas
            .get_or_insert_with(&AtlasKey::Glyph(params.clone()), &mut || {
                let (size, bytes) = self.rasterize_glyph(&params, raster_bounds)?;
                Ok(Some((size, std::borrow::Cow::Owned(bytes))))
            })?
            .expect("the callback above only errors or returns Some");
        let bounds = Bounds {
            origin: glyph_origin.map(|px| ScaledPixels(px.0.floor())) + raster_bounds.origin.map(Into::into),
            size: tile.bounds.size.map(Into::into),
        };
        if glyph.is_emoji {
            scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: false,
                opacity: color.a,
                bounds,
                corner_radii: Default::default(),
                content_mask,
                tile,
                transformation,
            });
        } else {
            scene.insert_primitive(MonochromeSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color,
                tile,
                transformation,
            });
        }
        Ok(())
    }
}

/// `layout`'s lines as they're drawn: each line's glyphs from its own left edge, left to right, each with the
/// characters it draws, its width without the spaces it hangs past its end, and the bytes it holds without the newline
/// it ends at; its glyphs' and lines' indices moved on by `offset`.
fn drawn(layout: &Layout<()>, text: &str, offset: usize, faces: &mut Vec<Face>) -> Vec<Line> {
    let rtl = layout.is_rtl();
    let mut lines = Vec::with_capacity(layout.len());
    for line in layout.lines() {
        let metrics = line.metrics();
        let range = line.text_range();
        let end = if text[range.clone()].ends_with('\n') { range.end - 1 } else { range.end };
        // From where its ink starts: the spaces a line hangs past its end are past its right, or, in a paragraph that
        // runs right to left, its left, as CSS hangs them.
        let mut glyphs = Vec::new();
        let mut x = metrics.offset;
        // Its ascent and descent, its fonts' own, as CSS sets a line box's content from them; a line with no glyphs has
        // its text's.
        let root = layout.root_span_metrics();
        let (mut ascent, mut descent) = root.map_or((0.0, 0.0), |root| (root.ascent, root.descent));
        for run in line.runs() {
            ascent = f32::max(ascent, run.font_metrics().ascent);
            descent = f32::max(descent, run.font_metrics().descent);
            let (font_id, emoji) = face(faces, run.font(), run.normalized_coords().iter().map(|coord| coord.to_bits()), run.synthesis().embolden());
            let size = run.font_size();
            for cluster in run.visual_clusters() {
                let range = cluster.text_range();
                // A ligature's first cluster draws it, for the characters of those after it too.
                let mut end = range.end;
                let mut next = cluster.is_ligature_start().then(|| cluster.next_logical()).flatten();
                while let Some(after) = next.filter(|after| after.is_ligature_continuation()) {
                    end = after.text_range().end;
                    next = after.next_logical();
                }
                let newline = text[range.start..].starts_with('\n');
                for glyph in cluster.glyphs() {
                    if !newline {
                        let position = point(Pixels(x + glyph.x), Pixels(glyph.y));
                        let index = range.start + offset;
                        glyphs.push((font_id, size, ShapedGlyph { id: GlyphId(glyph.id), position, index, end: end + offset, advance: glyph.advance, rtl: run.is_rtl(), is_emoji: emoji }));
                    }
                    x += glyph.advance;
                }
            }
        }
        lines.push(Line {
            glyphs,
            width: (metrics.advance - metrics.hanging_advance).max(0.0),
            ascent,
            descent,
            start: range.start + offset,
            end: end + offset,
            rtl,
        });
    }
    lines
}

/// The face a run's set in, and whether it's an emoji font's, kept so its glyphs are drawn as they were shaped.
fn face(faces: &mut Vec<Face>, font: &FontData, coords: impl Iterator<Item = i16> + Clone, embolden: bool) -> (FontId, bool) {
    let same = |face: &Face| face.font.data.id() == font.data.id() && face.font.index == font.index && face.embolden == embolden && face.coords.iter().copied().eq(coords.clone());
    if let Some(at) = faces.iter().position(same) {
        return (FontId(at), faces[at].emoji);
    }
    let emoji = skrifa::FontRef::from_index(font.data.data(), font.index)
        .is_ok_and(|face| [b"COLR", b"CBDT", b"sbix"].iter().any(|tag| face.table_data(skrifa::Tag::new(tag)).is_some()));
    faces.push(Face { font: font.clone(), coords: coords.collect(), embolden, emoji });
    (FontId(faces.len() - 1), emoji)
}

/// How Parley sets text in `style`.
fn properties(style: &TextStyle) -> [StyleProperty<'static, ()>; 5] {
    let tabular = if style.tabular { vec![FontFeature::new(Tag::new(b"tnum"), 1)] } else { Vec::new() };
    [
        StyleProperty::FontSize(style.size),
        StyleProperty::FontWeight(FontWeight::new(f32::from(style.weight))),
        StyleProperty::LetterSpacing(style.tracking * style.size),
        StyleProperty::FontVariations(FontVariations::List(Cow::Owned(vec![FontVariation::new(Tag::new(b"opsz"), style.size)]))),
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(tabular))),
    ]
}

/// `text`'s type styles, run by run: `style` where it has no runs, and runs side by side in the same one as one.
fn fonts(text: &str, style: &TextStyle, runs: &[TextRun]) -> Vec<(Range<usize>, TextStyle)> {
    let mut fonts: Vec<(Range<usize>, TextStyle)> = Vec::new();
    let mut at = 0;
    for run in runs.iter().chain([&TextRun { len: text.len().saturating_sub(runs.iter().map(|run| run.len).sum()), style: *style, color: Hsla::default(), background_color: None, underline: false }]) {
        let end = (at + run.len).min(text.len());
        match fonts.last_mut() {
            Some((last, was)) if *was == run.style => last.end = end,
            _ if at < end => fonts.push((at..end, run.style)),
            _ => {}
        }
        at = end;
    }
    fonts
}

/// The type style byte `index` is set in.
fn style_at(fonts: &[(Range<usize>, TextStyle)], index: usize) -> Option<TextStyle> {
    fonts.iter().find(|(range, _)| range.contains(&index)).or(fonts.last()).map(|(_, style)| *style)
}

/// `fonts` within bytes `range`, from its start.
fn within(fonts: &[(Range<usize>, TextStyle)], range: Range<usize>) -> Vec<(Range<usize>, TextStyle)> {
    fonts
        .iter()
        .filter_map(|(run, style)| {
            let (start, end) = (run.start.max(range.start), run.end.min(range.end));
            (start < end).then(|| (start - range.start..end - range.start, *style))
        })
        .collect()
}
