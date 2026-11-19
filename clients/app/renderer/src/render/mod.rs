//! Our renderer, copied from GPUI's: a frame's scene, the atlas its glyphs and images are packed into, and the
//! renderer that draws it on the GPU by wgpu (Vulkan on Linux and Android, Metal on a Mac, Direct3D 12 on Windows),
//! each named here as `Context`, `Atlas` and `Renderer`, which the rest of the crate draws with.

pub mod atlas;
mod bounds_tree;
pub mod scene;
pub mod wgpu_atlas;
pub mod wgpu_context;
pub mod wgpu_renderer;

pub use {wgpu_atlas::WgpuAtlas as Atlas, wgpu_context::WgpuContext as Context, wgpu_renderer::WgpuRenderer as Renderer};
