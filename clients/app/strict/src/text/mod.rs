//! Text: a platform's text system shapes a line and rasterizes its glyphs (`cosmic_text_system`, cosmic-text, on Linux
//! and Android; `windows`, DirectWrite; `mac`, Core Text), and this module, the same everywhere, wraps the line to a
//! width, clamps it with an ellipsis, measures it for layout and paints its glyphs from the atlas. The types are GPUI's (crates/gpui/src/text_system/line_layout.rs and platform.rs), as its
//! platform text systems speak them.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use anyhow::Result;

use crate::geometry::{Bounds, ContentMask, DevicePixels, Hsla, Pixels, Point, ScaledPixels, Size};
use crate::render::Atlas;
use crate::render::atlas::AtlasKey;
use crate::render::scene::{MonochromeSprite, PolychromeSprite, Scene, TransformationMatrix};

#[cfg(any(target_os = "linux", target_os = "android"))]
mod colr;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod cosmic_text_system;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub use cosmic_text_system::CosmicTextSystem as PlatformTextSystem;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::DirectWriteTextSystem as PlatformTextSystem;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::CoreTextSystem as PlatformTextSystem;

pub(crate) const SUBPIXEL_VARIANTS_X: u8 = 4;
pub(crate) const SUBPIXEL_VARIANTS_Y: u8 = if cfg!(target_os = "windows") || cfg!(target_os = "linux") || cfg!(target_os = "android") { 1 } else { SUBPIXEL_VARIANTS_X };

/// A face at a weight, as the platform text system loaded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FontId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(pub u32);

/// A laid out line of text, on one line however long.
#[derive(Default, Debug)]
pub struct LineLayout {
    pub font_size: Pixels,
    pub width: Pixels,
    pub ascent: Pixels,
    pub descent: Pixels,
    pub runs: Vec<ShapedRun>,
    /// The length of the line in utf-8 bytes
    pub len: usize,
    /// It runs right to left, by its first strong character (Unicode's bidi algorithm, P2 and P3).
    pub rtl: bool,
}

/// A run of glyphs from one face.
#[derive(Debug, Clone)]
pub struct ShapedRun {
    pub font_id: FontId,
    pub glyphs: Vec<ShapedGlyph>,
}

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
    /// Trial (Vello): its words, for a text system of its own to lay out again.
    pub words: String,
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

/// The platform's text system, with the lines it has shaped kept.
pub struct TextSystem {
    platform: PlatformTextSystem,
    shaped: HashMap<(String, [u32; 5]), Arc<LineLayout>>,
    raster_bounds: HashMap<RenderGlyphParams, Bounds<DevicePixels>>,
}

impl TextSystem {
    pub fn new(platform: PlatformTextSystem) -> TextSystem {
        TextSystem { platform, shaped: HashMap::new(), raster_bounds: HashMap::new() }
    }

    pub fn layout_line(&mut self, text: &str, style: &TextStyle) -> Arc<LineLayout> {
        let key = (text.to_owned(), style.key());
        if let Some(layout) = self.shaped.get(&key) {
            return layout.clone();
        }
        if self.shaped.len() > 4096 {
            self.shaped.clear();
        }
        let layout = Arc::new(self.platform.layout_line(text, style));
        self.shaped.insert(key, layout.clone());
        layout
    }

    /// `text` set in `style`, or in `runs` where it's given them (a text with texts in it), wrapped to `width` (none:
    /// as wide as it is) and cut short with an ellipsis after `max_lines`. A line ends at a newline, or where Unicode's
    /// line breaking (UAX #14) allows before it'd be wider than `width`; a word wider than `width` alone overflows it.
    /// Its lines are as tall as its tallest run's, and each run is shaped on its own where its type style changes, as
    /// a browser shapes an inline's text apart from what's beside it once its font differs.
    pub fn paragraph(&mut self, text: &str, style: &TextStyle, runs: &[TextRun], width: Option<f32>, max_lines: Option<usize>) -> Paragraph {
        let fonts = fonts(text, style, runs);
        // A run's line is as tall for its size as the text's is for its own, as an inline inherits CSS's unitless
        // `line-height`: a larger run makes its lines taller.
        let line_height = fonts.iter().map(|(_, run)| run.size * style.line_height / style.size).fold(style.line_height, f32::max);
        let mut lines = Vec::new();
        let mut offset = 0;
        for hard in text.split('\n') {
            let shaped = self.shape(text, offset..offset + hard.len(), &fonts);
            if shaped.glyphs.iter().any(|(_, _, glyph)| glyph.rtl) {
                // Text that runs right to left in it is broken by cosmic-text, which reorders each line once it's
                // broken, as Unicode's bidi algorithm says; ours breaks it as it's drawn, which is only right where
                // it runs one way. It's set in the type style it starts in.
                let style = style_at(&fonts, offset).unwrap_or(*style);
                let mut start = 0;
                for line in self.platform.layout_lines(hard, &style, width) {
                    let mut end = Vec::new();
                    wrap(&Shaped::of(&line), hard, offset, None, &mut end);
                    lines.extend(end.into_iter().map(|drawn| Line { start: start + offset, end: line.len + offset, ..drawn }));
                    start = line.len;
                }
            } else {
                wrap(&shaped, hard, offset, width, &mut lines);
            }
            offset += hard.len() + 1;
        }
        if let Some(max) = max_lines.filter(|max| *max > 0 && lines.len() > *max) {
            lines.truncate(max);
            let last = lines.pop().expect("max > 0");
            let end = text[last.start..].find('\n').map_or(text.len(), |end| last.start + end);
            let tail = style_at(&fonts, end.saturating_sub(1).max(last.start)).unwrap_or(*style);
            let ellipsis = self.layout_line("…", &tail);
            let room = width.unwrap_or(f32::INFINITY) - ellipsis.width.0;
            let full = self.shape(text, last.start..end, &fonts);
            let cut = full
                .glyphs
                .iter()
                .zip(full.glyphs.iter().skip(1).map(|(_, _, next)| next.position.x.0).chain([full.width]))
                .find(|(_, next)| *next > room)
                .map_or(end - last.start, |((_, _, glyph), _)| glyph.index);
            let kept = text[last.start..last.start + cut].trim_end().len();
            let mut shaped = self.shape(text, last.start..last.start + kept, &fonts);
            let tail = style_at(&fonts, (last.start + kept).saturating_sub(1).max(last.start)).unwrap_or(*style);
            shaped.append(&self.layout_line("…", &tail), tail.size, kept);
            let cut_text = format!("{}…", &text[last.start..last.start + kept]);
            let mut end = Vec::new();
            wrap(&shaped, &cut_text, last.start, None, &mut end);
            lines.extend(end);
        }
        Paragraph { lines, line_height, words: text.to_owned() }
    }

    /// Bytes `range` of `text` shaped as one line, each of `fonts` that's in it apart, one after another; its glyphs'
    /// indices are from `range`'s start.
    fn shape(&mut self, text: &str, range: Range<usize>, fonts: &[(Range<usize>, TextStyle)]) -> Shaped {
        let mut shaped = Shaped::default();
        for (run, style) in fonts {
            let (start, end) = (run.start.max(range.start), run.end.min(range.end));
            if start < end {
                let layout = self.layout_line(&text[start..end], style);
                shaped.append(&layout, style.size, start - range.start);
            }
        }
        shaped
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
            y: (glyph_origin.y.0.fract() * SUBPIXEL_VARIANTS_Y as f32).floor() as u8,
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
                let bounds = self.platform.raster_bounds(&params)?;
                self.raster_bounds.insert(params.clone(), bounds);
                bounds
            }
        };
        if raster_bounds.is_zero() {
            return Ok(());
        }
        let platform = &mut self.platform;
        let tile = atlas
            .get_or_insert_with(&AtlasKey::Glyph(params.clone()), &mut || {
                let (size, bytes) = platform.rasterize_glyph(&params, raster_bounds)?;
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

/// A hard line shaped, perhaps a run at a time: its glyphs, each with its face and size, placed one after another.
#[derive(Default)]
struct Shaped {
    glyphs: Vec<(FontId, f32, ShapedGlyph)>,
    width: f32,
    ascent: f32,
    descent: f32,
    rtl: bool,
}

impl Shaped {
    fn of(layout: &LineLayout) -> Shaped {
        let mut shaped = Shaped::default();
        shaped.append(layout, layout.font_size.0, 0);
        shaped
    }

    /// `layout`, set at `size`, after what's here, its glyphs' indices moved on by `from`.
    fn append(&mut self, layout: &LineLayout, size: f32, from: usize) {
        if self.glyphs.is_empty() {
            self.rtl = layout.rtl;
        }
        let x = self.width;
        for run in &layout.runs {
            self.glyphs.extend(run.glyphs.iter().map(|glyph| {
                let position = Point { x: Pixels(glyph.position.x.0 + x), ..glyph.position };
                (run.font_id, size, ShapedGlyph { position, index: glyph.index + from, end: glyph.end + from, ..glyph.clone() })
            }));
        }
        self.width += layout.width.0;
        self.ascent = self.ascent.max(layout.ascent.0);
        self.descent = self.descent.max(layout.descent.0);
    }
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

/// Breaks one hard line's layout into lines no wider than `width`, appending them to `lines`.
fn wrap(layout: &Shaped, text: &str, offset: usize, width: Option<f32>, lines: &mut Vec<Line>) {
    let glyphs = &layout.glyphs;
    let line = |glyphs: &[(FontId, f32, ShapedGlyph)], start: usize, end: usize, next_x: f32| {
        let left = glyphs.first().map_or(0.0, |(_, _, glyph)| glyph.position.x.0);
        // A line's width leaves out the spaces it ends with, as CSS hangs them.
        let ink_end = glyphs
            .iter()
            .rposition(|(_, _, glyph)| !text[glyph.index..].starts_with(char::is_whitespace))
            .map_or(left, |at| glyphs.get(at + 1).map_or(next_x, |(_, _, next)| next.position.x.0));
        Line {
            glyphs: glyphs
                .iter()
                .map(|(font, size, glyph)| (*font, *size, ShapedGlyph { position: Point { x: Pixels(glyph.position.x.0 - left), ..glyph.position }, index: glyph.index + offset, end: glyph.end + offset, ..glyph.clone() }))
                .collect(),
            width: (ink_end - left).max(0.0),
            ascent: layout.ascent,
            descent: layout.descent,
            start: start + offset,
            end: end + offset,
            rtl: layout.rtl,
        }
    };
    let Some(width) = width else {
        lines.push(line(glyphs, 0, text.len(), layout.width));
        return;
    };
    // Where a line may begin: after a break opportunity, at a glyph (shaping can join characters across one).
    let breaks: Vec<usize> = unicode_linebreak::linebreaks(text).map(|(at, _)| at).filter(|at| *at < text.len()).collect();
    let x_of = |at: usize| glyphs.get(at).map_or(layout.width, |(_, _, glyph)| glyph.position.x.0);
    let mut start = 0;
    let mut at = 1;
    while at <= glyphs.len() {
        let end_x = x_of(at);
        let ink = !text[glyphs[at - 1].2.index..].starts_with(char::is_whitespace);
        if ink && end_x - x_of(start) > width && at - 1 > start {
            // Back to the last break opportunity on this line. A word alone wider than the line runs on past it,
            // as CSS's `overflow-wrap: normal` has it, and breaks at the next opportunity.
            let Some(cut) = (start + 1..at).rev().find(|&cut| breaks.binary_search(&glyphs[cut].2.index).is_ok()) else {
                at += 1;
                continue;
            };
            let (begin, end) = (glyphs[start].2.index, glyphs[cut].2.index);
            lines.push(line(&glyphs[start..cut], begin, end, x_of(cut)));
            start = cut;
            at = cut + 1;
            continue;
        }
        at += 1;
    }
    let begin = glyphs.get(start).map_or(0, |(_, _, glyph)| glyph.index);
    lines.push(line(&glyphs[start..], begin, text.len(), layout.width));
}
