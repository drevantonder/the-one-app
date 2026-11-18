//! Text: a platform's text system shapes a line and rasterizes its glyphs (`linux`, cosmic-text), and this module,
//! the same everywhere, wraps the line to a width, clamps it with an ellipsis, measures it for layout and paints its
//! glyphs from the atlas. The types are GPUI's (crates/gpui/src/text_system/line_layout.rs and platform.rs), as its
//! platform text systems speak them.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use crate::geometry::{Bounds, ContentMask, DevicePixels, Hsla, Pixels, Point, ScaledPixels, Size};
use crate::render::atlas::{AtlasKey, BladeAtlas};
use crate::render::scene::{MonochromeSprite, PolychromeSprite, Scene, TransformationMatrix};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::CosmicTextSystem as PlatformTextSystem;

pub(crate) const SUBPIXEL_VARIANTS_X: u8 = 4;
pub(crate) const SUBPIXEL_VARIANTS_Y: u8 = if cfg!(target_os = "windows") || cfg!(target_os = "linux") { 1 } else { SUBPIXEL_VARIANTS_X };

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

/// One line of a paragraph as it's drawn: its glyphs, from its own left edge.
#[derive(Debug, Default)]
pub struct Line {
    pub glyphs: Vec<(FontId, ShapedGlyph)>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub font_size: f32,
    /// Where it is in the paragraph's text, in bytes.
    pub start: usize,
    pub end: usize,
}

/// Text wrapped to a width, line by line.
#[derive(Debug, Default)]
pub struct Paragraph {
    pub lines: Vec<Line>,
    pub line_height: f32,
}

impl Paragraph {
    pub fn size(&self) -> Size<f32> {
        Size { width: self.lines.iter().map(|line| line.width).fold(0.0, f32::max), height: self.lines.len().max(1) as f32 * self.line_height }
    }

    /// Where the caret goes before byte `index`: its line, and how far along it.
    pub fn caret(&self, index: usize) -> (usize, f32) {
        for (at, line) in self.lines.iter().enumerate() {
            if index > line.end && at + 1 < self.lines.len() {
                continue;
            }
            let x = line
                .glyphs
                .iter()
                .find(|(_, glyph)| glyph.index >= index)
                .map_or(line.width, |(_, glyph)| glyph.position.x.0);
            return (at, x);
        }
        (0, 0.0)
    }

    /// The byte nearest to a point inside the paragraph.
    pub fn index_at(&self, point: Point<f32>) -> usize {
        let at = ((point.y / self.line_height).floor().max(0.0) as usize).min(self.lines.len().saturating_sub(1));
        let Some(line) = self.lines.get(at) else { return 0 };
        let mut index = line.end;
        for (at, (_, glyph)) in line.glyphs.iter().enumerate() {
            let next = line.glyphs.get(at + 1).map_or(line.width, |(_, next)| next.position.x.0);
            if point.x < (glyph.position.x.0 + next) / 2.0 {
                index = glyph.index;
                break;
            }
        }
        index
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

    /// `text` set in `style`, wrapped to `width` (none: as wide as it is) and cut short with an ellipsis after
    /// `max_lines`. A line ends at a newline, or where Unicode's line breaking (UAX #14) allows before it'd be wider
    /// than `width`; a word wider than `width` alone overflows it.
    pub fn paragraph(&mut self, text: &str, style: &TextStyle, width: Option<f32>, max_lines: Option<usize>) -> Paragraph {
        let mut lines = Vec::new();
        let mut offset = 0;
        for hard in text.split('\n') {
            let layout = self.layout_line(hard, style);
            wrap(&layout, hard, offset, width, &mut lines);
            offset += hard.len() + 1;
        }
        if let Some(max) = max_lines.filter(|max| *max > 0 && lines.len() > *max) {
            lines.truncate(max);
            let last = lines.pop().expect("max > 0");
            let rest = &text[last.start..text[last.start..].find('\n').map_or(text.len(), |end| last.start + end)];
            let ellipsis = self.layout_line("…", style).width.0;
            let room = width.unwrap_or(f32::INFINITY) - ellipsis;
            let full = self.layout_line(rest, style);
            let cut = full
                .runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .zip(full.runs.iter().flat_map(|run| &run.glyphs).skip(1).map(|next| next.position.x.0).chain([full.width.0]))
                .find(|(_, next)| *next > room)
                .map_or(rest.len(), |(glyph, _)| glyph.index);
            let cut_text = format!("{}…", rest[..cut].trim_end());
            let layout = self.layout_line(&cut_text, style);
            let mut end = Vec::new();
            wrap(&layout, &cut_text, last.start, None, &mut end);
            lines.extend(end);
        }
        Paragraph { lines, line_height: style.line_height }
    }

    /// Paints `paragraph` with its top left at `origin`, each line placed along `width` by `align` (0 start, 0.5
    /// centre, 1 end), as the shaders draw glyphs: from the atlas, each at the nearest of a few positions between
    /// device pixels, rasterized at the window's scale.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        paragraph: &Paragraph,
        origin: Point<Pixels>,
        width: f32,
        align: f32,
        color: Hsla,
        scale_factor: f32,
        content_mask: ContentMask<ScaledPixels>,
        atlas: &BladeAtlas,
        scene: &mut Scene,
    ) -> Result<()> {
        for (at, line) in paragraph.lines.iter().enumerate() {
            // The line's baseline, with half its leading above, as CSS sets a line box.
            let leading = paragraph.line_height - (line.ascent + line.descent);
            let baseline = origin.y.0 + at as f32 * paragraph.line_height + leading / 2.0 + line.ascent;
            let left = origin.x.0 + (width - line.width).max(0.0) * align;
            for (font_id, glyph) in &line.glyphs {
                let point = Point { x: Pixels(left + glyph.position.x.0), y: Pixels(baseline + glyph.position.y.0) };
                self.paint_glyph(point, *font_id, line.font_size, glyph, color, scale_factor, content_mask, atlas, scene)?;
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
        atlas: &BladeAtlas,
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
            });
        } else {
            scene.insert_primitive(MonochromeSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color,
                tile,
                transformation: TransformationMatrix::unit(),
            });
        }
        Ok(())
    }
}

/// Breaks one hard line's layout into lines no wider than `width`, appending them to `lines`.
fn wrap(layout: &LineLayout, text: &str, offset: usize, width: Option<f32>, lines: &mut Vec<Line>) {
    let glyphs: Vec<(FontId, ShapedGlyph)> =
        layout.runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| (run.font_id, glyph.clone()))).collect();
    let line = |glyphs: &[(FontId, ShapedGlyph)], start: usize, end: usize, next_x: f32| {
        let left = glyphs.first().map_or(0.0, |(_, glyph)| glyph.position.x.0);
        // A line's width leaves out the spaces it ends with, as CSS hangs them.
        let ink_end = glyphs
            .iter()
            .rposition(|(_, glyph)| !text[glyph.index..].starts_with(char::is_whitespace))
            .map_or(left, |at| glyphs.get(at + 1).map_or(next_x, |(_, next)| next.position.x.0));
        Line {
            glyphs: glyphs
                .iter()
                .map(|(font, glyph)| (*font, ShapedGlyph { position: Point { x: Pixels(glyph.position.x.0 - left), ..glyph.position }, index: glyph.index + offset, ..glyph.clone() }))
                .collect(),
            width: (ink_end - left).max(0.0),
            ascent: layout.ascent.0,
            descent: layout.descent.0,
            font_size: layout.font_size.0,
            start: start + offset,
            end: end + offset,
        }
    };
    let Some(width) = width else {
        lines.push(line(&glyphs, 0, text.len(), layout.width.0));
        return;
    };
    // Where a line may begin: after a break opportunity, at a glyph (shaping can join characters across one).
    let breaks: Vec<usize> = unicode_linebreak::linebreaks(text).map(|(at, _)| at).filter(|at| *at < text.len()).collect();
    let x_of = |at: usize| glyphs.get(at).map_or(layout.width.0, |(_, glyph)| glyph.position.x.0);
    let mut start = 0;
    let mut at = 1;
    while at <= glyphs.len() {
        let end_x = x_of(at);
        let ink = !text[glyphs[at - 1].1.index..].starts_with(char::is_whitespace);
        if ink && end_x - x_of(start) > width && at - 1 > start {
            // Back to the last break opportunity on this line. A word alone wider than the line runs on past it,
            // as CSS's `overflow-wrap: normal` has it, and breaks at the next opportunity.
            let Some(cut) = (start + 1..at).rev().find(|&cut| breaks.binary_search(&glyphs[cut].1.index).is_ok()) else {
                at += 1;
                continue;
            };
            let (begin, end) = (glyphs[start].1.index, glyphs[cut].1.index);
            lines.push(line(&glyphs[start..cut], begin, end, x_of(cut)));
            start = cut;
            at = cut + 1;
            continue;
        }
        at += 1;
    }
    let begin = glyphs.get(start).map_or(0, |(_, glyph)| glyph.index);
    lines.push(line(&glyphs[start..], begin, text.len(), layout.width.0));
}
