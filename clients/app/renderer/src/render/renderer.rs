// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_renderer.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host as its Blade renderer was: it draws a
// scene's quads, shadows, underlines and sprites into a window's surface, or into an offscreen texture it reads back
// for a picture; no paths, video surfaces, subpixel text or MSAA. Backdrops, what's behind a node blurred, are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use super::scene::{PrimitiveBatch, Scene};
use super::atlas::Atlas;
use super::context::Context;
use crate::geometry::{Bounds, ContentMask, DevicePixels, ScaledPixels, Size};
use std::sync::Arc;

mod blur;
mod frame;
mod params;
mod pipelines;
mod shadows;
mod stamps;

use blur::{BackdropTextures, Blur};
use params::{GammaParams, GlobalParams, gamma_ratios};
pub(crate) use pipelines::Pipelines;
use shadows::{TileKey, tiled};
use stamps::Stamps;

/// Where a frame is drawn: a window's surface, or a texture of our own, which `read` reads back.
enum Target {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Offscreen { texture: wgpu::Texture, view: wgpu::TextureView, readback: wgpu::Buffer, row: u32 },
}

/// An image `size` device pixels to draw into offscreen, and what it's read back through.
fn offscreen_target(device: &wgpu::Device, (width, height): (u32, u32)) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: u64::from(row) * u64::from(height), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    Target::Offscreen { texture, view, readback, row }
}

/// The format an offscreen frame is drawn in: 8-bit RGBA, as a PNG's pixels are.
pub(crate) const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;



/// How long after its last frame a renderer lets go of what only makes frames drawn one after another quicker
/// (`Renderer::idle`).
const IDLE: web_time::Duration = web_time::Duration::from_secs(2);

/// How many textures a renderer keeps that its frames let go of (`Renderer::spare`): a launcher's pages a wheel turns
/// let go of a page's raster as they take another of its size.
const SPARE: usize = 2;

pub struct Renderer {
    /// When it last drew a frame (`idle`).
    drew: web_time::Instant,
    /// An image of its surface taken ahead, as its window went from the screen (`reserve`), drawn on first as it's
    /// back; let go before the surface it's of.
    reserved: Option<wgpu::SurfaceTexture>,
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
    /// What the frame's data is copied into its buffer from, in the frame's own commands. `Queue::write_buffer` copies
    /// it in commands of the queue's own, a second command encoder each frame; wgpu keeps four idle (our wgpu's
    /// change), so a few frames in flight make and free encoders as they go, and Intel's Direct3D 12 driver waits for
    /// the GPU to make one: a Surface took twice the CPU to draw a frame (p95 10 ms, 6.7 with one).
    belt: wgpu::util::StagingBelt,
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
    /// Textures a frame let go of (a raster's, a frame kept), each taken again by the next that needs one of its size,
    /// format and use, rather than made (`made`): one a window's size is more than a heap of 8 MB (`Context::create`),
    /// so a heap of its own, which Intel's Direct3D 12 driver makes resident as it's made, waiting on its GPU, and
    /// frees as it goes: on a Surface, a launcher's pages turned by a wheel made and freed one each few frames. The last
    /// `SPARE` are kept, and none once it idles.
    spare: Vec<wgpu::Texture>,
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
    /// How many draw calls and render passes the last frame made (`draws`, `passes`).
    draws: std::cell::Cell<u32>,
    passes: std::cell::Cell<u32>,
    /// What the last frame's GPU did (`Work`).
    work: std::cell::Cell<Work>,
    /// Its thread's time on a CPU drawing since it was last asked (`drawing`).
    on_cpu: std::cell::Cell<web_time::Duration>,
    /// Its passes' time on the GPU, where the GPU can say (`Stamps`), and how long it last waited for an image of its
    /// surface to draw on.
    stamps: Option<Stamps>,
    acquired: web_time::Duration,
    /// How long the last frame took to finish its commands and hand them to the GPU (`CommandEncoder::finish`,
    /// `Queue::submit`), and to hand its image to the system to show (`Queue::present`).
    submitted: web_time::Duration,
    presented: web_time::Duration,
    /// Whether the GPU is still on the frame it submitted last (`Queue::on_submitted_work_done`).
    working: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
/// pixels it copied of the frames before, the bytes it uploaded to the atlas, and the device pixels of what it blurred
/// afresh behind backdrops (`blurred`), where a blur kept from the frames before costs none.
#[derive(Clone, Copy, Default)]
struct Work {
    quads: f64,
    shadows: f64,
    sprites: f64,
    backdrops: f64,
    rasters: f64,
    copied: f64,
    uploaded: u64,
    blurred: f64,
}

/// What a layer's texture is for, a raster's or a frame kept: drawn into, sampled, and copied out of.
const LAYER: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING).union(wgpu::TextureUsages::COPY_SRC);

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
        // A browser's canvas may be read, as WebGPU's `GPUCanvasConfiguration.usage` has it, which wgpu's WebGPU
        // backend doesn't say (only `RENDER_ATTACHMENT`): without it a backdrop drew nothing on the web, so its glass
        // showed what's under it unblurred.
        #[cfg(target_arch = "wasm32")]
        let capabilities = wgpu::SurfaceCapabilities { usages: capabilities.usages | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING, ..capabilities };
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
        Self::with_target(context, atlas, offscreen_target(&context.device, (width, height)), OFFSCREEN_FORMAT, false, (width, height))
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
            drew: web_time::Instant::now(),
            reserved: None,
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
            belt: wgpu::util::StagingBelt::new(context.device.clone(), 1 << 20),
            atlas,
            sampler,
            gamma: GammaParams { gamma_ratios: gamma_ratios(1.8), grayscale_enhanced_contrast: 1.0, pad: [0.0; 3] },
            backdrop: None,
            tiles: Default::default(),
            last: Drawn::default(),
            kept: None,
            spare: Vec::new(),
            instances: None,
            rasters: Default::default(),
            blurred: Default::default(),
            atlas_groups: Default::default(),
            draws: Default::default(),
            passes: Default::default(),
            work: Default::default(),
            on_cpu: Default::default(),
            stamps: Stamps::new(&context.device, &context.queue),
            acquired: Default::default(),
            submitted: Default::default(),
            presented: Default::default(),
            working: Default::default(),
        }
    }

    /// The window's new size, in device pixels.
    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let size = extent(size);
        match &mut self.target {
            Target::Surface { surface, config } if (config.width, config.height) != size => {
                (config.width, config.height) = size;
                self.reserved = None;
                surface.configure(&self.device, config);
                self.size = size;
            }
            // An image drawn offscreen as a window would be (the fake platform's) is made again at its new size.
            Target::Offscreen { .. } if self.size != size => {
                self.target = offscreen_target(&self.device, size);
                self.size = size;
            }
            _ => {}
        }
    }

    /// Its window is going from the screen: it lets go of what it lets go of when idle (`idle`), and of its surface's
    /// images (three, each the window's size) on Windows and an iPhone, configured as small as a surface is until its
    /// next frame configures it at its window's size again, as a resize does: a DirectComposition swap chain and a
    /// CAMetalLayer's drawables are any size; an offscreen image (the fake platform's) as a surface's. On a Mac, it
    /// takes an image to draw on now, while its window's on the screen, and keeps it for its next frame, as a Mac's
    /// window going from the screen is drawn on first as it's back, where wgpu gives no image for a window macOS hasn't
    /// yet said is visible again (`occlusionState`), which it says a few frames after it's shown, and each summon of
    /// the launcher waited 30 to 45 ms for it. On X11, Vulkan's swap chain is its window's size (`currentExtent`), and
    /// it keeps its images; Android's view destroys its surface as it's hidden.
    pub fn put_away(&mut self) {
        self.idle();
        match &mut self.target {
            Target::Surface { surface, .. } if cfg!(target_os = "macos") => {
                if self.reserved.is_none()
                    && let wgpu::CurrentSurfaceTexture::Success(frame) = surface.get_current_texture()
                {
                    self.reserved = Some(frame);
                }
            }
            Target::Surface { surface, config } if cfg!(any(windows, target_os = "ios")) && (config.width, config.height) != (1, 1) => {
                (config.width, config.height) = (1, 1);
                surface.configure(&self.device, config);
                self.size = (1, 1);
            }
            Target::Offscreen { .. } if self.size != (1, 1) => {
                self.target = offscreen_target(&self.device, (1, 1));
                self.size = (1, 1);
            }
            _ => {}
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

    /// Waits until the GPU has done all it was given, and reads its timestamps of the frame it last drew, which `gpu`
    /// then says.
    pub fn finish(&mut self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        if let Some(stamps) = &mut self.stamps {
            stamps.read();
        }
    }

    /// How many draw calls the last frame made: each batch, shadow tile, raster and blur pass drawn, but what was kept
    /// of the frames before (`Kept`), as a GPU profiler counts them.
    pub fn draws(&self) -> u32 {
        self.draws.get()
    }

    /// Its thread's time on a CPU (`platform::on_cpu`) drawing frames since this was last asked: making their commands
    /// and handing them to the GPU, not the GPU's own time, nor waiting for it.
    pub fn drawing(&self) -> web_time::Duration {
        self.on_cpu.take()
    }

    fn drew(&self) {
        self.draws.set(self.draws.get() + 1);
    }

    /// How many render passes the last frame made: each costs a driver command lists of its own (wgpu-core records a
    /// pass and its barriers in two), which Intel's Direct3D 12 driver took 1 to 2 ms each to make ready on a Surface.
    pub fn passes(&self) -> u32 {
        self.passes.get()
    }

    /// Begins a render pass drawing into `view`, `load`ing it first, counted (`passes`).
    fn pass<'e>(&self, encoder: &'e mut wgpu::CommandEncoder, view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>, timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>) -> wgpu::RenderPass<'e> {
        self.passes.set(self.passes.get() + 1);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
            timestamp_writes,
            ..Default::default()
        })
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
            "blurred": px(work.blurred),
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

    /// The bytes of GPU memory it holds: its surface's images (three, `desired_maximum_frame_latency` 2's), or its
    /// offscreen texture and readback, and each texture of its own (a frame kept, a backdrop's, a raster, a shadow's
    /// tile), by kind.
    pub fn memory(&self) -> serde_json::Value {
        let bytes = |texture: &wgpu::Texture| u64::from(texture.width()) * u64::from(texture.height()) * u64::from(texture.format().block_copy_size(None).unwrap_or(4));
        let surface = match &self.target {
            Target::Surface { config, .. } => 3 * u64::from(config.width) * u64::from(config.height) * u64::from(config.format.block_copy_size(None).unwrap_or(4)),
            Target::Offscreen { texture, readback, .. } => bytes(texture) + readback.size(),
        };
        let backdrop = self.backdrop.iter().flat_map(|backdrop| backdrop.source.iter().chain(backdrop.levels.iter().flatten().flatten())).map(|(texture, _)| bytes(texture));
        let blurred = self.blurred.values().map(|(view, _)| bytes(view.texture()));
        serde_json::json!({
            "surface": surface,
            "kept": self.kept.as_ref().map_or(0, |kept| bytes(&kept.texture)),
            "spare": self.spare.iter().map(bytes).sum::<u64>(),
            "backdrop": backdrop.chain(blurred).sum::<u64>(),
            "rasters": self.rasters.values().map(|(texture, _)| bytes(texture)).sum::<u64>(),
            "tiles": self.tiles.values().map(|(texture, _)| bytes(texture)).sum::<u64>(),
            "buffer": self.buffer.as_ref().map_or(0, wgpu::Buffer::size),
        })
    }

    /// How long until it's drawn nothing for a while (`IDLE`), where it holds what it lets go of then (`idle`).
    pub fn idle_in(&self) -> Option<web_time::Duration> {
        (self.kept.is_some() || !self.spare.is_empty() || self.backdrop.is_some() || !self.blurred.is_empty()).then(|| IDLE.saturating_sub(self.drew.elapsed()))
    }

    /// It's drawn nothing for a while (`IDLE`): it lets go of the frame it kept and its backdrops' textures and blurs,
    /// each its window's size or a part of it (12 MB a kept frame of 1024 × 720 points at 2x), which make frames
    /// drawn one after another quicker and are made again by the next that need them, as browsers let go of an idle
    /// tab's tiles.
    pub fn idle(&mut self) {
        self.kept = None;
        self.spare.clear();
        self.backdrop = None;
        self.blurred.clear();
    }

    /// Lets go of its surface, waiting for what's drawn on it, before its window goes.
    pub fn destroy(&mut self) {
        self.reserved = None;
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
        let started = crate::platform::on_cpu();
        let drawn = self.draw_frame(scene, origin);
        self.on_cpu.set(self.on_cpu.get() + crate::platform::on_cpu().saturating_sub(started));
        drawn
    }



    /// The GPU time of the last frame it has, summed over its passes, and its blurs' (a backdrop's glass), where the
    /// GPU says (a frame or two behind).
    pub fn gpu(&self) -> Option<(web_time::Duration, web_time::Duration)> {
        self.stamps.as_ref()?.gpu
    }

    /// How long the last frame waited for an image of its surface to draw on, which a display that hasn't shown the
    /// frames before keeps it waiting for.
    pub fn acquired(&self) -> web_time::Duration {
        self.acquired
    }

    /// How long the last frame took to finish its commands and submit them, and to present its image.
    pub fn submitted(&self) -> (web_time::Duration, web_time::Duration) {
        (self.submitted, self.presented)
    }

    /// Whether the GPU hasn't finished the frame it was given last, so another would only queue behind it: a window
    /// waits to draw its next till it has, as a browser's compositor does, else what's shown falls further behind with
    /// each frame a GPU slower than the display is given (the web's on a CPU, SwiftShader's, had a page that moves for
    /// good pile up frames till a screenshot waited forever). A native surface's next image already waits for it.
    pub fn working(&self) -> bool {
        self.working.load(std::sync::atomic::Ordering::Acquire)
    }

    /// A texture of `size` in its target's format, for `usage`: one it let go of lately (`spare`), else made.
    fn made(&mut self, (width, height): (u32, u32), usage: wgpu::TextureUsages) -> wgpu::Texture {
        let fits = |texture: &wgpu::Texture| (texture.width(), texture.height()) == (width, height) && texture.format() == self.format && texture.usage() == usage;
        if let Some(at) = self.spare.iter().position(fits) {
            return self.spare.remove(at);
        }
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage,
            view_formats: &[],
        })
    }

    /// Keeps a texture a frame let go of for the next that needs one like it (`spare`), letting go of the oldest kept.
    fn let_go(&mut self, texture: wgpu::Texture) {
        if self.spare.len() >= SPARE {
            self.spare.remove(0);
        }
        self.spare.push(texture);
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

fn extent(size: Size<DevicePixels>) -> (u32, u32) {
    (size.width.0.max(1) as u32, size.height.0.max(1) as u32)
}
