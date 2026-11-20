// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/scene.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the native host: the scene a
// frame draws, of quads, shadows, underlines and sprites, without paths, surfaces or layers, and with backdrops, what's
// behind a node blurred, inset shadows, and a transformation on quads, shadows and images as on text, which are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

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
    pub(crate) rasters: Vec<Raster>,
    /// What each of its rasters is of, by its id: a layer's scene, drawn once into a texture of its own by the
    /// renderer, from `origin` (device pixels) and `size` big.
    pub(crate) recordings: Vec<Recording>,
}

/// A layer's scene, as it was drawn (`view::Layer`), which a raster is an image of.
#[derive(Clone)]
pub(crate) struct Recording {
    pub id: u32,
    pub scene: std::sync::Arc<Scene>,
    pub origin: [f32; 2],
    pub size: [u32; 2],
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
        self.rasters.clear();
        self.recordings.clear();
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
            Primitive::Raster(raster) => {
                raster.order = order;
                self.rasters.push(raster.clone());
            }
        }
    }

    /// Draws raster `id`'s image (`Renderer::picture`) over `bounds`, `opacity` as opaque, over what's drawn there.
    pub fn picture(&mut self, id: u32, bounds: Bounds<ScaledPixels>, opacity: f32) {
        let transformation = TransformationMatrix::unit();
        self.insert_primitive(Raster { order: 0, raster: id, opacity, pad: 0, bounds, content_mask: ContentMask { bounds }, transformation });
    }

    /// Draws `recording` in this scene as an image of it, drawn once, grown `scale` times and moved `by` (`p ↦
    /// p·scale + by`), `opacity` as opaque, and cut to `mask`, as a compositor draws a layer: as `splice` draws it,
    /// and as cheaply as one picture however much is in it.
    pub(crate) fn raster(&mut self, recording: &Recording, scale: f32, by: [f32; 2], opacity: f32, mask: Bounds<ScaledPixels>) {
        let px = |length: f32| ScaledPixels(length * scale);
        let origin = crate::geometry::Point { x: ScaledPixels(recording.origin[0] * scale + by[0]), y: ScaledPixels(recording.origin[1] * scale + by[1]) };
        let bounds = Bounds::new(origin, crate::geometry::Size { width: px(recording.size[0] as f32), height: px(recording.size[1] as f32) });
        // Unseen, it isn't drawn, but its image is still made, ahead of its showing.
        if opacity > 0.0 {
            self.insert_primitive(Raster {
                order: 0,
                raster: recording.id,
                opacity,
                pad: 0,
                bounds,
                content_mask: ContentMask { bounds: mask },
                transformation: TransformationMatrix::unit(),
            });
        }
        // What's drawn in it as images is drawn into their textures before it.
        for inner in recording.scene.recordings.iter().chain([recording]) {
            if !self.recordings.iter().any(|kept| kept.id == inner.id) {
                self.recordings.push(inner.clone());
            }
        }
    }

    /// Where everything it draws is, in device pixels, a shadow's blur and all: what a raster of it covers.
    pub(crate) fn extent(&self) -> Option<Bounds<ScaledPixels>> {
        let grown = |shadow: &Shadow| {
            let (margin, bounds) = (3.0 * shadow.blur_radius.0, shadow.bounds);
            Bounds::new(
                crate::geometry::Point { x: ScaledPixels(bounds.origin.x.0 - margin), y: ScaledPixels(bounds.origin.y.0 - margin) },
                crate::geometry::Size { width: ScaledPixels(bounds.size.width.0 + 2.0 * margin), height: ScaledPixels(bounds.size.height.0 + 2.0 * margin) },
            )
        };
        let all = self
            .shadows
            .iter()
            .map(grown)
            .chain(self.quads.iter().map(|one| one.bounds))
            .chain(self.underlines.iter().map(|one| one.bounds))
            .chain(self.monochrome_sprites.iter().map(|one| one.bounds))
            .chain(self.polychrome_sprites.iter().map(|one| one.bounds))
            .chain(self.rasters.iter().map(|one| one.bounds));
        all.reduce(|a, b| a.union(&b))
    }

    /// Draws what `other` holds in this scene, in the order it was drawn there, grown `scale` times and moved `by`
    /// (`p ↦ p·scale + by`), `opacity` as opaque, and cut to `mask`: as a frame's page is drawn in its box, and as a
    /// layer kept from an earlier frame (`view::Layer`) is drawn where it's moved, grown or faded to now.
    pub fn splice(&mut self, other: &Scene, scale: f32, by: [f32; 2], opacity: f32, mask: Bounds<ScaledPixels>) {
        let px = |length: ScaledPixels| ScaledPixels(length.0 * scale);
        let moved = |bounds: &Bounds<ScaledPixels>| {
            let origin = crate::geometry::Point { x: ScaledPixels(bounds.origin.x.0 * scale + by[0]), y: ScaledPixels(bounds.origin.y.0 * scale + by[1]) };
            Bounds::new(origin, crate::geometry::Size { width: px(bounds.size.width), height: px(bounds.size.height) })
        };
        let masked = |content: &ContentMask<ScaledPixels>| ContentMask { bounds: moved(&content.bounds).intersect(&mask) };
        let corners = |radii: &Corners<ScaledPixels>| Corners { top_left: px(radii.top_left), top_right: px(radii.top_right), bottom_right: px(radii.bottom_right), bottom_left: px(radii.bottom_left) };
        let faded = |colour: Hsla| Hsla { a: colour.a * opacity, ..colour };
        // What was turned about a point there is turned about where that point is here: p ↦ A·p + (scale·t + by − A·by).
        let turned = |matrix: &TransformationMatrix| {
            let [[a, b], [c, d]] = matrix.rotation_scale;
            let [x, y] = matrix.translation;
            TransformationMatrix { rotation_scale: matrix.rotation_scale, translation: [x * scale + by[0] - (a * by[0] + b * by[1]), y * scale + by[1] - (c * by[0] + d * by[1])] }
        };
        let mut all: Vec<(DrawOrder, Primitive)> = Vec::new();
        all.extend(other.shadows.iter().map(|shadow| {
            let mut shadow = shadow.clone();
            (shadow.bounds, shadow.outer, shadow.content_mask, shadow.transformation) = (moved(&shadow.bounds), moved(&shadow.outer), masked(&shadow.content_mask), turned(&shadow.transformation));
            (shadow.corner_radii, shadow.outer_radii, shadow.blur_radius, shadow.color) = (corners(&shadow.corner_radii), corners(&shadow.outer_radii), px(shadow.blur_radius), faded(shadow.color));
            (shadow.order, shadow.into())
        }));
        all.extend(other.quads.iter().map(|quad| {
            let mut quad = quad.clone();
            (quad.bounds, quad.content_mask, quad.transformation) = (moved(&quad.bounds), masked(&quad.content_mask), turned(&quad.transformation));
            (quad.corner_radii, quad.border_color) = (corners(&quad.corner_radii), faded(quad.border_color));
            let widths = quad.border_widths;
            quad.border_widths = crate::geometry::Edges { top: px(widths.top), right: px(widths.right), bottom: px(widths.bottom), left: px(widths.left) };
            quad.background.solid = faded(quad.background.solid);
            for stop in &mut quad.background.colors {
                stop.color = faded(stop.color);
            }
            (quad.order, quad.into())
        }));
        all.extend(other.underlines.iter().map(|underline| {
            let mut underline = underline.clone();
            (underline.bounds, underline.content_mask, underline.thickness, underline.color) = (moved(&underline.bounds), masked(&underline.content_mask), px(underline.thickness), faded(underline.color));
            (underline.order, underline.into())
        }));
        all.extend(other.monochrome_sprites.iter().map(|sprite| {
            let mut sprite = sprite.clone();
            (sprite.bounds, sprite.content_mask, sprite.transformation, sprite.color) = (moved(&sprite.bounds), masked(&sprite.content_mask), turned(&sprite.transformation), faded(sprite.color));
            (sprite.order, sprite.into())
        }));
        all.extend(other.polychrome_sprites.iter().map(|sprite| {
            let mut sprite = sprite.clone();
            (sprite.bounds, sprite.content_mask, sprite.transformation) = (moved(&sprite.bounds), masked(&sprite.content_mask), turned(&sprite.transformation));
            (sprite.corner_radii, sprite.opacity) = (corners(&sprite.corner_radii), sprite.opacity * opacity);
            (sprite.order, sprite.into())
        }));
        all.extend(other.rasters.iter().map(|raster| {
            let mut raster = raster.clone();
            (raster.bounds, raster.content_mask, raster.opacity) = (moved(&raster.bounds), masked(&raster.content_mask), raster.opacity * opacity);
            (raster.order, raster.into())
        }));
        for recording in &other.recordings {
            if !self.recordings.iter().any(|kept| kept.id == recording.id) {
                self.recordings.push(recording.clone());
            }
        }
        all.extend(other.backdrops.iter().map(|backdrop| {
            let mut backdrop = backdrop.clone();
            (backdrop.bounds, backdrop.content_mask, backdrop.corner_radii, backdrop.blur, backdrop.opacity) = (moved(&backdrop.bounds), masked(&backdrop.content_mask), corners(&backdrop.corner_radii), px(backdrop.blur), backdrop.opacity * opacity);
            (backdrop.order, backdrop.into())
        }));
        all.sort_by_key(|(order, _)| *order);
        for (_, primitive) in all {
            self.insert_primitive(primitive);
        }
    }

    pub fn finish(&mut self) {
        self.occlude();
        self.sort();
    }

    /// Each kind of primitive in the order it was drawn, which its batches are drawn in (`batches`).
    pub(crate) fn sort(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.backdrops.sort_by_key(|backdrop| backdrop.order);
        self.rasters.sort_by_key(|raster| raster.order);
    }

    /// What an opaque quad drawn later covers isn't drawn, as a compositor's occlusion culling leaves it out: a quad
    /// wholly under one goes, and one mostly under it (a window's background under a card) is drawn only in the bands
    /// around it. A GPU's fill rate is what a scrolled page spends, and a long list drew its window and its card both,
    /// whole, every frame. Nothing goes where something blurs what's behind it (a backdrop), which reads what's under.
    fn occlude(&mut self) {
        if !self.backdrops.is_empty() {
            return;
        }
        let px = ScaledPixels;
        // What each opaque quad covers for certain: inside its corners and any border that isn't opaque, cut to its mask.
        let opaque: Vec<(DrawOrder, Bounds<ScaledPixels>)> = self
            .quads
            .iter()
            .filter(|quad| quad.transformation == TransformationMatrix::unit() && quad.background.tag == 0 && quad.background.solid.a >= 1.0)
            .map(|quad| {
                let radii = &quad.corner_radii;
                let corner = radii.top_left.0.max(radii.top_right.0).max(radii.bottom_left.0).max(radii.bottom_right.0);
                let widths = &quad.border_widths;
                let border = if quad.border_color.a >= 1.0 && quad.border_style == BorderStyle::Solid { 0.0 } else { widths.top.0.max(widths.right.0).max(widths.bottom.0).max(widths.left.0) };
                let inset = corner.max(border);
                let Bounds { origin, size } = quad.bounds;
                let inside = Bounds { origin: crate::geometry::Point { x: px(origin.x.0 + inset), y: px(origin.y.0 + inset) }, size: crate::geometry::Size { width: px(size.width.0 - 2.0 * inset), height: px(size.height.0 - 2.0 * inset) } };
                (quad.order, inside.intersect(&quad.content_mask.bounds))
            })
            .filter(|(_, covered)| !covered.is_empty())
            .collect();
        if opaque.is_empty() {
            return;
        }
        let area = |bounds: &Bounds<ScaledPixels>| bounds.size.width.0 * bounds.size.height.0;
        let mut kept = Vec::with_capacity(self.quads.len());
        for quad in std::mem::take(&mut self.quads) {
            let mut pieces = vec![quad.content_mask.bounds.intersect(&quad.bounds)];
            if quad.transformation == TransformationMatrix::unit() {
                for (_, covered) in opaque.iter().filter(|(order, _)| *order > quad.order) {
                    pieces = pieces
                        .into_iter()
                        .flat_map(|shown| {
                            let under = shown.intersect(covered);
                            // Wholly shown, or little of it under: drawn as it is.
                            if under.is_empty() || area(&under) < area(&shown) / 4.0 {
                                return vec![shown];
                            }
                            let (left, top) = (shown.origin, shown.bottom_right());
                            let (low, high) = (under.origin, under.bottom_right());
                            let band = |x0: f32, y0: f32, x1: f32, y1: f32| Bounds::from_corners(crate::geometry::Point { x: px(x0), y: px(y0) }, crate::geometry::Point { x: px(x1), y: px(y1) });
                            [
                                band(left.x.0, left.y.0, top.x.0, low.y.0),
                                band(left.x.0, high.y.0, top.x.0, top.y.0),
                                band(left.x.0, low.y.0, low.x.0, high.y.0),
                                band(high.x.0, low.y.0, top.x.0, high.y.0),
                            ]
                            .into_iter()
                            .filter(|band| !band.is_empty())
                            .collect()
                        })
                        .collect();
                }
            }
            kept.extend(pieces.into_iter().map(|shown| Quad { content_mask: ContentMask { bounds: shown }, ..quad.clone() }));
        }
        self.quads = kept;
    }

    pub(crate) fn batches(&self) -> impl Iterator<Item = PrimitiveBatch<'_>> {
        // Batches are drawn by order across kinds, so a kind out of order draws over what was drawn after it: a
        // layer's image drew a badge's digits under its own fill.
        debug_assert!(
            self.shadows.is_sorted_by_key(|one| one.order)
                && self.quads.is_sorted_by_key(|one| one.order)
                && self.underlines.is_sorted_by_key(|one| one.order)
                && self.monochrome_sprites.is_sorted_by_key(|one| one.order)
                && self.polychrome_sprites.is_sorted_by_key(|one| one.order)
                && self.backdrops.is_sorted_by_key(|one| one.order)
                && self.rasters.is_sorted_by_key(|one| one.order),
            "a scene's primitives are drawn in batches by their order, so each kind is sorted first (`Scene::sort`)"
        );
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
            rasters: &self.rasters,
            rasters_start: 0,
            rasters_iter: self.rasters.iter().peekable(),
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
    Raster,
}

#[derive(Clone)]
pub(crate) enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    PolychromeSprite(PolychromeSprite),
    Backdrop(Backdrop),
    Raster(Raster),
}

impl Primitive {
    /// What it draws over: a shadow's box grown by its blur's reach, three deviations, as its shader draws it, so one
    /// whose box is out of its mask but whose blur isn't is still drawn, and ordered with what's over its blur.
    pub fn bounds(&self) -> Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => shadow.reach(),
            Primitive::Quad(quad) => quad.bounds,
            Primitive::Underline(underline) => underline.bounds,
            Primitive::MonochromeSprite(sprite) => sprite.bounds,
            Primitive::PolychromeSprite(sprite) => sprite.bounds,
            Primitive::Backdrop(backdrop) => backdrop.bounds,
            Primitive::Raster(raster) => raster.bounds,
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
            Primitive::Raster(raster) => &raster.content_mask,
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
    rasters: &'a [Raster],
    rasters_start: usize,
    rasters_iter: Peekable<slice::Iter<'a, Raster>>,
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
            (self.rasters_iter.peek().map(|r| r.order), PrimitiveKind::Raster),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max) = if first.0.is_some() { (first.1, (second.0.unwrap_or(u32::MAX), second.1)) } else { return None };

        match batch_kind {
            PrimitiveKind::Shadow => Some(PrimitiveBatch::Shadows(batch!(self, shadows_iter, shadows_start, shadows, batch_kind, max))),
            PrimitiveKind::Quad => Some(PrimitiveBatch::Quads(batch!(self, quads_iter, quads_start, quads, batch_kind, max))),
            PrimitiveKind::Backdrop => Some(PrimitiveBatch::Backdrops(batch!(self, backdrops_iter, backdrops_start, backdrops, batch_kind, max))),
            // Each raster its own batch, as each is its own texture.
            PrimitiveKind::Raster => {
                let start = self.rasters_start;
                self.rasters_iter.next();
                self.rasters_start = start + 1;
                Some(PrimitiveBatch::Rasters(&self.rasters[start..start + 1]))
            }
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

impl PrimitiveBatch<'_> {
    /// Whether anything in it is drawn in `area`, cut to where it's drawn inside.
    pub(crate) fn reaches(&self, area: &Bounds<ScaledPixels>) -> bool {
        fn any<'a>(mut all: impl Iterator<Item = (Bounds<ScaledPixels>, &'a ContentMask<ScaledPixels>)>, area: &Bounds<ScaledPixels>) -> bool {
            all.any(|(bounds, mask)| bounds.intersect(&mask.bounds).intersects(area))
        }
        match self {
            PrimitiveBatch::Shadows(all) => any(all.iter().map(|one| (one.reach(), &one.content_mask)), area),
            PrimitiveBatch::Quads(all) => any(all.iter().map(|one| (one.bounds, &one.content_mask)), area),
            PrimitiveBatch::Underlines(all) => any(all.iter().map(|one| (one.bounds, &one.content_mask)), area),
            PrimitiveBatch::MonochromeSprites { sprites, .. } => any(sprites.iter().map(|one| (one.bounds, &one.content_mask)), area),
            PrimitiveBatch::PolychromeSprites { sprites, .. } => any(sprites.iter().map(|one| (one.bounds, &one.content_mask)), area),
            PrimitiveBatch::Backdrops(all) => any(all.iter().map(|one| (one.bounds, &one.content_mask)), area),
            PrimitiveBatch::Rasters(all) => any(all.iter().map(|one| (one.bounds, &one.content_mask)), area),
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
    Rasters(&'a [Raster]),
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

impl Shadow {
    /// Its box grown by its blur's reach, three deviations, as its shader draws it.
    fn reach(&self) -> Bounds<ScaledPixels> {
        let reach = 3.0 * self.blur_radius.0;
        let Bounds { origin, size } = self.bounds;
        Bounds {
            origin: crate::geometry::Point { x: ScaledPixels(origin.x.0 - reach), y: ScaledPixels(origin.y.0 - reach) },
            size: crate::geometry::Size { width: ScaledPixels(size.width.0 + 2.0 * reach), height: ScaledPixels(size.height.0 + 2.0 * reach) },
        }
    }
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
    /// 1 to draw it grey: a whole `u32`, as a frame's batches are told apart by their bytes (`Drawn`), which a
    /// `bool`'s padding would make differ.
    pub grayscale: u32,
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
    /// How much of the blur shows over what's behind, as what it's on fades.
    pub opacity: f32,
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

/// An image of a layer's scene (`Recording`), drawn once into a texture of its own, drawn `opacity` as opaque in
/// `bounds`, cut to `content_mask`: what a compositor draws a layer as.
#[derive(Clone, Debug)]
#[repr(C)]
pub(crate) struct Raster {
    pub order: DrawOrder,
    pub raster: u32,
    pub opacity: f32,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub transformation: TransformationMatrix,
}

impl From<Raster> for Primitive {
    fn from(raster: Raster) -> Self {
        Primitive::Raster(raster)
    }
}
