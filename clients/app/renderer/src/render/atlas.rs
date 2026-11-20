// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_atlas.rs at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and the keys and tiles of crates/gpui/src/platform.rs at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624), and cut down for the native host: the glyph and image atlas, its tiles
// uploaded by the queue as the next frame starts.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use super::context::Context;
use crate::geometry::{Bounds, DevicePixels, Point, Size};
use crate::text::RenderGlyphParams;
use anyhow::Result;
use etagere::BucketedAtlasAllocator;
use parking_lot::Mutex;
use std::{borrow::Cow, collections::HashMap, ops};

pub struct Atlas(Mutex<AtlasState>);

struct PendingUpload {
    id: AtlasTextureId,
    bounds: Bounds<DevicePixels>,
    data: Vec<u8>,
}

struct AtlasState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_size: i32,
    storage: AtlasStorage,
    tiles_by_key: HashMap<AtlasKey, AtlasTile>,
    uploads: Vec<PendingUpload>,
    /// How many times what's in it has changed: a frame whose sprites are the same bytes as another's, with this the
    /// same too, looks the same.
    generation: u64,
    /// How many textures it has made, whose bind groups a renderer keeps (`Renderer::atlas_group`).
    made: u64,
    /// How many bytes it's written to its textures since it was last asked (`Atlas::uploaded`).
    uploaded: u64,
}

impl Atlas {
    pub fn new(context: &Context) -> Self {
        Atlas(Mutex::new(AtlasState {
            device: context.device.clone(),
            queue: context.queue.clone(),
            max_texture_size: context.device.limits().max_texture_dimension_2d as i32,
            storage: AtlasStorage::default(),
            tiles_by_key: Default::default(),
            uploads: Vec::new(),
            generation: 0,
            made: 0,
            uploaded: 0,
        }))
    }

    /// Writes what's been added since the last frame into its textures, before the frame that draws it.
    pub fn before_frame(&self) {
        self.0.lock().flush();
    }

    /// How many bytes it's written to its textures since this was last asked: the glyphs and pictures a frame
    /// uploaded.
    pub(crate) fn uploaded(&self) -> u64 {
        std::mem::take(&mut self.0.lock().uploaded)
    }

    /// How many textures it has made (`AtlasState::made`).
    pub(crate) fn made(&self) -> u64 {
        self.0.lock().made
    }

    /// The widest and tallest a tile can be: its GPU's largest texture.
    pub(crate) fn largest(&self) -> i32 {
        self.0.lock().max_texture_size
    }

    /// How many times what's in it has changed (`AtlasState::generation`).
    pub(crate) fn generation(&self) -> u64 {
        self.0.lock().generation
    }

    /// What it holds, for the inspector's `profile`: its `tiles` (glyphs and pictures), its `textures`, the `bytes`
    /// they take on the GPU, and how much of them its tiles fill (`filled`, 0 to 1).
    pub fn usage(&self) -> serde_json::Value {
        let lock = self.0.lock();
        let textures: Vec<&AtlasTexture> = lock.storage.monochrome_textures.iter().chain(&lock.storage.polychrome_textures).collect();
        let size = |texture: &AtlasTexture| texture.allocator.size().area() as u64;
        let bytes: u64 = textures.iter().map(|texture| size(texture) * u64::from(texture.bytes_per_pixel)).sum();
        let area: u64 = textures.iter().map(|texture| size(texture)).sum();
        let filled: u64 = textures.iter().map(|texture| texture.allocator.allocated_space() as u64).sum();
        let filled = if area == 0 { 0.0 } else { (filled as f64 / area as f64 * 1000.0).round() / 1000.0 };
        serde_json::json!({"tiles": lock.tiles_by_key.len(), "textures": textures.len(), "bytes": bytes, "filled": filled})
    }

    /// The view of texture `id`, which a batch of sprites samples.
    pub(crate) fn view(&self, id: AtlasTextureId) -> wgpu::TextureView {
        self.0.lock().storage[id].view.clone()
    }

    /// Writes `data`, `size` pixels in its format (a video's frame, BGRA), into `tile`, inside its clear pixel all round,
    /// before the next frame that draws it.
    pub(crate) fn write(&self, tile: &AtlasTile, size: Size<DevicePixels>, data: Vec<u8>) {
        let bounds = crate::geometry::Bounds { origin: crate::geometry::point(tile.bounds.origin.x + DevicePixels(1), tile.bounds.origin.y + DevicePixels(1)), size };
        self.0.lock().uploads.push(PendingUpload { id: tile.texture_id, bounds, data });
    }

    pub(crate) fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut lock = self.0.lock();
        if let Some(tile) = lock.tiles_by_key.get(key) {
            return Ok(Some(tile.clone()));
        }
        let Some((size, bytes)) = build()? else {
            return Ok(None);
        };
        // What's larger than any texture its GPU makes isn't drawn: its maker draws it smaller (`view::pictures`).
        let largest = lock.max_texture_size;
        if size.width.0 > largest || size.height.0 > largest {
            anyhow::bail!("a tile of {} × {} pixels is larger than this GPU's largest texture, {largest}", size.width.0, size.height.0);
        }
        let tile = lock.allocate(size, key.texture_kind());
        lock.uploads.push(PendingUpload { id: tile.texture_id, bounds: tile.bounds, data: bytes.into_owned() });
        lock.tiles_by_key.insert(key.clone(), tile.clone());
        Ok(Some(tile))
    }
}

impl AtlasState {
    fn allocate(&mut self, size: Size<DevicePixels>, kind: AtlasTextureKind) -> AtlasTile {
        if let Some(tile) = self.storage[kind].iter_mut().rev().find_map(|texture| texture.allocate(size)) {
            return tile;
        }
        self.push_texture(size, kind).allocate(size).expect("a new texture is at least as large as the tile")
    }

    fn push_texture(&mut self, min_size: Size<DevicePixels>, kind: AtlasTextureKind) -> &mut AtlasTexture {
        const DEFAULT_ATLAS_SIZE: Size<DevicePixels> = Size { width: DevicePixels(1024), height: DevicePixels(1024) };
        let side = |at_least: DevicePixels, default: DevicePixels| DevicePixels(at_least.0.min(self.max_texture_size).max(default.0));
        let size = Size { width: side(min_size.width, DEFAULT_ATLAS_SIZE.width), height: side(min_size.height, DEFAULT_ATLAS_SIZE.height) };
        // A glyph's coverage, or an image's or an emoji's colours, as the text systems and the image decoder give
        // them: blue, green, red, alpha.
        let format = match kind {
            AtlasTextureKind::Monochrome => wgpu::TextureFormat::R8Unorm,
            AtlasTextureKind::Polychrome => wgpu::TextureFormat::Bgra8Unorm,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size.width.0 as u32, height: size.height.0 as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.made += 1;
        let list = &mut self.storage[kind];
        list.push(AtlasTexture {
            id: AtlasTextureId { index: list.len() as u32, kind },
            allocator: BucketedAtlasAllocator::new(size.into()),
            texture,
            view,
            bytes_per_pixel: format.block_copy_size(None).unwrap_or(4),
        });
        list.last_mut().expect("just pushed")
    }

    fn flush(&mut self) {
        self.generation += u64::from(!self.uploads.is_empty());
        for upload in self.uploads.drain(..) {
            self.uploaded += upload.data.len() as u64;
            let texture = &self.storage[upload.id];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: upload.bounds.origin.x.0 as u32, y: upload.bounds.origin.y.0 as u32, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.bounds.size.width.0 as u32 * texture.bytes_per_pixel),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: upload.bounds.size.width.0 as u32,
                    height: upload.bounds.size.height.0 as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

/// Its textures of each kind, none of which goes while it's kept: a texture's id is where it is in its list.
#[derive(Default)]
struct AtlasStorage {
    monochrome_textures: Vec<AtlasTexture>,
    polychrome_textures: Vec<AtlasTexture>,
}

impl ops::Index<AtlasTextureKind> for AtlasStorage {
    type Output = Vec<AtlasTexture>;
    fn index(&self, kind: AtlasTextureKind) -> &Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
        }
    }
}

impl ops::IndexMut<AtlasTextureKind> for AtlasStorage {
    fn index_mut(&mut self, kind: AtlasTextureKind) -> &mut Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
        }
    }
}

impl ops::Index<AtlasTextureId> for AtlasStorage {
    type Output = AtlasTexture;
    fn index(&self, id: AtlasTextureId) -> &Self::Output {
        &self[id.kind][id.index as usize]
    }
}

struct AtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes_per_pixel: u32,
}

impl AtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(size.into())?;
        Some(AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds { origin: allocation.rectangle.min.into(), size },
        })
    }
}

/// What a tile in the atlas is of: a glyph, or an image by its bytes' hash.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AtlasKey {
    Glyph(RenderGlyphParams),
    Image(u64),
}

impl AtlasKey {
    fn texture_kind(&self) -> AtlasTextureKind {
        match self {
            AtlasKey::Glyph(params) if params.is_emoji => AtlasTextureKind::Polychrome,
            AtlasKey::Glyph(_) => AtlasTextureKind::Monochrome,
            AtlasKey::Image(_) => AtlasTextureKind::Polychrome,
        }
    }
}

/// Where a tile is in the atlas, as the shaders read it (`AtlasTile` in shaders.wgsl).
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
        Point { x: DevicePixels::from(value.x), y: DevicePixels::from(value.y) }
    }
}
