// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/directx_devices.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: the Direct3D 11
// device every strict window and offscreen frame draws with, on the first adapter that has one (Windows' own software
// one, WARP, last, where there's no GPU), with the pipelines compiled once for all of them. No debug layer.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1},
        Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS, D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS,
            D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
        },
        Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIAdapter1, IDXGIFactory6},
    },
};
use windows::core::Interface;

/// The device and its immediate context, and what draws with it (`DirectXRenderer`), shared by every renderer.
#[derive(Clone)]
pub struct DirectXDevices {
    pub(crate) dxgi_factory: IDXGIFactory6,
    pub(crate) device: ID3D11Device,
    pub(crate) device_context: ID3D11DeviceContext,
    pub(crate) pipelines: Arc<std::sync::OnceLock<Arc<super::directx_renderer::DirectXRenderPipelines>>>,
}

impl DirectXDevices {
    /// The device. Windows presents a window's frames with DirectComposition whatever `_presentation` says, so a
    /// context for windows and one for offscreen frames are the same.
    pub fn new(_presentation: bool) -> Result<Self> {
        let dxgi_factory: IDXGIFactory6 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS::default()) }.context("Creating DXGI factory")?;
        let adapter = get_adapter(&dxgi_factory).context("Getting DXGI adapter")?;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        let device = get_device(&adapter, Some(&mut context), Some(&mut feature_level)).context("Creating Direct3D device")?;
        log::info!("Direct3D feature level {:#x}", feature_level.0);
        Ok(Self { dxgi_factory, device, device_context: context.context("no immediate context")?, pipelines: Default::default() })
    }
}

fn get_adapter(dxgi_factory: &IDXGIFactory6) -> Result<IDXGIAdapter1> {
    for adapter_index in 0.. {
        let adapter: IDXGIAdapter1 = unsafe { dxgi_factory.EnumAdapters(adapter_index)?.cast()? };
        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            let gpu_name = String::from_utf16_lossy(&desc.Description).trim_matches(char::from(0)).to_string();
            log::info!("Using GPU: {gpu_name}");
        }
        // Check to see whether the adapter supports Direct3D 11, but don't
        // create the actual device yet.
        if get_device(&adapter, None, None).is_ok() {
            return Ok(adapter);
        }
    }
    unreachable!()
}

fn get_device(
    adapter: &IDXGIAdapter1,
    context: Option<*mut Option<ID3D11DeviceContext>>,
    feature_level: Option<*mut D3D_FEATURE_LEVEL>,
) -> Result<ID3D11Device> {
    let mut device: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            feature_level,
            context,
        )?;
    }
    let device = device.context("no device")?;
    let mut data = D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS::default();
    unsafe {
        device
            .CheckFeatureSupport(
                D3D11_FEATURE_D3D10_X_HARDWARE_OPTIONS,
                &mut data as *mut _ as _,
                std::mem::size_of::<D3D11_FEATURE_DATA_D3D10_X_HARDWARE_OPTIONS>() as u32,
            )
            .context("Checking GPU device feature support")?;
    }
    if data.ComputeShaders_Plus_RawAndStructuredBuffers_Via_Shader_4_x.as_bool() {
        Ok(device)
    } else {
        Err(anyhow::anyhow!("Required feature StructuredBuffer is not supported by GPU/driver"))
    }
}
