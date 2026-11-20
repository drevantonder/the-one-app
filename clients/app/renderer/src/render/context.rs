// Adapted from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/wgpu_context.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host: the GPU, by wgpu, on one backend a
// platform: Vulkan on Linux and Android, Metal on a Mac, Direct3D 12 on Windows.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use anyhow::Context as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The GPU, shared by what draws: every window's surface and every offscreen texture, with the pipelines each format
/// of target draws with, compiled once for every window.
pub struct Context {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) pipelines: Compiled,
    /// Its device was lost (its GPU was reset, as a Windows driver's TDR resets it, or went), so nothing drawn with
    /// it draws again: what draws makes everything again, on a new one (`Windows::recover`).
    lost: Arc<AtomicBool>,
}

/// The pipelines each format of target draws with, by that format and whether it's premultiplied, compiled once for
/// every window: by the first that draws to one, or ahead of it (`Context::prepare`).
pub(crate) type Compiled = Arc<Mutex<Vec<(wgpu::TextureFormat, bool, Arc<super::renderer::Pipelines>)>>>;

/// The one backend each platform draws with: the browser's WebGPU on the web.
const BACKENDS: wgpu::Backends = if cfg!(target_arch = "wasm32") {
    wgpu::Backends::BROWSER_WEBGPU
} else if cfg!(target_vendor = "apple") {
    wgpu::Backends::METAL
} else if cfg!(target_os = "windows") {
    wgpu::Backends::DX12
} else {
    wgpu::Backends::VULKAN
};

impl Context {
    /// Compiles, in the background, the pipelines a window of this platform draws its frames with, as it starts, so its
    /// first window's first frame doesn't wait for its GPU to compile them (half a second on a Mac's Metal): its
    /// format (`Renderer::new`), opaque first, as an app's window is (`transparent`), then see-through, as only a
    /// launcher's panel is on Windows and Linux, then a snapshot's. A phone's windows are never see-through, and Metal
    /// composites no surface premultiplied (it offers Opaque and PostMultiplied), so a Mac's windows draw only straight
    /// alpha too. Compiling the premultiplied pipelines first had the first window wait for them and then compile its
    /// own: a Mac's Home Screen (its renderer 279 ms, 20 Nov), and the Surface's windows reopened after an update
    /// (2 s, issue 26).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare(&self) {
        let (device, queue, compiled) = (self.device.clone(), self.queue.clone(), self.pipelines.clone());
        let format = if cfg!(target_os = "android") { wgpu::TextureFormat::Rgba8Unorm } else { wgpu::TextureFormat::Bgra8Unorm };
        let kinds: &'static [bool] = if cfg!(any(target_os = "windows", target_os = "linux")) { &[false, true] } else { &[false] };
        std::thread::spawn(move || {
            for &premultiplied in kinds {
                super::renderer::Pipelines::shared(&device, &queue, &compiled, format, premultiplied);
            }
            // Then a window's snapshot's, drawn offscreen as a phone's app goes to the background.
            super::renderer::Pipelines::shared(&device, &queue, &compiled, super::renderer::OFFSCREEN_FORMAT, false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> anyhow::Result<Self> {
        futures_lite::future::block_on(Self::create())
    }

    /// The GPU, once it's said it's there, which the web asks with a promise and a native host waits for.
    pub async fn create() -> anyhow::Result<Self> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = BACKENDS;
        // Objects unnamed: some Vulkan drivers, as Android's emulator's, crash naming them with VK_EXT_debug_utils.
        descriptor.flags = wgpu::InstanceFlags::DISCARD_HAL_LABELS;
        // A window's swap chain on Windows is DirectComposition's, as the window has no redirection surface of its
        // own (WS_EX_NOREDIRECTIONBITMAP) and what isn't drawn on a panel is see-through.
        descriptor.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let instance = wgpu::Instance::new(descriptor.with_env());
        let adapter = adapter(&instance).await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
            label: None,
            // Its passes' GPU time, which every frame says, where the GPU can say it (`renderer::Stamps`).
            // And Windows' shaders, compiled ahead of time, passed through (`Layouts::stages`).
            required_features: adapter.features() & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::PASSTHROUGH_SHADERS),
            // Direct3D 12 makes room for this many descriptors up front (1,000,000 unless said, ~100 MB on an
            // integrated GPU); a frame binds a few hundred.
            required_limits: wgpu::Limits { max_non_sampler_bindings: 65_536, ..adapter.limits() },
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            // Textures and buffers are placed in heaps of 8 MB (4 MB for those the CPU writes), and a bigger one,
            // such as a window-sized texture, in one of its own, freed with it. `MemoryUsage` grew each new heap to
            // twice the one before, up to 64 MB, and a heap goes only once nothing's left in it, so a few small
            // textures that stayed held most of them: a Surface's Direct3D 12 kept 67 to 134 MB for 9 to 32 MB.
            memory_hints: wgpu::MemoryHints::Manual { suballocated_device_memory_block_size: 8 << 20..8 << 20 },
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|error| anyhow::anyhow!("no GPU to draw with: {error}"))?;
        device.on_uncaptured_error(Arc::new(|error| log::error!("GPU error: {error}")));
        let lost = Arc::new(AtomicBool::new(false));
        let told = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            log::error!("the GPU was lost ({reason:?}): {message}");
            told.store(true, Ordering::Relaxed);
        });
        Ok(Self { instance, adapter, device, queue, pipelines: Default::default(), lost })
    }

    /// The GPU memory its allocator has taken from the system (`reserved`) and what it's given out of that
    /// (`allocated`), in bytes, where its backend says (Direct3D 12's and Vulkan's; Metal's makes each texture alone),
    /// and how many command encoders it holds (`encoders`): wgpu keeps every one it's needed at once, and a driver
    /// keeps what each recorded at most (Intel's Direct3D 12, 0.3 to 0.6 MB a render pass).
    pub fn memory(&self) -> serde_json::Value {
        let report = self.device.generate_allocator_report();
        serde_json::json!({
            "reserved": report.as_ref().map(|report| report.total_reserved_bytes),
            "allocated": report.as_ref().map(|report| report.total_allocated_bytes),
            "encoders": self.device.get_internal_counters().hal.command_encoders.read(),
        })
    }

    /// Whether its device was lost (`lost`).
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }
}

/// The GPU to draw with: the one `ONE_GPU_DEVICE_ID` names by its PCI id, where there's more than one (as Zed's
/// `ZED_DEVICE_ID`), or the one wgpu prefers, or a software one where there's none (Windows' WARP, Mesa's lavapipe).
async fn adapter(instance: &wgpu::Instance) -> anyhow::Result<wgpu::Adapter> {
    if let Ok(value) = std::env::var("ONE_GPU_DEVICE_ID") {
        let device = parse_pci_id(&value).context("ONE_GPU_DEVICE_ID isn't a PCI id")?;
        return instance
            .enumerate_adapters(BACKENDS)
            .await
            .into_iter()
            .find(|adapter| adapter.get_info().device == device)
            .with_context(|| format!("no GPU {value}"));
    }
    let options = |force_fallback_adapter| wgpu::RequestAdapterOptions { force_fallback_adapter, ..Default::default() };
    match instance.request_adapter(&options(false)).await {
        Ok(adapter) => Ok(adapter),
        Err(_) => instance.request_adapter(&options(true)).await.map_err(|error| anyhow::anyhow!("no GPU to draw with: {error}")),
    }
}

fn parse_pci_id(id: &str) -> anyhow::Result<u32> {
    let id = id.trim();
    let id = id.strip_prefix("0x").or_else(|| id.strip_prefix("0X")).unwrap_or(id);
    anyhow::ensure!(id.len() == 4 && id.chars().all(|c| c.is_ascii_hexdigit()), "Expected a 4 digit PCI ID in hexadecimal format");
    u32::from_str_radix(id, 16).context("parsing PCI ID as hex")
}
