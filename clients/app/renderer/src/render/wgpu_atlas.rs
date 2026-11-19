// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_atlas.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host: the glyph and image atlas, its tiles
// uploaded by the queue as the next frame starts.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use super::atlas::{AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTextureList, AtlasTile};
use super::wgpu_context::WgpuContext;
use crate::geometry::{Bounds, DevicePixels, Size};
use anyhow::Result;
use etagere::BucketedAtlasAllocator;
use parking_lot::Mutex;
use std::{borrow::Cow, collections::HashMap, ops};

pub struct WgpuAtlas(Mutex<WgpuAtlasState>);

struct PendingUpload {
    id: AtlasTextureId,
    bounds: Bounds<DevicePixels>,
    data: Vec<u8>,
}

struct WgpuAtlasState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_size: i32,
    storage: WgpuAtlasStorage,
    tiles_by_key: HashMap<AtlasKey, AtlasTile>,
    uploads: Vec<PendingUpload>,
    /// How many times what's in it has changed: a frame whose sprites are the same bytes as another's, with this the
    /// same too, looks the same.
    generation: u64,
    /// How many textures it has made, whose bind groups a renderer keeps (`WgpuRenderer::atlas_group`).
    made: u64,
}

impl WgpuAtlas {
    pub fn new(context: &WgpuContext) -> Self {
        WgpuAtlas(Mutex::new(WgpuAtlasState {
            device: context.device.clone(),
            queue: context.queue.clone(),
            max_texture_size: context.device.limits().max_texture_dimension_2d as i32,
            storage: WgpuAtlasStorage::default(),
            tiles_by_key: Default::default(),
            uploads: Vec::new(),
            generation: 0,
            made: 0,
        }))
    }

    /// Writes what's been added since the last frame into its textures, before the frame that draws it.
    pub fn before_frame(&self) {
        self.0.lock().flush();
    }

    /// How many textures it has made (`WgpuAtlasState::made`).
    pub(crate) fn made(&self) -> u64 {
        self.0.lock().made
    }

    /// The widest and tallest a tile can be: its GPU's largest texture.
    pub(crate) fn largest(&self) -> i32 {
        self.0.lock().max_texture_size
    }

    /// How many times what's in it has changed (`WgpuAtlasState::generation`).
    pub(crate) fn generation(&self) -> u64 {
        self.0.lock().generation
    }

    /// The view of texture `id`, which a batch of sprites samples.
    pub(crate) fn view(&self, id: AtlasTextureId) -> wgpu::TextureView {
        self.0.lock().storage[id].view.clone()
    }

    /// Writes `data`, `size` pixels in its format (a video's frame, BGRA), into `tile`, inside its clear pixel all round,
    /// before the next frame that draws it.
    #[cfg(target_arch = "wasm32")]
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

impl WgpuAtlasState {
    fn allocate(&mut self, size: Size<DevicePixels>, kind: AtlasTextureKind) -> AtlasTile {
        if let Some(tile) = self.storage[kind].iter_mut().rev().find_map(|texture| texture.allocate(size)) {
            return tile;
        }
        self.push_texture(size, kind).allocate(size).expect("a new texture is at least as large as the tile")
    }

    fn push_texture(&mut self, min_size: Size<DevicePixels>, kind: AtlasTextureKind) -> &mut WgpuAtlasTexture {
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
        let index = list.free_list.pop();
        let atlas_texture = WgpuAtlasTexture {
            id: AtlasTextureId { index: index.unwrap_or(list.textures.len()) as u32, kind },
            allocator: BucketedAtlasAllocator::new(size.into()),
            texture,
            view,
            bytes_per_pixel: format.block_copy_size(None).unwrap_or(4),
        };
        let slot = match index {
            Some(index) => {
                list.textures[index] = Some(atlas_texture);
                &mut list.textures[index]
            }
            None => {
                list.textures.push(Some(atlas_texture));
                list.textures.last_mut().expect("just pushed")
            }
        };
        slot.as_mut().expect("just filled")
    }

    fn flush(&mut self) {
        self.generation += u64::from(!self.uploads.is_empty());
        for upload in self.uploads.drain(..) {
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

#[derive(Default)]
struct WgpuAtlasStorage {
    monochrome_textures: AtlasTextureList<WgpuAtlasTexture>,
    polychrome_textures: AtlasTextureList<WgpuAtlasTexture>,
}

impl ops::Index<AtlasTextureKind> for WgpuAtlasStorage {
    type Output = AtlasTextureList<WgpuAtlasTexture>;
    fn index(&self, kind: AtlasTextureKind) -> &Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
        }
    }
}

impl ops::IndexMut<AtlasTextureKind> for WgpuAtlasStorage {
    fn index_mut(&mut self, kind: AtlasTextureKind) -> &mut Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
        }
    }
}

impl ops::Index<AtlasTextureId> for WgpuAtlasStorage {
    type Output = WgpuAtlasTexture;
    fn index(&self, id: AtlasTextureId) -> &Self::Output {
        self[id.kind][id.index as usize].as_ref().expect("a tile's texture is kept while it's in use")
    }
}

struct WgpuAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes_per_pixel: u32,
}

impl WgpuAtlasTexture {
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
