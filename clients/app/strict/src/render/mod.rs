//! Our renderer, copied from GPUI's: a frame's scene, the atlas its glyphs and images are packed into, and the
//! renderer that draws it on the GPU: GPUI's Blade renderer (Vulkan) on Linux and Android, and its Direct3D 11 one on
//! Windows, each named here as `Context`, `Atlas` and `Renderer`, which the rest of the crate draws with.

pub mod atlas;
mod bounds_tree;
pub mod scene;

#[cfg(not(target_os = "windows"))]
pub mod blade_atlas;
#[cfg(not(target_os = "windows"))]
pub mod context;
#[cfg(not(target_os = "windows"))]
pub mod renderer;
#[cfg(not(target_os = "windows"))]
pub use {blade_atlas::BladeAtlas as Atlas, context::BladeContext as Context, renderer::BladeRenderer as Renderer};

#[cfg(target_os = "windows")]
pub mod directx_atlas;
#[cfg(target_os = "windows")]
pub mod directx_devices;
#[cfg(target_os = "windows")]
pub mod directx_renderer;
#[cfg(target_os = "windows")]
pub use {directx_atlas::DirectXAtlas as Atlas, directx_devices::DirectXDevices as Context, directx_renderer::DirectXRenderer as Renderer};
