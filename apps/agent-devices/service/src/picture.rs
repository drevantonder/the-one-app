//! Pictures of devices' screens: as they come (PNG or JPEG), and as they go out, JPEG at the width asked.

use anyhow::Result;
use image::{DynamicImage, GenericImageView, RgbImage};

pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    Ok(image::load_from_memory(bytes)?)
}

pub fn decode_file(path: &std::path::Path) -> Result<DynamicImage> {
    decode(&std::fs::read(path)?)
}

pub fn jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&DynamicImage::ImageRgb8(img.to_rgb8()))?;
    Ok(out)
}

/// An image as a JPEG, scaled to width (0: its own size, at most 1600 wide), with the size it is.
pub fn jpeg_of(img: &DynamicImage, width: u32) -> Result<(Vec<u8>, u32, u32)> {
    let (w, h) = img.dimensions();
    let width = if width == 0 { w.min(1600) } else { width };
    let img = if width < w && w > 0 { DynamicImage::ImageRgb8(shrink(img, width, (h * width / w).max(1))) } else { img.clone() };
    Ok((jpeg(&img, 75)?, img.width(), img.height()))
}

fn shrink(img: &DynamicImage, w: u32, h: u32) -> RgbImage {
    let src = img.to_rgb8();
    let (sw, sh) = src.dimensions();
    RgbImage::from_fn(w, h, |x, y| {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1));
        let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1));
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for sy in y0..y1.min(sh) {
            for sx in x0..x1.min(sw) {
                let p = src.get_pixel(sx, sy);
                (r, g, b, n) = (r + p[0] as u32, g + p[1] as u32, b + p[2] as u32, n + 1);
            }
        }
        let n = n.max(1);
        image::Rgb([(r / n) as u8, (g / n) as u8, (b / n) as u8])
    })
}

/// Whether two pictures differ in under 1 pixel in 200.
pub fn alike(a: &DynamicImage, b: &DynamicImage) -> bool {
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let (x, y) = (a.to_rgb8(), b.to_rgb8());
    let differ = x.pixels().zip(y.pixels()).filter(|(p, q)| (0..3).map(|i| (p[i] as i32 - q[i] as i32).unsigned_abs()).sum::<u32>() > 24).count();
    differ * 200 < (a.width() * a.height()) as usize
}
