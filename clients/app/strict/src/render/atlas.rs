// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform.rs and platform/blade/blade_atlas.rs)
// at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: what
// every atlas of glyphs and images keys and hands out (Blade's, `blade_atlas`, and Direct3D 11's, `directx_atlas`).
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0 (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use crate::geometry::{Bounds, DevicePixels, Point, Size};
use crate::text::RenderGlyphParams;
use std::ops;

/// What a tile in the atlas is of: a glyph, or an image by its bytes' hash.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AtlasKey {
    Glyph(RenderGlyphParams),
    Image(u64),
}

impl AtlasKey {
    pub(crate) fn texture_kind(&self) -> AtlasTextureKind {
        match self {
            AtlasKey::Glyph(params) if params.is_emoji => AtlasTextureKind::Polychrome,
            AtlasKey::Glyph(_) => AtlasTextureKind::Monochrome,
            AtlasKey::Image(_) => AtlasTextureKind::Polychrome,
        }
    }
}

pub(crate) struct AtlasTextureList<T> {
    pub(crate) textures: Vec<Option<T>>,
    pub(crate) free_list: Vec<usize>,
}

impl<T> Default for AtlasTextureList<T> {
    fn default() -> Self {
        Self { textures: Vec::default(), free_list: Vec::default() }
    }
}

impl<T> ops::Index<usize> for AtlasTextureList<T> {
    type Output = Option<T>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.textures[index]
    }
}

impl<T> AtlasTextureList<T> {
    #[allow(dead_code)]
    pub(crate) fn drain(&mut self) -> std::vec::Drain<'_, Option<T>> {
        self.free_list.clear();
        self.textures.drain(..)
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        self.textures.iter_mut().flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct AtlasTile {
    pub(crate) texture_id: AtlasTextureId,
    pub(crate) tile_id: TileId,
    pub(crate) padding: u32,
    pub(crate) bounds: Bounds<DevicePixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct AtlasTextureId {
    // We use u32 instead of usize for Metal Shader Language compatibility
    pub(crate) index: u32,
    pub(crate) kind: AtlasTextureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) enum AtlasTextureKind {
    Monochrome = 0,
    Polychrome = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub(crate) struct TileId(pub(crate) u32);

impl From<etagere::AllocId> for TileId {
    fn from(id: etagere::AllocId) -> Self {
        Self(id.serialize())
    }
}

impl From<Size<DevicePixels>> for etagere::Size {
    fn from(size: Size<DevicePixels>) -> Self {
        etagere::Size::new(size.width.into(), size.height.into())
    }
}

impl From<etagere::Point> for Point<DevicePixels> {
    fn from(value: etagere::Point) -> Self {
        Point {
            x: DevicePixels::from(value.x),
            y: DevicePixels::from(value.y),
        }
    }
}

impl From<etagere::Size> for Size<DevicePixels> {
    fn from(size: etagere::Size) -> Self {
        Size {
            width: DevicePixels::from(size.width),
            height: DevicePixels::from(size.height),
        }
    }
}

impl From<etagere::Rectangle> for Bounds<DevicePixels> {
    fn from(rectangle: etagere::Rectangle) -> Self {
        Bounds {
            origin: rectangle.min.into(),
            size: rectangle.size().into(),
        }
    }
}
