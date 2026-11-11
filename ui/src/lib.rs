//! The UI (`@one/ui`): what every screen is drawn with and the frame around them. A screen's page (`screen`), the
//! client's own screens (the frame's in ui/home and ui/settings, each feature's in its `<feature>/screens/`), the web
//! client's files (`web`, from ui/web and onet/web), the native apps' signed copy of it (`bundle`), and which files of a
//! checkout the Workers and the web client are made of (`ui_files`, `ui_folder`).

/// What this build carries of them: the Workers' and web client's files, the client's own screens, and the WebAssembly
/// (build.rs).
pub mod carried {
    include!(concat!(env!("OUT_DIR"), "/carried.rs"));
}

#[cfg(feature = "web-bundle")]
pub mod bundle;
pub mod screen;
pub mod ui_files;
pub mod ui_folder;
pub mod web;

/// What a screen's HTML has in it to use `@one/ui` (ui/kit: React, and shadcn's components): its page then has an import
/// map giving `react`, `react-dom/client`, `@one/ui` and the rest (`ui_imports`), and ui.js, which they come from. Every
/// page has ui.css, `@one/ui`'s theme (shadcn's neutral, light and dark) and what a page starts with, so a plain screen,
/// of HTML and JS alone, looks as the rest do. The screens a deploy builds have it.
pub const UI: &str = r#"<meta name="@one/ui">"#;
