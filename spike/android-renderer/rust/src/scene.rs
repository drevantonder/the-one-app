// From Zed's GPUI (Apache-2.0, LICENSE-APACHE beside this file): the primitives of crates/gpui/src/scene.rs, with the
// geometry and colour types they're made of (geometry.rs, color.rs, window.rs's ContentMask, platform.rs's AtlasTile),
// at zed-industries/zed 8249ef56187b966c33f6667d0d3a35d88d8f2dc0.
// Changed: only the primitives a strict screen draws (quads, monochrome and polychrome sprites, underlines), as plain
// `#[repr(C)]` structs in device pixels with GPUI's field order and sizes, so shaders.wgsl reads them unchanged; and the
// scene is batched in paint order, one batch per run of one kind and texture, where GPUI sorts by draw order and layer.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Pod, Zeroable)]
pub struct Bounds {
    pub origin: [f32; 2],
    pub size: [f32; 2],
}

impl Bounds {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { origin: [x, y], size: [w, h] }
    }

    pub fn intersect(&self, other: &Bounds) -> Bounds {
        let x0 = self.origin[0].max(other.origin[0]);
        let y0 = self.origin[1].max(other.origin[1]);
        let x1 = (self.origin[0] + self.size[0]).min(other.origin[0] + other.size[0]);
        let y1 = (self.origin[1] + self.size[1]).min(other.origin[1] + other.size[1]);
        Bounds::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.origin[0] && y >= self.origin[1] && x < self.origin[0] + self.size[0] && y < self.origin[1] + self.size[1]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod, Zeroable)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub fn all(r: f32) -> Corners {
        Corners { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod, Zeroable)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Pod, Zeroable)]
pub struct Hsla {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

impl Hsla {
    /// GPUI's `From<Rgba> for Hsla` (color.rs).
    pub fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Hsla {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = (max + min) / 2.0;
        let s = if l == 0.0 || l == 1.0 { 0.0 } else if l < 0.5 { delta / (2.0 * l) } else { delta / (2.0 - 2.0 * l) };
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        Hsla { h, s, l, a }
    }

    pub fn alpha(self, a: f32) -> Hsla {
        Hsla { a: self.a * a, ..self }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod, Zeroable)]
pub struct LinearColorStop {
    pub color: Hsla,
    pub percentage: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod, Zeroable)]
pub struct Background {
    pub tag: u32,
    pub color_space: u32,
    pub solid: Hsla,
    pub gradient_angle_or_pattern_height: f32,
    pub colors: [LinearColorStop; 2],
    pub pad: u32,
}

impl Background {
    pub fn solid(color: Hsla) -> Background {
        Background { solid: color, ..Default::default() }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct AtlasTextureId {
    pub index: u32,
    /// 0 monochrome, 2 polychrome, as GPUI's AtlasTextureKind.
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod, Zeroable)]
pub struct AtlasTile {
    pub texture_id: AtlasTextureId,
    pub tile_id: u32,
    pub padding: u32,
    pub origin: [i32; 2],
    pub size: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TransformationMatrix {
    pub rotation_scale: [[f32; 2]; 2],
    pub translation: [f32; 2],
}

impl TransformationMatrix {
    pub const UNIT: TransformationMatrix = TransformationMatrix { rotation_scale: [[1.0, 0.0], [0.0, 1.0]], translation: [0.0, 0.0] };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Quad {
    pub order: u32,
    pub border_style: u32,
    pub bounds: Bounds,
    pub content_mask: Bounds,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners,
    pub border_widths: Edges,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Underline {
    pub order: u32,
    pub pad: u32,
    pub bounds: Bounds,
    pub content_mask: Bounds,
    pub color: Hsla,
    pub thickness: f32,
    pub wavy: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MonochromeSprite {
    pub order: u32,
    pub pad: u32,
    pub bounds: Bounds,
    pub content_mask: Bounds,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PolychromeSprite {
    pub order: u32,
    pub pad: u32,
    pub grayscale: u32,
    pub opacity: f32,
    pub bounds: Bounds,
    pub content_mask: Bounds,
    pub corner_radii: Corners,
    pub tile: AtlasTile,
}

/// A run of primitives of one kind (and one atlas texture), drawn with one instanced draw.
pub enum Batch {
    Quads(std::ops::Range<usize>),
    Underlines(std::ops::Range<usize>),
    MonochromeSprites { texture_id: AtlasTextureId, range: std::ops::Range<usize> },
    PolychromeSprites { texture_id: AtlasTextureId, range: std::ops::Range<usize> },
}

#[derive(Default)]
pub struct Scene {
    /// Cleared to black, else white: the design's background token is pure white in light and near-black in dark.
    pub dark: bool,
    pub quads: Vec<Quad>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub batches: Vec<Batch>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.polychrome_sprites.clear();
        self.batches.clear();
    }

    pub fn quad(&mut self, quad: Quad) {
        let at = self.quads.len();
        self.quads.push(quad);
        match self.batches.last_mut() {
            Some(Batch::Quads(range)) => range.end = at + 1,
            _ => self.batches.push(Batch::Quads(at..at + 1)),
        }
    }

    pub fn underline(&mut self, underline: Underline) {
        let at = self.underlines.len();
        self.underlines.push(underline);
        match self.batches.last_mut() {
            Some(Batch::Underlines(range)) => range.end = at + 1,
            _ => self.batches.push(Batch::Underlines(at..at + 1)),
        }
    }

    pub fn monochrome(&mut self, sprite: MonochromeSprite) {
        let at = self.monochrome_sprites.len();
        let texture = sprite.tile.texture_id;
        self.monochrome_sprites.push(sprite);
        match self.batches.last_mut() {
            Some(Batch::MonochromeSprites { texture_id, range }) if *texture_id == texture => range.end = at + 1,
            _ => self.batches.push(Batch::MonochromeSprites { texture_id: texture, range: at..at + 1 }),
        }
    }

    pub fn polychrome(&mut self, sprite: PolychromeSprite) {
        let at = self.polychrome_sprites.len();
        let texture = sprite.tile.texture_id;
        self.polychrome_sprites.push(sprite);
        match self.batches.last_mut() {
            Some(Batch::PolychromeSprites { texture_id, range }) if *texture_id == texture => range.end = at + 1,
            _ => self.batches.push(Batch::PolychromeSprites { texture_id: texture, range: at..at + 1 }),
        }
    }
}
