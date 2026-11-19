// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_renderer.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host as its Blade renderer was: it draws a
// scene's quads, shadows, underlines and sprites into a window's surface, or into an offscreen texture it reads back
// for a picture; no paths, video surfaces, subpixel text or MSAA. Backdrops, what's behind a node blurred, are ours.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use super::scene::{PrimitiveBatch, Scene};
use super::wgpu_atlas::WgpuAtlas;
use super::wgpu_context::WgpuContext;
use crate::geometry::{DevicePixels, Size};
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
pub(crate) struct WgpuPipelines {
    layouts: Layouts,
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    poly_sprites: wgpu::RenderPipeline,
    /// A backdrop's and its blur's, which few windows draw: compiled after the rest, on a thread of their own, so a
    /// window's first frame doesn't wait for them (`backdrops`).
    backdrops: OnceLock<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
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
}

impl Layouts {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, premultiplied: bool) -> Self {
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
        let globals = layout(&[uniform(0), uniform(1)]);
        let instances = layout(&[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
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
        Self { device: device.clone(), shader, format, blend, globals, instances, blur_params, texture }
    }

    /// A pipeline of `vertex` and `fragment` binding `data` as group 1, and where it samples a texture, the texture
    /// as group 2.
    fn pipeline(&self, data: &wgpu::BindGroupLayout, vertex: &str, fragment: &str, blend: Option<wgpu::BlendState>, samples: bool) -> wgpu::RenderPipeline {
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
                targets: &[Some(wgpu::ColorTargetState { format: self.format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

impl WgpuPipelines {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, premultiplied: bool) -> Self {
        let layouts = Layouts::new(device, format, premultiplied);
        let (l, blend) = (&layouts, Some(layouts.blend));
        // Compiled side by side, as each takes its compiler a while (Direct3D's most).
        let [quads, shadows, underlines, mono_sprites, poly_sprites] = std::thread::scope(|scope| {
            [
                scope.spawn(|| l.pipeline(&l.instances, "vs_quad", "fs_quad", blend, false)),
                scope.spawn(|| l.pipeline(&l.instances, "vs_shadow", "fs_shadow", blend, false)),
                scope.spawn(|| l.pipeline(&l.instances, "vs_underline", "fs_underline", blend, false)),
                scope.spawn(|| l.pipeline(&l.instances, "vs_mono_sprite", "fs_mono_sprite", blend, true)),
                scope.spawn(|| l.pipeline(&l.instances, "vs_poly_sprite", "fs_poly_sprite", blend, true)),
            ]
            .map(|compiling| compiling.join().expect("compiling a pipeline doesn't panic"))
        });
        Self { layouts, quads, shadows, underlines, mono_sprites, poly_sprites, backdrops: OnceLock::new() }
    }

    /// A backdrop's pipeline and its blur's, compiled now unless they are, or waited for where they're being.
    fn backdrops(&self) -> &(wgpu::RenderPipeline, wgpu::RenderPipeline) {
        self.backdrops.get_or_init(|| {
            let l = &self.layouts;
            std::thread::scope(|scope| {
                let backdrops = scope.spawn(|| l.pipeline(&l.instances, "vs_backdrop", "fs_backdrop", Some(l.blend), true));
                // Each pass of a blur replaces what's in its target.
                let blur = l.pipeline(&l.blur_params, "vs_blur", "fs_blur", None, true);
                (backdrops.join().expect("compiling a pipeline doesn't panic"), blur)
            })
        })
    }

    /// The pipelines for targets of `format`, compiled the first time a renderer draws to one.
    fn shared(context: &WgpuContext, format: wgpu::TextureFormat, premultiplied: bool) -> Arc<WgpuPipelines> {
        let mut compiled = context.pipelines.lock().expect("nothing panics holding it");
        if let Some((_, _, pipelines)) = compiled.iter().find(|(f, p, _)| *f == format && *p == premultiplied) {
            return pipelines.clone();
        }
        let pipelines = Arc::new(WgpuPipelines::new(&context.device, format, premultiplied));
        compiled.push((format, premultiplied, pipelines.clone()));
        let compiling = pipelines.clone();
        std::thread::spawn(move || {
            compiling.backdrops();
        });
        pipelines
    }
}

/// Where a frame is drawn: a window's surface, or a texture of our own, which `read` reads back.
enum Target {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Offscreen { texture: wgpu::Texture, view: wgpu::TextureView, readback: wgpu::Buffer, row: u32 },
}

/// The format an offscreen frame is drawn in: 8-bit RGBA, as a PNG's pixels are.
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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

pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    target: Target,
    pipelines: Arc<WgpuPipelines>,
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
    atlas: Arc<WgpuAtlas>,
    sampler: wgpu::Sampler,
    gamma: GammaParams,
    /// What a backdrop blurs: the frame as drawn so far, halved and halved again, then blurred one way and the other.
    backdrop: Option<BackdropTextures>,
}

impl WgpuRenderer {
    /// A renderer for a window, drawing into `target`, its surface as its platform gives it.
    pub fn new(
        context: &WgpuContext,
        atlas: Arc<WgpuAtlas>,
        target: wgpu::SurfaceTargetUnsafe,
        size: Size<DevicePixels>,
        transparent: bool,
    ) -> anyhow::Result<Self> {
        // Safety: a window's renderer is let go before its window is.
        let surface = unsafe { context.instance.create_surface_unsafe(target) }?;
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
        // The most recent frame shown, as soon as it's drawn, as Blade's `DisplaySync::Recent` did.
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::FifoRelaxed, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|mode| capabilities.present_modes.contains(mode))
            .unwrap_or(wgpu::PresentMode::Fifo);
        let (width, height) = extent(size);
        let config = wgpu::SurfaceConfiguration {
            // A backdrop blurs what's drawn so far, read from the frame where the surface lets a shader read it
            // (Vulkan's), else copied out of it (`blur_behind`).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if capabilities.usages.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
                    wgpu::TextureUsages::TEXTURE_BINDING
                } else {
                    capabilities.usages & wgpu::TextureUsages::COPY_SRC
                },
            format,
            width,
            height,
            present_mode,
            // Two images, one shown and one drawn (wgpu keeps one more than this), where Blade kept three: a
            // window's surface is a third smaller (the launcher's on a Mac, 1840×1502, ~11 MB), and as a window
            // draws only as it changes, no frame waits for a third.
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&context.device, &config);
        let premultiplied = alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
        Ok(Self::with_target(context, atlas, Target::Surface { surface, config }, format, premultiplied, (width, height)))
    }

    /// A renderer that draws offscreen, `size` device pixels, read back by `read`.
    pub fn offscreen(context: &WgpuContext, atlas: Arc<WgpuAtlas>, size: Size<DevicePixels>) -> Self {
        let (width, height) = extent(size);
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
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
    fn with_target(context: &WgpuContext, atlas: Arc<WgpuAtlas>, target: Target, format: wgpu::TextureFormat, premultiplied: bool, size: (u32, u32)) -> Self {
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
            pipelines: WgpuPipelines::shared(context, format, premultiplied),
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

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
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
    pub fn draw(&mut self, scene: &Scene, origin: [f32; 2]) {
        self.atlas.before_frame();
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
                let frame = match frame {
                    wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return,
                };
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let texture = frame.texture.clone();
                (Some(frame), view, texture)
            }
            Target::Offscreen { texture, view, .. } => (None, view.clone(), texture.clone()),
        };
        let size = (texture.width(), texture.height());
        let blurrable = texture.usage().intersects(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);

        // The frame's data: its globals, then each batch's instances, then each backdrop's blur's parameters.
        self.data.clear();
        let globals = GlobalParams { viewport_size: [size.0 as f32, size.1 as f32], origin, premultiplied_alpha: u32::from(self.premultiplied), pad: [0; 3] };
        let globals = self.push(bytemuck::bytes_of(&globals));
        let gamma = self.gamma;
        let gamma = self.push(bytemuck::bytes_of(&gamma));
        let batches: Vec<PrimitiveBatch<'_>> = scene.batches().collect();
        let instances: Vec<(u64, u64)> = batches
            .iter()
            .map(|batch| match batch {
                PrimitiveBatch::Quads(quads) => self.push_instances(quads),
                PrimitiveBatch::Shadows(shadows) => self.push_instances(shadows),
                PrimitiveBatch::Underlines(underlines) => self.push_instances(underlines),
                PrimitiveBatch::MonochromeSprites { sprites, .. } => self.push_instances(sprites),
                PrimitiveBatch::PolychromeSprites { sprites, .. } => self.push_instances(sprites),
                PrimitiveBatch::Backdrops(backdrops) => self.push_instances(backdrops),
            })
            .collect();
        let blurs: Vec<Vec<Blur>> = batches
            .iter()
            .map(|batch| match batch {
                PrimitiveBatch::Backdrops(backdrops) => backdrops.iter().map(|backdrop| self.plan_blur(size, backdrop.blur.0)).collect(),
                _ => Vec::new(),
            })
            .collect();
        self.write_data();
        let buffer = self.buffer.clone().expect("written above");
        let binding = |(offset, size): (u64, u64)| wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset, size: wgpu::BufferSize::new(size) });
        let globals = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipelines.layouts.globals,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: binding(globals) }, wgpu::BindGroupEntry { binding: 1, resource: binding(gamma) }],
        });
        let instances: Vec<wgpu::BindGroup> = instances
            .into_iter()
            .map(|range| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.pipelines.layouts.instances,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: binding(range) }],
                })
            })
            .collect();

        // The frame is drawn in runs of batches, one pass each, broken at each backdrop, which blurs what's drawn
        // before it.
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut load = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        let mut run: Vec<usize> = Vec::new();
        for (at, batch) in batches.iter().enumerate() {
            let PrimitiveBatch::Backdrops(backdrops) = batch else {
                run.push(at);
                continue;
            };
            self.draw_run(&mut encoder, &view, load, &globals, &batches, &instances, &run, None);
            run.clear();
            load = wgpu::LoadOp::Load;
            for (one, blur) in (0..backdrops.len()).zip(&blurs[at]) {
                let blurred = blurrable.then(|| self.blur_behind(&mut encoder, &texture, &view, &globals, &buffer, blur));
                self.draw_run(&mut encoder, &view, wgpu::LoadOp::Load, &globals, &batches, &instances, &[at], blurred.map(|view| (view, one)));
            }
        }
        self.draw_run(&mut encoder, &view, load, &globals, &batches, &instances, &run, None);

        self.queue.submit([encoder.finish()]);
        if let Some(frame) = frame {
            self.queue.present(frame);
        }
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
        instances: &[wgpu::BindGroup],
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
            ..Default::default()
        });
        pass.set_bind_group(0, globals, &[]);
        for &at in run {
            let (pipeline, texture, count) = match &batches[at] {
                PrimitiveBatch::Quads(quads) => (&self.pipelines.quads, None, quads.len()),
                PrimitiveBatch::Shadows(shadows) => (&self.pipelines.shadows, None, shadows.len()),
                PrimitiveBatch::Underlines(underlines) => (&self.pipelines.underlines, None, underlines.len()),
                PrimitiveBatch::MonochromeSprites { texture_id, sprites } => (&self.pipelines.mono_sprites, Some(self.atlas.view(*texture_id)), sprites.len()),
                PrimitiveBatch::PolychromeSprites { texture_id, sprites } => (&self.pipelines.poly_sprites, Some(self.atlas.view(*texture_id)), sprites.len()),
                PrimitiveBatch::Backdrops(_) => {
                    let Some((blurred, one)) = &blurred else { continue };
                    pass.set_pipeline(&self.pipelines.backdrops().0);
                    pass.set_bind_group(1, &instances[at], &[]);
                    pass.set_bind_group(2, &self.texture(blurred), &[]);
                    pass.draw(0..4, *one as u32..*one as u32 + 1);
                    continue;
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &instances[at], &[]);
            if let Some(view) = texture {
                pass.set_bind_group(2, &self.texture(&view), &[]);
            }
            pass.draw(0..4, 0..count as u32);
        }
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
        Blur { level, shrink, across, down }
    }

    /// Blurs what's drawn into `frame` so far, as `blur` says, into the backdrop's textures: shrunk, then blurred
    /// across into one at its level's size and down into another, each a separable Gaussian's pass. The view of the
    /// last. Only the textures this blur goes through are made, each the first time one needs it, and the frame is
    /// read as it is where its surface lets a shader read it.
    fn blur_behind(&mut self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture, view: &wgpu::TextureView, globals: &wgpu::BindGroup, buffer: &wgpu::Buffer, blur: &Blur) -> wgpu::TextureView {
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
                backdrop.levels[at][which] = Some(make(halved(size, at + 1), wgpu::TextureUsages::RENDER_ATTACHMENT));
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
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.backdrops().1);
            pass.set_bind_group(0, globals, &[]);
            pass.set_bind_group(1, &params, &[]);
            pass.set_bind_group(2, &texture, &[]);
            pass.draw(0..4, 0..1);
        }
        down.clone()
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
