//! Pictures decoded in the background, as GPUI's image cache loads its images on its background executor, so a large
//! one never holds up a frame: a box draws the same picture as it was last decoded at another size, or nothing, until
//! it's ready, and the window is woken to draw it then.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};

use crate::geometry::{DevicePixels, Size};
use crate::render::Atlas;
use crate::render::atlas::{AtlasKey, AtlasTile};

use super::{Decoded, Picture};

type Job = Box<dyn FnOnce() + Send>;

/// The threads pictures decode on, as many as the computer has cores, shared by every screen; on the web, which has none
/// here, the screen's own, at once.
#[cfg(target_arch = "wasm32")]
fn run(job: Job) {
    job();
}
#[cfg(not(target_arch = "wasm32"))]
fn run(job: Job) {
    use std::sync::{OnceLock, mpsc};
    static JOBS: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Job>();
        let receive = Arc::new(Mutex::new(receive));
        for _ in 0..std::thread::available_parallelism().map_or(2, usize::from) {
            let receive = receive.clone();
            std::thread::Builder::new()
                .name("pictures".into())
                .spawn(move || {
                    while let Ok(job) = receive.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv() {
                        job();
                    }
                })
                .expect("a thread to decode pictures on");
        }
        Mutex::new(send)
    });
    let _ = jobs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).send(job);
}

/// What's decoded, waiting to go in the atlas, and how many are still decoding.
#[derive(Default)]
struct Decoding {
    done: Vec<(u64, Option<Decoded>)>,
    running: usize,
}

#[derive(Default)]
pub(super) struct Pictures {
    /// Each picture asked for, by its key in the atlas: its size there, or none where it didn't decode.
    sizes: HashMap<u64, Option<Picture>>,
    /// What's decoding, by its key.
    asked: HashSet<u64>,
    decoding: Arc<(Mutex<Decoding>, Condvar)>,
    /// What's decoded and not yet in the atlas, which the next frame that draws it puts there.
    ready: HashMap<u64, Option<Decoded>>,
    /// Each address's picture as it was last put in the atlas, drawn while it decodes at another size.
    latest: HashMap<u64, (u64, Picture)>,
    /// How many have been decoded, which a frame that has more to show than the last is drawn for.
    pub(super) decoded: u64,
    /// What wakes its window as one's decoded.
    pub(super) wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

fn hash(key: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

impl Pictures {
    /// Takes what's been decoded since it was last asked. → whether anything was.
    pub(super) fn take(&mut self) -> bool {
        let done = std::mem::take(&mut self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).done);
        let any = !done.is_empty();
        for (id, decoded) in done {
            self.asked.remove(&id);
            self.sizes.insert(id, decoded.as_ref().map(|decoded| decoded.picture));
            self.ready.insert(id, decoded);
            self.decoded += 1;
        }
        any
    }

    /// How many are decoding, or decoded and not yet taken.
    pub(super) fn pending(&self) -> usize {
        let decoding = self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        decoding.running + decoding.done.len()
    }

    /// Waits for what's decoding: for a harness, which draws what a step left once it's done.
    pub(super) fn wait(&self) {
        let (decoding, done) = &*self.decoding;
        let mut decoding = decoding.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while decoding.running > 0 {
            decoding = done.wait(decoding).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// What's been decoded and wasn't drawn this frame goes: the atlas alone keeps a picture's pixels.
    pub(super) fn drawn(&mut self) {
        self.ready.clear();
    }

    /// Forgets every picture put in the atlas, as the atlas is made again (its GPU was lost): each is decoded again as
    /// it's next drawn.
    pub(super) fn forget(&mut self) {
        self.sizes.clear();
        self.ready.clear();
        self.latest.clear();
    }

    /// An icon's path, drawn by `draw` in this frame the first time, as GPUI draws its paths in the frame: small, so a
    /// page's icons show with it.
    pub(super) fn now(&mut self, atlas: &Atlas, key: &str, draw: impl Fn() -> Option<Decoded>) -> Option<AtlasTile> {
        let id = hash(key);
        if let Some(None) = self.sizes.get(&id) {
            return None;
        }
        let tile = atlas.get_or_insert_with(&AtlasKey::Image(id), &mut || {
            let decoded = draw();
            self.sizes.insert(id, decoded.as_ref().map(|decoded| decoded.picture));
            Ok(decoded.map(tiled))
        });
        tile.ok().flatten()
    }

    /// The picture `src` decoded for a box of `device` pixels by `decode`, and its tile in the atlas: once it's there,
    /// at once; before, decoded in the background, with the same picture as it was last put there at another size, if
    /// it was, in the meantime.
    pub(super) fn get(&mut self, atlas: &Atlas, src: &str, device: Size<u32>, decode: fn(&str, Size<u32>) -> Option<Decoded>) -> Option<(Picture, AtlasTile)> {
        let device = fitting(device, atlas);
        let id = hash((device.width, device.height, src));
        let address = hash(src);
        let latest = |pictures: &Self| {
            let (id, picture) = *pictures.latest.get(&address)?;
            let tile = atlas.get_or_insert_with(&AtlasKey::Image(id), &mut || Ok(None)).ok()??;
            Some((picture, tile))
        };
        if let Some(None) = self.sizes.get(&id) {
            return latest(self);
        }
        let mut ready = self.ready.remove(&id).flatten();
        let tile = atlas
            .get_or_insert_with(&AtlasKey::Image(id), &mut || {
                Ok(ready.take().map(tiled))
            })
            .ok()
            .flatten();
        if let (Some(tile), Some(Some(picture))) = (tile, self.sizes.get(&id).copied()) {
            self.latest.insert(address, (id, picture));
            return Some((picture, tile));
        }
        if self.asked.insert(id) {
            let (decoding, wake, src) = (self.decoding.clone(), self.wake.clone(), src.to_owned());
            decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).running += 1;
            run(Box::new(move || {
                let decoded = decode(&src, device);
                let mut state = decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.done.push((id, decoded));
                state.running -= 1;
                drop(state);
                decoding.1.notify_all();
                if let Some(wake) = wake {
                    wake();
                }
            }));
        }
        latest(self)
    }
}

/// A picture's pixels as the atlas keeps them, with a clear pixel all round, so one drawn between device pixels, as a
/// layer moving by a fraction of one draws it (`view::Layer`), samples nothing beside it in the atlas and loses none of
/// its own edge (`Painter::sprite` draws that pixel outside its box).
/// `device` pixels, or as large as fits its atlas's largest texture, its border and all, the same shape: a picture
/// larger than that is drawn from that, scaled up, as a browser draws one larger than its GPU's textures.
pub(super) fn fitting(device: Size<u32>, atlas: &Atlas) -> Size<u32> {
    let largest = (atlas.largest().max(3) - 2) as f32;
    let (width, height) = (device.width as f32, device.height as f32);
    if width <= largest && height <= largest {
        return device;
    }
    let by = (largest / width).min(largest / height);
    let side = |length: f32| (length * by).round().clamp(1.0, largest) as u32;
    Size { width: side(width), height: side(height) }
}

fn tiled(decoded: Decoded) -> (Size<DevicePixels>, std::borrow::Cow<'static, [u8]>) {
    let (width, height) = (decoded.picture.width as usize, decoded.picture.height as usize);
    let mut bgra = vec![0; (width + 2) * (height + 2) * 4];
    for (row, line) in decoded.bgra.chunks_exact(width * 4).enumerate() {
        let at = ((row + 1) * (width + 2) + 1) * 4;
        bgra[at..at + width * 4].copy_from_slice(line);
    }
    (Size { width: (width as i32 + 2).into(), height: (height as i32 + 2).into() }, std::borrow::Cow::Owned(bgra))
}
