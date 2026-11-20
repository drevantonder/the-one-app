//! A window's snapshot: the last frame its app drew, kept on disk, which it shows as it opens while its page starts
//! and hears back from what it first asks, as iOS shows an app's last snapshot as it launches
//! (`WindowOptions::snapshot`).

use std::path::{Path, PathBuf};

/// The raster a snapshot is drawn as (`Renderer::picture`), which no layer's is.
pub(super) const RASTER: u32 = u32::MAX;

/// Its file, as light or dark as the system is, where the page's are as it follows it.
fn file(stem: &Path) -> PathBuf {
    let appearance = match super::appearance() {
        crate::Appearance::Dark => "dark",
        crate::Appearance::Light => "light",
    };
    let mut name = stem.as_os_str().to_owned();
    name.push(format!("-{appearance}.png"));
    PathBuf::from(name)
}

/// The snapshot kept at `stem`, as RGBA rows, where there's one `width` by `height` device pixels: one of another
/// size is of a window that isn't this one's now.
pub(super) fn read(stem: &Path, (width, height): (u32, u32)) -> Option<Vec<u8>> {
    let bytes = std::fs::read(file(stem)).ok()?;
    let picture = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.into_rgba8();
    (picture.dimensions() == (width, height)).then(|| picture.into_raw())
}

/// Keeps `rgba`, `width` by `height` device pixels, as the snapshot at `stem`, in the background, written whole or
/// not at all.
pub(super) fn keep(stem: &Path, width: u32, height: u32, rgba: Vec<u8>) {
    let file = file(stem);
    std::thread::spawn(move || {
        // Nobody waits on it.
        let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Min);
        let written = (|| -> anyhow::Result<()> {
            let folder = file.parent().ok_or_else(|| anyhow::anyhow!("a snapshot is kept in a folder"))?;
            std::fs::create_dir_all(folder)?;
            let mut png = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive);
            image::ImageEncoder::write_image(encoder, &rgba, width, height, image::ExtendedColorType::Rgba8)?;
            let mut writing = file.clone().into_os_string();
            writing.push(".writing");
            std::fs::write(&writing, png)?;
            std::fs::rename(&writing, &file)?;
            Ok(())
        })();
        if let Err(error) = written {
            log::warn!("a window's snapshot isn't kept: {error:#}");
        }
    });
}
