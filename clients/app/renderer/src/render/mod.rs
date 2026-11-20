//! Our renderer, copied from GPUI's: a frame's scene, the atlas its glyphs and images are packed into, and the
//! renderer that draws it on the GPU by wgpu (Vulkan on Linux and Android, Metal on a Mac, Direct3D 12 on Windows,
//! WebGPU on the web), with the GPU's `Context` they share.

pub mod atlas;
mod bindings;
mod bounds_tree;
pub mod context;
pub mod renderer;
pub mod scene;

/// What a renderer draws into, as its platform gives it: a native window's raw handles, or on the web, its canvas.
#[cfg(not(target_arch = "wasm32"))]
pub type SurfaceTarget = wgpu::SurfaceTargetUnsafe;
#[cfg(target_arch = "wasm32")]
pub type SurfaceTarget = wgpu::SurfaceTarget<'static>;
