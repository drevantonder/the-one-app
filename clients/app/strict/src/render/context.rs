// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/blade/blade_context.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: the GPU context.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0 (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use anyhow::Context as _;
use blade_graphics as gpu;
use std::sync::Arc;

/// The GPU, shared by what draws (its objects unnamed: some Vulkan drivers, as Android's emulator's, crash naming them
/// with VK_EXT_debug_utils, which Blade skips for an empty name): a window's surface, or an offscreen texture (`presentation: false`), with the
/// pipelines each format of target draws with, compiled once for every window.
pub struct BladeContext {
    pub(crate) gpu: Arc<gpu::Context>,
    pub(crate) pipelines: std::sync::Mutex<Vec<(gpu::TextureFormat, gpu::AlphaMode, Arc<super::renderer::BladePipelines>)>>,
}

impl BladeContext {
    pub fn new(presentation: bool) -> anyhow::Result<Self> {
        // Which GPU, by its PCI id, where there's more than one (as Zed's `ZED_DEVICE_ID`).
        let device_id = match std::env::var("ONE_GPU_DEVICE_ID") {
            Ok(value) => Some(parse_pci_id(&value).context("ONE_GPU_DEVICE_ID isn't a PCI id")?),
            Err(_) => None,
        };
        let gpu = unsafe { gpu::Context::init(gpu::ContextDesc { presentation, validation: false, device_id, ..Default::default() }) }
            .map_err(|error| anyhow::anyhow!("no GPU to draw with: {error}"))?;
        Ok(Self { gpu: Arc::new(gpu), pipelines: Default::default() })
    }
}

fn parse_pci_id(id: &str) -> anyhow::Result<u32> {
    let mut id = id.trim();

    if id.starts_with("0x") || id.starts_with("0X") {
        id = &id[2..];
    }
    let is_hex_string = id.chars().all(|c| c.is_ascii_hexdigit());
    let is_4_chars = id.len() == 4;
    anyhow::ensure!(is_4_chars && is_hex_string, "Expected a 4 digit PCI ID in hexadecimal format");

    u32::from_str_radix(id, 16).context("parsing PCI ID as hex")
}
