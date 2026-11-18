// From Zed's GPUI (Apache-2.0, LICENSE-APACHE beside this file), crates/gpui/src/platform/blade/blade_renderer.rs and
// blade_context.rs at zed-industries/zed 8249ef56187b966c33f6667d0d3a35d88d8f2dc0.
// Changed: for Blade 3d86d53's API (a belt's target, a shader's prebuilt module, an encoder's barriers, a wait's
// error); only the pipelines a strict screen uses (quads, underlines, monochrome and polychrome sprites); the scene is
// this crate's (scene.rs); no paths, shadows, subpixel sprites or video surfaces; the rendering parameters are GPUI's
// defaults, not read from the environment; the GPU context is made once per process and shared by every screen.

use std::sync::{Arc, OnceLock};

use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use bytemuck::{Pod, Zeroable};

use crate::atlas::Atlas;
use crate::scene::{Batch, MonochromeSprite, PolychromeSprite, Quad, Scene, Underline};

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

struct Pipelines {
    quads: gpu::RenderPipeline,
    underlines: gpu::RenderPipeline,
    mono_sprites: gpu::RenderPipeline,
    poly_sprites: gpu::RenderPipeline,
}

impl Pipelines {
    fn new(gpu: &gpu::Context, surface_info: gpu::SurfaceInfo) -> Self {
        use gpu::ShaderData as _;

        let shader = gpu.create_shader(gpu::ShaderDesc { source: include_str!("shaders.wgsl"), naga_module: None });
        shader.check_struct_size::<GlobalParams>();
        shader.check_struct_size::<Quad>();
        shader.check_struct_size::<Underline>();
        shader.check_struct_size::<MonochromeSprite>();
        shader.check_struct_size::<PolychromeSprite>();

        // See https://apoorvaj.io/alpha-compositing-opengl-blending-and-premultiplied-alpha/
        let blend_mode = match surface_info.alpha {
            gpu::AlphaMode::Ignored => gpu::BlendState::ALPHA_BLENDING,
            gpu::AlphaMode::PreMultiplied => gpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            gpu::AlphaMode::PostMultiplied => gpu::BlendState::ALPHA_BLENDING,
        };
        let color_targets = &[gpu::ColorTargetState { format: surface_info.format, blend: Some(blend_mode), write_mask: gpu::ColorWrites::default() }];
        let pipeline = |name, layout: &gpu::ShaderDataLayout, vs, fs| {
            let _ = name;
            gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                // No names on GPU objects: Blade names them with VK_EXT_debug_utils, and the emulator's driver crashes
                // naming them (vk_common_SetDebugUtilsObjectNameEXT); Blade skips an empty name.
                name: "",
                data_layouts: &[layout],
                vertex: shader.at(vs),
                vertex_fetches: &[],
                primitive: gpu::PrimitiveState { topology: gpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                fragment: Some(shader.at(fs)),
                color_targets,
                multisample_state: gpu::MultisampleState::default(),
            })
        };
        Self {
            quads: pipeline("quads", &ShaderQuadsData::layout(), "vs_quad", "fs_quad"),
            underlines: pipeline("underlines", &ShaderUnderlinesData::layout(), "vs_underline", "fs_underline"),
            mono_sprites: pipeline("mono-sprites", &ShaderMonoSpritesData::layout(), "vs_mono_sprite", "fs_mono_sprite"),
            poly_sprites: pipeline("poly-sprites", &ShaderPolySpritesData::layout(), "vs_poly_sprite", "fs_poly_sprite"),
        }
    }

    fn destroy(&mut self, gpu: &gpu::Context) {
        gpu.destroy_render_pipeline(&mut self.quads);
        gpu.destroy_render_pipeline(&mut self.underlines);
        gpu.destroy_render_pipeline(&mut self.mono_sprites);
        gpu.destroy_render_pipeline(&mut self.poly_sprites);
    }
}

/// The GPU, opened once per process (GPUI's BladeContext).
pub fn context() -> anyhow::Result<Arc<gpu::Context>> {
    static GPU: OnceLock<Arc<gpu::Context>> = OnceLock::new();
    if let Some(gpu) = GPU.get() {
        return Ok(gpu.clone());
    }
    let gpu = Arc::new(
        unsafe { gpu::Context::init(gpu::ContextDesc { presentation: true, validation: false, ..Default::default() }) }
            .map_err(|e| anyhow::anyhow!("{e:?}"))?,
    );
    Ok(GPU.get_or_init(|| gpu).clone())
}

pub struct Renderer {
    gpu: Arc<gpu::Context>,
    surface: gpu::Surface,
    surface_config: gpu::SurfaceConfig,
    command_encoder: gpu::CommandEncoder,
    last_sync_point: Option<gpu::SyncPoint>,
    pipelines: Pipelines,
    instance_belt: BufferBelt,
    pub atlas: Atlas,
    atlas_sampler: gpu::Sampler,
    gamma_ratios: [f32; 4],
}

impl Renderer {
    pub fn new<I: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle>(
        gpu: Arc<gpu::Context>,
        window: &I,
        size: gpu::Extent,
    ) -> anyhow::Result<Self> {
        let surface_config = gpu::SurfaceConfig {
            size,
            usage: gpu::TextureUsage::TARGET,
            display_sync: gpu::DisplaySync::Recent,
            color_space: gpu::ColorSpace::Srgb,
            allow_exclusive_full_screen: false,
            transparent: false,
        };
        let surface = gpu.create_surface_configured(window, surface_config).map_err(|err| anyhow::anyhow!("Failed to create surface: {err:?}"))?;
        let command_encoder = gpu.create_command_encoder(gpu::CommandEncoderDesc { name: "", buffer_count: 2, manual_barriers: false });
        let pipelines = Pipelines::new(&gpu, surface.info());
        let instance_belt = BufferBelt::new(BufferBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40, // Vulkan `minStorageBufferOffsetAlignment` on Intel Xe
            target: gpu::BufferTarget::Data,
        });
        let atlas = Atlas::new(&gpu);
        let atlas_sampler = gpu.create_sampler(gpu::SamplerDesc {
            name: "",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            gpu,
            surface,
            surface_config,
            command_encoder,
            last_sync_point: None,
            pipelines,
            instance_belt,
            atlas,
            atlas_sampler,
            gamma_ratios: gamma_correction_ratios(1.8),
        })
    }

    fn wait_for_gpu(&mut self) {
        if let Some(last_sp) = self.last_sync_point.take() {
            while !self.gpu.wait_for(&last_sp, MAX_FRAME_TIME_MS).unwrap_or(true) {
                log::error!("GPU hung");
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let size = gpu::Extent { width, height, depth: 1 };
        if size != self.surface_config.size {
            self.wait_for_gpu();
            self.surface_config.size = size;
            self.gpu.reconfigure_surface(&mut self.surface, self.surface_config);
        }
    }

    pub fn draw(&mut self, scene: &Scene) {
        self.command_encoder.start();
        self.atlas.before_frame(&mut self.command_encoder);

        let frame = self.surface.acquire_frame();
        self.command_encoder.init_texture(frame.texture());

        let globals = GlobalParams {
            viewport_size: [self.surface_config.size.width as f32, self.surface_config.size.height as f32],
            premultiplied_alpha: match self.surface.info().alpha {
                gpu::AlphaMode::Ignored | gpu::AlphaMode::PostMultiplied => 0,
                gpu::AlphaMode::PreMultiplied => 1,
            },
            pad: 0,
        };

        let mut pass = self.command_encoder.render(
            "main",
            gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: frame.texture_view(),
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            },
        );

        for batch in &scene.batches {
            match batch {
                Batch::Quads(range) => {
                    let quads = &scene.quads[range.clone()];
                    let instance_buf = unsafe { self.instance_belt.alloc_typed(quads, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.quads);
                    encoder.bind(0, &ShaderQuadsData { globals, b_quads: instance_buf });
                    encoder.draw(0, 4, 0, quads.len() as u32);
                }
                Batch::Underlines(range) => {
                    let underlines = &scene.underlines[range.clone()];
                    let instance_buf = unsafe { self.instance_belt.alloc_typed(underlines, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.underlines);
                    encoder.bind(0, &ShaderUnderlinesData { globals, b_underlines: instance_buf });
                    encoder.draw(0, 4, 0, underlines.len() as u32);
                }
                Batch::MonochromeSprites { texture_id, range } => {
                    let sprites = &scene.monochrome_sprites[range.clone()];
                    let instance_buf = unsafe { self.instance_belt.alloc_typed(sprites, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.mono_sprites);
                    encoder.bind(
                        0,
                        &ShaderMonoSpritesData {
                            globals,
                            gamma_ratios: self.gamma_ratios,
                            grayscale_enhanced_contrast: 1.0,
                            t_sprite: self.atlas.view(*texture_id),
                            s_sprite: self.atlas_sampler,
                            b_mono_sprites: instance_buf,
                        },
                    );
                    encoder.draw(0, 4, 0, sprites.len() as u32);
                }
                Batch::PolychromeSprites { texture_id, range } => {
                    let sprites = &scene.polychrome_sprites[range.clone()];
                    let instance_buf = unsafe { self.instance_belt.alloc_typed(sprites, &self.gpu) };
                    let mut encoder = pass.with(&self.pipelines.poly_sprites);
                    encoder.bind(
                        0,
                        &ShaderPolySpritesData { globals, t_sprite: self.atlas.view(*texture_id), s_sprite: self.atlas_sampler, b_poly_sprites: instance_buf },
                    );
                    encoder.draw(0, 4, 0, sprites.len() as u32);
                }
            }
        }
        drop(pass);

        self.command_encoder.present(frame);
        let sync_point = self.gpu.submit(&mut self.command_encoder);

        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);

        self.wait_for_gpu();
        self.last_sync_point = Some(sync_point);
    }

    pub fn destroy(&mut self) {
        self.wait_for_gpu();
        self.atlas.destroy();
        self.gpu.destroy_sampler(self.atlas_sampler);
        self.instance_belt.destroy(&self.gpu);
        self.gpu.destroy_command_encoder(&mut self.command_encoder);
        self.pipelines.destroy(&self.gpu);
        self.gpu.destroy_surface(&mut self.surface);
    }
}

/// GPUI's `get_gamma_correction_ratios` (platform.rs).
fn gamma_correction_ratios(gamma: f32) -> [f32; 4] {
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
    [ratios[0] * NORM13, ratios[1] * NORM24, ratios[2] * NORM13, ratios[3] * NORM24]
}
