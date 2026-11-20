// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_renderer.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host as its Blade renderer was: it draws a
// scene's quads, shadows, underlines and sprites into a window's surface, or into an offscreen texture it reads back
// for a picture; no paths, video surfaces, subpixel text or MSAA. Backdrops, what's behind a node blurred, are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use super::scene::{PrimitiveBatch, Recording, Scene, Shadow};
use super::atlas::Atlas;
use super::context::Context;
use crate::geometry::{Bounds, ContentMask, DevicePixels, ScaledPixels, Size};
use bytemuck::{Pod, Zeroable};
use std::sync::{Arc, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    origin: [f32; 2],
    premultiplied_alpha: u32,
    pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams {
    step: [f32; 2],
    sigma: f32,
    pad: f32,
}

/// The pipelines a format of target draws with, and the layouts of what they bind: the frame's globals (group 0), a
/// batch's instances or a blur's parameters (group 1), and the texture a batch samples (group 2).
pub(crate) struct Pipelines {
    layouts: Layouts,
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    poly_sprites: wgpu::RenderPipeline,
    /// A raster's: a layer's texture drawn over what's there, premultiplied, as it was drawn over nothing.
    rasters: wgpu::RenderPipeline,
    /// A backdrop's and its blur's, which few windows draw: compiled after the rest, on a thread of their own, so a
    /// window's first frame doesn't wait for them (`backdrops`).
    backdrops: OnceLock<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    /// A shadow's tile's: what draws one (its blurred box, alone, into a texture of one channel), and what draws a
    /// shadow from its tile (`tiles`). Compiled after the rest, as the backdrops' are; till then shadows are worked
    /// out where they're drawn.
    tiles: OnceLock<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
}

/// What every pipeline of a format is made from: the shaders, the target's format and blending, and the layouts of
/// what the pipelines bind.
struct Layouts {
    device: wgpu::Device,
    shader: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    blend: wgpu::BlendState,
    globals: wgpu::BindGroupLayout,
    instances: wgpu::BindGroupLayout,
    blur_params: wgpu::BindGroupLayout,
    texture: wgpu::BindGroupLayout,
    /// A 64×64 tile of blue noise (`blue-noise.png`, made by Ulichney's void-and-cluster, σ 1.5), which the shaders
    /// dither what would band with (`dither`), bound with the globals.
    noise: wgpu::TextureView,
}

impl Layouts {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, premultiplied: bool) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders.wgsl").into()),
        });
        let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = |entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries });
        let noise = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let globals = layout(&[uniform(0), uniform(1), noise]);
        // A batch's instances are at an offset of their own in the frame's data, which one bind group of it gives.
        let instances = layout(&[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: true, min_binding_size: None },
            count: None,
        }]);
        let blur_params = layout(&[uniform(0)]);
        let texture = layout(&[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ]);
        // See https://apoorvaj.io/alpha-compositing-opengl-blending-and-premultiplied-alpha/
        let blend = if premultiplied { wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING } else { wgpu::BlendState::ALPHA_BLENDING };
        let pixels = image::load_from_memory(include_bytes!("blue-noise.png")).expect("it's a PNG").to_luma8();
        let size = wgpu::Extent3d { width: pixels.width(), height: pixels.height(), depth_or_array_layers: 1 };
        let tile = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tile, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.width), rows_per_image: None },
            size,
        );
        let noise = tile.create_view(&wgpu::TextureViewDescriptor::default());
        Self { device: device.clone(), shader, format, blend, globals, instances, blur_params, texture, noise }
    }

    /// The frame's globals and its gamma, at their places in `buffer`, with the noise.
    fn globals(&self, buffer: &wgpu::Buffer, globals: (u64, u64), gamma: (u64, u64)) -> wgpu::BindGroup {
        let binding = |(offset, size): (u64, u64)| wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset, size: wgpu::BufferSize::new(size) });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.globals,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: binding(globals) },
                wgpu::BindGroupEntry { binding: 1, resource: binding(gamma) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.noise) },
            ],
        })
    }

    /// A pipeline of `vertex` and `fragment` binding `data` as group 1, and where it samples a texture, the texture
    /// as group 2.
    fn pipeline(&self, data: &wgpu::BindGroupLayout, vertex: &str, fragment: &str, blend: Option<wgpu::BlendState>, samples: bool) -> wgpu::RenderPipeline {
        self.pipeline_into(self.format, data, vertex, fragment, blend, samples)
    }

    /// `pipeline`, drawing into a target of `format`.
    fn pipeline_into(&self, format: wgpu::TextureFormat, data: &wgpu::BindGroupLayout, vertex: &str, fragment: &str, blend: Option<wgpu::BlendState>, samples: bool) -> wgpu::RenderPipeline {
        let groups = [Some(&self.globals), Some(data), Some(&self.texture)];
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &groups[..if samples { 3 } else { 2 }],
            immediate_size: 0,
        });
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &self.shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
type Compile<'a> = &'a (dyn Fn() -> wgpu::RenderPipeline + Sync);
#[cfg(target_arch = "wasm32")]
type Compile<'a> = &'a dyn Fn() -> wgpu::RenderPipeline;

/// Compiles each pipeline at once, as each takes its compiler a while (Direct3D's most): on threads of their own, but
/// on the web, which has none here and compiles in the browser's GPU process anyway, one after another.
fn side_by_side<const N: usize>(compile: [Compile; N]) -> [wgpu::RenderPipeline; N] {
    #[cfg(target_arch = "wasm32")]
    return compile.map(|compile| compile());
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::scope(|scope| compile.map(|compile| scope.spawn(compile)).map(|compiling| compiling.join().expect("compiling a pipeline doesn't panic")))
}

impl Pipelines {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, premultiplied: bool) -> Self {
        let layouts = Layouts::new(device, queue, format, premultiplied);
        let (l, blend) = (&layouts, Some(layouts.blend));
        let [quads, shadows, underlines, mono_sprites, poly_sprites, rasters] = side_by_side([
            &|| l.pipeline(&l.instances, "vs_quad", "fs_quad", blend, false),
            &|| l.pipeline(&l.instances, "vs_shadow", "fs_shadow", blend, false),
            &|| l.pipeline(&l.instances, "vs_underline", "fs_underline", blend, false),
            &|| l.pipeline(&l.instances, "vs_mono_sprite", "fs_mono_sprite", blend, true),
            &|| l.pipeline(&l.instances, "vs_poly_sprite", "fs_poly_sprite", blend, true),
            &|| l.pipeline(&l.instances, "vs_raster", "fs_raster", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), true),
        ]);
        Self { layouts, quads, shadows, underlines, mono_sprites, poly_sprites, rasters, backdrops: OnceLock::new(), tiles: OnceLock::new() }
    }

    /// A shadow's tile's pipelines (`tiles`), compiled now unless they are, or waited for where they're being.
    fn tiles(&self) -> &(wgpu::RenderPipeline, wgpu::RenderPipeline) {
        self.tiles.get_or_init(|| {
            let l = &self.layouts;
            // A tile holds how much of each point the blurred box covers, nothing else, and replaces what's there.
            let coverage = l.pipeline_into(TILE_FORMAT, &l.instances, "vs_shadow_whole", "fs_shadow_coverage", None, false);
            (coverage, l.pipeline(&l.instances, "vs_shadow", "fs_shadow_tile", Some(l.blend), true))
        })
    }

    /// A backdrop's pipeline and its blur's, compiled now unless they are, or waited for where they're being.
    fn backdrops(&self) -> &(wgpu::RenderPipeline, wgpu::RenderPipeline) {
        self.backdrops.get_or_init(|| {
            let l = &self.layouts;
            // Each pass of a blur replaces what's in its target.
            let [backdrops, blur] = side_by_side([&|| l.pipeline(&l.instances, "vs_backdrop", "fs_backdrop", Some(l.blend), true), &|| l.pipeline(&l.blur_params, "vs_blur", "fs_blur", None, true)]);
            (backdrops, blur)
        })
    }

    /// The pipelines for targets of `format`, compiled the first time a renderer draws to one, or ahead of it
    /// (`Context::prepare`), which one that comes meanwhile waits for.
    pub(crate) fn shared(device: &wgpu::Device, queue: &wgpu::Queue, compiled: &super::context::Compiled, format: wgpu::TextureFormat, premultiplied: bool) -> Arc<Pipelines> {
        let mut compiled = compiled.lock().expect("nothing panics holding it");
        if let Some((_, _, pipelines)) = compiled.iter().find(|(f, p, _)| *f == format && *p == premultiplied) {
            return pipelines.clone();
        }
        let pipelines = Arc::new(Pipelines::new(device, queue, format, premultiplied));
        compiled.push((format, premultiplied, pipelines.clone()));
        // The rest compiled in the background where there are threads; on the web, as each is first drawn.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let compiling = pipelines.clone();
            std::thread::spawn(move || {
                compiling.backdrops();
                compiling.tiles();
            });
        }
        pipelines
    }
}

/// Where a frame is drawn: a window's surface, or a texture of our own, which `read` reads back.
enum Target {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Offscreen { texture: wgpu::Texture, view: wgpu::TextureView, readback: wgpu::Buffer, row: u32 },
}

/// The format an offscreen frame is drawn in: 8-bit RGBA, as a PNG's pixels are.
pub(crate) const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A shadow's tile's format: one channel, how much of each point its box covers.
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The widest a shadow's tile is, in device pixels; a wider one's shadow is worked out where it's drawn.
const TILE_MOST: f32 = 2048.0;

/// What tells a shadow's tile from another's: its blur, its corners and its box's size in the tile, to a quarter of a
/// device pixel.
type TileKey = [u32; 7];

/// A shadow drawn from a tile, as Skia and Chromium draw a blurred rounded box: its box blurred once, exactly, into a
/// texture of its own, and drawn from that. Along an axis longer than two corners (each its radius and three blurs),
/// the tile's box is two corners long, and drawn stretched: its corners as they are and between them its middle, as
/// along an edge the blur no longer depends on how far the other edge is; along a shorter one, the box is as long as it
/// is. One with no blur, or inside its box (`inset`), is worked out where it's drawn. → its tile's key, and its box in
/// the tile.
fn tiled(shadow: &Shadow) -> Option<(TileKey, [f32; 2])> {
    let blur = shadow.blur_radius.0;
    let radii = &shadow.corner_radii;
    let corner = radii.top_left.0.max(radii.top_right.0).max(radii.bottom_right.0).max(radii.bottom_left.0) + 3.0 * blur;
    let tile = [shadow.bounds.size.width.0.min(2.0 * corner), shadow.bounds.size.height.0.min(2.0 * corner)];
    if shadow.inset != 0 || blur <= 0.0 || tile[0].max(tile[1]) + 6.0 * blur > TILE_MOST {
        return None;
    }
    let quarter = |value: f32| (value * 4.0).round() as u32;
    let key = [quarter(blur), quarter(radii.top_left.0), quarter(radii.top_right.0), quarter(radii.bottom_right.0), quarter(radii.bottom_left.0), quarter(tile[0]), quarter(tile[1])];
    Some((key, tile))
}

/// How many times a backdrop's frame is halved at most: a sixteenth of it, for the widest blur.
const LEVELS: usize = 4;

/// The textures a backdrop's blur is drawn through, for frames of `size`: for each level (half its size, a quarter,
/// …) the frame shrunk to it and the two passes of its blur, each made as a blur first needs it, as only the widest
/// blur's last level has the two passes; and the frame copied, where it can't be read as it is.
struct BackdropTextures {
    size: (u32, u32),
    source: Option<(wgpu::Texture, wgpu::TextureView)>,
    levels: [[Option<(wgpu::Texture, wgpu::TextureView)>; 3]; LEVELS],
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    pipelines: Arc<Pipelines>,
    /// The format of the target, which the pipelines and a backdrop's textures are for, and whether what's drawn
    /// into it is premultiplied by its alpha.
    format: wgpu::TextureFormat,
    premultiplied: bool,
    size: (u32, u32),
    /// What a frame binds, each part at a multiple of this.
    alignment: u64,
    /// The frame's globals, its batches' instances and its blurs' parameters, written once a frame.
    data: Vec<u8>,
    buffer: Option<wgpu::Buffer>,
    atlas: Arc<Atlas>,
    sampler: wgpu::Sampler,
    gamma: GammaParams,
    /// What a backdrop blurs: the frame as drawn so far, halved and halved again, then blurred one way and the other.
    backdrop: Option<BackdropTextures>,
    /// Each shadow's tile, drawn the first frame it's in, with what binds it (`tiled`).
    tiles: std::collections::HashMap<TileKey, (wgpu::Texture, wgpu::BindGroup)>,
    /// What the last frame drew, batch by batch (`Drawn`), and what of the frames before was the same and is kept.
    last: Drawn,
    kept: Option<Kept>,
    /// The bind group of the frame's data for its batches' instances, each at its offset, and how much of the data
    /// from there it binds; made again only as the data's buffer is, or a batch needs more.
    instances: Option<(wgpu::Buffer, u64, wgpu::BindGroup)>,
    /// Each raster's texture, by its id, drawn the first frame it's in, kept while frames have it, and its bind group.
    rasters: std::collections::HashMap<u32, (wgpu::Texture, wgpu::BindGroup)>,
    /// Each blur of what's behind a backdrop, by what was drawn before it (`Blur::behind`), kept while frames draw the
    /// same behind one, as iOS's materials blur what's behind them once while it holds still; and whether this frame
    /// used it.
    blurred: std::collections::HashMap<u64, (wgpu::TextureView, bool)>,
    /// Each atlas texture's bind group, as of the atlas's having made so many textures.
    atlas_groups: std::cell::RefCell<(u64, std::collections::HashMap<super::atlas::AtlasTextureId, wgpu::BindGroup>)>,
    /// How many draw calls the last frame made (`draws`).
    draws: std::cell::Cell<u32>,
    /// What the last frame's GPU did (`Work`).
    work: std::cell::Cell<Work>,
    /// Its passes' time on the GPU, where the GPU can say (`Stamps`), and how long it last waited for an image of its
    /// surface to draw on.
    stamps: Option<Stamps>,
    acquired: web_time::Duration,
}

/// How many passes a frame's GPU time is taken of, each a timestamp as it begins and one as it ends.
const STAMPED: u32 = 32;

/// Timestamps of a frame's passes, as WebGPU's `timestamp-query` takes them (at a pass's start and end: a tiled GPU's
/// stage boundaries), resolved into one of a few buffers that are read back once the GPU's done, never waited for:
/// the GPU time of the frame drawn a frame or two before.
struct Stamps {
    set: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    /// Each buffer read back, whether it's mapped, and how many timestamps it holds, while it's in use.
    read: Vec<(wgpu::Buffer, Arc<std::sync::atomic::AtomicBool>, Option<u32>)>,
    /// The buffer this frame's go to, and how many it's taken.
    writing: Option<usize>,
    taken: std::cell::Cell<u32>,
    /// Nanoseconds a tick.
    period: f64,
    gpu: Option<web_time::Duration>,
}

impl Stamps {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Stamps> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = u64::from(STAMPED * 2) * 8;
        let set = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: STAMPED * 2 });
        let resolved = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = (0..3)
            .map(|_| (device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }), Arc::default(), None))
            .collect();
        Some(Stamps { set, resolved, read, writing: None, taken: Default::default(), period: f64::from(queue.get_timestamp_period()), gpu: None })
    }

    /// As a frame starts: what the GPU has finished of the frames before is read, and a buffer free for this one's
    /// chosen, if one is.
    fn begin(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for (buffer, mapped, held) in &mut self.read {
            let Some(count) = *held else { continue };
            if !mapped.swap(false, std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            if let Ok(view) = buffer.get_mapped_range(..u64::from(count) * 8) {
                let ticks: &[u64] = bytemuck::cast_slice(&view);
                let total: u64 = ticks.chunks_exact(2).map(|pass| pass[1].saturating_sub(pass[0])).sum();
                self.gpu = Some(web_time::Duration::from_nanos((total as f64 * self.period) as u64));
            }
            buffer.unmap();
            *held = None;
        }
        self.writing = self.read.iter().position(|(_, _, held)| held.is_none());
        self.taken.set(0);
    }

    /// The timestamps of a pass about to begin, while there's room for them this frame.
    fn pass(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let at = self.taken.get();
        (self.writing.is_some() && at + 2 <= STAMPED * 2).then(|| {
            self.taken.set(at + 2);
            wgpu::RenderPassTimestampWrites { query_set: &self.set, beginning_of_pass_write_index: Some(at), end_of_pass_write_index: Some(at + 1) }
        })
    }

    /// As the frame's commands are finished: its timestamps resolved and copied out to be read back.
    fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(writing), count @ 1..) = (self.writing, self.taken.get()) else { return };
        encoder.resolve_query_set(&self.set, 0..count, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.read[writing].0, 0, u64::from(count) * 8);
    }

    /// Once they're submitted: read back when the GPU's done with them.
    fn submitted(&mut self) {
        let (Some(writing), count @ 1..) = (self.writing.take(), self.taken.get()) else { return };
        let (buffer, mapped, held) = &mut self.read[writing];
        let mapped = mapped.clone();
        buffer.map_async(wgpu::MapMode::Read, ..u64::from(count) * 8, move |done| mapped.store(done.is_ok(), std::sync::atomic::Ordering::Release));
        *held = Some(count);
    }
}

/// The frame's batches' instances: the bind group of its data, and each batch's offset in it.
struct Instances {
    group: wgpu::BindGroup,
    offsets: Vec<u32>,
}

/// What a frame drew: each batch, as a hash of what it is (its kind, texture and instances), and what they were drawn
/// over (the frame's size and origin, and the atlas's generation).
#[derive(Default, PartialEq)]
struct Drawn {
    over: u64,
    batches: Vec<u64>,
}

/// The start of a frame that's the same as the frames before it, drawn once into a texture of its own, as a
/// compositor keeps a layer: a frame starts as a copy of it and draws only what follows. What stays the same as the
/// rest moves (a launcher's card under its turning pages, a page under a sheet sliding up), shadows, blurs and all.
/// What a frame's GPU did, the same on any GPU: the device pixels of quads (and underlines), shadows, glyphs and
/// pictures (sprites), backdrops and layers' images (rasters) it drew, each counted as often as it's drawn over, the
/// pixels it copied of the frames before, and the bytes it uploaded to the atlas.
#[derive(Clone, Copy, Default)]
struct Work {
    quads: f64,
    shadows: f64,
    sprites: f64,
    backdrops: f64,
    rasters: f64,
    copied: f64,
    uploaded: u64,
}

struct Kept {
    texture: wgpu::Texture,
    drawn: Drawn,
}

impl Renderer {
    /// A renderer for a window, drawing into `target`, its surface as its platform gives it.
    pub fn new(
        context: &Context,
        atlas: Arc<Atlas>,
        target: super::SurfaceTarget,
        size: Size<DevicePixels>,
        transparent: bool,
    ) -> anyhow::Result<Self> {
        // Safety: a window's renderer is let go before its window is.
        #[cfg(not(target_arch = "wasm32"))]
        let surface = unsafe { context.instance.create_surface_unsafe(target) }?;
        #[cfg(target_arch = "wasm32")]
        let surface = context.instance.create_surface(target)?;
        let capabilities = surface.get_capabilities(&context.adapter);
        // 8 bits a channel, unconverted: the shaders blend in sRGB, as the DOM does. BGRA where the surface has it,
        // as most do, else RGBA, as Android's.
        let format = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
            .into_iter()
            .find(|format| capabilities.formats.contains(format))
            .or_else(|| capabilities.formats.iter().copied().find(|format| !format.is_srgb()))
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| anyhow::anyhow!("this GPU can't draw on this window"))?;
        // A see-through window's frame holds colours premultiplied by their alpha, as blending them over the cleared
        // frame leaves them, which is what system compositors take: DirectComposition and Vulkan are told so (their
        // "PostMultiplied" would read it as straight), and Core Animation reads any layer that way.
        let alpha_mode = (if transparent {
            &[wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::PostMultiplied, wgpu::CompositeAlphaMode::Inherit][..]
        } else {
            &[wgpu::CompositeAlphaMode::Opaque, wgpu::CompositeAlphaMode::Inherit][..]
        })
        .iter()
        .copied()
        .find(|mode| capabilities.alpha_modes.contains(mode))
        .unwrap_or(capabilities.alpha_modes[0]);
        // Each frame shown on the display's refresh, never as soon as it's drawn: Mailbox, the newest frame at each
        // refresh, where the system has it (Vulkan, Direct3D 12), else Fifo, every frame in turn, as a Mac's and a
        // phone's Metal have it. Immediate, which Blade's `DisplaySync::Recent` was on Metal (a CAMetalLayer's
        // `displaySyncEnabled` off), handed each frame to the compositor whenever it was done, so it showed some
        // twice and skipped others as they came either side of its refresh: what moved never moved smoothly on a Mac.
        let present_mode = if capabilities.present_modes.contains(&wgpu::PresentMode::Mailbox) { wgpu::PresentMode::Mailbox } else { wgpu::PresentMode::Fifo };
        let (width, height) = extent(size);
        let config = wgpu::SurfaceConfiguration {
            // A backdrop blurs what's drawn so far, read from the frame where the surface lets a shader read it
            // (Vulkan's), else copied out of it (`blur_behind`).
            // A frame starts as a copy of what's kept of the frames before, where the surface lets it be copied into.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | (capabilities.usages & wgpu::TextureUsages::COPY_DST)
                | if capabilities.usages.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    capabilities.usages & wgpu::TextureUsages::COPY_SRC
                },
            format,
            width,
            height,
            present_mode,
            // Three images, as Blade kept and Apple has a CAMetalLayer keep: one shown, one waiting for the next
            // refresh and one drawn, so a frame drawn as the display's link says never waits for the one still on
            // screen, as it did with two.
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&context.device, &config);
        let premultiplied = alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
        Ok(Self::with_target(context, atlas, Target::Surface { surface, config }, format, premultiplied, (width, height)))
    }

    /// A renderer that draws offscreen, `size` device pixels, read back by `read`.
    pub fn offscreen(context: &Context, atlas: Arc<Atlas>, size: Size<DevicePixels>) -> Self {
        let (width, height) = extent(size);
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // A row of the copy is a multiple of 256 bytes.
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self::with_target(context, atlas, Target::Offscreen { texture, view, readback, row }, OFFSCREEN_FORMAT, false, (width, height))
    }

    /// A renderer drawing into `target`, from `atlas`, which every renderer on the context shares.
    fn with_target(context: &Context, atlas: Arc<Atlas>, target: Target, format: wgpu::TextureFormat, premultiplied: bool, size: (u32, u32)) -> Self {
        let limits = context.device.limits();
        let sampler = context.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            device: context.device.clone(),
            queue: context.queue.clone(),
            target,
            pipelines: Pipelines::shared(&context.device, &context.queue, &context.pipelines, format, premultiplied),
            format,
            premultiplied,
            size,
            alignment: u64::from(limits.min_storage_buffer_offset_alignment.max(limits.min_uniform_buffer_offset_alignment)),
            data: Vec::new(),
            buffer: None,
            atlas,
            sampler,
            gamma: GammaParams { gamma_ratios: gamma_ratios(1.8), grayscale_enhanced_contrast: 1.0, pad: [0.0; 3] },
            backdrop: None,
            tiles: Default::default(),
            last: Drawn::default(),
            kept: None,
            instances: None,
            rasters: Default::default(),
            blurred: Default::default(),
            atlas_groups: Default::default(),
            draws: Default::default(),
            work: Default::default(),
            stamps: Stamps::new(&context.device, &context.queue),
            acquired: Default::default(),
        }
    }

    /// The window's new size, in device pixels.
    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let size = extent(size);
        if let Target::Surface { surface, config } = &mut self.target
            && (config.width, config.height) != size
        {
            (config.width, config.height) = size;
            surface.configure(&self.device, config);
            self.size = size;
        }
    }

    /// Keeps `rgba`, `width` by `height` pixels as raster `id`'s image, a frame drawn before (a window's snapshot,
    /// `platform::WindowOptions::snapshot`), which a scene draws by `Scene::picture` and which goes once a frame
    /// doesn't.
    pub fn picture(&mut self, id: u32, width: u32, height: u32, rgba: &[u8]) {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
            size,
        );
        let bound = self.texture(&texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.rasters.insert(id, (texture, bound));
    }

    /// Waits until the GPU has done all it was given: how long a frame took it, timed from its `draw`.
    pub fn finish(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// How many draw calls the last frame made: each batch, shadow tile, raster and blur pass drawn, but what was kept
    /// of the frames before (`Kept`), as a GPU profiler counts them.
    pub fn draws(&self) -> u32 {
        self.draws.get()
    }

    fn drew(&self) {
        self.draws.set(self.draws.get() + 1);
    }

    /// What the last frame's GPU did, the same on any GPU, as the inspector's `profile` and `replay` say it: how many
    /// device pixels each kind of thing it drew covered, what it copied of the frames before, and the bytes it uploaded.
    pub fn work(&self) -> serde_json::Value {
        let work = self.work.get();
        let px = |area: f64| area.round() as u64;
        serde_json::json!({
            "quads": px(work.quads),
            "shadows": px(work.shadows),
            "sprites": px(work.sprites),
            "backdrops": px(work.backdrops),
            "rasters": px(work.rasters),
            "copied": px(work.copied),
            "uploaded": work.uploaded,
        })
    }

    /// The device pixels batch `batch` covers, each thing in it cut to what it's drawn inside, added to its kind's.
    fn covered(&self, batch: &PrimitiveBatch<'_>) {
        fn area<'a>(all: impl Iterator<Item = (&'a Bounds<ScaledPixels>, &'a ContentMask<ScaledPixels>)>) -> f64 {
            all.map(|(bounds, mask)| bounds.intersect(&mask.bounds)).map(|cut| f64::from(cut.size.width.0) * f64::from(cut.size.height.0)).sum()
        }
        let mut work = self.work.get();
        match batch {
            PrimitiveBatch::Quads(quads) => work.quads += area(quads.iter().map(|quad| (&quad.bounds, &quad.content_mask))),
            PrimitiveBatch::Underlines(lines) => work.quads += area(lines.iter().map(|line| (&line.bounds, &line.content_mask))),
            PrimitiveBatch::Shadows(shadows) => work.shadows += area(shadows.iter().map(|shadow| (&shadow.bounds, &shadow.content_mask))),
            PrimitiveBatch::MonochromeSprites { sprites, .. } => work.sprites += area(sprites.iter().map(|sprite| (&sprite.bounds, &sprite.content_mask))),
            PrimitiveBatch::PolychromeSprites { sprites, .. } => work.sprites += area(sprites.iter().map(|sprite| (&sprite.bounds, &sprite.content_mask))),
            PrimitiveBatch::Backdrops(backdrops) => work.backdrops += area(backdrops.iter().map(|backdrop| (&backdrop.bounds, &backdrop.content_mask))),
            PrimitiveBatch::Rasters(rasters) => work.rasters += area(rasters.iter().map(|raster| (&raster.bounds, &raster.content_mask))),
        }
        self.work.set(work);
    }

    pub fn sprite_atlas(&self) -> &Arc<Atlas> {
        &self.atlas
    }

    /// Lets go of its surface, waiting for what's drawn on it, before its window goes.
    pub fn destroy(&mut self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Adds `bytes` to the frame's data, at the next multiple of the alignment, and says where.
    fn push(&mut self, bytes: &[u8]) -> (u64, u64) {
        let offset = (self.data.len() as u64).next_multiple_of(self.alignment);
        self.data.resize(offset as usize, 0);
        self.data.extend_from_slice(bytes);
        (offset, bytes.len().max(4) as u64)
    }

    /// Adds `items`, the scene's `#[repr(C)]` primitives, for a shader to read as an array of its struct.
    fn push_instances<T>(&mut self, items: &[T]) -> (u64, u64) {
        // Safety: the scene's primitives are `#[repr(C)]` structs of numbers, in the layout the shader reads.
        let bytes = unsafe { std::slice::from_raw_parts(items.as_ptr().cast::<u8>(), std::mem::size_of_val(items)) };
        self.push(bytes)
    }

    /// Draws the box of `scene` whose top left is `origin`, in device pixels, as big as the target.
    pub fn draw(&mut self, scene: &Scene, origin: [f32; 2]) -> bool {
        self.atlas.before_frame();
        self.draws.set(0);
        self.work.set(Work { uploaded: self.atlas.uploaded(), ..Work::default() });
        if let Some(stamps) = &mut self.stamps {
            stamps.begin(&self.device);
        }
        let acquiring = web_time::Instant::now();
        let (frame, view, texture) = match &self.target {
            Target::Surface { surface, config } => {
                // A surface its window has outgrown, or lost, is configured again and its frame taken from that, so the
                // frame drawn as a window changes size isn't dropped.
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(&self.device, config);
                        surface.get_current_texture()
                    }
                    taken => taken,
                };
                // None to draw on now (a window hidden or asleep, a surface lost): it's drawn on the next.
                let frame = match frame {
                    wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return false,
                };
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let texture = frame.texture.clone();
                (Some(frame), view, texture)
            }
            Target::Offscreen { texture, view, .. } => (None, view.clone(), texture.clone()),
        };
        self.acquired = acquiring.elapsed();
        let size = (texture.width(), texture.height());

        // The frame's data: its globals, then each batch's instances, then each backdrop's blur's parameters.
        self.data.clear();
        let globals = GlobalParams { viewport_size: [size.0 as f32, size.1 as f32], origin, premultiplied_alpha: u32::from(self.premultiplied), pad: [0; 3] };
        let globals = self.push(bytemuck::bytes_of(&globals));
        let gamma = self.gamma;
        let gamma = self.push(bytemuck::bytes_of(&gamma));
        let batches: Vec<PrimitiveBatch<'_>> = scene.batches().collect();
        let instances: Vec<(u64, u64)> = batches.iter().map(|batch| self.push_batch(batch)).collect();
        // What each batch is, and what of the frame's start is the same as the last's.
        let drawn = {
            use std::hash::{Hash, Hasher};
            let hash = |hashing: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                hashing(&mut hasher);
                hasher.finish()
            };
            let over = hash(&|hasher| (size, origin.map(f32::to_bits), self.atlas.generation()).hash(hasher));
            let batches = batches
                .iter()
                .zip(&instances)
                .map(|(batch, (offset, length))| {
                    let texture = match batch {
                        PrimitiveBatch::MonochromeSprites { texture_id, .. } | PrimitiveBatch::PolychromeSprites { texture_id, .. } => Some(*texture_id),
                        _ => None,
                    };
                    let bytes = &self.data[*offset as usize..(*offset + *length) as usize];
                    hash(&|hasher| (std::mem::discriminant(batch), texture, bytes).hash(hasher))
                })
                .collect();
            Drawn { over, batches }
        };
        // A backdrop's blur is of the whole frame drawn before it, wherever the backdrop is: the same while that is,
        // as the first of its batch moves (a sheet carried up), it's kept.
        let blurs: Vec<Vec<Blur>> = batches
            .iter()
            .enumerate()
            .map(|(at, batch)| match batch {
                PrimitiveBatch::Backdrops(backdrops) => backdrops
                    .iter()
                    .enumerate()
                    .map(|(one, backdrop)| {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        let before = if one == 0 { &drawn.batches[..at] } else { &drawn.batches[..=at] };
                        (drawn.over, before, one, backdrop.blur.0.to_bits()).hash(&mut hasher);
                        let mut blur = self.plan_blur(size, backdrop.blur.0);
                        blur.behind = Some(hasher.finish());
                        blur
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        // The tiles of the frame's shadows that haven't one yet, each its box alone, blurred, in a frame of its own.
        let mut planned: Vec<(TileKey, (u32, u32), (u64, u64), (u64, u64))> = Vec::new();
        if self.pipelines.tiles.get().is_some() {
            for batch in &batches {
                let PrimitiveBatch::Shadows(shadows) = batch else { continue };
                for shadow in shadows.iter() {
                    let Some((key, tile)) = tiled(shadow) else { continue };
                    if self.tiles.contains_key(&key) || planned.iter().any(|(planned, ..)| *planned == key) {
                        continue;
                    }
                    let blur = shadow.blur_radius.0;
                    let side = [(tile[0] + 6.0 * blur).ceil() as u32, (tile[1] + 6.0 * blur).ceil() as u32];
                    let globals = GlobalParams { viewport_size: [side[0] as f32, side[1] as f32], origin: [0.0; 2], premultiplied_alpha: 0, pad: [0; 3] };
                    let globals = self.push(bytemuck::bytes_of(&globals));
                    let mut alone = shadow.clone();
                    alone.bounds = crate::geometry::Bounds {
                        origin: crate::geometry::Point { x: crate::geometry::ScaledPixels(3.0 * blur), y: crate::geometry::ScaledPixels(3.0 * blur) },
                        size: crate::geometry::Size { width: crate::geometry::ScaledPixels(tile[0]), height: crate::geometry::ScaledPixels(tile[1]) },
                    };
                    alone.content_mask.bounds = crate::geometry::Bounds {
                        origin: crate::geometry::Point { x: crate::geometry::ScaledPixels(0.0), y: crate::geometry::ScaledPixels(0.0) },
                        size: crate::geometry::Size { width: crate::geometry::ScaledPixels(side[0] as f32), height: crate::geometry::ScaledPixels(side[1] as f32) },
                    };
                    alone.transformation = super::scene::TransformationMatrix::unit();
                    let instance = self.push_instances(std::slice::from_ref(&alone));
                    planned.push((key, (side[0], side[1]), globals, instance));
                }
            }
        }
        // The frame's rasters' scenes that haven't a texture yet, each drawn into one of its own, from its origin.
        let recordings: Vec<Recording> = scene.recordings.iter().filter(|recording| !self.rasters.contains_key(&recording.id)).cloned().collect();
        let mut rastering: Vec<(Recording, (u64, u64), Vec<(u64, u64)>)> = Vec::new();
        for recording in recordings {
            let globals = GlobalParams {
                viewport_size: [recording.size[0] as f32, recording.size[1] as f32],
                origin: recording.origin,
                premultiplied_alpha: u32::from(self.premultiplied),
                pad: [0; 3],
            };
            let globals = self.push(bytemuck::bytes_of(&globals));
            let scene = recording.scene.clone();
            let instances = scene.batches().map(|batch| self.push_batch(&batch)).collect();
            rastering.push((recording, globals, instances));
        }
        // What a batch's instances bind is as much as the largest needs, from its offset, so one bind group serves all.
        let largest = instances
            .iter()
            .chain(planned.iter().map(|(.., instance)| instance))
            .chain(rastering.iter().flat_map(|(.., instances)| instances))
            .map(|(_, length)| *length)
            .max()
            .unwrap_or(4);
        let window = self.instances.as_ref().map_or(0, |(_, window, _)| *window).max(largest.next_power_of_two().max(256));
        self.data.resize(self.data.len() + window as usize, 0);
        self.write_data();
        let buffer = self.buffer.clone().expect("written above");
        let binding = |(offset, size): (u64, u64)| wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset, size: wgpu::BufferSize::new(size) });
        let globals = self.pipelines.layouts.globals(&buffer, globals, gamma);
        if self.instances.as_ref().is_none_or(|(bound, bound_window, _)| *bound != buffer || *bound_window != window) {
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipelines.layouts.instances,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: binding((0, window)) }],
            });
            self.instances = Some((buffer.clone(), window, group));
        }
        let group = self.instances.as_ref().expect("made above").2.clone();
        let instances = Instances { group, offsets: instances.iter().map(|(offset, _)| *offset as u32).collect() };

        // The frame is drawn in runs of batches, one pass each, broken at each backdrop, which blurs what's drawn
        // before it.
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for (key, (width, height), globals, instance) in planned {
            self.draw_tile(&mut encoder, &buffer, &instances.group, key, (width, height), globals, gamma, instance);
        }
        for (recording, globals, offsets) in rastering {
            self.draw_raster(&mut encoder, &buffer, &instances.group, &recording, globals, gamma, &offsets);
        }
        self.rasters.retain(|id, _| scene.recordings.iter().any(|recording| recording.id == *id) || scene.rasters.iter().any(|raster| raster.raster == *id));
        // The frame starts from what's kept, where it starts as that did, and is drawn only from there; or what's the
        // same as the last frame's start, two frames running, is kept now, where it's worth more than its copy.
        let copies = texture.usage().contains(wgpu::TextureUsages::COPY_DST);
        let same = |kept: &Drawn| kept.over == drawn.over && drawn.batches.starts_with(&kept.batches);
        if !copies || self.kept.as_ref().is_some_and(|kept| !same(&kept.drawn)) {
            self.kept = None;
        }
        if copies && self.kept.is_none() && self.last.over == drawn.over {
            let start = drawn.batches.iter().zip(&self.last.batches).take_while(|(now, last)| now == last).count();
            // A shadow's or a blur's pixels cost more than copying the frame; anything else is drawn again.
            let worth = batches[..start].iter().any(|batch| matches!(batch, PrimitiveBatch::Shadows(_) | PrimitiveBatch::Backdrops(_)));
            if worth {
                let made = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let made_view = made.create_view(&wgpu::TextureViewDescriptor::default());
                self.draw_batches(&mut encoder, &made, &made_view, &globals, &buffer, &batches, &instances, &blurs, 0..start);
                self.kept = Some(Kept { texture: made, drawn: Drawn { over: drawn.over, batches: drawn.batches[..start].to_vec() } });
            }
        }
        let from = match &self.kept {
            Some(kept) => {
                encoder.copy_texture_to_texture(kept.texture.as_image_copy(), texture.as_image_copy(), wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 });
                let mut work = self.work.get();
                work.copied += f64::from(size.0) * f64::from(size.1);
                self.work.set(work);
                kept.drawn.batches.len()
            }
            None => 0,
        };
        self.draw_batches(&mut encoder, &texture, &view, &globals, &buffer, &batches, &instances, &blurs, from..batches.len());
        self.last = drawn;
        // What's behind no backdrop this frame isn't kept; what's still drawn starts the next frame unused.
        self.blurred.retain(|_, (_, used)| std::mem::take(used));

        if let Some(stamps) = &self.stamps {
            stamps.resolve(&mut encoder);
        }
        self.queue.submit([encoder.finish()]);
        if let Some(stamps) = &mut self.stamps {
            stamps.submitted();
        }
        if let Some(frame) = frame {
            self.queue.present(frame);
        }
        true
    }

    /// The GPU time of the last frame it has, summed over its passes, where the GPU says (a frame or two behind).
    pub fn gpu(&self) -> Option<web_time::Duration> {
        self.stamps.as_ref()?.gpu
    }

    /// How long the last frame waited for an image of its surface to draw on, which a display that hasn't shown the
    /// frames before keeps it waiting for.
    pub fn acquired(&self) -> web_time::Duration {
        self.acquired
    }

    /// Writes the frame's data into its buffer, grown to hold it.
    fn write_data(&mut self) {
        let needed = (self.data.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        self.data.resize(needed as usize, 0);
        if self.buffer.as_ref().is_none_or(|buffer| buffer.size() < needed) {
            self.buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: needed.next_power_of_two().max(0x10000),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        self.queue.write_buffer(self.buffer.as_ref().expect("made above"), 0, &self.data);
    }

    /// Draws the batches `run` names into `view` in one pass; a backdrop with what's behind it blurred, and which
    /// of its batch's it is.
    #[allow(clippy::too_many_arguments)]
    fn draw_run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        globals: &wgpu::BindGroup,
        batches: &[PrimitiveBatch<'_>],
        instances: &Instances,
        run: &[usize],
        blurred: Option<(wgpu::TextureView, usize)>,
    ) {
        if run.is_empty() && !matches!(load, wgpu::LoadOp::Clear(_)) {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            timestamp_writes: self.stamps.as_ref().and_then(Stamps::pass),
            ..Default::default()
        });
        pass.set_bind_group(0, globals, &[]);
        for &at in run {
            self.covered(&batches[at]);
            if let PrimitiveBatch::Shadows(shadows) = &batches[at] {
                self.draw_shadows(&mut pass, shadows, &instances.group, instances.offsets[at]);
                continue;
            }
            if let PrimitiveBatch::Rasters(rasters) = &batches[at] {
                if let Some((_, group)) = self.rasters.get(&rasters[0].raster) {
                    pass.set_pipeline(&self.pipelines.rasters);
                    pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
                    pass.set_bind_group(2, group, &[]);
                    pass.draw(0..4, 0..1);
                    self.drew();
                }
                continue;
            }
            let (pipeline, texture, count) = match &batches[at] {
                PrimitiveBatch::Quads(quads) => (&self.pipelines.quads, None, quads.len()),
                PrimitiveBatch::Shadows(_) | PrimitiveBatch::Rasters(_) => unreachable!("drawn above"),
                PrimitiveBatch::Underlines(underlines) => (&self.pipelines.underlines, None, underlines.len()),
                PrimitiveBatch::MonochromeSprites { texture_id, sprites } => (&self.pipelines.mono_sprites, Some(*texture_id), sprites.len()),
                PrimitiveBatch::PolychromeSprites { texture_id, sprites } => (&self.pipelines.poly_sprites, Some(*texture_id), sprites.len()),
                PrimitiveBatch::Backdrops(_) => {
                    let Some((blurred, one)) = &blurred else { continue };
                    pass.set_pipeline(&self.pipelines.backdrops().0);
                    pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
                    pass.set_bind_group(2, &self.texture(blurred), &[]);
                    pass.draw(0..4, *one as u32..*one as u32 + 1);
                    self.drew();
                    continue;
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
            if let Some(id) = texture {
                pass.set_bind_group(2, &self.atlas_group(id), &[]);
            }
            pass.draw(0..4, 0..count as u32);
            self.drew();
        }
    }

    /// Draws `range` of the frame's batches into `texture` (`view` of it), in runs, one pass each, broken at each
    /// backdrop, which blurs what's drawn before it. From the first batch, it's cleared first; from a later one, it
    /// already has what's drawn before.
    #[allow(clippy::too_many_arguments)]
    fn draw_batches(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        view: &wgpu::TextureView,
        globals: &wgpu::BindGroup,
        buffer: &wgpu::Buffer,
        batches: &[PrimitiveBatch<'_>],
        instances: &Instances,
        blurs: &[Vec<Blur>],
        range: std::ops::Range<usize>,
    ) {
        let blurrable = texture.usage().intersects(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let mut load = if range.start == 0 { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
        let mut run: Vec<usize> = Vec::new();
        for at in range {
            let PrimitiveBatch::Backdrops(backdrops) = &batches[at] else {
                run.push(at);
                continue;
            };
            self.draw_run(encoder, view, load, globals, batches, instances, &run, None);
            run.clear();
            load = wgpu::LoadOp::Load;
            for (one, blur) in (0..backdrops.len()).zip(&blurs[at]) {
                let blurred = blurrable.then(|| self.blur_behind(encoder, texture, view, globals, buffer, blur));
                self.draw_run(encoder, view, wgpu::LoadOp::Load, globals, batches, instances, &[at], blurred.map(|view| (view, one)));
            }
        }
        self.draw_run(encoder, view, load, globals, batches, instances, &run, None);
    }

    /// Adds a batch's instances to the frame's data, and says where.
    fn push_batch(&mut self, batch: &PrimitiveBatch<'_>) -> (u64, u64) {
        match batch {
            PrimitiveBatch::Quads(quads) => self.push_instances(quads),
            PrimitiveBatch::Shadows(shadows) => self.push_instances(shadows),
            PrimitiveBatch::Underlines(underlines) => self.push_instances(underlines),
            PrimitiveBatch::MonochromeSprites { sprites, .. } => self.push_instances(sprites),
            PrimitiveBatch::PolychromeSprites { sprites, .. } => self.push_instances(sprites),
            PrimitiveBatch::Backdrops(backdrops) => self.push_instances(backdrops),
            PrimitiveBatch::Rasters(rasters) => self.push_instances(rasters),
        }
    }

    /// Draws a raster's scene (`recording`) into a texture of its own, over nothing, from its origin (with its own
    /// `globals`, the frame's `gamma`, and its batches' instances at `offsets`), and keeps it, by its id.
    #[allow(clippy::too_many_arguments)]
    fn draw_raster(&mut self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, group: &wgpu::BindGroup, recording: &Recording, globals: (u64, u64), gamma: (u64, u64), offsets: &[(u64, u64)]) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: recording.size[0].max(1), height: recording.size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let globals = self.pipelines.layouts.globals(buffer, globals, gamma);
        let batches: Vec<PrimitiveBatch<'_>> = recording.scene.batches().collect();
        let instances = Instances { group: group.clone(), offsets: offsets.iter().map(|(offset, _)| *offset as u32).collect() };
        let blurs: Vec<Vec<Blur>> = batches.iter().map(|_| Vec::new()).collect();
        self.draw_batches(encoder, &texture, &view, &globals, buffer, &batches, &instances, &blurs, 0..batches.len());
        let bound = self.texture(&view);
        self.rasters.insert(recording.id, (texture, bound));
    }

    /// Draws a batch of shadows, each from its tile where it has one, else worked out where it's drawn, in runs.
    fn draw_shadows(&self, pass: &mut wgpu::RenderPass<'_>, shadows: &[Shadow], instances: &wgpu::BindGroup, offset: u32) {
        let tiles = self.pipelines.tiles.get();
        pass.set_bind_group(1, instances, &[offset]);
        let mut at = 0;
        while at < shadows.len() {
            let tile = tiles.zip(tiled(&shadows[at]).and_then(|(key, _)| self.tiles.get(&key)));
            match tile {
                Some(((_, pipeline), (_, group))) => {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, group, &[]);
                    pass.draw(0..10, at as u32..at as u32 + 1);
                    self.drew();
                    at += 1;
                }
                None => {
                    // The run of those worked out where they're drawn.
                    let end = (at..shadows.len()).find(|one| tiles.is_some() && tiled(&shadows[*one]).is_some_and(|(key, _)| self.tiles.contains_key(&key))).unwrap_or(shadows.len());
                    pass.set_pipeline(&self.pipelines.shadows);
                    pass.draw(0..10, at as u32..end as u32);
                    self.drew();
                    at = end;
                }
            }
        }
    }

    /// Draws a shadow's tile, `size`, from its box alone (`instance`, with the frame's `globals` and `gamma` of its
    /// own), and keeps it, as `key`'s. A window's shadows are few and keep their tiles; past 64 they're made again.
    #[allow(clippy::too_many_arguments)]
    fn draw_tile(&mut self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, instances: &wgpu::BindGroup, key: TileKey, (width, height): (u32, u32), globals: (u64, u64), gamma: (u64, u64), instance: (u64, u64)) {
        if self.tiles.len() >= 64 {
            self.tiles.clear();
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let globals = self.pipelines.layouts.globals(buffer, globals, gamma);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                // No timestamps: on an iPhone's GPU a tile drawn in a pass that took them was kept as if wholly covered,
                // its shadow a dark square round what casts it, for as long as the tile was kept; it's drawn once a
                // shadow's shape, no frame's cost.
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.tiles().0);
            pass.set_bind_group(0, &globals, &[]);
            pass.set_bind_group(1, instances, &[instance.0 as u32]);
            pass.draw(0..4, 0..1);
            self.drew();
        }
        let group = self.texture(&view);
        self.tiles.insert(key, (texture, group));
    }

    /// An atlas texture's bind group, kept till the atlas makes another texture (which may be in its place).
    fn atlas_group(&self, id: super::atlas::AtlasTextureId) -> wgpu::BindGroup {
        let mut groups = self.atlas_groups.borrow_mut();
        let made = self.atlas.made();
        if groups.0 != made {
            *groups = (made, Default::default());
        }
        groups.1.entry(id).or_insert_with(|| self.texture(&self.atlas.view(id))).clone()
    }

    /// A bind group of `view` and the sampler, for a pass to sample.
    fn texture(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipelines.layouts.texture,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// How a backdrop's blur of `sigma` device pixels is drawn over a frame of `size`: halved until the blur left is
    /// a few texels, each halving the average of four, so the Gaussian then takes every texel it reaches (a wide blur
    /// taking texels further apart streaks what's sharp behind it), then blurred across and down, with the
    /// parameters of each pass in the frame's data.
    fn plan_blur(&mut self, size: (u32, u32), sigma: f32) -> Blur {
        let level = (1..=LEVELS).find(|level| sigma / (1 << level) as f32 <= 6.0).unwrap_or(LEVELS);
        let (width, height) = halved(size, level);
        let sigma = sigma / (1 << level) as f32;
        let shrink = self.push(bytemuck::bytes_of(&BlurParams { step: [0.0, 0.0], sigma: 0.0, pad: 0.0 }));
        let across = self.push(bytemuck::bytes_of(&BlurParams { step: [1.0 / width as f32, 0.0], sigma, pad: 0.0 }));
        let down = self.push(bytemuck::bytes_of(&BlurParams { step: [0.0, 1.0 / height as f32], sigma, pad: 0.0 }));
        Blur { level, shrink, across, down, behind: None }
    }

    /// Blurs what's drawn into `frame` so far, as `blur` says, into the backdrop's textures: shrunk, then blurred
    /// across into one at its level's size and down into another, each a separable Gaussian's pass. The view of the
    /// last. Only the textures this blur goes through are made, each the first time one needs it, and the frame is
    /// read as it is where its surface lets a shader read it.
    fn blur_behind(&mut self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture, view: &wgpu::TextureView, globals: &wgpu::BindGroup, buffer: &wgpu::Buffer, blur: &Blur) -> wgpu::TextureView {
        if let Some((view, used)) = blur.behind.and_then(|behind| self.blurred.get_mut(&behind)) {
            *used = true;
            return view.clone();
        }
        let size = (frame.width(), frame.height());
        if self.backdrop.as_ref().is_none_or(|backdrop| backdrop.size != size) {
            self.backdrop = Some(BackdropTextures { size, source: None, levels: Default::default() });
        }
        let make = |(width, height): (u32, u32), usage: wgpu::TextureUsages| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let backdrop = self.backdrop.as_mut().expect("made above");
        let wanted = (0..blur.level).map(|at| (at, 0)).chain([(blur.level - 1, 1), (blur.level - 1, 2)]);
        for (at, which) in wanted {
            if backdrop.levels[at][which].is_none() {
                backdrop.levels[at][which] = Some(make(halved(size, at + 1), wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC));
            }
        }
        // A Metal drawable or a Direct3D 12 swap chain's buffer isn't one wgpu lets a shader read: what's drawn so
        // far is copied out of it first.
        let readable = frame.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING);
        if !readable {
            let source = backdrop.source.get_or_insert_with(|| make(size, wgpu::TextureUsages::COPY_DST));
            encoder.copy_texture_to_texture(frame.as_image_copy(), source.0.as_image_copy(), wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 });
        }
        let backdrop = self.backdrop.as_ref().expect("made above");
        let level = |at: usize, which: usize| &backdrop.levels[at][which].as_ref().expect("made above").1;
        let mut passes = Vec::new();
        let mut from = if readable { view } else { &backdrop.source.as_ref().expect("copied above").1 };
        for at in 0..blur.level {
            passes.push((from, level(at, 0), blur.shrink));
            from = level(at, 0);
        }
        let (across, down) = (level(blur.level - 1, 1), level(blur.level - 1, 2));
        passes.push((from, across, blur.across));
        passes.push((across, down, blur.down));
        for (from, into, params) in passes {
            let params = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipelines.layouts.blur_params,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: params.0, size: wgpu::BufferSize::new(params.1) }),
                }],
            });
            let texture = self.texture(from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: into,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                timestamp_writes: self.stamps.as_ref().and_then(Stamps::pass),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.backdrops().1);
            pass.set_bind_group(0, globals, &[]);
            pass.set_bind_group(1, &params, &[]);
            pass.set_bind_group(2, &texture, &[]);
            pass.draw(0..4, 0..1);
            self.drew();
        }
        let Some(behind) = blur.behind else { return down.clone() };
        // Kept in a texture of its own, as the next backdrop blurs into the same ones.
        let down = &backdrop.levels[blur.level - 1][2].as_ref().expect("made above").0;
        let (width, height) = (down.width(), down.height());
        let kept = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        encoder.copy_texture_to_texture(down.as_image_copy(), kept.as_image_copy(), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
        let view = kept.create_view(&wgpu::TextureViewDescriptor::default());
        self.blurred.insert(behind, (view.clone(), true));
        view
    }

    /// The last frame drawn offscreen, as RGBA rows, top first.
    pub fn read(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let Target::Offscreen { texture, readback, row, .. } = &self.target else { return None };
        let (width, height) = self.size;
        // Copied out only as it's read, so a frame that isn't costs no copy.
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*row), rows_per_image: None } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let mapped = readback.get_mapped_range(..).ok()?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for line in mapped.chunks(*row as usize).take(height as usize) {
            rgba.extend_from_slice(&line[..(width * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        Some((width, height, rgba))
    }
}

/// Where a backdrop's blur's parameters are in the frame's data, and at which level it's blurred.
struct Blur {
    /// What was drawn before it, by which a blur of it is kept (`Renderer::blurred`).
    behind: Option<u64>,
    level: usize,
    shrink: (u64, u64),
    across: (u64, u64),
    down: (u64, u64),
}

fn extent(size: Size<DevicePixels>) -> (u32, u32) {
    (size.width.0.max(1) as u32, size.height.0.max(1) as u32)
}

/// `size` halved `times` times, at least a pixel.
fn halved((width, height): (u32, u32), times: usize) -> (u32, u32) {
    ((width >> times).max(1), (height >> times).max(1))
}

/// Ratios for brightening and darkening glyphs' edges for contrast, GPUI's defaults: a gamma of 1.8.
// https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.cpp#L50
fn gamma_ratios(gamma: f32) -> [f32; 4] {
    const GAMMA_INCORRECT_TARGET_RATIOS: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0], // gamma = 1.0
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0], // gamma = 1.1
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0], // gamma = 1.2
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0], // gamma = 1.3
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0], // gamma = 1.4
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0], // gamma = 1.5
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0], // gamma = 1.6
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0], // gamma = 1.7
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0], // gamma = 1.8
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0], // gamma = 1.9
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0], // gamma = 2.0
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0], // gamma = 2.1
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0], // gamma = 2.2
    ];
    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / (255.0) * 4.0) as f32;
    let ratios = GAMMA_INCORRECT_TARGET_RATIOS[((gamma * 10.0).round() as usize).clamp(10, 22) - 10];
    [ratios[0] * NORM13, ratios[1] * NORM24, ratios[2] * NORM13, ratios[3] * NORM24]
}
