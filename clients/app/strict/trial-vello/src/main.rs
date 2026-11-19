//! Trial, not shipped: the frames the strict host's harness dumped (`screenshot --dump <dir>`) drawn again by one of
//! Vello's GPU renderers on wgpu, with text laid out by Parley, and our backdrop blur as a pass of its own beside it.
//!
//!     trial-vello <frames dir> <gpu|classic> <out.png> [--measure]
//!
//! `gpu` is `vello_gpu` (what was `vello_hybrid`: paths cut into sparse strips on the CPU, drawn in render passes);
//! `classic` is `vello`, the compute renderer. It draws `frame.json` to `out.png`, and with `--measure` draws the
//! dumped phases back to back, each frame waiting for the one before, as the harness draws ours, and says on stdout,
//! as a line `measured {…}`, each phase's frame time (median, worst), how long building the scene took (Parley
//! included), and the memory drawing took.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use parley::{FontContext, FontFamily, FontFeature, FontVariation, FontWeight, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty};
use serde_json::{Value, json};
use vello::kurbo::{Affine, BezPath, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke};
use vello::peniko::{Color, Fill};

type Rgba = [f32; 4];

/// A paragraph laid out by Parley, its glyphs in runs, at its scale, from its own top left.
struct Laid {
    runs: Vec<GlyphRun>,
}

struct GlyphRun {
    font: parley::FontData,
    size: f32,
    coords: Vec<i16>,
    color: Rgba,
    glyphs: Vec<(u32, f32, f32)>,
}

/// The text system: Allo (the UI library's typeface), then Noto Color Emoji, as the harness has them.
struct Text {
    fonts: FontContext,
    layouts: LayoutContext<Rgba>,
    families: Vec<String>,
    laid: HashMap<String, Arc<Laid>>,
}

impl Text {
    fn new() -> Result<Text> {
        let mut fonts = FontContext::new();
        let allo = wuff::decompress_woff2(include_bytes!("../../../../../core/ui/src/Allo-Variable.woff2")).map_err(|error| anyhow!("{error:?}"))?;
        let mut families = Vec::new();
        for data in [allo, std::fs::read("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf").unwrap_or_default()] {
            if data.is_empty() {
                continue;
            }
            for (family, _) in fonts.collection.register_fonts(parley::fontique::Blob::new(Arc::new(data)), None) {
                if let Some(name) = fonts.collection.family_name(family) {
                    families.push(name.to_owned());
                }
            }
        }
        Ok(Text { fonts, layouts: LayoutContext::new(), families, laid: HashMap::new() })
    }

    /// `text` (a dumped paragraph) laid out, kept by what it is, as our text system keeps shaped lines.
    /// `words` with `runs` (dumped, their ranges from `from` in the paragraph) as Parley lays them out, unbroken.
    fn build(&mut self, words: &str, from: usize, runs: &[Value], scale: f32, line_height: f32) -> parley::Layout<Rgba> {
        let names: Vec<parley::FontFamilyName> = self.families.iter().map(|name| parley::FontFamilyName::named(name)).collect();
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, words, scale, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(names.into())));
        builder.push_default(LineHeight::Absolute(line_height));
        for run in runs {
            let start = (run["start"].as_u64().unwrap_or(0) as usize).saturating_sub(from).min(words.len());
            let end = (run["end"].as_u64().unwrap_or(0) as usize).saturating_sub(from).min(words.len());
            if start >= end {
                continue;
            }
            let range = start..end;
            let size = run["size"].as_f64().unwrap_or(16.0) as f32;
            let weight = run["weight"].as_f64().unwrap_or(400.0) as f32;
            builder.push(StyleProperty::FontSize(size), range.clone());
            builder.push(StyleProperty::FontWeight(FontWeight::new(weight)), range.clone());
            let variations = [FontVariation::new(parley::setting::Tag::new(b"wght"), weight), FontVariation::new(parley::setting::Tag::new(b"opsz"), size)];
            builder.push(StyleProperty::FontVariations(parley::FontVariations::List(variations.to_vec().into())), range.clone());
            builder.push(StyleProperty::LetterSpacing(run["tracking"].as_f64().unwrap_or(0.0) as f32 * size), range.clone());
            if run["tabular"].as_bool() == Some(true) {
                builder.push(StyleProperty::FontFeatures(parley::FontFeatures::List(vec![FontFeature::new(parley::setting::Tag::new(b"tnum"), 1)].into())), range.clone());
            }
            builder.push(StyleProperty::Brush(rgba(&run["color"])), range);
        }
        builder.build(words)
    }

    /// A line's glyph runs, each glyph moved by `shift` and down by `down`, up to `room` along it.
    fn runs_of(line: parley::Line<'_, Rgba>, shift: f32, down: f32, room: f32, runs: &mut Vec<GlyphRun>) {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
            let run = glyph_run.run();
            let glyphs: Vec<(u32, f32, f32)> =
                glyph_run.positioned_glyphs().filter(|glyph| glyph.x + glyph.advance <= room + 0.01).map(|glyph| (glyph.id, glyph.x + shift, glyph.y + down)).collect();
            if glyphs.is_empty() {
                continue;
            }
            runs.push(GlyphRun {
                font: run.font().clone(),
                size: run.font_size(),
                coords: run.normalized_coords().iter().map(|coord| coord.to_bits()).collect(),
                color: glyph_run.style().brush,
                glyphs,
            });
        }
    }

    /// `text` (a dumped paragraph) laid out, kept by what it is, as our text system keeps shaped lines; cut short
    /// with an ellipsis after `max_lines` as ours is (Parley has no ellipsis of its own).
    fn lay_out(&mut self, text: &Value) -> Arc<Laid> {
        let key = json!([text["words"], text["runs"], text["width"], text["wrap"], text["max_lines"], text["align"], text["line_height"], text["scale"]]).to_string();
        if let Some(laid) = self.laid.get(&key) {
            return laid.clone();
        }
        let words = text["words"].as_str().unwrap_or_default().to_owned();
        let scale = text["scale"].as_f64().unwrap_or(1.0) as f32;
        let width = text["width"].as_f64().unwrap_or(0.0) as f32 * scale;
        let line_height = text["line_height"].as_f64().unwrap_or(16.0) as f32;
        let dumped: Vec<Value> = text["runs"].as_array().cloned().unwrap_or_default();
        let mut layout = self.build(&words, 0, &dumped, scale, line_height);
        let align = text["align"].as_f64().unwrap_or(0.0) as f32;
        let alignment = if align >= 0.75 { parley::Alignment::End } else if align >= 0.25 { parley::Alignment::Center } else { parley::Alignment::Start };
        layout.set_alignment(alignment, parley::AlignmentOptions::default());
        let wrap = text["wrap"].as_bool() == Some(true);
        let max_lines = text["max_lines"].as_u64().map(|max| max as usize);
        layout.break_all_lines(if wrap { Some(width) } else { None });
        let mut runs = Vec::new();
        // A line that doesn't wrap is aligned in the box ours gives it, not in its own width.
        let shift = if wrap { 0.0 } else { (width - layout.width()).max(0.0) * align };
        let kept = max_lines.unwrap_or(usize::MAX);
        for (at, line) in layout.lines().enumerate() {
            if at + 1 < kept {
                Self::runs_of(line, shift, 0.0, f32::INFINITY, &mut runs);
            } else if at + 1 == kept && max_lines.is_some() {
                // The last line kept, laid out on to the end and cut where an ellipsis still fits after it.
                let from = line.text_range().start;
                let baseline = line.metrics().baseline;
                let rest = words[from..].replace('\n', " ");
                let mut tail = self.build(&rest, from, &dumped, scale, line_height);
                tail.break_all_lines(None);
                let last = dumped.iter().rev().find(|run| (run["start"].as_u64().unwrap_or(0) as usize) <= from).or(dumped.last()).cloned().into_iter().collect::<Vec<_>>();
                let style: Vec<Value> = last.iter().map(|run| { let mut run = run.clone(); run["start"] = json!(0); run["end"] = json!(3); run }).collect();
                let mut dots = self.build("…", 0, &style, scale, line_height);
                dots.break_all_lines(None);
                let room = width - dots.width();
                let Some(tail_line) = tail.lines().next() else { continue };
                let down = baseline - tail_line.metrics().baseline;
                let cut = tail_line.items().filter_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => run.positioned_glyphs().filter(|glyph| glyph.x + glyph.advance <= room + 0.01).map(|glyph| glyph.x + glyph.advance).reduce(f32::max),
                    _ => None,
                }).fold(0.0, f32::max);
                Self::runs_of(tail_line, 0.0, down, room, &mut runs);
                if let Some(dots_line) = dots.lines().next() {
                    let dots_down = baseline - dots_line.metrics().baseline;
                    Self::runs_of(dots_line, cut, dots_down, f32::INFINITY, &mut runs);
                }
            }
        }
        let laid = Arc::new(Laid { runs });
        self.laid.insert(key, laid.clone());
        laid
    }
}

fn rgba(value: &Value) -> Rgba {
    let at = |index: usize| value[index].as_f64().unwrap_or(0.0) as f32;
    [at(0), at(1), at(2), at(3)]
}

fn colour(value: Rgba) -> Color {
    Color::new(value)
}

fn rect(value: &Value) -> Rect {
    let at = |index: usize| value[index].as_f64().unwrap_or(0.0);
    Rect::new(at(0), at(1), at(0) + at(2), at(1) + at(3))
}

fn radii(value: &Value) -> RoundedRectRadii {
    let at = |index: usize| value[index].as_f64().unwrap_or(0.0);
    RoundedRectRadii::new(at(0), at(1), at(2), at(3))
}

fn affine(value: &Value) -> Affine {
    let at = |index: usize| value[index].as_f64().unwrap_or(0.0);
    // Ours is row-major [[a, b], [c, d]] with x' = a x + b y; kurbo's coefficients are columns.
    Affine::new([at(0), at(2), at(1), at(3), at(4), at(5)])
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x0 >= outer.x0 - 0.5 && inner.y0 >= outer.y0 - 0.5 && inner.x1 <= outer.x1 + 0.5 && inner.y1 <= outer.y1 + 0.5
}

/// The rounded box, clamped as CSS clamps radii that overlap.
fn rounded(bounds: Rect, radii: RoundedRectRadii) -> RoundedRect {
    let most = bounds.width().min(bounds.height()) / 2.0;
    let clamp = |radius: f64| radius.clamp(0.0, most.max(0.0));
    RoundedRect::from_rect(bounds, RoundedRectRadii::new(clamp(radii.top_left), clamp(radii.top_right), clamp(radii.bottom_right), clamp(radii.bottom_left)))
}

/// What's drawn, in the order ours draws it: by draw order, then kind as GPUI batches them.
enum Item<'a> {
    Shadow(&'a Value),
    Quad(&'a Value),
    Underline(&'a Value),
    Text(&'a Value),
    Sprite(&'a Value),
    Backdrop(&'a Value),
}

fn items(frame: &Value) -> Vec<(u64, u8, Item<'_>)> {
    let mut items = Vec::new();
    let list = |name: &str| frame[name].as_array().map(Vec::as_slice).unwrap_or_default().iter();
    items.extend(list("shadows").map(|v| (v["order"].as_u64().unwrap_or(0), 0, Item::Shadow(v))));
    items.extend(list("quads").map(|v| (v["order"].as_u64().unwrap_or(0), 1, Item::Quad(v))));
    items.extend(list("underlines").map(|v| (v["order"].as_u64().unwrap_or(0), 2, Item::Underline(v))));
    items.extend(list("texts").map(|v| (v["order"].as_u64().unwrap_or(0), 3, Item::Text(v))));
    items.extend(list("sprites").map(|v| (v["order"].as_u64().unwrap_or(0), 4, Item::Sprite(v))));
    items.extend(list("backdrops").map(|v| (v["order"].as_u64().unwrap_or(0), 5, Item::Backdrop(v))));
    items.sort_by_key(|(order, kind, _)| (*order, *kind));
    items
}

/// An image's pixels at the size it's drawn, straight RGBA, decoded once: a `data:` address, an SVG drawn by resvg
/// as ours draws it.
fn decode(src: &str, size: [u32; 2]) -> Option<(u32, u32, Vec<u8>)> {
    let (head, body) = src.strip_prefix("data:")?.split_once(',')?;
    let bytes = if head.ends_with(";base64") { data_encoding::BASE64.decode(body.as_bytes()).ok()? } else { percent_decode(body) };
    if head.starts_with("image/svg+xml") {
        let tree = resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size[0].max(1), size[1].max(1))?;
        let fit = (size[0] as f32 / tree.size().width()).min(size[1] as f32 / tree.size().height());
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(fit, fit), &mut pixmap.as_mut());
        let mut data = pixmap.take();
        for pixel in data.chunks_exact_mut(4) {
            let alpha = pixel[3] as f32 / 255.0;
            if alpha > 0.0 {
                for channel in &mut pixel[..3] {
                    *channel = (*channel as f32 / alpha).round().min(255.0) as u8;
                }
            }
        }
        return Some((size[0].max(1), size[1].max(1), data));
    }
    let image = image::load_from_memory(&bytes).ok()?.resize_exact(size[0].max(1), size[1].max(1), image::imageops::FilterType::Triangle).to_rgba8();
    Some((image.width(), image.height(), image.into_raw()))
}

fn percent_decode(body: &str) -> Vec<u8> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() + 0 && let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or(""), 16) {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

/// An SVG path (`d` in its `viewBox`) stretched to `bounds`, as ours draws it (`preserveAspectRatio="none"`).
fn path(source: &Value, bounds: Rect) -> Option<BezPath> {
    let d = BezPath::from_svg(source["path"].as_str()?).ok()?;
    let view: Vec<f64> = source["view_box"].as_str().unwrap_or("0 0 24 24").split([' ', ',']).filter_map(|part| part.parse().ok()).collect();
    let [x, y, w, h] = view[..] else { return None };
    Some(Affine::translate((bounds.x0, bounds.y0)) * Affine::scale_non_uniform(bounds.width() / w, bounds.height() / h) * Affine::translate((-x, -y)) * d)
}

/// A renderer's half of the trial: a scene built from a frame's items, drawn by it into the frame.
trait Vello {
    fn begin(&mut self);
    fn clip(&mut self, clip: Option<Rect>);
    fn fill(&mut self, transform: Affine, shape: &BezPath, colour: Rgba, even_odd: bool);
    fn stroke(&mut self, shape: &BezPath, colour: Rgba, width: f64);
    fn shadow(&mut self, transform: Affine, bounds: Rect, radius: f64, sigma: f64, colour: Rgba, within: Option<(&BezPath, bool)>);
    fn glyphs(&mut self, transform: Affine, run: &GlyphRun);
    fn image(&mut self, transform: Affine, shape: &BezPath, key: &str, pixels: &(u32, u32, Vec<u8>), place: Affine, alpha: f32);
    /// The scene so far drawn into `frame`, cleared first with `clear`; the scene then starts again.
    fn flush(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, clear: bool) -> Result<()>;
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    frame: wgpu::Texture,
    frame_view: wgpu::TextureView,
    size: [u32; 2],
    passes: Passes,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Our backdrop's pipelines, and the blit that puts a straight-alpha frame over ours.
struct Passes {
    blur: wgpu::RenderPipeline,
    backdrop: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    copy_of_frame: wgpu::Texture,
    levels: Vec<[wgpu::Texture; 3]>,
}

const LEVELS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    step: [f32; 2],
    sigma: f32,
    saturation: f32,
    bounds: [f32; 4],
    mask: [f32; 4],
    radii: [f32; 4],
    viewport: [f32; 2],
    pad: [f32; 2],
}

impl Passes {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Passes {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let premultiplied = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
        let straight_over_premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let texture = |size: [u32; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let levels = (1..=LEVELS).map(|level| {
            let halved = [size[0] >> level, size[1] >> level];
            [texture(halved), texture(halved), texture(halved)]
        });
        Passes {
            blur: pipeline("fs_blur", None),
            backdrop: pipeline("fs_backdrop", Some(premultiplied)),
            copy: pipeline("fs_copy", Some(straight_over_premultiplied)),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
            copy_of_frame: texture(size),
            levels: levels.collect(),
            layout,
        }
    }

    fn pass(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, from: &wgpu::Texture, into: &wgpu::TextureView, params: Params, load: bool, scissor: Option<[u32; 4]>) {
        let buffer = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::bytes_of(&params), usage: wgpu::BufferUsages::UNIFORM });
        let view = from.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: into,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: if load { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) }, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        if let Some([x, y, w, h]) = scissor {
            pass.set_scissor_rect(x, y, w, h);
        }
        pass.draw(0..4, 0..1);
    }

    /// Ours: the frame so far halved until the blur left is a few texels, blurred both ways, drawn in the backdrop.
    fn backdrop(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, backdrop: &Value) {
        let size = gpu.size;
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        encoder.copy_texture_to_texture(gpu.frame.as_image_copy(), self.copy_of_frame.as_image_copy(), extent);
        let sigma = backdrop["blur"].as_f64().unwrap_or(0.0) as f32;
        let level = (1..=LEVELS).find(|level| sigma / (1 << level) as f32 <= 6.0).unwrap_or(LEVELS);
        let params = |step: [f32; 2], sigma: f32| Params { step, sigma, saturation: 1.0, bounds: [0.0; 4], mask: [0.0; 4], radii: [0.0; 4], viewport: [size[0] as f32, size[1] as f32], pad: [0.0; 2] };
        let mut from = &self.copy_of_frame;
        for at in 0..level {
            let into = self.levels[at][0].create_view(&Default::default());
            self.pass(&gpu.device, encoder, &self.blur, from, &into, params([0.0, 0.0], 0.0), false, None);
            from = &self.levels[at][0];
        }
        let last = &self.levels[level - 1];
        let halved = [(size[0] >> level).max(1) as f32, (size[1] >> level).max(1) as f32];
        let sigma = sigma / (1 << level) as f32;
        self.pass(&gpu.device, encoder, &self.blur, &last[0], &last[1].create_view(&Default::default()), params([1.0 / halved[0], 0.0], sigma), false, None);
        self.pass(&gpu.device, encoder, &self.blur, &last[1], &last[2].create_view(&Default::default()), params([0.0, 1.0 / halved[1]], sigma), false, None);
        let bounds = rect(&backdrop["bounds"]);
        let mask = rect(&backdrop["mask"]);
        let shown = bounds.intersect(mask).intersect(Rect::new(0.0, 0.0, size[0] as f64, size[1] as f64));
        if shown.width() < 1.0 || shown.height() < 1.0 {
            return;
        }
        let r = radii(&backdrop["radii"]);
        let p = Params {
            saturation: backdrop["saturation"].as_f64().unwrap_or(1.0) as f32,
            bounds: [bounds.x0 as f32, bounds.y0 as f32, bounds.width() as f32, bounds.height() as f32],
            mask: [mask.x0 as f32, mask.y0 as f32, mask.width() as f32, mask.height() as f32],
            radii: [r.top_left as f32, r.top_right as f32, r.bottom_right as f32, r.bottom_left as f32],
            ..params([0.0, 0.0], 0.0)
        };
        let scissor = [shown.x0.floor() as u32, shown.y0.floor() as u32, (shown.width().ceil() as u32).max(1), (shown.height().ceil() as u32).max(1)];
        let scissor = [scissor[0], scissor[1], scissor[2].min(size[0] - scissor[0]), scissor[3].min(size[1] - scissor[1])];
        self.pass(&gpu.device, encoder, &self.backdrop, &last[2], &gpu.frame_view, p, true, Some(scissor));
    }
}

/// vello_gpu (was vello_hybrid).
struct Hybrid {
    scene: vello_gpu::Scene,
    renderer: vello_gpu::Renderer,
    resources: vello_gpu::Resources,
    textures: HashMap<String, (vello_gpu::TextureId, wgpu::TextureView)>,
    bindings: vello_gpu::TextureBindings,
    depth: wgpu::TextureView,
    clipped: bool,
}

impl Hybrid {
    fn new(gpu: &Gpu) -> Hybrid {
        let [width, height] = gpu.size;
        let (renderer, resources) = vello_gpu::Renderer::new(&gpu.device, &gpu.queue, &vello_gpu::RenderTargetConfig { format: FORMAT, width: width as u16, height: height as u16 });
        let depth = renderer.create_depth_texture_view(&vello_gpu::RenderSize { width: width as u16, height: height as u16 });
        Hybrid { scene: vello_gpu::Scene::new(width as u16, height as u16), renderer, resources, textures: HashMap::new(), bindings: vello_gpu::TextureBindings::new(), depth, clipped: false }
    }
}

fn paint(colour: Rgba) -> vello_gpu::peniko::Color {
    vello_gpu::peniko::Color::new(colour)
}

impl Vello for Hybrid {
    fn begin(&mut self) {
        self.scene.reset();
        self.clipped = false;
    }

    fn clip(&mut self, clip: Option<Rect>) {
        if self.clipped {
            self.scene.pop_clip();
        }
        self.clipped = clip.is_some();
        if let Some(clip) = clip {
            self.scene.reset_transform();
            self.scene.push_clip_rect(&clip);
        }
    }

    fn fill(&mut self, transform: Affine, shape: &BezPath, colour: Rgba, even_odd: bool) {
        self.scene.set_transform(transform);
        self.scene.set_fill_rule(if even_odd { Fill::EvenOdd } else { Fill::NonZero });
        self.scene.set_paint(paint(colour));
        self.scene.fill_path(shape);
        self.scene.set_fill_rule(Fill::NonZero);
    }

    fn stroke(&mut self, shape: &BezPath, colour: Rgba, width: f64) {
        self.scene.reset_transform();
        self.scene.set_stroke(Stroke::new(width).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round));
        self.scene.set_paint(paint(colour));
        self.scene.stroke_path(shape);
    }

    fn shadow(&mut self, transform: Affine, bounds: Rect, radius: f64, sigma: f64, colour: Rgba, within: Option<(&BezPath, bool)>) {
        self.scene.set_transform(transform);
        self.scene.set_paint(paint(colour));
        match within {
            Some((path, invert)) => self.scene.fill_blurred_rounded_rect_in(path, &bounds, radius as f32, sigma as f32, invert),
            None => self.scene.fill_blurred_rounded_rect(&bounds, radius as f32, sigma as f32),
        }
    }

    fn glyphs(&mut self, transform: Affine, run: &GlyphRun) {
        self.scene.set_transform(transform);
        self.scene.set_paint(paint(run.color));
        let glyphs = run.glyphs.iter().map(|(id, x, y)| glifo::Glyph { id: *id, x: *x, y: *y });
        let _ = self.scene.glyph_run(&mut self.resources, &run.font).font_size(run.size).normalized_coords(&run.coords).hint(false).atlas_cache(true).fill_glyphs(glyphs);
    }

    fn image(&mut self, transform: Affine, shape: &BezPath, key: &str, pixels: &(u32, u32, Vec<u8>), place: Affine, alpha: f32) {
        let next = self.textures.len() as u64;
        let (id, _) = self.textures.entry(key.to_owned()).or_insert_with(|| (vello_gpu::TextureId(next), wgpu::TextureView::clone(&TEXTURES.with(|textures| textures.borrow()[key].clone()))));
        let id = *id;
        let source = vello_common::paint::ImageSource::external_texture(id, vello_common::paint::TextureRegion::Full, true);
        let sampler = vello_gpu::peniko::ImageSampler { alpha, ..Default::default() };
        let _ = pixels;
        self.scene.set_transform(transform);
        self.scene.set_paint_transform(place);
        self.scene.set_paint(vello_common::paint::PaintType::Image(vello_gpu::peniko::ImageBrush { image: source, sampler }));
        self.scene.fill_path(shape);
        self.scene.reset_paint_transform();
    }

    fn flush(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, clear: bool) -> Result<()> {
        if self.clipped {
            self.scene.pop_clip();
            self.clipped = false;
        }
        self.bindings = vello_gpu::TextureBindings::new();
        for (id, view) in self.textures.values() {
            self.bindings.insert(*id, view.clone());
        }
        let mut options = vello_gpu::RenderOptions::default();
        if !clear {
            options.target_init = vello_gpu::TargetInit::SrcOver;
        }
        self.renderer
            .render(&self.scene, &mut self.resources, encoder, vello_gpu::RenderTarget { view: &gpu.frame_view, depth_view: Some(&self.depth) }, &self.bindings, options)
            .map_err(|error| anyhow!("vello_gpu: {error:?}"))?;
        self.scene.reset();
        Ok(())
    }
}

/// Nothing drawn: what reading the dumped frame and laying its text out costs alone, to take from the others'.
struct Null;

impl Vello for Null {
    fn begin(&mut self) {}
    fn clip(&mut self, _: Option<Rect>) {}
    fn fill(&mut self, _: Affine, _: &BezPath, _: Rgba, _: bool) {}
    fn stroke(&mut self, _: &BezPath, _: Rgba, _: f64) {}
    fn shadow(&mut self, _: Affine, _: Rect, _: f64, _: f64, _: Rgba, _: Option<(&BezPath, bool)>) {}
    fn glyphs(&mut self, _: Affine, _: &GlyphRun) {}
    fn image(&mut self, _: Affine, _: &BezPath, _: &str, _: &(u32, u32, Vec<u8>), _: Affine, _: f32) {}
    fn flush(&mut self, _: &Gpu, _: &mut wgpu::CommandEncoder, _: bool) -> Result<()> {
        Ok(())
    }
}

thread_local! {
    /// Each image's texture, by its key, for vello_gpu's external textures.
    static TEXTURES: std::cell::RefCell<HashMap<String, wgpu::TextureView>> = std::cell::RefCell::new(HashMap::new());
}

/// vello, the compute renderer: it draws into a texture of its own (straight alpha), which is blitted over ours.
struct Classic {
    scene: vello::Scene,
    renderer: vello::Renderer,
    layer: wgpu::Texture,
    images: HashMap<String, vello::peniko::ImageData>,
    clipped: bool,
}

impl Classic {
    fn new(gpu: &Gpu) -> Result<Classic> {
        let renderer = vello::Renderer::new(&gpu.device, vello::RendererOptions { use_cpu: false, antialiasing_support: vello::AaSupport::area_only(), num_init_threads: None, pipeline_cache: None })
            .map_err(|error| anyhow!("vello: {error}"))?;
        let layer = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: gpu.size[0], height: gpu.size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Ok(Classic { scene: vello::Scene::new(), renderer, layer, images: HashMap::new(), clipped: false })
    }
}

impl Vello for Classic {
    fn begin(&mut self) {
        self.scene.reset();
        self.clipped = false;
    }

    fn clip(&mut self, clip: Option<Rect>) {
        if self.clipped {
            self.scene.pop_layer();
        }
        self.clipped = clip.is_some();
        if let Some(clip) = clip {
            self.scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
        }
    }

    fn fill(&mut self, transform: Affine, shape: &BezPath, colour: Rgba, even_odd: bool) {
        self.scene.fill(if even_odd { Fill::EvenOdd } else { Fill::NonZero }, transform, self::colour(colour), None, shape);
    }

    fn stroke(&mut self, shape: &BezPath, colour: Rgba, width: f64) {
        self.scene.stroke(&Stroke::new(width).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round), Affine::IDENTITY, self::colour(colour), None, shape);
    }

    fn shadow(&mut self, transform: Affine, bounds: Rect, radius: f64, sigma: f64, colour: Rgba, within: Option<(&BezPath, bool)>) {
        match within {
            Some((path, invert)) => self.scene.draw_blurred_rounded_rect_in(path, transform, bounds, self::colour(colour), radius, sigma, invert),
            None => self.scene.draw_blurred_rounded_rect(transform, bounds, self::colour(colour), radius, sigma),
        }
    }

    fn glyphs(&mut self, transform: Affine, run: &GlyphRun) {
        self.scene
            .draw_glyphs(&run.font)
            .font_size(run.size)
            .transform(transform)
            .normalized_coords(&run.coords)
            .hint(false)
            .brush(self::colour(run.color))
            .draw(Fill::NonZero, run.glyphs.iter().map(|(id, x, y)| vello::Glyph { id: *id, x: *x, y: *y }));
    }

    fn image(&mut self, transform: Affine, shape: &BezPath, key: &str, pixels: &(u32, u32, Vec<u8>), place: Affine, alpha: f32) {
        let image = self.images.entry(key.to_owned()).or_insert_with(|| vello::peniko::ImageData {
            data: vello::peniko::Blob::new(Arc::new(pixels.2.clone())),
            format: vello::peniko::ImageFormat::Rgba8,
            alpha_type: vello::peniko::ImageAlphaType::Alpha,
            width: pixels.0,
            height: pixels.1,
        });
        let brush = vello::peniko::ImageBrush { image: image.clone(), sampler: vello::peniko::ImageSampler { alpha, ..Default::default() } };
        self.scene.fill(Fill::NonZero, transform, &brush, Some(place), shape);
    }

    fn flush(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, clear: bool) -> Result<()> {
        if self.clipped {
            self.scene.pop_layer();
            self.clipped = false;
        }
        // Vello submits its own work: what's recorded so far goes first, and the blit right after it, before the
        // next segment draws into the same texture.
        let earlier = std::mem::replace(encoder, gpu.device.create_command_encoder(&Default::default()));
        gpu.queue.submit([earlier.finish()]);
        let view = self.layer.create_view(&Default::default());
        let params = vello::RenderParams { base_color: Color::TRANSPARENT, width: gpu.size[0], height: gpu.size[1], antialiasing_method: vello::AaConfig::Area };
        self.renderer.render_to_texture(&gpu.device, &gpu.queue, &self.scene, &view, &params).map_err(|error| anyhow!("vello: {error}"))?;
        let none = Params { step: [0.0; 2], sigma: 0.0, saturation: 1.0, bounds: [0.0; 4], mask: [0.0; 4], radii: [0.0; 4], viewport: [0.0; 2], pad: [0.0; 2] };
        gpu.passes.pass(&gpu.device, encoder, &gpu.passes.copy, &self.layer, &gpu.frame_view, none, !clear, None);
        let blit = std::mem::replace(encoder, gpu.device.create_command_encoder(&Default::default()));
        gpu.queue.submit([blit.finish()]);
        self.scene.reset();
        Ok(())
    }
}

/// What a frame drew that the trial couldn't, by kind, to report.
#[derive(Default, Debug)]
struct Missing {
    dashed: usize,
    wavy: usize,
    unsourced: usize,
    per_corner_shadow: usize,
}

/// A frame drawn: its items into the renderer's scene, flushed at each backdrop, which our pass then draws.
fn draw(vello: &mut dyn Vello, gpu: &Gpu, text: &mut Text, frame: &Value, missing: &mut Missing) -> Result<(wgpu::CommandEncoder, f64)> {
    let started = std::time::Instant::now();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    vello.begin();
    let mut clip: Option<Rect> = None;
    let mut clear = true;
    let mut set_clip = |vello: &mut dyn Vello, mask: Rect, bounds: Rect, clip: &mut Option<Rect>| {
        if !contains(mask, bounds) {
            if *clip != Some(mask) {
                vello.clip(Some(mask));
                *clip = Some(mask);
            }
        } else if clip.is_some_and(|current| !contains(current, bounds)) {
            vello.clip(None);
            *clip = None;
        }
    };
    for (_, _, item) in items(frame) {
        match item {
            Item::Quad(quad) => {
                let bounds = rect(&quad["bounds"]);
                set_clip(vello, rect(&quad["mask"]), bounds, &mut clip);
                let transform = affine(&quad["transform"]);
                let outer = rounded(bounds, radii(&quad["radii"]));
                let background = rgba(&quad["background"]);
                if background[3] > 0.0 {
                    vello.fill(transform, &outer.to_path(0.1), background, false);
                }
                let borders: Vec<f64> = quad["borders"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
                let border = rgba(&quad["border_color"]);
                if borders.iter().any(|width| *width > 0.0) && border[3] > 0.0 {
                    if quad["dashed"].as_bool() == Some(true) {
                        missing.dashed += 1;
                    }
                    let [top, right, bottom, left] = [borders[0], borders[1], borders[2], borders[3]];
                    let inner_bounds = Rect::new(bounds.x0 + left, bounds.y0 + top, bounds.x1 - right, bounds.y1 - bottom);
                    let r = outer.radii();
                    let inner = rounded(inner_bounds, RoundedRectRadii::new((r.top_left - left.max(top)).max(0.0), (r.top_right - right.max(top)).max(0.0), (r.bottom_right - right.max(bottom)).max(0.0), (r.bottom_left - left.max(bottom)).max(0.0)));
                    let mut ring = outer.to_path(0.1);
                    ring.extend(inner.to_path(0.1));
                    vello.fill(transform, &ring, border, true);
                }
            }
            Item::Shadow(shadow) => {
                let bounds = rect(&shadow["bounds"]);
                let sigma = shadow["blur"].as_f64().unwrap_or(0.0);
                set_clip(vello, rect(&shadow["mask"]), bounds.inflate(3.0 * sigma, 3.0 * sigma), &mut clip);
                let r = radii(&shadow["radii"]);
                if r.top_left != r.top_right || r.top_left != r.bottom_left || r.top_left != r.bottom_right {
                    missing.per_corner_shadow += 1;
                }
                let colour = rgba(&shadow["color"]);
                let transform = affine(&shadow["transform"]);
                if shadow["inset"].as_bool() == Some(true) {
                    let outer = rounded(rect(&shadow["outer"]), radii(&shadow["outer_radii"])).to_path(0.1);
                    vello.shadow(transform, bounds, r.top_left, sigma, colour, Some((&outer, true)));
                } else {
                    // As CSS's, an outer shadow isn't drawn under its box: its reach, less the box (wound the other
                    // way, so non-zero leaves it out).
                    let mut around = bounds.inflate(3.0 * sigma + 1.0, 3.0 * sigma + 1.0).to_path(0.1);
                    let outer = rounded(rect(&shadow["outer"]), radii(&shadow["outer_radii"])).to_path(0.1);
                    around.extend(outer.reverse_subpaths());
                    vello.shadow(transform, bounds, r.top_left, sigma, colour, Some((&around, false)));
                }
            }
            Item::Underline(line) => {
                let bounds = rect(&line["bounds"]);
                set_clip(vello, rect(&line["mask"]), bounds, &mut clip);
                missing.wavy += usize::from(line["wavy"].as_u64().unwrap_or(0) == 1);
                vello.fill(Affine::IDENTITY, &bounds.to_path(0.1), rgba(&line["color"]), false);
            }
            Item::Text(paragraph) => {
                let laid = text.lay_out(paragraph);
                let scale = paragraph["scale"].as_f64().unwrap_or(1.0);
                let origin = (paragraph["origin"][0].as_f64().unwrap_or(0.0) * scale, paragraph["origin"][1].as_f64().unwrap_or(0.0) * scale);
                let mask = rect(&paragraph["mask"]);
                let transform = affine(&paragraph["transform"]) * Affine::translate(origin);
                // Its glyphs' box isn't known without walking them: clip where the mask is smaller than the frame.
                set_clip(vello, mask, Rect::new(0.0, 0.0, gpu.size[0] as f64, gpu.size[1] as f64), &mut clip);
                for run in &laid.runs {
                    vello.glyphs(transform, run);
                }
            }
            Item::Sprite(sprite) => {
                let bounds = rect(&sprite["bounds"]);
                set_clip(vello, rect(&sprite["mask"]), bounds, &mut clip);
                let transform = affine(&sprite["transform"]);
                let opacity = sprite["opacity"].as_f64().unwrap_or(1.0) as f32;
                let source = &sprite["source"];
                if let Some(path) = path(source, bounds) {
                    let fill = source["fill"].as_array().map(|_| rgba(&source["fill"]));
                    if let Some(mut fill) = fill {
                        fill[3] *= opacity;
                        vello.fill(transform, &path, fill, false);
                    }
                    if source["stroke"].is_array() {
                        let mut stroke = rgba(&source["stroke"]);
                        stroke[3] *= opacity;
                        vello.stroke(&(transform * path), stroke, source["stroke_width"].as_f64().unwrap_or(1.0));
                    }
                } else if let Some(src) = source["image"].as_str() {
                    let size = [source["size"][0].as_u64().unwrap_or(1) as u32, source["size"][1].as_u64().unwrap_or(1) as u32];
                    let key = {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        src.hash(&mut hasher);
                        format!("{}x{}-{:x}", size[0], size[1], hasher.finish())
                    };
                    let pixels = IMAGES.with(|images| images.borrow_mut().entry(key.clone()).or_insert_with(|| Arc::new(decode(src, size))).clone());
                    let Some(pixels) = pixels.as_ref() else { continue };
                    upload(gpu, &key, pixels);
                    let place = Affine::translate((bounds.x0, bounds.y0)) * Affine::scale_non_uniform(bounds.width() / pixels.0 as f64, bounds.height() / pixels.1 as f64);
                    let shape = rounded(bounds, radii(&sprite["radii"])).to_path(0.1);
                    vello.image(transform, &shape, &key, pixels, place, opacity);
                } else {
                    missing.unsourced += 1;
                }
            }
            Item::Backdrop(backdrop) => {
                if clip.take().is_some() {
                    vello.clip(None);
                }
                vello.flush(gpu, &mut encoder, clear)?;
                clear = false;
                gpu.passes.backdrop(gpu, &mut encoder, backdrop);
                vello.begin();
            }
        }
    }
    vello.flush(gpu, &mut encoder, clear)?;
    Ok((encoder, started.elapsed().as_secs_f64() * 1000.0))
}

thread_local! {
    static IMAGES: std::cell::RefCell<HashMap<String, Arc<Option<(u32, u32, Vec<u8>)>>>> = std::cell::RefCell::new(HashMap::new());
}

/// An image's texture, made once, for vello_gpu, which draws images from textures it's given.
fn upload(gpu: &Gpu, key: &str, pixels: &(u32, u32, Vec<u8>)) {
    if TEXTURES.with(|textures| textures.borrow().contains_key(key)) {
        return;
    }
    let size = wgpu::Extent3d { width: pixels.0, height: pixels.1, depth_or_array_layers: 1 };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // vello_gpu samples premultiplied pixels.
    let mut premultiplied = pixels.2.clone();
    for pixel in premultiplied.chunks_exact_mut(4) {
        let alpha = pixel[3] as f32 / 255.0;
        for channel in &mut pixel[..3] {
            *channel = (*channel as f32 * alpha).round() as u8;
        }
    }
    gpu.queue.write_texture(texture.as_image_copy(), &premultiplied, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(pixels.0 * 4), rows_per_image: None }, size);
    TEXTURES.with(|textures| textures.borrow_mut().insert(key.to_owned(), texture.create_view(&Default::default())));
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, which, png, ..] = &args[..] else { anyhow::bail!("trial-vello <frames dir> <gpu|classic> <out.png> [--measure]") };
    let dir = PathBuf::from(dir);
    let read = |name: &str| -> Result<Value> { Ok(serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).with_context(|| name.to_owned())?)?) };
    let first = read("frame")?;
    let size = [first["size"][0].as_u64().unwrap_or(1) as u32, first["size"][1].as_u64().unwrap_or(1) as u32];

    let resident = || memory_stats::memory_stats().map_or(0, |memory| memory.physical_mem);
    let before = resident();
    let started = std::time::Instant::now();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() }))?;
    let frame = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let passes = Passes::new(&device, size);
    let gpu = Gpu { frame_view: frame.create_view(&Default::default()), frame, size, passes, device, queue };
    let mut vello: Box<dyn Vello> = match which.as_str() {
        "gpu" => Box::new(Hybrid::new(&gpu)),
        "classic" => Box::new(Classic::new(&gpu)?),
        "none" => Box::new(Null),
        other => anyhow::bail!("{other}: gpu or classic"),
    };
    let mut text = Text::new()?;
    let bytes_per_row = (size[0] * 4).next_multiple_of(256);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: u64::from(bytes_per_row) * u64::from(size[1]), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let copy = |encoder: &mut wgpu::CommandEncoder| {
        encoder.copy_texture_to_buffer(
            gpu.frame.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: None } },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        )
    };
    let mut missing = Missing::default();

    // The frame, read back, as a PNG (straight alpha).
    let (mut encoder, _) = draw(vello.as_mut(), &gpu, &mut text, &first, &mut missing)?;
    copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    let first_frame_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut pixels = Vec::with_capacity((size[0] * size[1] * 4) as usize);
    for row in readback.slice(..).get_mapped_range()?.chunks_exact(bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..size[0] as usize * 4]);
    }
    readback.unmap();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as f32 / 255.0;
        if alpha > 0.0 {
            for channel in &mut pixel[..3] {
                *channel = (*channel as f32 / alpha).round().min(255.0) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(size[0], size[1], pixels).ok_or_else(|| anyhow!("wrong size"))?.save(png)?;
    eprintln!("{} on {} ({:?}), {}x{}, first frame {first_frame_ms:.1} ms, not drawn: {missing:?}", which, info.name, info.device_type, size[0], size[1]);

    if args.iter().any(|arg| arg == "--measure") {
        let mut phases = serde_json::Map::new();
        for phase in ["idle", "typing", "scrolling"] {
            let frames: Vec<Value> = if phase == "idle" {
                if !dir.join("idle-00.json").exists() {
                    continue;
                }
                let idle = read("idle-00")?;
                vec![idle; 30]
            } else {
                (0..30).map_while(|at| read(&format!("{phase}-{at:02}")).ok()).collect()
            };
            if frames.is_empty() {
                continue;
            }
            // Back to back, each waiting for the one before, as a window's frames are; each copied out as ours are.
            let mut times = Vec::new();
            let mut encodes = Vec::new();
            let mut last: Option<wgpu::SubmissionIndex> = None;
            for frame in &frames {
                let started = std::time::Instant::now();
                let (mut encoder, encoded) = draw(vello.as_mut(), &gpu, &mut text, frame, &mut missing)?;
                copy(&mut encoder);
                let submitted = gpu.queue.submit([encoder.finish()]);
                if let Some(last) = last.take() {
                    gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(last), timeout: None })?;
                }
                last = Some(submitted);
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                encodes.push(encoded);
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            times.sort_by(f64::total_cmp);
            encodes.sort_by(f64::total_cmp);
            phases.insert(phase.into(), json!({"median_ms": times[times.len() / 2], "worst_ms": times[times.len() - 1], "encode_median_ms": encodes[encodes.len() / 2]}));
        }
        let gpu_mb = gpu.device.generate_allocator_report().map(|report| report.total_allocated_bytes as f64 / 1e6);
        let memory_mb = resident().saturating_sub(before) as f64 / 1e6;
        println!("measured {}", json!({"renderer": which, "adapter": info.name, "first_frame_ms": first_frame_ms, "phases": phases, "memory_mb": memory_mb, "gpu_allocated_mb": gpu_mb}));
    }
    let _ = Path::new(png);
    Ok(())
}
