//! Deploy: an app's code into the network. A deploy builds an app into a bundle (`bundle`), a blob its signed deploy
//! names, with the pinned Bun (`build`); adding an app puts it on a page, and removing takes it off (`tools`). What
//! serves them, and the deploy's routes, are the runtime's (`deploy-runtime`).

pub mod bundle;
pub mod says;
pub mod tools;
#[cfg(not(target_family = "wasm"))]
pub mod build;

/// What a screen's HTML has in it to use `@one/ui` (`src/screen/ui/`: React, and shadcn's components): its page then
/// has an import map giving `react`, `react-dom/client`, `@one/ui` and the rest (`ui_imports`), and ui.js, which they
/// come from. Every page has ui.css, `@one/ui`'s theme (shadcn's neutral, light and dark) and what a page starts with,
/// so a plain screen, of HTML and JS alone, looks as the rest do. The screens a deploy builds have it.
pub const UI: &str = r#"<meta name="@one/ui">"#;
