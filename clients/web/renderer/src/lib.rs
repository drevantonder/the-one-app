//! What the web client's frame calls: the renderer's web platform (`renderer::platform::web`), linked in here.

#[cfg(target_arch = "wasm32")]
pub use renderer::platform::web;
