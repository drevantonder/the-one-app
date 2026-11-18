// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/blade/blade_renderer.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: it draws a
// scene's quads, shadows, underlines and sprites into a window's surface, or into an offscreen texture it reads back
// for a picture; no paths, video surfaces or MSAA. Backdrops, what's behind a node blurred, are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

// Doing `if let` gives you nice scoping with passes/encoders
#![allow(irrefutable_let_patterns)]

use super::blade_atlas::BladeAtlas;
use super::context::BladeContext;
use super::scene::{Backdrop, MonochromeSprite, PolychromeSprite, PrimitiveBatch, Quad, Scene, Shadow, Underline};
use crate::geometry::{DevicePixels, Size};
use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

const MAX_FRAME_TIME_MS: u32 = 10000;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[derive(blade_macros::ShaderData)]
struct ShaderQuadsData {
    globals: GlobalParams,
    b_quads: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderShadowsData {
    globals: GlobalParams,
    b_shadows: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderUnderlinesData {
    globals: GlobalParams,
    b_underlines: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderMonoSpritesData {
    globals: GlobalParams,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_mono_sprites: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPolySpritesData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_poly_sprites: gpu::BufferPiece,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams {
    step: [f32; 2],
    sigma: f32,
    pad: f32,
}

#[derive(blade_macros::ShaderData)]
struct ShaderBlurData {
    blur: BlurParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
}

#[derive(blade_macros::ShaderData)]
struct ShaderBackdropsData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_backdrops: gpu::BufferPiece,
}

pub(crate) struct BladePipelines {
    quads: gpu::RenderPipeline,
    blur: gpu::RenderPipeline,
    backdrops: gpu::RenderPipeline,
    shadows: gpu::RenderPipeline,
    underlines: gpu::RenderPipeline,
    mono_sprites: gpu::RenderPipeline,
    poly_sprites: gpu::RenderPipeline,
}

impl BladePipelines {
    fn new(gpu: &gpu::Context, surface_info: gpu::SurfaceInfo) -> Self {
        use gpu::ShaderData as _;

        let shader = gpu.create_shader(gpu::ShaderDesc { source: include_str!("shaders.wgsl"), naga_module: None });
        shader.check_struct_size::<GlobalParams>();
        shader.check_struct_size::<Quad>();
        shader.check_struct_size::<Shadow>();
        shader.check_struct_size::<Underline>();
        shader.check_struct_size::<MonochromeSprite>();
        shader.check_struct_size::<PolychromeSprite>();
        shader.check_struct_size::<Backdrop>();
        shader.check_struct_size::<BlurParams>();

        // See https://apoorvaj.io/alpha-compositing-opengl-blending-and-premultiplied-alpha/
        let blend_mode = match surface_info.alpha {
            gpu::AlphaMode::Ignored => gpu::BlendState::ALPHA_BLENDING,
            gpu::AlphaMode::PreMultiplied => gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            gpu::AlphaMode::PostMultiplied => gpu::BlendState::ALPHA_BLENDING,
        };
        let color_targets = &[gpu::ColorTargetState {
            format: surface_info.format,
            blend: Some(blend_mode),
            write_mask: gpu::ColorWrites::default(),
        }];
        let pipeline = |layout: &gpu::ShaderDataLayout, vertex: &str, fragment: &str| {
            gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "",
                data_layouts: &[layout],
                vertex: shader.at(vertex),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState { topology: gpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                fragment: Some(shader.at(fragment)),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            })
        };

        Self {
            quads: pipeline(&ShaderQuadsData::layout(), "vs_quad", "fs_quad"),
            shadows: pipeline(&ShaderShadowsData::layout(), "vs_shadow", "fs_shadow"),
            underlines: pipeline(&ShaderUnderlinesData::layout(), "vs_underline", "fs_underline"),
            mono_sprites: pipeline(&ShaderMonoSpritesData::layout(), "vs_mono_sprite", "fs_mono_sprite"),
            poly_sprites: pipeline(&ShaderPolySpritesData::layout(), "vs_poly_sprite", "fs_poly_sprite"),
            backdrops: pipeline(&ShaderBackdropsData::layout(), "vs_backdrop", "fs_backdrop"),
            // Each pass of a blur replaces what's in its target.
            blur: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "",
                data_layouts: &[&ShaderBlurData::layout()],
                vertex: shader.at("vs_blur"),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState { topology: gpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                fragment: Some(shader.at("fs_blur")),
                color_targets: &[gpu::ColorTargetState { format: surface_info.format, blend: None, write_mask: gpu::ColorWrites::default() }],
                multisample_state: gpu::MultisampleState::default(),
            }),
        }
    }

    /// The pipelines for targets of `info`'s format, compiled the first time a renderer draws to one.
    fn shared(context: &BladeContext, info: gpu::SurfaceInfo) -> Arc<BladePipelines> {
        let mut compiled = context.pipelines.lock().expect("nothing panics holding it");
        if let Some((_, _, pipelines)) = compiled.iter().find(|(format, alpha, _)| *format == info.format && *alpha == info.alpha) {
            return pipelines.clone();
        }
        let pipelines = Arc::new(BladePipelines::new(&context.gpu, info));
        compiled.push((info.format, info.alpha, pipelines.clone()));
        pipelines
    }
}

/// Where a frame is drawn: a window's surface, or a texture of our own, which `read` reads back.
enum Target {
    Surface { surface: gpu::Surface, config: gpu::SurfaceConfig },
    Offscreen { texture: gpu::Texture, view: gpu::TextureView, readback: gpu::Buffer, size: gpu::Extent },
}

/// The format an offscreen frame is drawn in: 8-bit RGBA, as a PNG's pixels are.
const OFFSCREEN_FORMAT: gpu::TextureFormat = gpu::TextureFormat::Rgba8Unorm;

pub struct BladeRenderer {
    gpu: Arc<gpu::Context>,
    target: Target,
    command_encoder: gpu::CommandEncoder,
    last_sync_point: Option<gpu::SyncPoint>,
    pipelines: Arc<BladePipelines>,
    /// The format of the target, which the pipelines and a backdrop's textures are for.
    pipelines_format: gpu::TextureFormat,
    instance_belt: BufferBelt,
    atlas: Arc<BladeAtlas>,
    atlas_sampler: gpu::Sampler,
    rendering_parameters: RenderingParameters,
    /// What a backdrop blurs: the frame as drawn so far, halved and halved again, then blurred one way and the other.
    backdrop: Option<BackdropTextures>,
}

/// How many times a backdrop's frame is halved at most: a sixteenth of it, for the widest blur.
const LEVELS: usize = 4;

/// The textures a backdrop's blur is drawn through, for frames of `size`: for each level (half its size, a quarter,
/// …) the frame shrunk to it and the two passes of its blur, each made as a blur first needs it, as only the widest
/// blur's last level has the two passes; and which level the last blur's in. The frame itself is read as it is.
struct BackdropTextures {
    size: gpu::Extent,
    format: gpu::TextureFormat,
    levels: [[Option<(gpu::Texture, gpu::TextureView)>; 3]; LEVELS],
    blurred: usize,
}

impl BackdropTextures {
    fn all(&self) -> impl Iterator<Item = (gpu::Texture, gpu::TextureView)> + '_ {
        self.levels.iter().flatten().flatten().copied()
    }
}

impl BladeRenderer {
    /// A renderer for a window, by its raw handles.
    pub fn new<I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle>(
        context: &BladeContext,
        atlas: Arc<BladeAtlas>,
        window: &I,
        size: Size<DevicePixels>,
        transparent: bool,
    ) -> anyhow::Result<Self> {
        let config = gpu::SurfaceConfig {
            size: extent(size),
            // A backdrop blurs what's drawn so far, read from the frame (`blur_behind`).
            usage: gpu::TextureUsage::TARGET | gpu::TextureUsage::RESOURCE,
            display_sync: gpu::DisplaySync::Recent,
            color_space: gpu::ColorSpace::Srgb,
            allow_exclusive_full_screen: false,
            transparent,
        };
        let surface = context
            .gpu
            .create_surface_configured(window, config)
            .map_err(|err| anyhow::anyhow!("Failed to create surface: {err:?}"))?;
        let info = surface.info();
        Ok(Self::with_target(context, atlas, Target::Surface { surface, config }, info))
    }

    /// A renderer that draws offscreen, `size` device pixels, read back by `read`.
    pub fn offscreen(context: &BladeContext, atlas: Arc<BladeAtlas>, size: Size<DevicePixels>) -> Self {
        let size = extent(size);
        let texture = context.gpu.create_texture(gpu::TextureDesc {
            name: "",
            format: OFFSCREEN_FORMAT,
            size,
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage: gpu::TextureUsage::COPY | gpu::TextureUsage::TARGET | gpu::TextureUsage::RESOURCE,
            external: None,
        });
        let view = context.gpu.create_texture_view(
            texture,
            gpu::TextureViewDesc { name: "", format: OFFSCREEN_FORMAT, dimension: gpu::ViewDimension::D2, subresources: &Default::default() },
        );
        let readback = context.gpu.create_buffer(gpu::BufferDesc {
            name: "",
            size: u64::from(size.width) * u64::from(size.height) * 4,
            memory: gpu::Memory::Shared,
        });
        let info = gpu::SurfaceInfo { format: OFFSCREEN_FORMAT, alpha: gpu::AlphaMode::PostMultiplied };
        Self::with_target(context, atlas, Target::Offscreen { texture, view, readback, size }, info)
    }

    /// A renderer drawing into `target`, from `atlas`, which every renderer on the context shares.
    fn with_target(context: &BladeContext, atlas: Arc<BladeAtlas>, target: Target, info: gpu::SurfaceInfo) -> Self {
        let command_encoder = context.gpu.create_command_encoder(gpu::CommandEncoderDesc { name: "", buffer_count: 2, manual_barriers: false });
        let rendering_parameters = RenderingParameters::from_env();
        let pipelines = BladePipelines::shared(context, info);
        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40, // Vulkan `minStorageBufferOffsetAlignment` on Intel Xe
            target: gpu::BufferTarget::Data,
        });
        let atlas_sampler = context.gpu.create_sampler(gpu::SamplerDesc {
            name: "",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            gpu: Arc::clone(&context.gpu),
            target,
            command_encoder,
            last_sync_point: None,
            pipelines,
            pipelines_format: info.format,
            instance_belt,
            atlas,
            atlas_sampler,
            rendering_parameters,
            backdrop: None,
        }
    }

    fn wait_for_gpu(&mut self) {
        if let Some(last_sp) = self.last_sync_point.take() {
            match self.gpu.wait_for(&last_sp, MAX_FRAME_TIME_MS) {
                Ok(true) => {}
                Ok(false) => {
                    log::error!("GPU hung: {:?}", self.gpu.device_information());
                    while self.gpu.wait_for(&last_sp, MAX_FRAME_TIME_MS) == Ok(false) {}
                }
                Err(error) => log::error!("GPU lost: {error}"),
            }
        }
    }

    /// The window's new size, in device pixels.
    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let size = extent(size);
        if let Target::Surface { surface, config } = &mut self.target
            && config.size != size
        {
            if let Some(last_sp) = self.last_sync_point.take() {
                let _ = self.gpu.wait_for(&last_sp, MAX_FRAME_TIME_MS);
            }
            config.size = size;
            self.gpu.reconfigure_surface(surface, *config);
        }
    }

    pub fn viewport_size(&self) -> gpu::Extent {
        match &self.target {
            Target::Surface { config, .. } => config.size,
            Target::Offscreen { size, .. } => *size,
        }
    }

    pub fn sprite_atlas(&self) -> &Arc<BladeAtlas> {
        &self.atlas
    }

    pub fn destroy(&mut self) {
        self.wait_for_gpu();
        self.gpu.destroy_sampler(self.atlas_sampler);
        if let Some(backdrop) = self.backdrop.take() {
            for (texture, view) in backdrop.all() {
                self.gpu.destroy_texture_view(view);
                self.gpu.destroy_texture(texture);
            }
        }
        self.instance_belt.destroy(&self.gpu);
        self.gpu.destroy_command_encoder(&mut self.command_encoder);
        match &mut self.target {
            Target::Surface { surface, .. } => self.gpu.destroy_surface(surface),
            Target::Offscreen { texture, view, readback, .. } => {
                self.gpu.destroy_texture_view(*view);
                self.gpu.destroy_texture(*texture);
                self.gpu.destroy_buffer(*readback);
            }
        }
    }

    /// Copies `items` into the instance belt, for a shader to read as an array of its struct.
    fn instances<T>(&mut self, items: &[T]) -> gpu::BufferPiece {
        // Safety: the scene's primitives are `#[repr(C)]` structs of numbers, in the layout the shader reads.
        let bytes = unsafe { std::slice::from_raw_parts(items.as_ptr().cast::<u8>(), std::mem::size_of_val(items)) };
        self.instance_belt.alloc_bytes(bytes, &self.gpu)
    }

    pub fn draw(&mut self, scene: &Scene) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);

        let (frame, view, size, alpha) = match &mut self.target {
            Target::Surface { surface, config } => {
                let frame = surface.acquire_frame();
                let view = frame.texture_view();
                self.command_encoder.init_texture(frame.texture());
                (Some(frame), view, config.size, surface.info().alpha)
            }
            Target::Offscreen { texture, view, size, .. } => {
                self.command_encoder.init_texture(*texture);
                (None, *view, *size, gpu::AlphaMode::PostMultiplied)
            }
        };

        let globals = GlobalParams {
            viewport_size: [size.width as f32, size.height as f32],
            premultiplied_alpha: match alpha {
                gpu::AlphaMode::Ignored | gpu::AlphaMode::PostMultiplied => 0,
                gpu::AlphaMode::PreMultiplied => 1,
            },
            pad: 0,
        };

        // The frame is drawn in runs of batches, one pass each, broken at each backdrop, which blurs what's drawn
        // before it.
        let mut init = gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack);
        let mut run: Vec<PrimitiveBatch<'_>> = Vec::new();
        for batch in scene.batches() {
            let PrimitiveBatch::Backdrops(backdrops) = batch else {
                run.push(batch);
                continue;
            };
            self.draw_run(view, init, globals, &run);
            run.clear();
            init = gpu::InitOp::Load;
            for at in 0..backdrops.len() {
                self.blur_behind(view, size, backdrops[at].blur.0);
                self.draw_run(view, gpu::InitOp::Load, globals, &[PrimitiveBatch::Backdrops(&backdrops[at..at + 1])]);
            }
        }
        self.draw_run(view, init, globals, &run);

        if let Target::Offscreen { texture, readback, size, .. } = &self.target {
            let mut transfer = self.command_encoder.transfer("");
            transfer.copy_texture_to_buffer(
                gpu::TexturePiece { texture: *texture, mip_level: 0, array_layer: 0, origin: [0, 0, 0] },
                readback.at(0),
                size.width * 4,
                *size,
            );
        }
        if let Some(frame) = frame {
            self.command_encoder.present(frame);
        }
        let sync_point = self.gpu.submit(&mut self.command_encoder);

        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);

        self.wait_for_gpu();
        self.last_sync_point = Some(sync_point);
    }

    /// Draws `run`'s batches into `view` in one pass.
    fn draw_run(&mut self, view: gpu::TextureView, init: gpu::InitOp, globals: GlobalParams, run: &[PrimitiveBatch<'_>]) {
        if run.is_empty() && !matches!(init, gpu::InitOp::Clear(_)) {
            return;
        }
        // Each batch's instances first, as the pass borrows the encoder.
        let instances: Vec<gpu::BufferPiece> = run
            .iter()
            .map(|batch| match batch {
                PrimitiveBatch::Quads(quads) => self.instances(quads),
                PrimitiveBatch::Shadows(shadows) => self.instances(shadows),
                PrimitiveBatch::Underlines(underlines) => self.instances(underlines),
                PrimitiveBatch::MonochromeSprites { sprites, .. } => self.instances(sprites),
                PrimitiveBatch::PolychromeSprites { sprites, .. } => self.instances(sprites),
                PrimitiveBatch::Backdrops(backdrops) => self.instances(backdrops),
            })
            .collect();
        let blurred = self.backdrop.as_ref().and_then(|backdrop| backdrop.levels[backdrop.blurred][2]).map(|(_, view)| view);
        if let mut pass = self.command_encoder.render(
            "",
            gpu::RenderTargetSet { colors: &[gpu::RenderTarget { view, init_op: init, finish_op: gpu::FinishOp::Store }], depth_stencil: None },
        ) {
            for (batch, instances) in run.iter().zip(instances) {
                match batch {
                    PrimitiveBatch::Quads(quads) => {
                        let mut encoder = pass.with(&self.pipelines.quads);
                        encoder.bind(0, &ShaderQuadsData { globals, b_quads: instances });
                        encoder.draw(0, 4, 0, quads.len() as u32);
                    }
                    PrimitiveBatch::Shadows(shadows) => {
                        let mut encoder = pass.with(&self.pipelines.shadows);
                        encoder.bind(0, &ShaderShadowsData { globals, b_shadows: instances });
                        encoder.draw(0, 4, 0, shadows.len() as u32);
                    }
                    PrimitiveBatch::Underlines(underlines) => {
                        let mut encoder = pass.with(&self.pipelines.underlines);
                        encoder.bind(0, &ShaderUnderlinesData { globals, b_underlines: instances });
                        encoder.draw(0, 4, 0, underlines.len() as u32);
                    }
                    PrimitiveBatch::MonochromeSprites { texture_id, sprites } => {
                        let tex_info = self.atlas.get_texture_info(*texture_id);
                        let mut encoder = pass.with(&self.pipelines.mono_sprites);
                        encoder.bind(
                            0,
                            &ShaderMonoSpritesData {
                                globals,
                                gamma_ratios: self.rendering_parameters.gamma_ratios,
                                grayscale_enhanced_contrast: self.rendering_parameters.grayscale_enhanced_contrast,
                                t_sprite: tex_info.raw_view,
                                s_sprite: self.atlas_sampler,
                                b_mono_sprites: instances,
                            },
                        );
                        encoder.draw(0, 4, 0, sprites.len() as u32);
                    }
                    PrimitiveBatch::PolychromeSprites { texture_id, sprites } => {
                        let tex_info = self.atlas.get_texture_info(*texture_id);
                        let mut encoder = pass.with(&self.pipelines.poly_sprites);
                        encoder.bind(
                            0,
                            &ShaderPolySpritesData { globals, t_sprite: tex_info.raw_view, s_sprite: self.atlas_sampler, b_poly_sprites: instances },
                        );
                        encoder.draw(0, 4, 0, sprites.len() as u32);
                    }
                    PrimitiveBatch::Backdrops(backdrops) => {
                        let Some(blurred) = blurred else { continue };
                        let mut encoder = pass.with(&self.pipelines.backdrops);
                        encoder.bind(0, &ShaderBackdropsData { globals, t_sprite: blurred, s_sprite: self.atlas_sampler, b_backdrops: instances });
                        encoder.draw(0, 4, 0, backdrops.len() as u32);
                    }
                }
            }
        }
    }

    /// Blurs what's drawn into `frame` so far, `sigma` device pixels, into the backdrop's textures: shrunk, then
    /// blurred across into one at that size and down into another, each a separable Gaussian's pass.
    fn blur_behind(&mut self, frame: gpu::TextureView, size: gpu::Extent, sigma: f32) {
        let format = self.pipelines_format;
        let halved = |times: usize| gpu::Extent { width: (size.width >> times).max(1), height: (size.height >> times).max(1), depth: 1 };
        if self.backdrop.as_ref().is_none_or(|backdrop| backdrop.size != size || backdrop.format != format) {
            if let Some(backdrop) = self.backdrop.take() {
                for (texture, view) in backdrop.all() {
                    self.gpu.destroy_texture_view(view);
                    self.gpu.destroy_texture(texture);
                }
            }
            self.backdrop = Some(BackdropTextures { size, format, levels: Default::default(), blurred: 0 });
        }
        // Halved until the blur left is a few texels, each halving the average of four, so the Gaussian then takes
        // every texel it reaches: a wide blur taking texels further apart streaks what's sharp behind it.
        let level = (1..=LEVELS).find(|level| sigma / (1 << level) as f32 <= 6.0).unwrap_or(LEVELS);
        let wanted = (0..level).map(|at| (at, 0)).chain([(level - 1, 1), (level - 1, 2)]);
        let backdrop = self.backdrop.as_mut().expect("made above");
        for (at, which) in wanted {
            if backdrop.levels[at][which].is_none() {
                let texture = self.gpu.create_texture(gpu::TextureDesc {
                    name: "",
                    format,
                    size: halved(at + 1),
                    array_layer_count: 1,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: gpu::TextureDimension::D2,
                    usage: gpu::TextureUsage::RESOURCE | gpu::TextureUsage::TARGET,
                    external: None,
                });
                let view = self.gpu.create_texture_view(texture, gpu::TextureViewDesc { name: "", format, dimension: gpu::ViewDimension::D2, subresources: &Default::default() });
                self.command_encoder.init_texture(texture);
                backdrop.levels[at][which] = Some((texture, view));
            }
        }
        backdrop.blurred = level - 1;
        let view = |at: usize, which: usize| backdrop.levels[at][which].expect("made above").1;
        let mut passes = Vec::new();
        let mut from = frame;
        for at in 0..level {
            passes.push((from, view(at, 0), [0.0, 0.0], 0.0));
            from = view(at, 0);
        }
        let (across, down) = (view(level - 1, 1), view(level - 1, 2));
        let extent = halved(level);
        let sigma = sigma / (1 << level) as f32;
        passes.push((from, across, [1.0 / extent.width as f32, 0.0], sigma));
        passes.push((across, down, [0.0, 1.0 / extent.height as f32], sigma));
        for (from, into, step, sigma) in passes {
            if let mut pass = self.command_encoder.render(
                "",
                gpu::RenderTargetSet {
                    colors: &[gpu::RenderTarget { view: into, init_op: gpu::InitOp::DontCare, finish_op: gpu::FinishOp::Store }],
                    depth_stencil: None,
                },
            ) {
                let mut encoder = pass.with(&self.pipelines.blur);
                encoder.bind(0, &ShaderBlurData { blur: BlurParams { step, sigma, pad: 0.0 }, t_sprite: from, s_sprite: self.atlas_sampler });
                encoder.draw(0, 4, 0, 1);
            }
        }
    }

    /// The last frame drawn offscreen, as RGBA rows, top first.
    pub fn read(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        self.wait_for_gpu();
        let Target::Offscreen { readback, size, .. } = &self.target else { return None };
        let length = (size.width * size.height * 4) as usize;
        // Safety: the readback buffer is host-visible and the GPU is done writing it.
        let bytes = unsafe { std::slice::from_raw_parts(readback.data(), length) }.to_vec();
        Some((size.width, size.height, bytes))
    }
}

fn extent(size: Size<DevicePixels>) -> gpu::Extent {
    gpu::Extent { width: size.width.0.max(1) as u32, height: size.height.0.max(1) as u32, depth: 1 }
}

/// How glyphs' edges are drawn: GPUI's defaults, a gamma of 1.8 and no extra contrast.
struct RenderingParameters {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn from_env() -> Self {
        Self { gamma_ratios: Self::get_gamma_ratios(1.8), grayscale_enhanced_contrast: 1.0 }
    }

    // Gamma ratios for brightening/darkening edges for better contrast
    // https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.cpp#L50
    fn get_gamma_ratios(gamma: f32) -> [f32; 4] {
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

        let index = ((gamma * 10.0).round() as usize).clamp(10, 22) - 10;
        let ratios = GAMMA_INCORRECT_TARGET_RATIOS[index];

        [
            ratios[0] * NORM13,
            ratios[1] * NORM24,
            ratios[2] * NORM13,
            ratios[3] * NORM24,
        ]
    }
}
