//! The files a page and the Workers are made of, as this build holds them or as a checkout has them: on a throwaway
//! network started with a UI folder, and when `one-dev` assembles the web client and the Workers with no Rust build.

use std::borrow::Cow;

/// The text of the file at `path` in the repo, which this build holds as `built`.
pub fn text(path: &str, built: &'static str) -> Cow<'static, str> {
    match from_folder(path) {
        Some(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
        None => Cow::Borrowed(built),
    }
}

/// The files of `config::CLOUDFLARE`: as a checkout has them, every one that's there now, when there is one to read,
/// else as this build holds them. The logo and icons stay this build's, since its config names them.
pub fn cloudflare() -> Vec<(String, Vec<u8>)> {
    let built = crate::config::CLOUDFLARE.iter().map(|(name, bytes)| (name.to_string(), bytes.to_vec()));
    let Some(root) = folder() else { return built.collect() };
    let mut files: Vec<(String, Vec<u8>)> = crate::ui_files::list(&root).into_iter().filter_map(|(name, path)| Some((name, std::fs::read(path).ok()?))).collect();
    files.extend(built.filter(|(name, _)| ["web/logo.png", "web/icon.png", "web/icon-full.png"].contains(&name.as_str())));
    files.sort();
    files
}

#[cfg(feature = "node")]
static CHOSEN: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Has this program read every page and Worker file from the checkout at `folder`, as `one-dev` does to send them out.
#[cfg(feature = "node")]
pub fn read_from(folder: &std::path::Path) {
    let _ = CHOSEN.set(folder.to_path_buf());
}

#[cfg(feature = "node")]
fn folder() -> Option<std::path::PathBuf> {
    CHOSEN.get().cloned().or_else(|| match crate::mode::here() {
        crate::mode::Mode::Throwaway(network) => network.ui(),
        crate::mode::Mode::Own => None,
    })
}

#[cfg(not(feature = "node"))]
fn folder() -> Option<std::path::PathBuf> {
    None
}

fn from_folder(path: &str) -> Option<Vec<u8>> {
    std::fs::read(folder()?.join(path)).ok()
}
