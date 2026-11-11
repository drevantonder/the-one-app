//! Deploy: an app's code into the network. A deploy builds an app into a bundle (`bundle`), a blob its signed deploy
//! names, with the pinned Bun (`build`), its service where it's deployed from (`service`); adding an app puts it on a page, and removing takes it off (`tools`). What
//! serves them, and the deploy's routes, are the runtime's (`deploy-runtime`).

pub mod bundle;
pub mod says;
pub mod tools;
#[cfg(not(target_family = "wasm"))]
pub mod build;
#[cfg(not(target_family = "wasm"))]
pub mod service;

