// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/directx_renderer.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: it draws a
// scene's quads, shadows, underlines and sprites into a window's swap chain, presented by DirectComposition, or into an
// offscreen texture it reads back for a picture; no paths, video surfaces, MSAA or recovery from a lost device yet. Its
// shaders are compiled from source as the window's thread starts (GPUI's debug builds do the same; its release builds
// compile them with fxc.exe, which a build on Linux hasn't), once for every window. Backdrops, what's behind a node
// blurred, are ours, as the Blade renderer draws them.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D::{Fxc::D3DCompile, *},
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{Interface, PCSTR},
};

use super::atlas::AtlasTextureId;
use super::directx_atlas::DirectXAtlas;
use super::directx_devices::DirectXDevices;
use super::scene::{Backdrop, MonochromeSprite, PolychromeSprite, PrimitiveBatch, Quad, Scene, Shadow, Underline};
use crate::geometry::{DevicePixels, Size};

const RENDER_TARGET_FORMAT: DXGI_FORMAT = DXGI_FORMAT_B8G8R8A8_UNORM;
/// The format an offscreen frame is drawn in: 8-bit RGBA, as a PNG's pixels are.
const OFFSCREEN_FORMAT: DXGI_FORMAT = DXGI_FORMAT_R8G8B8A8_UNORM;
const BUFFER_COUNT: usize = 3;

pub(crate) struct FontInfo {
    pub gamma_ratios: [f32; 4],
    pub grayscale_enhanced_contrast: f32,
}

pub struct DirectXRenderer {
    devices: DirectXDevices,
    atlas: Arc<DirectXAtlas>,
    pipelines: Arc<DirectXRenderPipelines>,
    target: Target,
    instances: Instances,
    global_params_buffer: [Option<ID3D11Buffer>; 1],
    blur_params_buffer: [Option<ID3D11Buffer>; 1],
    width: u32,
    height: u32,
    viewport: [D3D11_VIEWPORT; 1],
    font_info: &'static FontInfo,
    /// What a backdrop blurs: the frame as drawn so far, halved and halved again, then blurred one way and the other.
    backdrop: Option<BackdropTextures>,
}

/// Where a frame is drawn: a window's swap chain, or a texture of our own, which `read` reads back.
enum Target {
    Window { swap_chain: IDXGISwapChain1, _composition: DirectComposition, view: [Option<ID3D11RenderTargetView>; 1] },
    Offscreen { texture: ID3D11Texture2D, view: [Option<ID3D11RenderTargetView>; 1], staging: ID3D11Texture2D },
    /// An offscreen target that couldn't be made: nothing is drawn, and `read` reads nothing.
    None,
}

struct DirectComposition {
    comp_device: IDCompositionDevice,
    comp_target: IDCompositionTarget,
    comp_visual: IDCompositionVisual,
}

/// How many times a backdrop's frame is halved at most: a sixteenth of it, for the widest blur.
const LEVELS: u32 = 4;

/// A texture a backdrop's blur draws into and reads from.
type BlurTexture = (ID3D11Texture2D, [Option<ID3D11ShaderResourceView>; 1], [Option<ID3D11RenderTargetView>; 1]);

/// The textures a backdrop's blur is drawn through, for frames of `size`: the frame, then for each level (half its
/// size, a quarter, …) the frame shrunk to it and the two passes of its blur, and which level the last blur's in.
struct BackdropTextures {
    size: (u32, u32),
    format: DXGI_FORMAT,
    source: (ID3D11Texture2D, [Option<ID3D11ShaderResourceView>; 1]),
    levels: Vec<[BlurTexture; 3]>,
    blurred: usize,
}

/// The shaders and states every renderer draws with, compiled once.
pub(crate) struct DirectXRenderPipelines {
    shadow: Pipeline,
    quad: Pipeline,
    underline: Pipeline,
    mono_sprites: Pipeline,
    poly_sprites: Pipeline,
    backdrops: Pipeline,
    blur: Pipeline,
    blend_state: ID3D11BlendState,
    rasterizer_state: ID3D11RasterizerState,
    sampler: [Option<ID3D11SamplerState>; 1],
}

struct Pipeline {
    vertex: ID3D11VertexShader,
    fragment: ID3D11PixelShader,
}

/// A renderer's buffers of each kind of primitive, each grown as a frame needs.
struct Instances {
    shadows: InstanceBuffer,
    quads: InstanceBuffer,
    underlines: InstanceBuffer,
    mono_sprites: InstanceBuffer,
    poly_sprites: InstanceBuffer,
    backdrops: InstanceBuffer,
}

struct InstanceBuffer {
    element_size: usize,
    buffer: ID3D11Buffer,
    buffer_size: usize,
    view: [Option<ID3D11ShaderResourceView>; 1],
}

#[derive(Debug, Default)]
#[repr(C)]
struct GlobalParams {
    gamma_ratios: [f32; 4],
    viewport_size: [f32; 2],
    grayscale_enhanced_contrast: f32,
    _pad: u32,
}

#[derive(Debug, Default)]
#[repr(C)]
struct BlurParams {
    step: [f32; 2],
    sigma: f32,
    pad: f32,
}

impl DirectXRenderer {
    /// A renderer for a window, by its raw handle.
    pub fn new<I: raw_window_handle::HasWindowHandle>(
        devices: &DirectXDevices,
        atlas: Arc<DirectXAtlas>,
        window: &I,
        size: Size<DevicePixels>,
        _transparent: bool,
    ) -> Result<Self> {
        let raw_window_handle::RawWindowHandle::Win32(handle) = window.window_handle().map_err(|error| anyhow::anyhow!("{error}"))?.as_raw() else {
            anyhow::bail!("not a Win32 window");
        };
        let hwnd = HWND(handle.hwnd.get() as _);
        let (width, height) = extent(size);
        let swap_chain = create_swap_chain_for_composition(&devices.dxgi_factory, &devices.device, width, height)?;
        let composition = DirectComposition::new(&devices.device.cast().context("Creating DXGI device")?, hwnd).context("Creating DirectComposition")?;
        composition.set_swap_chain(&swap_chain).context("Setting swap chain for DirectComposition")?;
        let view = create_render_target_view(&devices.device, &unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }?)?;
        Self::with_target(devices, atlas, Target::Window { swap_chain, _composition: composition, view }, width, height)
    }

    /// A renderer that draws offscreen, `size` device pixels, read back by `read`.
    pub fn offscreen(devices: &DirectXDevices, atlas: Arc<DirectXAtlas>, size: Size<DevicePixels>) -> Self {
        let (width, height) = extent(size);
        let target = Self::offscreen_target(devices, width, height).unwrap_or_else(|error| {
            log::error!("no offscreen target: {error:#}");
            Target::None
        });
        Self::with_target(devices, atlas.clone(), target, width, height).unwrap_or_else(|error| {
            log::error!("no offscreen renderer: {error:#}");
            Self::with_target(devices, atlas, Target::None, width, height).expect("a renderer with no target needs nothing made")
        })
    }

    fn offscreen_target(devices: &DirectXDevices, width: u32, height: u32) -> Result<Target> {
        let texture = create_texture(&devices.device, width, height, OFFSCREEN_FORMAT, D3D11_BIND_RENDER_TARGET.0 as u32, D3D11_USAGE_DEFAULT, 0)?;
        let staging = create_texture(&devices.device, width, height, OFFSCREEN_FORMAT, 0, D3D11_USAGE_STAGING, D3D11_CPU_ACCESS_READ.0 as u32)?;
        let view = create_render_target_view(&devices.device, &texture)?;
        Ok(Target::Offscreen { texture, view, staging })
    }

    fn with_target(devices: &DirectXDevices, atlas: Arc<DirectXAtlas>, target: Target, width: u32, height: u32) -> Result<Self> {
        let pipelines = match devices.pipelines.get() {
            Some(pipelines) => pipelines.clone(),
            None => {
                let pipelines = Arc::new(DirectXRenderPipelines::new(&devices.device)?);
                devices.pipelines.get_or_init(|| pipelines).clone()
            }
        };
        let device = &devices.device;
        Ok(Self {
            devices: devices.clone(),
            atlas,
            pipelines,
            target,
            instances: Instances {
                shadows: InstanceBuffer::new::<Shadow>(device, 4)?,
                quads: InstanceBuffer::new::<Quad>(device, 64)?,
                underlines: InstanceBuffer::new::<Underline>(device, 4)?,
                mono_sprites: InstanceBuffer::new::<MonochromeSprite>(device, 512)?,
                poly_sprites: InstanceBuffer::new::<PolychromeSprite>(device, 16)?,
                backdrops: InstanceBuffer::new::<Backdrop>(device, 4)?,
            },
            global_params_buffer: [Some(create_constant_buffer::<GlobalParams>(device)?)],
            blur_params_buffer: [Some(create_constant_buffer::<BlurParams>(device)?)],
            width,
            height,
            viewport: viewport(width as f32, height as f32),
            font_info: Self::get_font_info(),
            backdrop: None,
        })
    }

    pub fn sprite_atlas(&self) -> &Arc<DirectXAtlas> {
        &self.atlas
    }

    /// The window's new size, in device pixels.
    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let (width, height) = extent(size);
        if self.width == width && self.height == height {
            return;
        }
        if let Err(error) = self.resize(width, height) {
            log::error!("resizing a window's frames: {error:#}");
        }
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.width = width;
        self.height = height;
        self.viewport = viewport(width as f32, height as f32);
        match &mut self.target {
            Target::Window { swap_chain, view, .. } => {
                // Every view of the swap chain's buffers goes before they're resized.
                unsafe { self.devices.device_context.OMSetRenderTargets(None, None) };
                drop(view[0].take());
                unsafe { swap_chain.ResizeBuffers(BUFFER_COUNT as u32, width, height, RENDER_TARGET_FORMAT, DXGI_SWAP_CHAIN_FLAG(0)) }
                    .context("Failed to resize swap chain")?;
                *view = create_render_target_view(&self.devices.device, &unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }?)?;
            }
            Target::Offscreen { .. } | Target::None => self.target = Self::offscreen_target(&self.devices, width, height)?,
        }
        Ok(())
    }

    pub fn destroy(&mut self) {
        self.target = Target::None;
        self.backdrop = None;
        unsafe {
            self.devices.device_context.OMSetRenderTargets(None, None);
            self.devices.device_context.Flush();
        }
    }

    pub fn draw(&mut self, scene: &Scene) {
        if let Err(error) = self.draw_frame(scene) {
            log::error!("drawing a frame: {error:#}");
        }
    }

    fn draw_frame(&mut self, scene: &Scene) -> Result<()> {
        let (view, texture, format) = match &self.target {
            Target::Window { swap_chain, view, .. } => (view.clone(), unsafe { swap_chain.GetBuffer::<ID3D11Texture2D>(0) }?, RENDER_TARGET_FORMAT),
            Target::Offscreen { texture, view, .. } => (view.clone(), texture.clone(), OFFSCREEN_FORMAT),
            Target::None => return Ok(()),
        };
        let context = self.devices.device_context.clone();
        update_buffer(
            &context,
            self.global_params_buffer[0].as_ref().expect("made with the renderer"),
            &[GlobalParams {
                gamma_ratios: self.font_info.gamma_ratios,
                viewport_size: [self.width as f32, self.height as f32],
                grayscale_enhanced_contrast: self.font_info.grayscale_enhanced_contrast,
                _pad: 0,
            }],
        )?;
        unsafe {
            context.ClearRenderTargetView(view[0].as_ref().expect("made with the target"), &[0.0; 4]);
            context.RSSetState(&self.pipelines.rasterizer_state);
            context.OMSetRenderTargets(Some(&view), None);
            context.RSSetViewports(Some(&self.viewport));
        }
        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Shadows(shadows) => self.draw_instances(Kind::Shadows, shadows)?,
                PrimitiveBatch::Quads(quads) => self.draw_instances(Kind::Quads, quads)?,
                PrimitiveBatch::Underlines(underlines) => self.draw_instances(Kind::Underlines, underlines)?,
                PrimitiveBatch::MonochromeSprites { texture_id, sprites } => self.draw_sprites(Kind::MonoSprites, texture_id, sprites)?,
                PrimitiveBatch::PolychromeSprites { texture_id, sprites } => self.draw_sprites(Kind::PolySprites, texture_id, sprites)?,
                PrimitiveBatch::Backdrops(backdrops) => {
                    // Each backdrop blurs what's drawn before it, itself among them.
                    for at in 0..backdrops.len() {
                        self.blur_behind(&texture, format, backdrops[at].blur.0)?;
                        unsafe {
                            context.OMSetRenderTargets(Some(&view), None);
                            context.RSSetViewports(Some(&self.viewport));
                        }
                        let backdrop = self.backdrop.as_ref().expect("made by blur_behind");
                        let blurred = backdrop.levels[backdrop.blurred][2].1.clone();
                        self.upload(Kind::Backdrops, &backdrops[at..at + 1])?;
                        self.set_pipeline(Kind::Backdrops);
                        unsafe {
                            context.PSSetSamplers(0, Some(&self.pipelines.sampler));
                            context.PSSetShaderResources(0, Some(&blurred));
                            context.DrawInstanced(4, 1, 0, 0);
                            context.PSSetShaderResources(0, Some(&[None]));
                        }
                    }
                }
            }
        }
        match &self.target {
            Target::Window { swap_chain, .. } => unsafe { swap_chain.Present(0, DXGI_PRESENT(0)) }.ok().context("Presenting swap chain failed")?,
            Target::Offscreen { texture, staging, .. } => unsafe { context.CopyResource(staging, texture) },
            Target::None => {}
        }
        Ok(())
    }

    /// Copies `items` into their kind's buffer, grown if it's too small.
    fn upload<T>(&mut self, kind: Kind, items: &[T]) -> Result<()> {
        let buffer = self.instances.of(kind);
        if buffer.buffer_size < items.len() {
            *buffer = InstanceBuffer::with_size(&self.devices.device, buffer.element_size, items.len().next_power_of_two())?;
        }
        update_buffer(&self.devices.device_context, &buffer.buffer, items)
    }

    fn set_pipeline(&mut self, kind: Kind) {
        let pipeline = self.pipelines.of(kind);
        let view = self.instances.of(kind).view.clone();
        let context = &self.devices.device_context;
        unsafe {
            context.VSSetShaderResources(1, Some(&view));
            context.PSSetShaderResources(1, Some(&view));
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.VSSetShader(&pipeline.vertex, None);
            context.PSSetShader(&pipeline.fragment, None);
            context.VSSetConstantBuffers(0, Some(&self.global_params_buffer));
            context.PSSetConstantBuffers(0, Some(&self.global_params_buffer));
            context.OMSetBlendState(&self.pipelines.blend_state, None, 0xFFFFFFFF);
        }
    }

    fn draw_instances<T>(&mut self, kind: Kind, items: &[T]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        self.upload(kind, items)?;
        self.set_pipeline(kind);
        unsafe { self.devices.device_context.DrawInstanced(4, items.len() as u32, 0, 0) };
        Ok(())
    }

    fn draw_sprites<T>(&mut self, kind: Kind, texture_id: AtlasTextureId, sprites: &[T]) -> Result<()> {
        if sprites.is_empty() {
            return Ok(());
        }
        self.upload(kind, sprites)?;
        self.set_pipeline(kind);
        let texture_view = self.atlas.get_texture_view(texture_id);
        let context = &self.devices.device_context;
        unsafe {
            context.PSSetSamplers(0, Some(&self.pipelines.sampler));
            context.VSSetShaderResources(0, Some(&texture_view));
            context.PSSetShaderResources(0, Some(&texture_view));
            context.DrawInstanced(4, sprites.len() as u32, 0, 0);
        }
        Ok(())
    }

    /// Blurs what's drawn into `target` so far, `sigma` device pixels, into the backdrop's textures: copied, halved
    /// until the blur left is a few texels, then blurred across and down, each a separable Gaussian's pass.
    fn blur_behind(&mut self, target: &ID3D11Texture2D, format: DXGI_FORMAT, sigma: f32) -> Result<()> {
        let size = (self.width, self.height);
        let halved = |times: u32| ((size.0 >> times).max(1), (size.1 >> times).max(1));
        if self.backdrop.as_ref().is_none_or(|backdrop| backdrop.size != size || backdrop.format != format) {
            let device = &self.devices.device;
            let bind = (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32;
            let source = create_texture(device, size.0, size.1, format, D3D11_BIND_SHADER_RESOURCE.0 as u32, D3D11_USAGE_DEFAULT, 0)?;
            let source_view = create_shader_resource_view(device, &source)?;
            let make = |(width, height): (u32, u32)| -> Result<BlurTexture> {
                let texture = create_texture(device, width, height, format, bind, D3D11_USAGE_DEFAULT, 0)?;
                let resource = create_shader_resource_view(device, &texture)?;
                let target = create_render_target_view(device, &texture)?;
                Ok((texture, resource, target))
            };
            let levels = (1..=LEVELS).map(|level| Ok([make(halved(level))?, make(halved(level))?, make(halved(level))?])).collect::<Result<_>>()?;
            self.backdrop = Some(BackdropTextures { size, format, source: (source, source_view), levels, blurred: 0 });
        }
        // Halved until the blur left is a few texels, each halving the average of four, so the Gaussian then takes
        // every texel it reaches: a wide blur taking texels further apart streaks what's sharp behind it.
        let level = (1..=LEVELS).find(|level| sigma / (1u32 << level) as f32 <= 6.0).unwrap_or(LEVELS);
        let backdrop = self.backdrop.as_mut().expect("made above");
        backdrop.blurred = level as usize - 1;
        let backdrop = self.backdrop.as_ref().expect("made above");
        let mut passes = Vec::new();
        let mut from = &backdrop.source.1;
        for (at, [shrunk, ..]) in backdrop.levels.iter().take(level as usize).enumerate() {
            passes.push((from, &shrunk.2, halved(at as u32 + 1), [0.0, 0.0], 0.0));
            from = &shrunk.1;
        }
        let [_, across, down] = &backdrop.levels[level as usize - 1];
        let extent = halved(level);
        let sigma = sigma / (1u32 << level) as f32;
        passes.push((from, &across.2, extent, [1.0 / extent.0 as f32, 0.0], sigma));
        passes.push((&across.1, &down.2, extent, [0.0, 1.0 / extent.1 as f32], sigma));
        let context = &self.devices.device_context;
        unsafe {
            context.OMSetRenderTargets(None, None);
            context.CopyResource(&backdrop.source.0, target);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.VSSetShader(&self.pipelines.blur.vertex, None);
            context.PSSetShader(&self.pipelines.blur.fragment, None);
            context.PSSetSamplers(0, Some(&self.pipelines.sampler));
            // Each pass replaces what's in its target.
            context.OMSetBlendState(None, None, 0xFFFFFFFF);
        }
        for (from, into, (width, height), step, sigma) in passes {
            update_buffer(context, self.blur_params_buffer[0].as_ref().expect("made with the renderer"), &[BlurParams { step, sigma, pad: 0.0 }])?;
            unsafe {
                context.PSSetShaderResources(0, Some(&[None]));
                context.OMSetRenderTargets(Some(into), None);
                context.RSSetViewports(Some(&viewport(width as f32, height as f32)));
                context.PSSetConstantBuffers(1, Some(&self.blur_params_buffer));
                context.PSSetShaderResources(0, Some(from));
                context.Draw(4, 0);
            }
        }
        unsafe {
            context.PSSetShaderResources(0, Some(&[None]));
            context.OMSetRenderTargets(None, None);
        }
        Ok(())
    }

    /// The last frame drawn offscreen, as RGBA rows, top first.
    pub fn read(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let Target::Offscreen { staging, .. } = &self.target else { return None };
        let context = &self.devices.device_context;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.ok()?;
        let row = self.width as usize * 4;
        let mut bytes = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            // Safety: the mapped texture is `height` rows of `RowPitch` bytes, each at least `row` long.
            let line = unsafe { std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row) };
            bytes.extend_from_slice(line);
        }
        unsafe { context.Unmap(staging, 0) };
        Some((self.width, self.height, bytes))
    }

    pub(crate) fn get_font_info() -> &'static FontInfo {
        static CACHED_FONT_INFO: OnceLock<FontInfo> = OnceLock::new();
        CACHED_FONT_INFO.get_or_init(|| {
            let read = || -> Result<FontInfo> {
                unsafe {
                    let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
                    let render_params: IDWriteRenderingParams1 = factory.CreateRenderingParams()?.cast()?;
                    Ok(FontInfo {
                        gamma_ratios: Self::get_gamma_ratios(render_params.GetGamma()),
                        grayscale_enhanced_contrast: render_params.GetGrayscaleEnhancedContrast(),
                    })
                }
            };
            read().unwrap_or(FontInfo { gamma_ratios: Self::get_gamma_ratios(1.8), grayscale_enhanced_contrast: 1.0 })
        })
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

        [ratios[0] * NORM13, ratios[1] * NORM24, ratios[2] * NORM13, ratios[3] * NORM24]
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Shadows,
    Quads,
    Underlines,
    MonoSprites,
    PolySprites,
    Backdrops,
}

impl Instances {
    fn of(&mut self, kind: Kind) -> &mut InstanceBuffer {
        match kind {
            Kind::Shadows => &mut self.shadows,
            Kind::Quads => &mut self.quads,
            Kind::Underlines => &mut self.underlines,
            Kind::MonoSprites => &mut self.mono_sprites,
            Kind::PolySprites => &mut self.poly_sprites,
            Kind::Backdrops => &mut self.backdrops,
        }
    }
}

impl InstanceBuffer {
    fn new<T>(device: &ID3D11Device, buffer_size: usize) -> Result<Self> {
        Self::with_size(device, std::mem::size_of::<T>(), buffer_size)
    }

    fn with_size(device: &ID3D11Device, element_size: usize, buffer_size: usize) -> Result<Self> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: (element_size * buffer_size) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: element_size as u32,
        };
        let mut buffer = None;
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }?;
        let buffer = buffer.context("no buffer")?;
        let mut view = None;
        unsafe { device.CreateShaderResourceView(&buffer, None, Some(&mut view)) }?;
        Ok(Self { element_size, buffer, buffer_size, view: [view] })
    }
}

impl DirectXRenderPipelines {
    fn new(device: &ID3D11Device) -> Result<Self> {
        // Every entry point compiled at once, each on a thread of its own, as compiling one takes tens of
        // milliseconds.
        let names = ["shadow", "quad", "underline", "monochrome_sprite", "polychrome_sprite", "backdrop", "blur"];
        let compiled: Vec<Result<(Vec<u8>, Vec<u8>)>> = std::thread::scope(|scope| {
            let threads: Vec<_> = names
                .iter()
                .map(|name| scope.spawn(move || Ok((compile(&format!("{name}_vertex"), "vs_4_1")?, compile(&format!("{name}_fragment"), "ps_4_1")?))))
                .collect();
            threads.into_iter().map(|thread| thread.join().unwrap_or_else(|_| Err(anyhow::anyhow!("compiling a shader panicked")))).collect()
        });
        let mut pipelines = Vec::new();
        for shaders in compiled {
            let (vertex, fragment) = shaders?;
            pipelines.push(Pipeline { vertex: create_vertex_shader(device, &vertex)?, fragment: create_fragment_shader(device, &fragment)? });
        }
        let [shadow, quad, underline, mono_sprites, poly_sprites, backdrops, blur] =
            <[Pipeline; 7]>::try_from(pipelines).map_err(|_| anyhow::anyhow!("a pipeline is missing"))?;
        Ok(Self {
            shadow,
            quad,
            underline,
            mono_sprites,
            poly_sprites,
            backdrops,
            blur,
            blend_state: create_blend_state(device)?,
            rasterizer_state: create_rasterizer_state(device)?,
            sampler: [Some(create_sampler(device)?)],
        })
    }

    fn of(&self, kind: Kind) -> &Pipeline {
        match kind {
            Kind::Shadows => &self.shadow,
            Kind::Quads => &self.quad,
            Kind::Underlines => &self.underline,
            Kind::MonoSprites => &self.mono_sprites,
            Kind::PolySprites => &self.poly_sprites,
            Kind::Backdrops => &self.backdrops,
        }
    }
}

/// `entry`, compiled from shaders.hlsl for `target`.
fn compile(entry: &str, target: &str) -> Result<Vec<u8>> {
    const SOURCE: &str = include_str!("shaders.hlsl");
    let entry = format!("{entry}\0");
    let target = format!("{target}\0");
    let mut compiled = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SOURCE.as_ptr() as _,
            SOURCE.len(),
            PCSTR(c"shaders.hlsl".as_ptr() as _),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut compiled,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let said = errors
            .map(|blob| unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())).into_owned() })
            .unwrap_or_default();
        anyhow::bail!("compiling {}: {error} {said}", entry.trim_end_matches('\0'));
    }
    let blob = compiled.context("no shader compiled")?;
    Ok(unsafe { std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()) }.to_vec())
}

impl DirectComposition {
    fn new(dxgi_device: &IDXGIDevice, hwnd: HWND) -> Result<Self> {
        let comp_device: IDCompositionDevice = unsafe { DCompositionCreateDevice(dxgi_device)? };
        let comp_target = unsafe { comp_device.CreateTargetForHwnd(hwnd, true) }?;
        let comp_visual = unsafe { comp_device.CreateVisual() }?;
        Ok(Self { comp_device, comp_target, comp_visual })
    }

    fn set_swap_chain(&self, swap_chain: &IDXGISwapChain1) -> Result<()> {
        unsafe {
            self.comp_visual.SetContent(swap_chain)?;
            self.comp_target.SetRoot(&self.comp_visual)?;
            self.comp_device.Commit()?;
        }
        Ok(())
    }
}

fn extent(size: Size<DevicePixels>) -> (u32, u32) {
    (size.width.0.max(1) as u32, size.height.0.max(1) as u32)
}

fn create_swap_chain_for_composition(dxgi_factory: &IDXGIFactory6, device: &ID3D11Device, width: u32, height: u32) -> Result<IDXGISwapChain1> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: RENDER_TARGET_FORMAT,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT as u32,
        // Composition SwapChains only support the DXGI_SCALING_STRETCH Scaling.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    };
    Ok(unsafe { dxgi_factory.CreateSwapChainForComposition(device, &desc, None)? })
}

fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    bind: u32,
    usage: D3D11_USAGE,
    cpu_access: u32,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu_access,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
    texture.context("no texture")
}

fn create_render_target_view(device: &ID3D11Device, texture: &ID3D11Texture2D) -> Result<[Option<ID3D11RenderTargetView>; 1]> {
    let mut view = None;
    unsafe { device.CreateRenderTargetView(texture, None, Some(&mut view)) }?;
    Ok([Some(view.context("no render target view")?)])
}

fn create_shader_resource_view(device: &ID3D11Device, texture: &ID3D11Texture2D) -> Result<[Option<ID3D11ShaderResourceView>; 1]> {
    let mut view = None;
    unsafe { device.CreateShaderResourceView(texture, None, Some(&mut view)) }?;
    Ok([view])
}

fn viewport(width: f32, height: f32) -> [D3D11_VIEWPORT; 1] {
    [D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: width, Height: height, MinDepth: 0.0, MaxDepth: 1.0 }]
}

fn create_rasterizer_state(device: &ID3D11Device) -> Result<ID3D11RasterizerState> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: false.into(),
        MultisampleEnable: true.into(),
        AntialiasedLineEnable: false.into(),
    };
    let mut state = None;
    unsafe { device.CreateRasterizerState(&desc, Some(&mut state)) }?;
    state.context("no rasterizer state")
}

// https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_blend_desc
fn create_blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    // If the feature level is set to greater than D3D_FEATURE_LEVEL_9_3, the display
    // device performs the blend in linear space, which is ideal.
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0].BlendEnable = true.into();
    desc.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    desc.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    desc.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    desc.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ONE;
    desc.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8;
    let mut state = None;
    unsafe { device.CreateBlendState(&desc, Some(&mut state)) }?;
    state.context("no blend state")
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut output = None;
    unsafe { device.CreateSamplerState(&desc, Some(&mut output)) }?;
    output.context("no sampler")
}

fn create_vertex_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11VertexShader> {
    let mut shader = None;
    unsafe { device.CreateVertexShader(bytes, None, Some(&mut shader)) }?;
    shader.context("no vertex shader")
}

fn create_fragment_shader(device: &ID3D11Device, bytes: &[u8]) -> Result<ID3D11PixelShader> {
    let mut shader = None;
    unsafe { device.CreatePixelShader(bytes, None, Some(&mut shader)) }?;
    shader.context("no pixel shader")
}

fn create_constant_buffer<T>(device: &ID3D11Device) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        // A constant buffer is a multiple of 16 bytes.
        ByteWidth: std::mem::size_of::<T>().next_multiple_of(16) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }?;
    buffer.context("no buffer")
}

fn update_buffer<T>(device_context: &ID3D11DeviceContext, buffer: &ID3D11Buffer, data: &[T]) -> Result<()> {
    unsafe {
        let mut dest = std::mem::zeroed();
        device_context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut dest))?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest.pData as _, data.len());
        device_context.Unmap(buffer, 0);
    }
    Ok(())
}
