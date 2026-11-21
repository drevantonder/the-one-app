//! Video the browser plays, which the frame's JS (`video.js`) gives the renderer a frame at a time.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use super::WINDOW;
use crate::Screen;
use crate::geometry::{DevicePixels, Size};
use crate::platform::Native;

thread_local! {
    /// Each video's latest frame, as the browser played it, and where it was last drawn (`video`).
    static VIDEOS: RefCell<std::collections::HashMap<crate::tree::Id, Video>> = RefCell::new(std::collections::HashMap::new());
}

/// A video: its latest frame and how many have come, how many were copied into the atlas, and its box as it was last
/// drawn, in CSS pixels, with the device pixels it fills there, which the frame's JS asks its frames at.
#[derive(Default)]
struct Video {
    frame: Option<(Size<u32>, Vec<u8>)>,
    came: u64,
    copied: u64,
    drawn: [f32; 6],
}

/// Video `id`'s latest frame, in its tile of the atlas, written there as it came, and its size, drawn in `content`,
/// `device` pixels, which its next frames are asked at.
pub(crate) fn video(id: crate::tree::Id, content: crate::geometry::Bounds<f32>, device: Size<u32>, atlas: &crate::render::atlas::Atlas) -> Option<(crate::render::atlas::AtlasTile, Size<u32>)> {
    VIDEOS.with_borrow_mut(|videos| {
        let video = videos.entry(id).or_default();
        video.drawn = [content.origin.x, content.origin.y, content.size.width, content.size.height, device.width as f32, device.height as f32];
        let size = video.frame.as_ref()?.0;
        if size.width == 0 || size.height == 0 {
            return None;
        }
        use std::hash::{Hash as _, Hasher as _};
        let mut key = std::collections::hash_map::DefaultHasher::new();
        ("video", id, size.width, size.height).hash(&mut key);
        let padded = Size { width: DevicePixels(size.width as i32 + 2), height: DevicePixels(size.height as i32 + 2) };
        let bytes = vec![0u8; (size.width as usize + 2) * (size.height as usize + 2) * 4];
        let tile = atlas.get_or_insert_with(&crate::render::atlas::AtlasKey::Image(key.finish()), &mut || Ok(Some((padded, std::borrow::Cow::Borrowed(&bytes[..]))))).ok()??;
        if video.copied != video.came {
            video.copied = video.came;
            let bgra = video.frame.as_ref().map(|(_, bgra)| bgra.clone()).unwrap_or_default();
            atlas.write(&tile, Size { width: DevicePixels(size.width as i32), height: DevicePixels(size.height as i32) }, bgra);
        }
        Some((tile, size))
    })
}

/// A frame of video `id`, `width` by `height`, its pixels RGBA, as the browser played it: drawn in the next frame.
#[wasm_bindgen]
pub fn video_frame(id: f64, width: u32, height: u32, mut rgba: Vec<u8>) {
    if rgba.len() != (width * height * 4) as usize {
        return;
    }
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    VIDEOS.with_borrow_mut(|videos| {
        let video = videos.entry(id as u64).or_default();
        video.frame = Some((Size { width, height }, rgba));
        video.came += 1;
    });
    let _ = Native::get().map(|native| {
        native.run(|windows| {
            windows.with(WINDOW, Screen::picture_came);
        })
    });
}

/// Where video `id` was last drawn, in CSS pixels from the canvas's top left, and the device pixels it filled there:
/// `[x, y, width, height, device width, device height]`, or nothing before it's drawn.
#[wasm_bindgen]
pub fn video_drawn(id: f64) -> Vec<f32> {
    VIDEOS.with_borrow(|videos| videos.get(&(id as u64)).map(|video| video.drawn.to_vec()).unwrap_or_default())
}

/// Video `id` is gone: its frames go with it.
#[wasm_bindgen]
pub fn video_gone(id: f64) {
    VIDEOS.with_borrow_mut(|videos| videos.remove(&(id as u64)));
}
