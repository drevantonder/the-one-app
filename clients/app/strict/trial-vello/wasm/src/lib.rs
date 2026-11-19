//! Trial, not shipped: the smallest web build of each renderer drawing a rounded box and a line of text into a
//! canvas on WebGPU, to weigh it (`wasm32-unknown-unknown`, `opt-level = "z"`, wasm-opt -Oz, Brotli).

use wasm_bindgen::prelude::*;

/// Draws into `canvas` with `font` (the UI library's Allo, as TTF), and says how long its first frame took.
#[wasm_bindgen]
pub async fn draw(canvas: web_sys::HtmlCanvasElement, font: Vec<u8>, text: String) -> Result<f64, JsValue> {
    let error = |error: &dyn std::fmt::Display| JsValue::from_str(&error.to_string());
    let started = js_sys_now();
    let (width, height) = (canvas.width(), canvas.height());
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::BROWSER_WEBGPU, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| error(&e))?;
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() }).await.map_err(|e| error(&e))?;
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.map_err(|e| error(&e))?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    surface.configure(&device, &wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Auto,
    });
    let frame = match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
        _ => return Err(JsValue::from_str("no frame to draw into")),
    };
    let view = frame.texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer::draw(&device, &queue, &mut encoder, &view, format, [width, height], &font, &text).map_err(|e| error(&e))?;
    queue.submit([encoder.finish()]);
    queue.present(frame);
    Ok(js_sys_now() - started)
}

#[wasm_bindgen(inline_js = "export function now() { return performance.now() }")]
extern "C" {
    #[wasm_bindgen(js_name = now)]
    fn js_sys_now() -> f64;
}

#[cfg(any(feature = "hybrid", feature = "classic", feature = "ours"))]
fn lay_out(font: &[u8], text: &str) -> parley::Layout<[f32; 4]> {
    let mut fonts = parley::FontContext::default();
    let families = fonts.collection.register_fonts(parley::fontique::Blob::new(std::sync::Arc::new(font.to_vec())), None);
    let name = fonts.collection.family_name(families[0].0).unwrap_or_default().to_owned();
    let mut layouts = parley::LayoutContext::new();
    let mut builder = layouts.ranged_builder(&mut fonts, text, 2.0, true);
    builder.push_default(parley::StyleProperty::FontFamily(parley::FontFamily::named(&name)));
    builder.push_default(parley::StyleProperty::FontSize(17.0));
    builder.push_default(parley::StyleProperty::Brush([0.1, 0.1, 0.12, 1.0]));
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(600.0));
    layout
}

#[cfg(feature = "hybrid")]
mod renderer {
    use vello_common::kurbo::{Affine, Rect, RoundedRect, Shape};
    pub fn draw(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, format: wgpu::TextureFormat, [width, height]: [u32; 2], font: &[u8], text: &str) -> Result<(), String> {
        let (width, height) = (width as u16, height as u16);
        let (mut renderer, mut resources) = vello_gpu::Renderer::new(device, queue, &vello_gpu::RenderTargetConfig { format, width, height });
        let depth = renderer.create_depth_texture_view(&vello_gpu::RenderSize { width, height });
        let mut scene = vello_gpu::Scene::new(width, height);
        scene.set_paint(vello_common::peniko::Color::new([0.97, 0.97, 0.98, 1.0]));
        scene.fill_path(&RoundedRect::from_rect(Rect::new(16.0, 16.0, 400.0, 120.0), 24.0).to_path(0.1));
        scene.fill_blurred_rounded_rect(&Rect::new(16.0, 140.0, 400.0, 240.0), 24.0, 8.0);
        let layout = super::lay_out(font, text);
        scene.set_transform(Affine::translate((32.0, 40.0)));
        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                let run = glyph_run.run();
                scene.set_paint(vello_common::peniko::Color::new(glyph_run.style().brush));
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                let _ = scene.glyph_run(&mut resources, run.font()).font_size(run.font_size()).normalized_coords(&coords).fill_glyphs(glyph_run.positioned_glyphs().map(|g| glifo::Glyph { id: g.id, x: g.x, y: g.y }));
            }
        }
        renderer.render(&scene, &mut resources, encoder, vello_gpu::RenderTarget { view, depth_view: Some(&depth) }, &vello_gpu::TextureBindings::new(), vello_gpu::RenderOptions::default()).map_err(|e| format!("{e:?}"))
    }
}

#[cfg(feature = "classic")]
mod renderer {
    use vello::kurbo::{Affine, Rect, RoundedRect};
    use vello::peniko::{Color, Fill};
    pub fn draw(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, format: wgpu::TextureFormat, [width, height]: [u32; 2], font: &[u8], text: &str) -> Result<(), String> {
        let mut renderer = vello::Renderer::new(device, vello::RendererOptions { use_cpu: false, antialiasing_support: vello::AaSupport::area_only(), num_init_threads: None, pipeline_cache: None }).map_err(|e| e.to_string())?;
        let mut scene = vello::Scene::new();
        scene.fill(Fill::NonZero, Affine::IDENTITY, Color::new([0.97, 0.97, 0.98, 1.0]), None, &RoundedRect::from_rect(Rect::new(16.0, 16.0, 400.0, 120.0), 24.0));
        scene.draw_blurred_rounded_rect(Affine::IDENTITY, Rect::new(16.0, 140.0, 400.0, 240.0), Color::BLACK, 24.0, 8.0);
        let layout = super::lay_out(font, text);
        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                let run = glyph_run.run();
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                scene.draw_glyphs(run.font()).font_size(run.font_size()).transform(Affine::translate((32.0, 40.0))).normalized_coords(&coords).brush(Color::new(glyph_run.style().brush)).draw(Fill::NonZero, glyph_run.positioned_glyphs().map(|g| vello::Glyph { id: g.id, x: g.x, y: g.y }));
            }
        }
        // The compute renderer draws into a storage texture, which is then copied to the canvas.
        let target = device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let target_view = target.create_view(&Default::default());
        renderer.render_to_texture(device, queue, &scene, &target_view, &vello::RenderParams { base_color: Color::WHITE, width, height, antialiasing_method: vello::AaConfig::Area }).map_err(|e| e.to_string())?;
        let blitter = wgpu::util::TextureBlitter::new(device, format);
        blitter.copy(device, encoder, &target_view, view);
        Ok(())
    }
}

#[cfg(feature = "ours")]
mod renderer {
    /// Our shaders and pipelines (the scene's batches draw with them), glyphs rasterized by swash into an atlas packed by
    /// etagere, laid out by Parley: what ours carries onto the web once it's on wgpu.
    pub fn draw(device: &wgpu::Device, _queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, format: wgpu::TextureFormat, _size: [u32; 2], font: &[u8], text: &str) -> Result<(), String> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(include_str!("../../../src/render/shaders.wgsl").into()) });
        let layout = super::lay_out(font, text);
        let mut atlas = etagere::BucketedAtlasAllocator::new(etagere::size2(1024, 1024));
        let mut context = swash::scale::ScaleContext::new();
        let face = swash::FontRef::from_index(font, 0).ok_or("font")?;
        let mut scaler = context.builder(face).size(34.0).build();
        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                for glyph in glyph_run.positioned_glyphs() {
                    if let Some(image) = swash::scale::Render::new(&[swash::scale::Source::Outline]).render(&mut scaler, glyph.id as u16) {
                        let _ = atlas.allocate(etagere::size2(image.placement.width as i32 + 1, image.placement.height as i32 + 1));
                    }
                }
            }
        }
        for (vertex, fragment) in [("vs_quad", "fs_quad"), ("vs_shadow", "fs_shadow"), ("vs_underline", "fs_underline"), ("vs_mono_sprite", "fs_mono_sprite"), ("vs_poly_sprite", "fs_poly_sprite"), ("vs_blur", "fs_blur"), ("vs_backdrop", "fs_backdrop")] {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vertex), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fragment), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        }
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        Ok(())
    }
}

#[cfg(not(any(feature = "hybrid", feature = "classic", feature = "ours")))]
mod renderer {
    pub fn draw(_: &wgpu::Device, _: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, _: wgpu::TextureFormat, _: [u32; 2], _: &[u8], _: &str) -> Result<(), String> {
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        Ok(())
    }
}
