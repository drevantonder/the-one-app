// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/scene.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: the scene a
// frame draws, of quads, shadows, underlines and sprites, without paths, surfaces or layers, and with backdrops, what's
// behind a node blurred, inset shadows, and a transformation on quads, shadows and images as on text, which are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::{iter::Peekable, slice};

use super::atlas::{AtlasTextureId, AtlasTile};
use super::bounds_tree::BoundsTree;
use crate::geometry::{Background, Bounds, ContentMask, Corners, Edges, Hsla, ScaledPixels};

pub(crate) type DrawOrder = u32;

#[derive(Default)]
pub struct Scene {
    primitive_bounds: BoundsTree<ScaledPixels>,
    pub(crate) shadows: Vec<Shadow>,
    pub(crate) quads: Vec<Quad>,
    pub(crate) underlines: Vec<Underline>,
    pub(crate) monochrome_sprites: Vec<MonochromeSprite>,
    pub(crate) polychrome_sprites: Vec<PolychromeSprite>,
    pub(crate) backdrops: Vec<Backdrop>,
}

impl Scene {
    pub fn clear(&mut self) {
        self.primitive_bounds.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.polychrome_sprites.clear();
        self.backdrops.clear();
    }

    pub(crate) fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        let clipped_bounds = primitive.bounds().intersect(&primitive.content_mask().bounds);
        if clipped_bounds.is_empty() {
            return;
        }
        let order = self.primitive_bounds.insert(clipped_bounds);
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(shadow.clone());
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(quad.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(underline.clone());
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(sprite.clone());
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(sprite.clone());
            }
            Primitive::Backdrop(backdrop) => {
                backdrop.order = order;
                self.backdrops.push(backdrop.clone());
            }
        }
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.backdrops.sort_by_key(|backdrop| backdrop.order);
    }

    pub(crate) fn batches(&self) -> impl Iterator<Item = PrimitiveBatch<'_>> {
        BatchIterator {
            shadows: &self.shadows,
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads: &self.quads,
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            underlines: &self.underlines,
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites: &self.monochrome_sprites,
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            polychrome_sprites: &self.polychrome_sprites,
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            backdrops: &self.backdrops,
            backdrops_start: 0,
            backdrops_iter: self.backdrops.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Underline,
    MonochromeSprite,
    PolychromeSprite,
    Backdrop,
}

#[derive(Clone)]
pub(crate) enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    PolychromeSprite(PolychromeSprite),
    Backdrop(Backdrop),
}

impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Backdrop(backdrop) => &backdrop.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Backdrop(backdrop) => &backdrop.content_mask,
        }
    }
}

struct BatchIterator<'a> {
    shadows: &'a [Shadow],
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads: &'a [Quad],
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    underlines: &'a [Underline],
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites: &'a [MonochromeSprite],
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    polychrome_sprites: &'a [PolychromeSprite],
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    backdrops: &'a [Backdrop],
    backdrops_start: usize,
    backdrops_iter: Peekable<slice::Iter<'a, Backdrop>>,
}

/// The run of one kind from `start` while each is drawn before `max`, as GPUI batches them.
macro_rules! batch {
    ($self:ident, $iter:ident, $start:ident, $all:ident, $kind:expr, $max:expr) => {{
        let start = $self.$start;
        let mut end = start + 1;
        $self.$iter.next();
        while $self.$iter.next_if(|primitive| (primitive.order, $kind) < $max).is_some() {
            end += 1;
        }
        $self.$start = end;
        &$self.$all[start..end]
    }};
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (self.shadows_iter.peek().map(|s| s.order), PrimitiveKind::Shadow),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.underlines_iter.peek().map(|u| u.order), PrimitiveKind::Underline),
            (self.monochrome_sprites_iter.peek().map(|s| s.order), PrimitiveKind::MonochromeSprite),
            (self.polychrome_sprites_iter.peek().map(|s| s.order), PrimitiveKind::PolychromeSprite),
            (self.backdrops_iter.peek().map(|b| b.order), PrimitiveKind::Backdrop),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max) = if first.0.is_some() { (first.1, (second.0.unwrap_or(u32::MAX), second.1)) } else { return None };

        match batch_kind {
            PrimitiveKind::Shadow => Some(PrimitiveBatch::Shadows(batch!(self, shadows_iter, shadows_start, shadows, batch_kind, max))),
            PrimitiveKind::Quad => Some(PrimitiveBatch::Quads(batch!(self, quads_iter, quads_start, quads, batch_kind, max))),
            PrimitiveKind::Backdrop => Some(PrimitiveBatch::Backdrops(batch!(self, backdrops_iter, backdrops_start, backdrops, batch_kind, max))),
            PrimitiveKind::Underline => {
                Some(PrimitiveBatch::Underlines(batch!(self, underlines_iter, underlines_start, underlines, batch_kind, max)))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let start = self.monochrome_sprites_start;
                let mut end = start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| (sprite.order, batch_kind) < max && sprite.tile.texture_id == texture_id)
                    .is_some()
                {
                    end += 1;
                }
                self.monochrome_sprites_start = end;
                Some(PrimitiveBatch::MonochromeSprites { texture_id, sprites: &self.monochrome_sprites[start..end] })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let start = self.polychrome_sprites_start;
                let mut end = start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| (sprite.order, batch_kind) < max && sprite.tile.texture_id == texture_id)
                    .is_some()
                {
                    end += 1;
                }
                self.polychrome_sprites_start = end;
                Some(PrimitiveBatch::PolychromeSprites { texture_id, sprites: &self.polychrome_sprites[start..end] })
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum PrimitiveBatch<'a> {
    Shadows(&'a [Shadow]),
    Quads(&'a [Quad]),
    Underlines(&'a [Underline]),
    MonochromeSprites { texture_id: AtlasTextureId, sprites: &'a [MonochromeSprite] },
    PolychromeSprites { texture_id: AtlasTextureId, sprites: &'a [PolychromeSprite] },
    Backdrops(&'a [Backdrop]),
}

#[derive(Default, Debug, Clone)]
#[repr(C)]
pub(crate) struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
    pub transformation: TransformationMatrix,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub(crate) struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: u32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Clone)]
#[repr(C)]
pub(crate) struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub transformation: TransformationMatrix,
    /// Ours: 1 for a shadow inside the box it's of (`outer`, with `outer_radii`), where its own box doesn't cover.
    pub inset: u32,
    pub pad: u32,
    pub outer: Bounds<ScaledPixels>,
    pub outer_radii: Corners<ScaledPixels>,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub enum BorderStyle {
    #[default]
    Solid = 0,
    Dashed = 1,
}

/// A 2D transformation, in device pixels, which the shaders apply to what they draw: a node turned (`rotate`), and
/// what's in it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale, stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    pub translation: [f32; 2],
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

impl TransformationMatrix {
    pub fn unit() -> Self {
        Self { rotation_scale: [[1.0, 0.0], [0.0, 1.0]], translation: [0.0, 0.0] }
    }

    /// Turned `degrees` clockwise, as CSS's `rotate`, around `centre`.
    pub fn rotation(degrees: f32, centre: [f32; 2]) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let [x, y] = centre;
        Self { rotation_scale: [[cos, -sin], [sin, cos]], translation: [x - (cos * x - sin * y), y - (sin * x + cos * y)] }
    }

    /// `then` after this one.
    pub fn then(&self, then: &Self) -> Self {
        let [[a, b], [c, d]] = then.rotation_scale;
        let [[e, f], [g, h]] = self.rotation_scale;
        let [x, y] = self.translation;
        Self {
            rotation_scale: [[a * e + b * g, a * f + b * h], [c * e + d * g, c * f + d * h]],
            translation: [a * x + b * y + then.translation[0], c * x + d * y + then.translation[1]],
        }
    }

    /// Where it takes `point`.
    pub fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [[a, b], [c, d]] = self.rotation_scale;
        [a * x + b * y + self.translation[0], c * x + d * y + self.translation[1]]
    }

    /// The same, for coordinates `factor` times as large: logical pixels to device pixels.
    pub fn scaled(&self, factor: f32) -> Self {
        Self { rotation_scale: self.rotation_scale, translation: [self.translation[0] * factor, self.translation[1] * factor] }
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub(crate) struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub(crate) struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub grayscale: bool,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

/// What's behind a node, blurred `blur` device pixels (the Gaussian's sigma) and saturated so much, inside its
/// corners, drawn under its background (`backdropFilter`).
#[derive(Clone, Debug)]
#[repr(C)]
pub(crate) struct Backdrop {
    pub order: DrawOrder,
    pub pad: u32,
    pub blur: ScaledPixels,
    pub saturation: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
}

impl From<Backdrop> for Primitive {
    fn from(backdrop: Backdrop) -> Self {
        Primitive::Backdrop(backdrop)
    }
}
