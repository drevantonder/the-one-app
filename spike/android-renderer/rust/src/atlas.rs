// From Zed's GPUI (Apache-2.0, LICENSE-APACHE beside this file), crates/gpui/src/platform/blade/blade_atlas.rs at
// zed-industries/zed 8249ef56187b966c33f6667d0d3a35d88d8f2dc0.
// Changed: keyed by what a strict screen draws (a glyph, as cosmic-text keys it, or an image by its address); one
// owner, so no lock; one per process, never freed; tiles are never removed (a screen's glyphs and icons fit one texture each); no subpixel textures.

use std::collections::HashMap;
use std::sync::Arc;

use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use etagere::BucketedAtlasAllocator;

use crate::scene::{AtlasTextureId, AtlasTile};

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum AtlasKey {
    Glyph(cosmic_text::CacheKey),
    Image(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Monochrome = 0,
    Polychrome = 2,
}

struct PendingUpload {
    id: AtlasTextureId,
    origin: [i32; 2],
    size: [i32; 2],
    data: gpu::BufferPiece,
}

struct Texture {
    allocator: BucketedAtlasAllocator,
    raw: gpu::Texture,
    raw_view: gpu::TextureView,
    bytes_per_pixel: u32,
}

pub struct Atlas {
    gpu: Arc<gpu::Context>,
    upload_belt: BufferBelt,
    monochrome: Vec<Texture>,
    polychrome: Vec<Texture>,
    tiles_by_key: HashMap<AtlasKey, Option<AtlasTile>>,
    initializations: Vec<AtlasTextureId>,
    uploads: Vec<PendingUpload>,
}

impl Atlas {
    pub fn new(gpu: &Arc<gpu::Context>) -> Atlas {
        Atlas {
            gpu: Arc::clone(gpu),
            upload_belt: BufferBelt::new(BufferBeltDescriptor {
                memory: gpu::Memory::Upload,
                min_chunk_size: 0x10000,
                alignment: 64, // Vulkan `optimalBufferCopyOffsetAlignment` on Intel XE
                target: gpu::BufferTarget::Data,
            }),
            monochrome: Vec::new(),
            polychrome: Vec::new(),
            tiles_by_key: HashMap::new(),
            initializations: Vec::new(),
            uploads: Vec::new(),
        }
    }

    /// The tile for `key`, rasterized by `build` the first time (its size in pixels and its bytes: one per pixel for
    /// a monochrome tile, BGRA for a polychrome one). `None` where there's nothing to draw (a space).
    pub fn get_or_insert_with(
        &mut self,
        key: &AtlasKey,
        kind: Kind,
        build: impl FnOnce() -> Option<([i32; 2], Vec<u8>)>,
    ) -> Option<AtlasTile> {
        if let Some(tile) = self.tiles_by_key.get(key) {
            return *tile;
        }
        let tile = build().filter(|(size, _)| size[0] > 0 && size[1] > 0).map(|(size, bytes)| {
            let tile = self.allocate(size, kind);
            let data = self.upload_belt.alloc_bytes(&bytes, &self.gpu);
            self.uploads.push(PendingUpload { id: tile.texture_id, origin: tile.origin, size, data });
            tile
        });
        self.tiles_by_key.insert(key.clone(), tile);
        tile
    }

    fn textures(&mut self, kind: Kind) -> &mut Vec<Texture> {
        match kind {
            Kind::Monochrome => &mut self.monochrome,
            Kind::Polychrome => &mut self.polychrome,
        }
    }

    fn texture(&self, id: AtlasTextureId) -> &Texture {
        if id.kind == Kind::Monochrome as u32 { &self.monochrome[id.index as usize] } else { &self.polychrome[id.index as usize] }
    }

    fn allocate(&mut self, size: [i32; 2], kind: Kind) -> AtlasTile {
        let textures = self.textures(kind);
        for (index, texture) in textures.iter_mut().enumerate().rev() {
            if let Some(allocation) = texture.allocator.allocate(etagere::size2(size[0], size[1])) {
                return tile(index, kind, allocation, size);
            }
        }
        let index = self.push_texture(size, kind);
        let allocation = self.textures(kind)[index].allocator.allocate(etagere::size2(size[0], size[1])).expect("a new texture fits");
        tile(index, kind, allocation, size)
    }

    fn push_texture(&mut self, min_size: [i32; 2], kind: Kind) -> usize {
        const DEFAULT_ATLAS_SIZE: i32 = 1024;
        let size = [min_size[0].max(DEFAULT_ATLAS_SIZE), min_size[1].max(DEFAULT_ATLAS_SIZE)];
        let format = match kind {
            Kind::Monochrome => gpu::TextureFormat::R8Unorm,
            Kind::Polychrome => gpu::TextureFormat::Bgra8Unorm,
        };
        let raw = self.gpu.create_texture(gpu::TextureDesc {
            name: "", // unnamed, as in renderer.rs
            format,
            size: gpu::Extent { width: size[0] as u32, height: size[1] as u32, depth: 1 },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::COPY | gpu::TextureUsage::RESOURCE,
            external: None,
        });
        let raw_view = self.gpu.create_texture_view(
            raw,
            gpu::TextureViewDesc { name: "", format, dimension: gpu::ViewDimension::D2, subresources: &Default::default() },
        );
        let bytes_per_pixel = format.block_info().size as u32;
        let textures = self.textures(kind);
        let index = textures.len();
        textures.push(Texture { allocator: BucketedAtlasAllocator::new(etagere::size2(size[0], size[1])), raw, raw_view, bytes_per_pixel });
        self.initializations.push(AtlasTextureId { index: index as u32, kind: kind as u32 });
        index
    }

    pub fn view(&self, id: AtlasTextureId) -> gpu::TextureView {
        self.texture(id).raw_view
    }

    pub fn before_frame(&mut self, encoder: &mut gpu::CommandEncoder) {
        for id in std::mem::take(&mut self.initializations) {
            encoder.init_texture(self.texture(id).raw);
        }
        let mut transfers = encoder.transfer("atlas");
        for upload in self.uploads.drain(..) {
            let texture = if upload.id.kind == Kind::Monochrome as u32 { &self.monochrome[upload.id.index as usize] } else { &self.polychrome[upload.id.index as usize] };
            transfers.copy_buffer_to_texture(
                upload.data,
                upload.size[0] as u32 * texture.bytes_per_pixel,
                gpu::TexturePiece { texture: texture.raw, mip_level: 0, array_layer: 0, origin: [upload.origin[0] as u32, upload.origin[1] as u32, 0] },
                gpu::Extent { width: upload.size[0] as u32, height: upload.size[1] as u32, depth: 1 },
            );
        }
    }

    pub fn after_frame(&mut self, sync_point: &gpu::SyncPoint) {
        self.upload_belt.flush(sync_point);
    }
}

fn tile(index: usize, kind: Kind, allocation: etagere::Allocation, size: [i32; 2]) -> AtlasTile {
    AtlasTile {
        texture_id: AtlasTextureId { index: index as u32, kind: kind as u32 },
        tile_id: allocation.id.serialize(),
        padding: 0,
        origin: [allocation.rectangle.min.x, allocation.rectangle.min.y],
        size,
    }
}
