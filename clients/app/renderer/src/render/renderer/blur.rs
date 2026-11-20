use super::Renderer;
use super::params::BlurParams;

/// How many times a backdrop's frame is halved at most: a sixteenth of it, for the widest blur.
const LEVELS: usize = 4;

/// The textures a backdrop's blur is drawn through, for frames of `size`: for each level (half its size, a quarter,
/// …) the frame shrunk to it and the two passes of its blur, each made as a blur first needs it, as only the widest
/// blur's last level has the two passes; and the frame copied, where it can't be read as it is.
pub(super) struct BackdropTextures {
    pub(super) size: (u32, u32),
    pub(super) source: Option<(wgpu::Texture, wgpu::TextureView)>,
    pub(super) levels: [[Option<(wgpu::Texture, wgpu::TextureView)>; 3]; LEVELS],
}

/// Where a backdrop's blur's parameters are in the frame's data, and at which level it's blurred.
pub(super) struct Blur {
    /// What was drawn before it, by which a blur of it is kept (`Renderer::blurred`).
    pub(super) behind: Option<u64>,
    pub(super) level: usize,
    /// What of the frame it blurs, in its device pixels, left, top, right and bottom: the backdrop and twice the
    /// blur's reach round it (`Renderer::plan_blur`).
    pub(super) region: [u32; 4],
    pub(super) shrink: (u64, u64),
    pub(super) across: (u64, u64),
    pub(super) down: (u64, u64),
}

/// `texture` from its texel at `left`, `top`, for a copy.
fn texel(texture: &wgpu::Texture, left: u32, top: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: left, y: top, z: 0 }, aspect: wgpu::TextureAspect::All }
}

/// `size` halved `times` times, at least a pixel.
fn halved((width, height): (u32, u32), times: usize) -> (u32, u32) {
    ((width >> times).max(1), (height >> times).max(1))
}

impl Renderer {
    /// How a backdrop's blur of `sigma` device pixels is drawn over a frame of `size`: halved until the blur left is
    /// a few texels, each halving the average of four, so the Gaussian then takes every texel it reaches (a wide blur
    /// taking texels further apart streaks what's sharp behind it), then blurred across and down, with the
    /// parameters of each pass in the frame's data.
    /// Only what's behind the backdrop, `area` (in device pixels, left, top, right and bottom), is blurred, with
    /// twice the blur's reach round it, as Skia blurs a backdrop: the Gaussian down takes what the one across made a
    /// reach away, and the one across what's a reach beyond that. Its edges are on its level's texels, so each halving
    /// takes whole ones.
    pub(super) fn plan_blur(&mut self, size: (u32, u32), sigma: f32, area: [f32; 4]) -> Blur {
        let level = (1..=LEVELS).find(|level| sigma / (1 << level) as f32 <= 6.0).unwrap_or(LEVELS);
        let (step, reach) = (1u32 << level, 2.0 * (sigma * 3.0).ceil());
        let out = |edge: f32, most: u32, up: bool| {
            let edge = if up { edge + reach } else { edge - reach }.clamp(0.0, most as f32) as u32;
            if up { edge.div_ceil(step) * step } else { edge / step * step }.min(most)
        };
        let region = [out(area[0], size.0, false), out(area[1], size.1, false), out(area[2], size.0, true), out(area[3], size.1, true)];
        let (width, height) = halved(size, level);
        let sigma = sigma / (1 << level) as f32;
        let shrink = self.push(bytemuck::bytes_of(&BlurParams { step: [0.0, 0.0], sigma: 0.0, pad: 0.0 }));
        let across = self.push(bytemuck::bytes_of(&BlurParams { step: [1.0 / width as f32, 0.0], sigma, pad: 0.0 }));
        let down = self.push(bytemuck::bytes_of(&BlurParams { step: [0.0, 1.0 / height as f32], sigma, pad: 0.0 }));
        Blur { level, region, shrink, across, down, behind: None }
    }

    /// Blurs what's drawn into `frame` so far, as `blur` says, into the backdrop's textures: shrunk, then blurred
    /// across into one at its level's size and down into another, each a separable Gaussian's pass. The view of the
    /// last. Only the textures this blur goes through are made, each the first time one needs it, and the frame is
    /// read as it is where its surface lets a shader read it.
    pub(super) fn blur_behind(&mut self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture, view: &wgpu::TextureView, globals: &wgpu::BindGroup, buffer: &wgpu::Buffer, blur: &Blur) -> wgpu::TextureView {
        if let Some((view, used)) = blur.behind.and_then(|behind| self.blurred.get_mut(&behind)) {
            *used = true;
            return view.clone();
        }
        let size = (frame.width(), frame.height());
        let mut work = self.work.get();
        work.blurred += f64::from(blur.region[2] - blur.region[0]) * f64::from(blur.region[3] - blur.region[1]);
        self.work.set(work);
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
            let (width, height) = (blur.region[2] - blur.region[0], blur.region[3] - blur.region[1]);
            if width > 0 && height > 0 {
                let (left, top) = (blur.region[0], blur.region[1]);
                encoder.copy_texture_to_texture(texel(frame, left, top), texel(&source.0, left, top), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
            }
        }
        let backdrop = self.backdrop.as_ref().expect("made above");
        let level = |at: usize, which: usize| &backdrop.levels[at][which].as_ref().expect("made above").1;
        let mut passes = Vec::new();
        let mut from = if readable { view } else { &backdrop.source.as_ref().expect("copied above").1 };
        for at in 0..blur.level {
            passes.push((from, level(at, 0), blur.shrink, at + 1));
            from = level(at, 0);
        }
        let (across, down) = (level(blur.level - 1, 1), level(blur.level - 1, 2));
        passes.push((from, across, blur.across, blur.level));
        passes.push((across, down, blur.down, blur.level));
        // Each pass draws only the region, at its level, of the texture it draws into.
        let region = |level: usize, (width, height): (u32, u32)| {
            let [left, top, right, bottom] = blur.region.map(|edge| edge >> level);
            let (left, top) = (left.min(width), top.min(height));
            (left, top, right.min(width).saturating_sub(left), bottom.min(height).saturating_sub(top))
        };
        for (from, into, params, at) in passes {
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
                timestamp_writes: self.stamps.as_ref().and_then(|stamps| stamps.pass(true)),
                ..Default::default()
            });
            let (left, top, width, height) = region(at, halved(size, at));
            if width == 0 || height == 0 {
                continue;
            }
            pass.set_scissor_rect(left, top, width, height);
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
        let (left, top, width, height) = region(blur.level, (width, height));
        if width > 0 && height > 0 {
            encoder.copy_texture_to_texture(texel(down, left, top), texel(&kept, left, top), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
        }
        let view = kept.create_view(&wgpu::TextureViewDescriptor::default());
        self.blurred.insert(behind, (view.clone(), true));
        view
    }
}
