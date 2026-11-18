//! Our renderer, copied from GPUI's Blade renderer: a frame's scene, the atlas its glyphs and images are packed into,
//! and the renderer that draws it on the GPU.

pub mod atlas;
mod bounds_tree;
pub mod context;
pub mod renderer;
pub mod scene;
