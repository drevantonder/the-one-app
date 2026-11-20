use super::super::bindings::{self, Binding, Data, Pipeline};
use super::shadows::TILE_FORMAT;
use std::sync::{Arc, OnceLock};

/// The pipelines a format of target draws with, and the layouts of what they bind: the frame's globals (group 0), a
/// batch's instances or a blur's parameters (group 1), and the texture a batch samples (group 2).
pub(crate) struct Pipelines {
    pub(super) layouts: Layouts,
    pub(super) quads: wgpu::RenderPipeline,
    pub(super) shadows: wgpu::RenderPipeline,
    pub(super) underlines: wgpu::RenderPipeline,
    pub(super) mono_sprites: wgpu::RenderPipeline,
    pub(super) poly_sprites: wgpu::RenderPipeline,
    /// A raster's: a layer's texture drawn over what's there, premultiplied, as it was drawn over nothing.
    pub(super) rasters: wgpu::RenderPipeline,
    /// A backdrop's and its blur's, which few windows draw: compiled after the rest, on a thread of their own, so a
    /// window's first frame doesn't wait for them (`backdrops`).
    pub(super) backdrops: OnceLock<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    /// A shadow's tile's: what draws one (its blurred box, alone, into a texture of one channel), and what draws a
    /// shadow from its tile (`tiles`). Compiled after the rest, as the backdrops' are; till then shadows are worked
    /// out where they're drawn.
    pub(super) tiles: OnceLock<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
}

/// What every pipeline of a format is made from: the shaders, the target's format and blending, and the layouts of
/// what the pipelines bind.
pub(super) struct Layouts {
    pub(super) device: wgpu::Device,
    /// The shaders, where they're WGSL (`stages`).
    #[cfg(not(target_os = "windows"))]
    pub(super) shader: wgpu::ShaderModule,
    pub(super) format: wgpu::TextureFormat,
    pub(super) blend: wgpu::BlendState,
    pub(super) globals: wgpu::BindGroupLayout,
    pub(super) instances: wgpu::BindGroupLayout,
    pub(super) blur_params: wgpu::BindGroupLayout,
    pub(super) texture: wgpu::BindGroupLayout,
    /// A 64×64 tile of blue noise (`blue-noise.png`, made by Ulichney's void-and-cluster, σ 1.5), which the shaders
    /// dither what would band with (`dither`), bound with the globals.
    pub(super) noise: wgpu::TextureView,
}

impl Layouts {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, premultiplied: bool) -> Self {
        #[cfg(not(target_os = "windows"))]
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders.wgsl").into()),
        });
        // A batch's instances are at an offset of their own in the frame's data, which one bind group of it gives.
        let layout = |bindings: &[Binding]| {
            let entries: Vec<_> = bindings
                .iter()
                .enumerate()
                .map(|(binding, kind)| wgpu::BindGroupLayoutEntry {
                    binding: binding as u32,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: match *kind {
                        Binding::Uniform => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        Binding::Storage => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: true, min_binding_size: None },
                        Binding::Texture { filterable } => wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        Binding::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    },
                    count: None,
                })
                .collect();
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries })
        };
        let (globals, texture) = (layout(bindings::GLOBALS), layout(bindings::TEXTURE));
        let (instances, blur_params) = (layout(Data::Instances.bindings()), layout(Data::BlurParams.bindings()));
        // See https://apoorvaj.io/alpha-compositing-opengl-blending-and-premultiplied-alpha/
        let blend = if premultiplied { wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING } else { wgpu::BlendState::ALPHA_BLENDING };
        let pixels = image::load_from_memory(include_bytes!("../blue-noise.png")).expect("it's a PNG").to_luma8();
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
        Self {
            device: device.clone(),
            #[cfg(not(target_os = "windows"))]
            shader,
            format,
            blend,
            globals,
            instances,
            blur_params,
            texture,
            noise,
        }
    }

    /// The frame's globals and its gamma, at their places in `buffer`, with the noise.
    pub(super) fn globals(&self, buffer: &wgpu::Buffer, globals: (u64, u64), gamma: (u64, u64)) -> wgpu::BindGroup {
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

    /// `pipeline`, binding its data as group 1, and where it samples a texture, the texture as group 2.
    fn pipeline(&self, pipeline: &Pipeline, blend: Option<wgpu::BlendState>) -> wgpu::RenderPipeline {
        self.pipeline_into(self.format, pipeline, blend)
    }

    /// `pipeline`, drawing into a target of `format`.
    fn pipeline_into(&self, format: wgpu::TextureFormat, pipeline: &Pipeline, blend: Option<wgpu::BlendState>) -> wgpu::RenderPipeline {
        let data = match pipeline.data {
            Data::Instances => &self.instances,
            Data::BlurParams => &self.blur_params,
        };
        let groups = [Some(&self.globals), Some(data), Some(&self.texture)];
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &groups[..if pipeline.samples { 3 } else { 2 }],
            immediate_size: 0,
        });
        let [vertex, fragment] = self.stages(pipeline);
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &vertex, entry_point: Some(pipeline.vertex), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
                entry_point: Some(pipeline.fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

// Each pipeline's stages as DXIL, by its shaders' names (`build.rs`).
#[cfg(target_os = "windows")]
include!(concat!(env!("OUT_DIR"), "/dxil.rs"));

impl Layouts {
    /// The modules of `pipeline`'s two stages: on Windows, the DXIL `build.rs` compiled, through wgpu's shader
    /// passthrough, so no shader compiler runs on the device; elsewhere, the WGSL, which wgpu translates for the GPU.
    #[cfg(target_os = "windows")]
    fn stages(&self, pipeline: &Pipeline) -> [wgpu::ShaderModule; 2] {
        let (_, _, vertex, fragment) = DXIL.iter().find(|(vertex, fragment, ..)| (*vertex, *fragment) == (pipeline.vertex, pipeline.fragment)).expect("build.rs compiles every pipeline in bindings::PIPELINES");
        [(pipeline.vertex, *vertex), (pipeline.fragment, *fragment)].map(|(entry, dxil)| {
            // Safety: it's what build.rs compiled from shaders.wgsl, bound as wgpu-hal binds this pipeline's layout.
            unsafe {
                self.device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    entry_points: std::borrow::Cow::Owned(vec![wgpu::PassthroughShaderEntryPoint { name: entry.into(), workgroup_size: (0, 0, 0) }]),
                    dxil: Some(std::borrow::Cow::Borrowed(dxil)),
                    ..Default::default()
                })
            }
        })
    }

    #[cfg(not(target_os = "windows"))]
    fn stages(&self, _pipeline: &Pipeline) -> [wgpu::ShaderModule; 2] {
        [self.shader.clone(), self.shader.clone()]
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
        // As long as a device a harness stands for takes to compile them (`fake::costs`).
        #[cfg(feature = "fake")]
        std::thread::sleep(crate::platform::fake::costs().pipelines);
        let layouts = Layouts::new(device, queue, format, premultiplied);
        let (l, blend) = (&layouts, Some(layouts.blend));
        let [quads, shadows, underlines, mono_sprites, poly_sprites, rasters] = side_by_side([
            &|| l.pipeline(&bindings::QUADS, blend),
            &|| l.pipeline(&bindings::SHADOWS, blend),
            &|| l.pipeline(&bindings::UNDERLINES, blend),
            &|| l.pipeline(&bindings::MONO_SPRITES, blend),
            &|| l.pipeline(&bindings::POLY_SPRITES, blend),
            &|| l.pipeline(&bindings::RASTERS, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)),
        ]);
        Self { layouts, quads, shadows, underlines, mono_sprites, poly_sprites, rasters, backdrops: OnceLock::new(), tiles: OnceLock::new() }
    }

    /// A shadow's tile's pipelines (`tiles`), compiled now unless they are, or waited for where they're being.
    pub(super) fn tiles(&self) -> &(wgpu::RenderPipeline, wgpu::RenderPipeline) {
        self.tiles.get_or_init(|| {
            let l = &self.layouts;
            // A tile holds how much of each point the blurred box covers, nothing else, and replaces what's there.
            let coverage = l.pipeline_into(TILE_FORMAT, &bindings::SHADOW_COVERAGE, None);
            (coverage, l.pipeline(&bindings::SHADOW_TILES, Some(l.blend)))
        })
    }

    /// A backdrop's pipeline and its blur's, compiled now unless they are, or waited for where they're being.
    pub(super) fn backdrops(&self) -> &(wgpu::RenderPipeline, wgpu::RenderPipeline) {
        self.backdrops.get_or_init(|| {
            let l = &self.layouts;
            // Each pass of a blur replaces what's in its target.
            let [backdrops, blur] = side_by_side([&|| l.pipeline(&bindings::BACKDROPS, Some(l.blend)), &|| l.pipeline(&bindings::BLUR, None)]);
            (backdrops, blur)
        })
    }

    /// The pipelines for targets of `format`, compiled the first time a renderer draws to one, or ahead of it
    /// (`Context::prepare`), which one that comes meanwhile waits for: only it, as each format's are compiled apart.
    pub(crate) fn shared(device: &wgpu::Device, queue: &wgpu::Queue, compiled: &crate::render::context::Compiled, format: wgpu::TextureFormat, premultiplied: bool) -> Arc<Pipelines> {
        Self::slot(compiled, format, premultiplied)
            .get_or_init(|| {
                let pipelines = Arc::new(Pipelines::new(device, queue, format, premultiplied));
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
            })
            .clone()
    }

    /// Where the pipelines for targets of `format` are kept, once they're compiled.
    pub(crate) fn slot(compiled: &crate::render::context::Compiled, format: wgpu::TextureFormat, premultiplied: bool) -> Arc<std::sync::OnceLock<Arc<Pipelines>>> {
        let mut compiled = compiled.lock().expect("nothing panics holding it");
        if let Some((_, _, slot)) = compiled.iter().find(|(f, p, _)| *f == format && *p == premultiplied) {
            return slot.clone();
        }
        let slot = Arc::default();
        compiled.push((format, premultiplied, Arc::clone(&slot)));
        slot
    }
}
