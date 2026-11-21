use super::blur::BackdropTextures;
use super::{Drawn, Kept, OFFSCREEN_FORMAT};
use super::super::context::Context;

/// What a renderer draws into: a window's surface, each of a panel's parts' (`platform::Window::arrange`), or an
/// image of its own offscreen; with what it kept of its own frames before.
pub(super) struct Target {
    pub(super) output: Output,
    /// An image of its surface taken ahead, as its window went from the screen (`put_away`), drawn on first as it's
    /// back; let go before the surface it's of.
    pub(super) reserved: Option<wgpu::SurfaceTexture>,
    pub(super) size: (u32, u32),
    /// What its last frame drew, batch by batch (`Drawn`), and what of the frames before was the same and is kept.
    pub(super) last: Drawn,
    pub(super) kept: Option<Kept>,
    /// What a backdrop blurs: the frame as drawn so far, halved and halved again, then blurred one way and the other.
    pub(super) backdrop: Option<BackdropTextures>,
    /// Each blur of what's behind a backdrop, by what was drawn before it (`Blur::behind`), kept while frames draw the
    /// same behind one, as iOS's materials blur what's behind them once while it holds still; and whether this frame
    /// used it.
    pub(super) blurred: std::collections::HashMap<u64, super::blur::KeptBlur>,
}

/// A window's surface, or a texture of our own, which `read` reads back.
pub(super) enum Output {
    Surface { surface: wgpu::Surface<'static>, config: wgpu::SurfaceConfiguration },
    Offscreen { texture: wgpu::Texture, view: wgpu::TextureView, readback: wgpu::Buffer, row: u32 },
}

/// The image a frame is drawn on: its surface's, which is presented once it's drawn, or its own offscreen texture.
pub(super) struct Image {
    pub(super) frame: Option<wgpu::SurfaceTexture>,
    pub(super) view: wgpu::TextureView,
    pub(super) texture: wgpu::Texture,
}

/// An image `size` device pixels to draw into offscreen, and what it's read back through.
fn offscreen(device: &wgpu::Device, (width, height): (u32, u32)) -> Output {
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
    Output::Offscreen { texture, view, readback, row }
}

impl Target {
    fn with(output: Output, size: (u32, u32)) -> Self {
        Self { output, reserved: None, size, last: Drawn::default(), kept: None, backdrop: None, blurred: Default::default() }
    }

    /// An image `size` device pixels drawn offscreen, read back by `read`.
    pub(super) fn offscreen(device: &wgpu::Device, size: (u32, u32)) -> Self {
        Self::with(offscreen(device, size), size)
    }

    /// A window's surface, as its platform gives it, `size` device pixels, see-through or not; and the format it's
    /// drawn in and whether what's drawn into it is premultiplied by its alpha.
    pub(super) fn surface(context: &Context, target: super::super::SurfaceTarget, (width, height): (u32, u32), transparent: bool) -> anyhow::Result<(Self, wgpu::TextureFormat, bool)> {
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
        Ok((Self::with(Output::Surface { surface, config }, (width, height)), format, premultiplied))
    }

    /// The image to draw this frame on: its surface's next (a surface its window has outgrown, or lost, configured
    /// again and its image taken from that, so the frame drawn as a window changes size isn't dropped; an image taken
    /// ahead drawn on first, where it's the surface's size), or its offscreen texture. None to draw on now (a window
    /// hidden or asleep, a surface lost).
    pub(super) fn acquire(&mut self, device: &wgpu::Device) -> Option<Image> {
        let reserved = self.reserved.take();
        match &self.output {
            Output::Surface { surface, config } => {
                let reserved = reserved.filter(|frame| (frame.texture.width(), frame.texture.height()) == (config.width, config.height));
                let frame = match reserved.map_or_else(|| surface.get_current_texture(), wgpu::CurrentSurfaceTexture::Success) {
                    wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                        surface.configure(device, config);
                        surface.get_current_texture()
                    }
                    taken => taken,
                };
                let frame = match frame {
                    wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => return None,
                };
                let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let texture = frame.texture.clone();
                Some(Image { frame: Some(frame), view, texture })
            }
            Output::Offscreen { texture, view, .. } => Some(Image { frame: None, view: view.clone(), texture: texture.clone() }),
        }
    }

    /// Its window's new size, in device pixels.
    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        match &mut self.output {
            Output::Surface { surface, config } if (config.width, config.height) != size => {
                (config.width, config.height) = size;
                self.reserved = None;
                surface.configure(device, config);
                self.size = size;
            }
            // An image drawn offscreen as a window would be (the fake platform's) is made again at its new size.
            Output::Offscreen { .. } if self.size != size => {
                self.output = offscreen(device, size);
                self.size = size;
            }
            _ => {}
        }
    }

    /// Its window is going from the screen (`Renderer::put_away`).
    pub(super) fn put_away(&mut self, device: &wgpu::Device) {
        self.idle();
        match &mut self.output {
            Output::Surface { surface, .. } if cfg!(target_os = "macos") => {
                if self.reserved.is_none()
                    && let wgpu::CurrentSurfaceTexture::Success(frame) = surface.get_current_texture()
                {
                    self.reserved = Some(frame);
                }
            }
            Output::Surface { surface, config } if cfg!(any(windows, target_os = "ios")) && (config.width, config.height) != (1, 1) => {
                (config.width, config.height) = (1, 1);
                surface.configure(device, config);
                self.size = (1, 1);
            }
            Output::Offscreen { .. } if self.size != (1, 1) => {
                self.output = offscreen(device, (1, 1));
                self.size = (1, 1);
            }
            _ => {}
        }
    }

    /// Whether it holds what it lets go of when idle (`idle`).
    pub(super) fn holds(&self) -> bool {
        self.kept.is_some() || self.backdrop.is_some() || !self.blurred.is_empty()
    }

    /// Lets go of the frame it kept and its backdrops' textures and blurs (`Renderer::idle`).
    pub(super) fn idle(&mut self) {
        self.kept = None;
        self.backdrop = None;
        self.blurred.clear();
    }

    /// The blur kept from the frames before of what's behind a backdrop, where what's drawn before it is the same as
    /// this frame's; or, where it changed only in part, that blur as `blur` takes it again there, into the one kept
    /// (`Blur::damaged`), which is no longer kept as it was.
    pub(super) fn kept_blur(&mut self, blur: &super::blur::Blur) -> Option<Result<wgpu::TextureView, super::blur::Blur>> {
        let behind = blur.behind?;
        if let Some(kept) = self.blurred.get_mut(&behind) {
            kept.used = true;
            return Some(Ok(kept.view.clone()));
        }
        // The one that leaves least to blur again, where several would do (another backdrop's over the same).
        let area = |[left, top, right, bottom]: [u32; 4]| u64::from(right.saturating_sub(left)) * u64::from(bottom.saturating_sub(top));
        let (was, (take, keep)) = self.blurred.iter().filter(|(_, kept)| !kept.used).filter_map(|(was, kept)| Some((*was, blur.damaged(kept)?))).min_by_key(|(was, (take, _))| (area(*take), *was))?;
        let kept = self.blurred.remove(&was).expect("found above");
        Some(Err(super::blur::Blur { region: take, into: Some((kept.view, keep)), ..blur.clone() }))
    }

    /// The bytes of GPU memory its surface's images hold (three, `desired_maximum_frame_latency` 2's), or its
    /// offscreen texture and readback.
    pub(super) fn surface_bytes(&self) -> u64 {
        match &self.output {
            Output::Surface { config, .. } => 3 * u64::from(config.width) * u64::from(config.height) * u64::from(config.format.block_copy_size(None).unwrap_or(4)),
            Output::Offscreen { texture, readback, .. } => super::bytes(texture) + readback.size(),
        }
    }

    /// Its last frame, if it's drawn offscreen, as RGBA rows, top first.
    pub(super) fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<(u32, u32, Vec<u8>)> {
        let Output::Offscreen { texture, readback, row, .. } = &self.output else { return None };
        let (width, height) = self.size;
        // Copied out only as it's read, so a frame that isn't costs no copy.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*row), rows_per_image: None } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        readback.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
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
