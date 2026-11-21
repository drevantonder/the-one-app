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
    Alignment, AlignmentOptions, FontContext, FontData, FontFamily, FontFamilyName, FontFeature, FontFeatures, FontVariation, FontVariations,
    FontWeight, GenericFamily, Layout, LayoutContext, StyleProperty, WhiteSpaceCollapse, fontique, setting::Tag,
};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

use crate::geometry::{Bounds, ContentMask, DevicePixels, Hsla, Pixels, Point, ScaledPixels, Size, point, size};
use crate::render::atlas::{Atlas, AtlasKey};
use crate::render::scene::{MonochromeSprite, PolychromeSprite, Scene, TransformationMatrix};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod colr;

/// How many positions between device pixels a glyph is rasterized at, along the line and across it, as GPUI does on a
/// Mac: a text moving with its box by a fraction of a pixel moves with it, rather than stepping a pixel at a time.
pub(crate) const SUBPIXEL_VARIANTS: u8 = 4;

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
/// whether its figures are tabular, and whether it's in the system's monospace, as CSS's `font-family: monospace`,
/// where it has no ligatures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: u16,
    pub tracking: f32,
    pub line_height: f32,
    pub tabular: bool,
    pub mono: bool,
}

impl TextStyle {
    fn key(&self) -> [u32; 6] {
        [self.size.to_bits(), u32::from(self.weight), self.tracking.to_bits(), self.line_height.to_bits(), u32::from(self.tabular), u32::from(self.mono)]
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
    /// How many bytes of spaces it hangs past its end, where it wraps after them.
    pub hangs: usize,
    /// Its paragraph runs right to left.
    pub rtl: bool,
}

/// Text wrapped to a width, line by line.
#[derive(Debug, Default, Clone)]
pub struct Paragraph {
    pub lines: Vec<Line>,
    pub line_height: f32,
    /// Whether it was cut short with an ellipsis, past its `max_lines`.
    pub cut: bool,
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
    /// Where a line wraps with no space to hang, `upstream` puts it at the end of that line, not the start of the
    /// next, as Flutter's `TextAffinity.upstream`.
    pub fn caret(&self, index: usize, upstream: bool) -> (usize, f32) {
        let mut at = self.line_of(index);
        if upstream && at > 0 && self.lines[at].start == index && self.lines[at - 1].end == index {
            at -= 1;
        }
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
        // A line that wraps ends before the spaces it hangs, so a caret at its end is still on it; one that wraps
        // with none (a long word, Chinese) ends where the next starts, where the caret is upstream.
        let last = self.end(at);
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

    /// Where line `at` ends: before the spaces it hangs, where it wraps.
    pub fn end(&self, at: usize) -> usize {
        let Some(line) = self.lines.get(at) else { return 0 };
        let wraps = self.lines.get(at + 1).is_some_and(|next| next.start == line.end);
        if wraps { line.end - line.hangs } else { line.end }
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
    style: [u32; 6],
    fonts: Vec<(Range<usize>, [u32; 6])>,
    layout: Layout<()>,
}

/// Text's fonts and what's been laid out with them: each text, by its type styles, shaped once and broken into lines
/// again for each width it's asked at.
pub struct TextSystem {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    family: String,
    /// The faces the system said have a character none before them had, after its emoji face and its UI face
    /// (`fallback`).
    fallbacks: Vec<String>,
    /// The characters no face had, since it was last asked (`undrawn`): each draws as its font's missing glyph.
    undrawn: Vec<char>,
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
            fallbacks: Vec::new(),
            undrawn: Vec::new(),
            shaped: HashMap::new(),
            faces: Vec::new(),
            scaler: ScaleContext::new(),
            raster_bounds: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// The family `name` is the system's monospace, as a host that loads no system fonts is given one.
    pub fn monospace(&mut self, name: &str) {
        if let Some(family) = self.fonts.collection.family_id(name) {
            self.fonts.collection.set_generic_families(GenericFamily::Monospace, [family].into_iter());
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
        let cut = max_lines.is_some_and(|max| max > 0 && lines.len() > max);
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
        Paragraph { lines, line_height, cut }
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
            let mut layout = self.shape(text, style, fonts);
            // A character no face had takes the face the system says has it, as a browser's does, from then on.
            while let Some((at, character)) = missing(&mut layout, text) {
                let Some(family) = self.fallback(text, at, character).filter(|family| !self.fallbacks.contains(family)) else {
                    if !self.undrawn.contains(&character) {
                        self.undrawn.push(character);
                    }
                    break;
                };
                self.fallbacks.push(family);
                self.shaped.clear();
                layout = self.shape(text, style, fonts);
            }
            let fonts = fonts.iter().map(|(range, style)| (range.clone(), style.key())).collect();
            self.shaped.insert(key, Shaped { text: text.to_owned(), style: style.key(), fonts, layout });
        }
        let layout = &mut self.shaped.get_mut(&key).expect("shaped above").layout;
        layout.break_all_lines(width);
        drawn(layout, text, offset, &mut self.faces)
    }

    /// The characters no face had, the system's or ours, since this was last asked.
    pub fn undrawn(&mut self) -> Vec<char> {
        std::mem::take(&mut self.undrawn)
    }

    /// The family of a face that has the character of `text` at byte `at`, which none it's tried had: the system's
    /// (`fallback`), or on the web, where the system's faces aren't ours to read, one of the browser's (`browser`),
    /// which replaces the one before it.
    fn fallback(&mut self, text: &str, at: usize, character: char) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = character;
            let (family, font) = browser::cover(text, at)?;
            let blob = fontique::Blob::new(Arc::new(font));
            browser::registered(blob.id());
            self.fonts.collection.register_fonts(blob, None);
            self.fallbacks.retain(|family| !browser::is_family(family));
            Some(family)
        }
        #[cfg(target_vendor = "apple")]
        {
            let _ = (text, at);
            fallback(character)
        }
        #[cfg(target_os = "windows")]
        {
            let _ = (text, at);
            fallback(character)
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "windows", target_arch = "wasm32")))]
        {
            let _ = (text, at);
            fallback(&mut self.fonts, character)
        }
    }

    /// `text` shaped in `style`, each of `fonts` in its own: Allo at its weight and its size on its optical size axis,
    /// as CSS's `font-optical-sizing: auto` sets it, with its letter spacing and figures, and its white space kept, as
    /// an input's is; a line's start is its left, whichever way it runs, as where it goes along its box is ours. Its
    /// lines are at least as tall as `style`'s font, as CSS's strut makes them.
    fn shape(&mut self, text: &str, style: &TextStyle, fonts: &[(Range<usize>, TextStyle)]) -> Layout<()> {
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, text, 1.0, false);
        // What Allo hasn't is the system's emoji face's, in colour, and then its own UI face's, a symbol (↵, ✓) as
        // much as a script, as CSS's `Allo, emoji, system-ui` has it, and then the faces the system said have what
        // none of those did (`fallback`): fontique's own fallback goes by script, and a symbol's (Common) finds
        // nothing on Apple's.
        let mut family = vec![FontFamilyName::named(&self.family), FontFamilyName::Generic(GenericFamily::Emoji), FontFamilyName::Generic(GenericFamily::SystemUi)];
        family.extend(self.fallbacks.iter().map(|name| FontFamilyName::named(name)));
        // Code is in the system's monospace, and in these where it has none (a host that loads no system fonts).
        let mono: Vec<FontFamilyName> = [FontFamilyName::Generic(GenericFamily::Monospace)].into_iter().chain(family.iter().cloned()).collect();
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(family.into())));
        builder.push_default(StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve));
        for property in properties(style) {
            builder.push_default(property);
        }
        if style.mono {
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(mono.clone().into())));
        }
        for (range, style) in fonts {
            for property in properties(style) {
                builder.push(property, range.clone());
            }
            if style.mono {
                builder.push(StyleProperty::FontFamily(FontFamily::List(mono.clone().into())), range.clone());
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
        self.render_unpadded(params).map(padded)
    }

    fn render_unpadded(&mut self, params: &RenderGlyphParams) -> Result<swash::scale::image::Image> {
        let face = &self.faces[params.font_id.0];
        let (data, index) = (face.font.data.data(), face.font.index);
        let ppem = params.font_size() * params.scale_factor();
        // One of the browser's glyphs, drawn by its canvas.
        #[cfg(target_arch = "wasm32")]
        if let Some((text, colored)) = browser::glyph(face.font.data.id(), params.glyph_id.0) {
            let (left, top, width, height, rgba) = crate::platform::web::glyph(&text, ppem).context("the browser didn't draw it")?;
            let mut image = swash::scale::image::Image::new();
            image.placement = swash::zeno::Placement { left, top, width, height };
            if colored {
                image.content = swash::scale::image::Content::Color;
                image.data = rgba;
            } else {
                image.content = swash::scale::image::Content::Mask;
                image.data = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
            }
            return Ok(image);
        }
        // A COLRv1 emoji, which only a system's emoji face is: the web has none of its own.
        #[cfg(not(target_arch = "wasm32"))]
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
        // Unhinted, as Core Text and DirectWrite draw it, so a glyph at any size or place is its outline scaled and moved,
        // and a text growing or moving with its box keeps its shape.
        let mut scaler = self.scaler.builder(font).size(ppem).hint(false).normalized_coords(&face.coords).build();
        let sources: &[Source] = if params.is_emoji {
            &[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };
        // In device pixels, the outline's y upward.
        let variant = |at: u8| f32::from(at) / f32::from(SUBPIXEL_VARIANTS);
        let offset = Vector::new(variant(params.subpixel_variant.x), -variant(params.subpixel_variant.y));
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
        // The nearest of the positions a glyph is rasterized at, from the device pixel it starts in.
        let glyph_origin = origin.scale(scale_factor);
        let nearest = |at: f32| {
            let steps = (at * f32::from(SUBPIXEL_VARIANTS)).round();
            ((steps / f32::from(SUBPIXEL_VARIANTS)).floor(), steps.rem_euclid(f32::from(SUBPIXEL_VARIANTS)) as u8)
        };
        let ((left, x), (top, y)) = (nearest(glyph_origin.x.0), nearest(glyph_origin.y.0));
        let subpixel_variant = Point { x, y };
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
            origin: Point { x: ScaledPixels(left), y: ScaledPixels(top) } + raster_bounds.origin.map(Into::into),
            size: tile.bounds.size.map(Into::into),
        };
        if glyph.is_emoji {
            scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: 0,
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
                        // One of the browser's glyphs is an emoji's where the browser draws it in colour.
                        #[cfg(target_arch = "wasm32")]
                        let emoji = emoji || browser::glyph(run.font().data.id(), glyph.id).is_some_and(|(_, colored)| colored);
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
            hangs: end - range.start - text[range.start..end].trim_end().len(),
            rtl,
        });
    }
    lines
}

/// The first character of `text` no face had a glyph for, in `layout`, laid out as one line: what draws as nothing but
/// a font's missing glyph.
fn missing(layout: &mut Layout<()>, text: &str) -> Option<(usize, char)> {
    layout.break_all_lines(None);
    let line = layout.lines().next()?;
    line.runs().flat_map(|run| run.visual_clusters().collect::<Vec<_>>()).find_map(|cluster| {
        let character = text[cluster.text_range()].chars().next()?;
        (!character.is_whitespace() && !character.is_control() && cluster.glyphs().any(|glyph| glyph.id == 0)).then_some((cluster.text_range().start, character))
    })
}

/// The family of the face the system draws `character` in where its UI font hasn't it, as Core Text's cascade finds
/// it for a browser (`CTFontCreateForString`). Elsewhere fontique's fallback finds it.
#[cfg(target_vendor = "apple")]
fn fallback(character: char) -> Option<String> {
    use objc2_core_foundation::{CFRange, CFString};
    use objc2_core_text::{CTFont, CTFontUIFontType};
    let text = CFString::from_str(&character.to_string());
    let ui = unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, 0.0, None) }?;
    let font = unsafe { ui.for_string(&text, CFRange { location: 0, length: text.length() }) };
    Some(unsafe { font.family_name() }.to_string())
}

/// The family of the face Windows draws `character` in where its UI font (Segoe UI) hasn't it, as DirectWrite's system
/// fallback maps it for any app (`IDWriteFontFallback::MapCharacters`): Segoe UI Symbol for ⇧, Segoe UI Emoji for an
/// emoji. Fontique's own fallback goes by script, and a symbol's (Common) finds nothing.
#[cfg(target_os = "windows")]
fn fallback(character: char) -> Option<String> {
    use windows::core::{implement, w, OutRef};
    use windows::Win32::Graphics::DirectWrite::*;

    /// The one character, as the text DirectWrite maps.
    #[implement(IDWriteTextAnalysisSource)]
    struct Source(Vec<u16>);

    impl IDWriteTextAnalysisSource_Impl for Source_Impl {
        fn GetTextAtPosition(&self, position: u32, text: *mut *mut u16, length: *mut u32) -> windows::core::Result<()> {
            let rest = self.0.get(position as usize..).unwrap_or_default();
            // SAFETY: DirectWrite passes valid out pointers, and the text outlives the call.
            unsafe {
                *text = if rest.is_empty() { std::ptr::null_mut() } else { rest.as_ptr().cast_mut() };
                *length = rest.len() as u32;
            }
            Ok(())
        }
        fn GetTextBeforePosition(&self, position: u32, text: *mut *mut u16, length: *mut u32) -> windows::core::Result<()> {
            let before = &self.0[..(position as usize).min(self.0.len())];
            // SAFETY: as `GetTextAtPosition`.
            unsafe {
                *text = if before.is_empty() { std::ptr::null_mut() } else { before.as_ptr().cast_mut() };
                *length = before.len() as u32;
            }
            Ok(())
        }
        fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
            DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
        }
        fn GetLocaleName(&self, position: u32, length: *mut u32, locale: *mut *mut u16) -> windows::core::Result<()> {
            // SAFETY: as `GetTextAtPosition`; the locale is a static string.
            unsafe {
                *length = (self.0.len() as u32).saturating_sub(position);
                *locale = w!("en-us").as_ptr().cast_mut();
            }
            Ok(())
        }
        fn GetNumberSubstitution(&self, position: u32, length: *mut u32, _: OutRef<IDWriteNumberSubstitution>) -> windows::core::Result<()> {
            // SAFETY: as `GetTextAtPosition`.
            unsafe { *length = (self.0.len() as u32).saturating_sub(position) };
            Ok(())
        }
    }

    static FALLBACK: std::sync::OnceLock<Option<IDWriteFontFallback>> = std::sync::OnceLock::new();
    let system = FALLBACK
        .get_or_init(|| {
            // SAFETY: a shared factory, and the system's fallback, which DirectWrite owns.
            unsafe { DWriteCreateFactory::<IDWriteFactory2>(DWRITE_FACTORY_TYPE_SHARED).ok()?.GetSystemFontFallback().ok() }
        })
        .as_ref()?;
    let mut utf16 = [0u16; 2];
    let source: IDWriteTextAnalysisSource = Source(character.encode_utf16(&mut utf16).to_vec()).into();
    let (mut length, mut font, mut scale) = (0, None, 0.0);
    // SAFETY: the source and the out pointers are valid for the call.
    unsafe {
        system
            .MapCharacters(&source, 0, character.len_utf16() as u32, None::<&IDWriteFontCollection>, w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, &mut length, &mut font, &mut scale)
            .ok()?;
        let names = font?.GetFontFamily().ok()?.GetFamilyNames().ok()?;
        let (mut at, mut found) = (0, windows::core::BOOL::default());
        let _ = names.FindLocaleName(w!("en-us"), &mut at, &mut found);
        let at = if found.as_bool() { at } else { 0 };
        let mut name = vec![0u16; names.GetStringLength(at).ok()? as usize + 1];
        names.GetString(at, &mut name).ok()?;
        name.pop();
        Some(String::from_utf16_lossy(&name))
    }
}

/// The family of the first face, by name, that has `character`, as Android's Skia walks its
/// fallback fonts for one: fontique's own fallback goes by script, and a symbol's (Common, as ⇧ and ⋮ are) finds
/// nothing in Android's or Linux's.
#[cfg(not(any(target_vendor = "apple", target_os = "windows", target_arch = "wasm32")))]
fn fallback(fonts: &mut FontContext, character: char) -> Option<String> {
    let collection = &mut fonts.collection;
    let mut families: Vec<String> = collection.family_names().map(str::to_owned).collect();
    families.sort();
    families.into_iter().find(|family| {
        collection.family_by_name(family).is_some_and(|family| {
            family.default_font().is_some_and(|font| {
                font.load(Some(&mut fonts.source_cache))
                    .is_some_and(|blob| font.charmap_index().charmap(blob.data()).and_then(|charmap| charmap.map(character)).is_some_and(|glyph| glyph != 0))
            })
        })
    })
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

/// `image` with a clear pixel all round, so a glyph drawn between device pixels, as a layer moving by a fraction of one
/// draws it (`view::Layer`), samples nothing of the glyph beside it in the atlas and loses none of its own edge.
fn padded(mut image: swash::scale::image::Image) -> swash::scale::image::Image {
    let (width, height) = (image.placement.width as usize, image.placement.height as usize);
    if width == 0 || height == 0 {
        return image;
    }
    let bytes = image.data.len() / (width * height);
    let mut data = vec![0; (width + 2) * (height + 2) * bytes];
    for (row, line) in image.data.chunks_exact(width * bytes).enumerate() {
        let at = ((row + 1) * (width + 2) + 1) * bytes;
        data[at..at + width * bytes].copy_from_slice(line);
    }
    image.data = data;
    image.placement = swash::zeno::Placement { left: image.placement.left - 1, top: image.placement.top + 1, width: image.placement.width + 2, height: image.placement.height + 2 };
    image
}

/// How Parley sets text in `style`.
fn properties(style: &TextStyle) -> Vec<StyleProperty<'static, ()>> {
    let mut features = if style.tabular { vec![FontFeature::new(Tag::new(b"tnum"), 1)] } else { Vec::new() };
    let mut properties = vec![
        StyleProperty::FontSize(style.size),
        StyleProperty::FontWeight(FontWeight::new(f32::from(style.weight))),
        StyleProperty::LetterSpacing(style.tracking * style.size),
        StyleProperty::FontVariations(FontVariations::List(Cow::Owned(vec![FontVariation::new(Tag::new(b"opsz"), style.size)]))),
    ];
    if style.mono {
        // Code reads as typed: `=>` stays two characters, whatever the face's ligatures.
        features.extend([FontFeature::new(Tag::new(b"liga"), 0), FontFeature::new(Tag::new(b"calt"), 0)]);
    }
    properties.push(StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(features))));
    properties
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
