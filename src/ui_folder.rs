//! The files a page and the Workers are made of, as this build holds them or, on a throwaway network started with a UI
//! folder, as that folder has them: so a change to them alone is tried with no Rust build.

use std::borrow::Cow;

/// The text of the file at `path` in the repo, which this build holds as `built`.
pub fn text(path: &str, built: &'static str) -> Cow<'static, str> {
    match from_folder(path) {
        Some(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
        None => Cow::Borrowed(built),
    }
}

/// `config::CLOUDFLARE`, each file as `text` has it.
pub fn cloudflare() -> Vec<(String, Vec<u8>)> {
    let source = |name: &str| crate::config::CLOUDFLARE_SOURCES.iter().find(|(n, _)| *n == name).map(|(_, source)| *source);
    crate::config::CLOUDFLARE.iter().map(|(name, built)| (name.to_string(), source(name).and_then(from_folder).unwrap_or_else(|| built.to_vec()))).collect()
}

#[cfg(feature = "node")]
fn from_folder(path: &str) -> Option<Vec<u8>> {
    match crate::mode::here() {
        crate::mode::Mode::Throwaway(network) => std::fs::read(network.ui()?.join(path)).ok(),
        crate::mode::Mode::Own => None,
    }
}

#[cfg(not(feature = "node"))]
fn from_folder(_path: &str) -> Option<Vec<u8>> {
    None
}
