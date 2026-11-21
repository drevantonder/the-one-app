//! Pictures decoded in the background, as GPUI's image cache loads its images on its background executor, so a large
//! one never holds up a frame: a box draws the same picture as it was last decoded at another size, or nothing, until
//! it's ready, and the window is woken to draw it then. One that doesn't decode is tried again a little later, as a
//! browser's decode under load is cut short and comes the next time, and is broken only once it's failed every time.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::geometry::{DevicePixels, Size};
use crate::render::atlas::{Atlas, AtlasKey, AtlasTile};

use super::{Decoded, Picture};

#[cfg(not(target_arch = "wasm32"))]
type Job = Box<dyn FnOnce() + Send>;

/// The threads pictures decode on, as many as the computer has cores, shared by every screen.
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
                    // The lock is let go before the job runs, as a `while let` wouldn't, so the threads decode at once.
                    loop {
                        let job = receive.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv();
                        let Ok(job) = job else { break };
                        job();
                    }
                })
                .expect("a thread to decode pictures on");
        }
        Mutex::new(send)
    });
    let _ = jobs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).send(job);
}

/// How many times as long as here a picture takes to decode on the device a harness stands for (`decoding_takes`).
#[cfg(feature = "fake")]
static SLOWER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1f32.to_bits());

/// Each picture decodes `times` as slowly as on this machine, as on a device with a slower CPU (the Surface's is a
/// third as fast as a current desktop's): for a harness, so a page that has many to decode shows them as late as that
/// device would.
#[cfg(feature = "fake")]
pub fn decoding_takes(times: f32) {
    SLOWER.store(times.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// How long decoding `src` into `pixels` pixels takes on the machine a harness runs on, in ms: its bytes' share and its
/// pixels', as the decodes of the Home Screen's and App Store's scenes took there (an SVG's bytes are parsed and its
/// paths filled, 0.5 µs a byte and 0.1 a pixel; a PNG's or a JPEG's are inflated and scaled, 0.01 and 0.055). A picture
/// is so there at the same time on the harness's clock on every run and every machine, where the CPU time it measured
/// varied fourfold from run to run for the same icon (0.53 to 2.38 ms), and with it which copy of a picture a frame drew
/// (the App Library's icons, sharp on one run and smoothed on the next, which read as two GPUs sampling apart).
#[cfg(feature = "fake")]
fn decoding_ms(src: &str, pixels: u32) -> f64 {
    let (per_byte, per_pixel) = if src.starts_with("data:image/svg+xml") { (0.0005, 0.0001) } else { (0.00001, 0.000055) };
    src.len() as f64 * per_byte + f64::from(pixels) * per_pixel
}

/// With a seed, one picture in three fails to decode the first time it's asked for, at once, as a browser's decode under
/// load is cut short (`HTMLImageElement.decode()` rejecting): for a harness, which then shows whether it's tried again.
#[cfg(feature = "fake")]
static CUT_SHORT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Decodes are cut short as `seed` says (`CUT_SHORT`), or none with 0.
#[cfg(feature = "fake")]
pub fn cut_short(seed: u64) {
    CUT_SHORT.store(seed, std::sync::atomic::Ordering::Relaxed);
}

/// How long after a picture fails to decode, or to be fetched on the web, it's tried again, each time it has: one that
/// fails once more is broken, within the second a harness's scene is given to settle.
pub(crate) const RETRIES_MS: [f64; 3] = [50.0, 200.0, 500.0];

/// A picture that didn't decode: how many times it hasn't, and when it's tried again, or that it's being tried again,
/// once its time's come and a frame's drawn it (`get`).
struct Failed {
    times: usize,
    again: Again,
}

#[derive(PartialEq)]
enum Again {
    At(f64),
    Due,
    Trying,
}

/// What's decoded, waiting to go in the atlas, each with when it's there (for a harness, as late as the device it
/// stands for would have it; on a device, as it's decoded), and how many are still decoding.
#[derive(Default)]
struct Decoding {
    done: Vec<(u64, Option<Decoded>, Option<f64>)>,
    running: usize,
}

/// Now, in ms: for a harness, its own clock's, which a picture's time is told by, the same on any machine however
/// busy; on a device, since it started.
fn now() -> f64 {
    #[cfg(feature = "fake")]
    return crate::platform::fake_time();
    #[cfg(not(feature = "fake"))]
    {
        static STARTED: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
        STARTED.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
}

#[derive(Default)]
pub(super) struct Pictures {
    /// Each picture asked for, by its key in the atlas: its size there, or none where it didn't decode.
    sizes: HashMap<u64, Option<Picture>>,
    /// What's decoding, by its key, and when it was asked for (`now`).
    asked: HashMap<u64, f64>,
    decoding: Arc<(Mutex<Decoding>, Condvar)>,
    /// What's decoded and not yet in the atlas, which the next frame that draws it puts there.
    ready: HashMap<u64, Option<Decoded>>,
    /// Each picture decoding's address, by its key: what's broken, if it doesn't.
    addresses: HashMap<u64, u64>,
    /// Each address's picture as it was last put in the atlas, drawn while it decodes at another size.
    latest: HashMap<u64, (u64, Picture)>,
    /// Each address's sizes it's been put in the atlas at, in device pixels.
    copies: HashMap<u64, Vec<Size<u32>>>,
    /// What didn't decode and is tried again, by its key.
    failed: HashMap<u64, Failed>,
    /// Each address that's failed to decode every time it was tried: broken, as a browser's broken image is.
    broken: std::collections::HashSet<u64>,
    /// How many have been decoded, which a frame that has more to show than the last is drawn for.
    pub(super) decoded: u64,
    /// What wakes its window as one's decoded.
    pub(super) wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The longest a picture took from when it was asked for to when its window had it to draw.
    pub(super) slowest: Duration,
    /// How many pictures the frame drawn last drew empty: decoding, or that didn't decode or come.
    pub(super) blank: usize,
    /// How many the frame being drawn has drawn empty.
    drawing_blank: usize,
}

fn hash(key: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

impl Pictures {
    /// Takes what's been decoded since it was last asked and is there by now, and has a frame draw again what's due to
    /// be tried again. → whether anything was.
    pub(super) fn take(&mut self) -> bool {
        let now = now();
        let done: Vec<_> = {
            let mut decoding = self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (there, later) = std::mem::take(&mut decoding.done).into_iter().partition(|(_, _, at)| at.is_none_or(|at| at <= now));
            decoding.done = later;
            there
        };
        let mut any = !done.is_empty();
        for (id, decoded, at) in done {
            let asked = self.asked.remove(&id);
            if decoded.is_none() {
                let failed = self.failed.entry(id).or_insert(Failed { times: 0, again: Again::Trying });
                if let Some(after) = RETRIES_MS.get(failed.times) {
                    failed.times += 1;
                    failed.again = Again::At(at.unwrap_or(now) + after);
                    continue;
                }
                self.failed.remove(&id);
                self.broken.insert(self.addresses.remove(&id).unwrap_or_default());
            }
            self.failed.remove(&id);
            // How long the try that came took: one tried again was cut short, not slow.
            if let Some(asked) = asked {
                self.slowest = self.slowest.max(Duration::from_secs_f64((at.unwrap_or(now) - asked).max(0.0) / 1000.0));
            }
            self.addresses.remove(&id);
            self.sizes.insert(id, decoded.as_ref().map(|decoded| decoded.picture));
            self.ready.insert(id, decoded);
            self.decoded += 1;
        }
        // What's due to be tried again is, as a frame draws it.
        for failed in self.failed.values_mut().filter(|failed| matches!(failed.again, Again::At(at) if at <= now)) {
            failed.again = Again::Due;
            self.decoded += 1;
            any = true;
        }
        any
    }

    /// How many are decoding, decoded and not yet taken, or to be tried again.
    pub(super) fn pending(&self) -> usize {
        let decoding = self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        decoding.running + decoding.done.len() + self.failed.values().filter(|failed| failed.again != Again::Trying).count()
    }

    /// Whether one's decoding, or decoded and there by now, to take: what a harness draws again for, not one that's
    /// there only later on its clock, which comes as its time does (`due`).
    pub(super) fn decoding(&self) -> bool {
        let decoding = self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = now();
        decoding.running > 0 || decoding.done.iter().any(|(_, _, at)| at.is_none_or(|at| at <= now)) || self.failed.values().any(|failed| failed.again == Again::Due)
    }

    /// How long until the next decoded is there, where one's waiting to be (for a harness), or the next that didn't
    /// is tried again, in ms: when its window has something due.
    pub(super) fn due(&self) -> Option<f64> {
        let decoding = self.decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = now();
        let again = self.failed.values().filter_map(|failed| if let Again::At(at) = failed.again { Some(at) } else { None });
        decoding.done.iter().filter_map(|(_, _, at)| *at).chain(again).map(|at| (at - now).max(0.0)).reduce(f64::min)
    }

    /// Whether `src` failed to decode every time it was tried: a broken picture, which its box shows as one.
    pub(super) fn broken(&self, src: &str) -> bool {
        self.broken.contains(&hash(src))
    }

    /// Waits for what's decoding: for a harness, which draws what a step left once it's done.
    pub(super) fn wait(&self) {
        let (decoding, done) = &*self.decoding;
        let mut decoding = decoding.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while decoding.running > 0 {
            decoding = done.wait(decoding).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// A picture the frame being drawn has drawn empty.
    pub(super) fn drew_blank(&mut self) {
        self.drawing_blank += 1;
    }

    /// What's been decoded and wasn't drawn this frame goes: the atlas alone keeps a picture's pixels. What was due to
    /// be tried again and wasn't drawn by this frame, which draws everything again (`decoded`), no longer shows.
    pub(super) fn drawn(&mut self) {
        self.ready.clear();
        self.failed.retain(|_, failed| failed.again != Again::Due);
        self.blank = std::mem::take(&mut self.drawing_blank);
    }

    /// Forgets every picture put in the atlas, as the atlas is made again (its GPU was lost): each is decoded again as
    /// it's next drawn.
    pub(super) fn forget(&mut self) {
        self.sizes.clear();
        self.ready.clear();
        self.latest.clear();
        self.broken.clear();
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
    pub(super) fn get(&mut self, atlas: &Atlas, src: &str, device: Size<u32>) -> Option<(Picture, AtlasTile)> {
        let address = hash(src);
        let device = fitting(device, atlas);
        // A copy up to twice as large is drawn smaller rather than decoded again, as a GPU's mipmap is for what's up to
        // half its size: an icon pressed in, a page's grown to fit its screen.
        let fits = |copy: &&Size<u32>| copy.width >= device.width && copy.height >= device.height && copy.width <= device.width * 2 && copy.height <= device.height * 2;
        let device = self.copies.get(&address).and_then(|copies| copies.iter().filter(fits).min_by_key(|copy| copy.width)).copied().unwrap_or(device);
        let id = hash((device.width, device.height, src));
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
            let copies = self.copies.entry(address).or_default();
            if !copies.contains(&device) {
                copies.push(device);
            }
            return Some((picture, tile));
        }
        // What didn't decode waits till it's due to be tried again.
        let failed = self.failed.get_mut(&id);
        if failed.as_ref().is_some_and(|failed| matches!(failed.again, Again::At(_))) {
            return latest(self);
        }
        if let Some(failed) = failed {
            failed.again = Again::Trying;
        }
        #[cfg(feature = "fake")]
        let retried = self.failed.contains_key(&id);
        if let std::collections::hash_map::Entry::Vacant(asked) = self.asked.entry(id) {
            let when = now();
            asked.insert(when);
            self.addresses.insert(id, address);
            let (decoding, wake, src) = (self.decoding.clone(), self.wake.clone(), src.to_owned());
            decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).running += 1;
            let done = move |decoded: Option<Decoded>, at: Option<f64>| {
                let mut state = decoding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.done.push((id, decoded, at));
                state.running -= 1;
                drop(state);
                decoding.1.notify_all();
                if let Some(wake) = wake {
                    wake();
                }
            };
            // The browser decodes it, as it's asked with a promise, on the web.
            #[cfg(target_arch = "wasm32")]
            wasm_bindgen_futures::spawn_local(async move { done(crate::platform::web::decode(&src, device).await, None) });
            #[cfg(not(target_arch = "wasm32"))]
            run(Box::new(move || {
                // For a harness, one cut short (`CUT_SHORT`) fails as it's asked for.
                #[cfg(feature = "fake")]
                {
                    let seed = CUT_SHORT.load(std::sync::atomic::Ordering::Relaxed);
                    if seed != 0 && !retried && hash((seed, id)) % 3 == 0 {
                        return done(None, Some(when));
                    }
                }
                let decoded = super::paint::decode(&src, device);
                // For a harness, it's there as late as on the device it stands for: its decoding's time here
                // (`decoding_ms`), `SLOWER` times over, on the harness's clock from when it was asked for, so a busy
                // machine's doesn't make it later; on a device, once it's decoded.
                #[cfg(feature = "fake")]
                let at = Some(when + decoding_ms(&src, decoded.as_ref().map_or(device.width * device.height, |decoded| decoded.picture.width * decoded.picture.height)) * f64::from(f32::from_bits(SLOWER.load(std::sync::atomic::Ordering::Relaxed))));
                #[cfg(not(feature = "fake"))]
                let at = None;
                done(decoded, at)
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
/// The picture `key` in the atlas, drawn by `draw` where it isn't there yet, as a host draws its own (`paint_loading`),
/// with no page's pictures to keep it among.
pub(super) fn drawn(atlas: &Atlas, key: &str, draw: impl Fn() -> Option<Decoded>) -> Option<AtlasTile> {
    atlas.get_or_insert_with(&AtlasKey::Image(hash(key)), &mut || Ok(draw().map(tiled))).ok().flatten()
}

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
