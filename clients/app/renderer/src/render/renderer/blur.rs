use super::Renderer;
use super::super::scene::{BLUR_LEVELS, Backdrop, PrimitiveBatch, blur_reach};
use crate::geometry::{Bounds, ScaledPixels, point, size};
use super::params::BlurParams;

/// The textures a backdrop's blur is drawn through, for frames of `size`: for each level (half its size, a quarter,
/// …) the frame shrunk to it and the two passes of its blur, each made as a blur at that level first needs it; and
/// the frame copied, where it can't be read as it is.
pub(super) struct BackdropTextures {
    pub(super) size: (u32, u32),
    pub(super) source: Option<(wgpu::Texture, wgpu::TextureView)>,
    pub(super) levels: [[Option<(wgpu::Texture, wgpu::TextureView)>; 3]; BLUR_LEVELS],
}

/// What's behind a backdrop where its blur reads, thing by thing: what each is, where (a hash of its instance, and of
/// the blur behind it where it's a backdrop), and the device pixels it's drawn in, left, top, right and bottom.
pub(super) type Prints = Vec<(u64, [u32; 4])>;

/// Where a backdrop's blur's parameters are in the frame's data, and at which level it's blurred.
#[derive(Clone)]
pub(super) struct Blur {
    /// What was drawn before it in what it blurs, by which a blur of it is kept (`Target::blurred`).
    pub(super) behind: Option<u64>,
    pub(super) level: usize,
    /// What of the frame it blurs, in its device pixels, left, top, right and bottom: the backdrop and twice the
    /// blur's reach round it (`Renderer::plan_blur`).
    pub(super) region: [u32; 4],
    /// Its region where it's blurred whole, which a blur taken again only where what it blurs changed (`into`) keeps.
    pub(super) whole: [u32; 4],
    /// What it's drawn over (`Drawn::over`) and what's behind it, by which a blur of it kept from the frames before is
    /// taken again only where they differ (`Blur::damaged`).
    pub(super) over: u64,
    pub(super) prints: Prints,
    /// Where it's taken again only in part: the blur kept from the frames before it's drawn into, and what of it, in
    /// device pixels, left, top, right and bottom.
    pub(super) into: Option<(wgpu::TextureView, [u32; 4])>,
    /// Its sigma, in device pixels, by which another's blur is drawn through the same textures (`together`).
    pub(super) sigma: f32,
    /// The earliest it can be blurred, as a place among the frame's batches, a batch's and a backdrop's of it: past the
    /// last thing drawn before its backdrop that reaches its region (`ready`).
    pub(super) ready: (usize, usize),
    pub(super) shrink: (u64, u64),
    pub(super) across: (u64, u64),
    pub(super) down: (u64, u64),
}

impl Blur {
    /// What of a blur kept from the frames before (`kept`) must be taken again to be this one, where it's of the same
    /// region and sigma over the same frame and what's behind it changed only in part (a glass button fading behind a
    /// veil): the things behind that differ, where they're drawn, and round them as far as the blur's Gaussian reaches
    /// (`fs_blur`'s taps), what it reads twice as far. → what to blur and what of it to keep, in device pixels, where
    /// that's less than half the region; else none, as it's blurred whole.
    pub(super) fn damaged(&self, kept: &KeptBlur) -> Option<([u32; 4], [u32; 4])> {
        if kept.sigma != self.sigma || kept.region != self.whole || kept.over != self.over {
            return None;
        }
        let mut count: std::collections::HashMap<u64, i32> = Default::default();
        for (print, _) in &self.prints {
            *count.entry(*print).or_default() += 1;
        }
        for (print, _) in &kept.prints {
            *count.entry(*print).or_default() -= 1;
        }
        let differs = |prints: &Prints, sign: i32| prints.iter().filter(|(print, _)| count[print] * sign > 0).map(|(_, rect)| *rect).collect::<Vec<_>>();
        let [left, top, right, bottom] = self.whole;
        let mut changed = [u32::MAX, u32::MAX, 0, 0];
        for rect in differs(&self.prints, 1).into_iter().chain(differs(&kept.prints, -1)) {
            changed = [changed[0].min(rect[0]), changed[1].min(rect[1]), changed[2].max(rect[2]), changed[3].max(rect[3])];
        }
        let step = 1u32 << self.level;
        let taps = (3.0 * (self.sigma / step as f32).max(0.5)).ceil().min(48.0) as u32 + 1;
        let round = |margin: u32| {
            let [l, t, r, b] = changed;
            let out = |edge: u32, margin: u32, up: bool| if up { (edge + margin).div_ceil(step) * step } else { edge.saturating_sub(margin) / step * step };
            [out(l, margin, false).max(left), out(t, margin, false).max(top), out(r, margin, true).min(right), out(b, margin, true).min(bottom)]
        };
        let (keep, take) = (round(taps * step), round(2 * taps * step));
        let area = |[l, t, r, b]: [u32; 4]| u64::from(r.saturating_sub(l)) * u64::from(b.saturating_sub(t));
        (2 * area(take) < area(self.whole)).then_some((take, keep))
    }

    /// Whether `next` can be blurred in the same passes as `blurs`: at the same level, through the same textures, where
    /// what it blurs is none of theirs or as much blurred.
    pub(super) fn together(blurs: &[&Blur], next: &Blur) -> bool {
        let meets = |a: [u32; 4], b: [u32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
        blurs.iter().all(|blur| blur.level == next.level && (blur.sigma == next.sigma || !meets(next.region, blur.region)))
    }

    /// What of the scene it blurs, its region, of a target whose top left is `origin` in the scene.
    pub(super) fn area(&self, origin: [f32; 2]) -> Bounds<ScaledPixels> {
        let px = ScaledPixels;
        let [left, top, right, bottom] = self.region.map(|edge| edge as f32);
        Bounds { origin: point(px(left + origin[0]), px(top + origin[1])), size: size(px(right - left), px(bottom - top)) }
    }

    /// The earliest backdrop `one` of batch `at`'s blur can be taken (`ready`): just past the last thing drawn before
    /// it that reaches its region, of a target whose top left is `origin` in the scene, as what's drawn after that is
    /// none of what it blurs.
    pub(super) fn ready(&self, batches: &[PrimitiveBatch<'_>], at: usize, one: usize, origin: [f32; 2]) -> (usize, usize) {
        let area = self.area(origin);
        let last = |backdrops: &[Backdrop]| backdrops.iter().rposition(|backdrop| backdrop.bounds.intersect(&backdrop.content_mask.bounds).intersects(&area));
        if let PrimitiveBatch::Backdrops(backdrops) = &batches[at]
            && let Some(before) = last(&backdrops[..one])
        {
            return (at, before + 1);
        }
        for earlier in (0..at).rev() {
            match &batches[earlier] {
                PrimitiveBatch::Backdrops(backdrops) => {
                    if let Some(before) = last(backdrops) {
                        return (earlier, before + 1);
                    }
                }
                batch if batch.reaches(&area) => return (earlier, 1),
                _ => {}
            }
        }
        (0, 0)
    }
}

/// A blur kept from the frames before, of what's behind a backdrop (`Target::blurred`): its texture, whether this frame
/// used it, and what it blurred, by which it's taken again only in part where that changed (`Blur::damaged`).
pub(super) struct KeptBlur {
    pub(super) view: wgpu::TextureView,
    pub(super) used: bool,
    pub(super) sigma: f32,
    pub(super) region: [u32; 4],
    pub(super) over: u64,
    pub(super) prints: Prints,
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
        let (level, reach) = blur_reach(sigma);
        let step = 1u32 << level;
        let out = |edge: f32, most: u32, up: bool| {
            let edge = if up { edge + reach } else { edge - reach }.clamp(0.0, most as f32) as u32;
            if up { edge.div_ceil(step) * step } else { edge / step * step }.min(most)
        };
        let region = [out(area[0], size.0, false), out(area[1], size.1, false), out(area[2], size.0, true), out(area[3], size.1, true)];
        let (width, height) = halved(size, level);
        let shrunk = sigma / (1 << level) as f32;
        let shrink = self.push(bytemuck::bytes_of(&BlurParams { step: [1.0 / size.0 as f32, 1.0 / size.1 as f32], sigma: 0.0, shrink: step as f32 }));
        let across = self.push(bytemuck::bytes_of(&BlurParams { step: [1.0 / width as f32, 0.0], sigma: shrunk, shrink: 0.0 }));
        let down = self.push(bytemuck::bytes_of(&BlurParams { step: [0.0, 1.0 / height as f32], sigma: shrunk, shrink: 0.0 }));
        Blur { level, region, whole: region, over: 0, prints: Vec::new(), into: None, sigma, ready: (0, 0), shrink, across, down, behind: None }
    }


    /// Blurs what's drawn into `frame` so far, as each of `blurs` says (all at one level, `together`), into its
    /// target's backdrop textures: shrunk, then blurred across into one at its level's size and down into another, each a
    /// separable Gaussian's pass, each blur's region drawn in the same three. Each blur's view, kept in a texture of
    /// its own. Only the textures the blurs go through are made, each the first time one needs it, and the frame is
    /// read as it is where its surface lets a shader read it.
    pub(super) fn blur_behind(&mut self, encoder: &mut wgpu::CommandEncoder, target: &mut super::Target, frame: &wgpu::Texture, view: &wgpu::TextureView, globals: &wgpu::BindGroup, buffer: &wgpu::Buffer, blurs: &[&Blur]) -> Vec<wgpu::TextureView> {
        let size = (frame.width(), frame.height());
        let level = blurs[0].level;
        let mut work = self.work.get();
        work.blurs += blurs.len() as u32;
        for blur in blurs {
            work.blurred += f64::from(blur.region[2] - blur.region[0]) * f64::from(blur.region[3] - blur.region[1]);
        }
        self.work.set(work);
        if target.backdrop.as_ref().is_none_or(|backdrop| backdrop.size != size) {
            target.backdrop = Some(BackdropTextures { size, source: None, levels: Default::default() });
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
        let backdrop = target.backdrop.as_mut().expect("made above");
        for which in 0..3 {
            if backdrop.levels[level - 1][which].is_none() {
                backdrop.levels[level - 1][which] = Some(make(halved(size, level), wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC));
            }
        }
        // A Metal drawable or a Direct3D 12 swap chain's buffer isn't one wgpu lets a shader read: what's drawn so
        // far is copied out of it first.
        let readable = frame.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING);
        if !readable {
            let source = backdrop.source.get_or_insert_with(|| make(size, wgpu::TextureUsages::COPY_DST));
            for blur in blurs {
                let (width, height) = (blur.region[2] - blur.region[0], blur.region[3] - blur.region[1]);
                if width > 0 && height > 0 {
                    let (left, top) = (blur.region[0], blur.region[1]);
                    encoder.copy_texture_to_texture(texel(frame, left, top), texel(&source.0, left, top), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
                }
            }
        }
        let backdrop = target.backdrop.as_ref().expect("made above");
        let texture_of = |which: usize| &backdrop.levels[level - 1][which].as_ref().expect("made above").1;
        let from = if readable { view } else { &backdrop.source.as_ref().expect("copied above").1 };
        let (shrunk, across, down) = (texture_of(0), texture_of(1), texture_of(2));
        // Each pass draws only each blur's region, at its level, of the texture it draws into.
        let region = |rect: [u32; 4], level: usize, (width, height): (u32, u32)| {
            let [left, top, right, bottom] = rect.map(|edge| edge >> level);
            let (left, top) = (left.min(width), top.min(height));
            (left, top, right.min(width).saturating_sub(left), bottom.min(height).saturating_sub(top))
        };
        // Shrunk to its level in one pass, not halved a pass at a time: each pass costs a command list or two
        // (`draw_batches`).
        let passes: [(&wgpu::TextureView, &wgpu::TextureView, fn(&Blur) -> (u64, u64)); 3] = [(from, shrunk, |blur| blur.shrink), (shrunk, across, |blur| blur.across), (across, down, |blur| blur.down)];
        for (from, into, params) in passes {
            let texture = self.texture(from);
            let mut pass = self.pass(encoder, into, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), self.stamps.as_ref().and_then(|stamps| stamps.pass(true)));
            pass.set_pipeline(&self.pipelines.blur);
            pass.set_bind_group(0, globals, &[]);
            pass.set_bind_group(2, &texture, &[]);
            for blur in blurs {
                let (left, top, width, height) = region(blur.region, level, halved(size, level));
                if width == 0 || height == 0 {
                    continue;
                }
                let params = params(blur);
                let params = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.pipelines.layouts.blur_params,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: params.0, size: wgpu::BufferSize::new(params.1) }),
                    }],
                });
                pass.set_scissor_rect(left, top, width, height);
                pass.set_bind_group(1, &params, &[]);
                pass.draw(0..4, 0..1);
                self.drew();
            }
        }
        // Each kept in a texture of its own, as the next backdrops blur into the same ones; one taken again in part
        // into the one it was kept in, what of it changed.
        let down = &backdrop.levels[level - 1][2].as_ref().expect("made above").0;
        let (width, height) = (down.width(), down.height());
        let mut views = Vec::with_capacity(blurs.len());
        for blur in blurs {
            let (view, copied) = match &blur.into {
                Some((view, keep)) => (view.clone(), *keep),
                None => {
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
                    (kept.create_view(&wgpu::TextureViewDescriptor::default()), blur.region)
                }
            };
            let (left, top, width, height) = region(copied, level, (width, height));
            if width > 0 && height > 0 {
                encoder.copy_texture_to_texture(texel(down, left, top), texel(view.texture(), left, top), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
            }
            if let Some(behind) = blur.behind {
                target.blurred.insert(behind, KeptBlur { view: view.clone(), used: true, sigma: blur.sigma, region: blur.whole, over: blur.over, prints: blur.prints.clone() });
            }
            views.push(view);
        }
        views
    }
}
