//! Every app's native window, drawn by one thread of one process: one connection to the display, one GPU context, one
//! glyph atlas and one font set, and a QuickJS sandbox per app (its `Screen`'s) on a thread of its own (`js::Thread`),
//! so another open app costs a few MB, as Zed's and Raycast's windows do, and one busy in its JS holds up no window's
//! frames. What the rest of the program asks of them (open one, tell one's app something) is a job run on that thread
//! (`Native::run`), as GPUI's foreground executor runs its work: on a computer a thread of its own, and on a phone the
//! app's UI thread, where its views are (`android`, `ios`).
//!
//! # The contract with a host
//!
//! The look is the same everywhere (the scene, its text and its motion are worked out here, the same on every host);
//! how smooth it is depends on what each platform's host gives the windows. So each gives the same three things, and
//! the inspector's `host` op checks them the same way on a live window of any host (`conformance`), as
//! `examples/host.rs` does on a computer:
//!
//! - **Input as it happened**, in order (`Display::events`, or a phone's touches handed to its window's screen), each
//!   with the device's own time for it, which how fast a finger went is worked out from (`motion::Velocity`), and,
//!   while a press moves, where the device predicts it's going (`Input::Predicted`: iOS's `predictedTouches`,
//!   Android's `MotionPredictor`, the web's `getPredictedEvents`). No host gives the device's time yet, so each event
//!   is timed as it's handled, which events handled together get wrong: the check says so.
//! - **A tick each time the display shows a frame** (`Display::frame`), with when it showed it, while a frame is
//!   wanted (`Display::request_frame`), and none while nothing moves: X11's Present, a Mac's CVDisplayLink, Windows'
//!   compositor clock (DComposition), Android's Choreographer, an iPhone's CADisplayLink, the web's
//!   `requestAnimationFrame`. A display that says nothing (`Display::paced` false) is ticked at 60 Hz by the loop.
//! - **A surface to present on**: a frame presented each tick while something moves, and nothing while it's still.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};

use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::render::{atlas::Atlas, context::Context, renderer::Renderer};
use crate::text::TextSystem;
use crate::Screen;

#[cfg(all(target_os = "linux", not(feature = "fake")))]
mod linux;
#[cfg(all(target_os = "linux", not(feature = "fake")))]
use linux::{Display, Handle, NativeWindow, appearance, button_layout, clipboard, open_files, reduced_motion};
#[cfg(all(target_os = "windows", not(feature = "fake")))]
mod windows;
#[cfg(all(target_os = "windows", not(feature = "fake")))]
use windows::{Display, Handle, NativeWindow, appearance, clipboard, open_files, reduced_motion};
#[cfg(all(target_os = "macos", not(feature = "fake")))]
mod mac;
#[cfg(all(target_os = "macos", not(feature = "fake")))]
use mac::{Display, Handle, NativeWindow, appearance, clipboard, open_files, reduced_motion};
/// A computer's host with nothing behind it, for a harness (`fake`'s top).
#[cfg(feature = "fake")]
pub(crate) mod fake;
#[cfg(feature = "fake")]
use fake::{Display, Handle, NativeWindow, appearance, button_layout, clipboard, open_files, reduced_motion};
#[cfg(target_os = "android")]
mod android;
/// A native screen's semantic tree as AccessKit's, for each phone's screen reader.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) mod a11y;
/// The rail's home line over a phone's app windows, pulled home (`Windows::line`), and an app's window opening out of
/// its icon.
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub(crate) mod rail;
mod conformance;
mod snapshot;
mod frame_timing;
pub use frame_timing::Frames;
pub(crate) use frame_timing::Timing;
mod pressure;
pub use pressure::Pressure;
pub(crate) use pressure::Deadlines;
#[cfg(target_os = "android")]
use android::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion, text_size};
#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "ios")]
use ios::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion, text_size};
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(target_arch = "wasm32")]
use web::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};

/// Whether the keys are a Mac's: ⌘ the edit chords' modifier, ⌥ a word's, as AppKit's text fields have them. A Mac's,
/// and on the web a browser's on a Mac, an iPhone or an iPad.
pub fn mac_keys() -> bool {
    #[cfg(target_arch = "wasm32")]
    return web::mac_keys();
    #[cfg(not(target_arch = "wasm32"))]
    cfg!(target_os = "macos")
}

/// A window, as the windows' thread names it.
pub type WindowId = u64;

/// Why the windows couldn't start, where they couldn't: a phone makes them as its first job comes, a job that's
/// dropped then, so whoever sent it hears why here.
static UNSTARTED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Why the windows couldn't start, if they couldn't.
pub fn unstarted() -> Option<String> {
    UNSTARTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// A job for the windows' thread.
pub type Job = Box<dyn FnOnce(&mut Windows) + Send>;

/// What a window tells whoever opened it.
pub enum Event {
    /// Its first frame is drawn: on screen, or for a panel put away, ready to show.
    Drawn,
    /// Its first frame of its page is drawn, where its first was its snapshot (`WindowOptions::snapshot`): as a
    /// browser's first contentful paint is its page's, not what it showed before.
    Live,
    /// It was closed: by the person, or by `close`.
    Closed,
    /// Its page uses what this host can't draw, which it says in its place: why.
    Mismatched(String),
    /// Its page's module didn't run, so it says only that its app didn't open: why.
    Failed(String),
    /// Its page crashed after it started (`Screen::crashed`), so it shows nothing of it: why.
    Crashed(String),
    /// A panel was summoned and its frame is on screen: how long after it was asked to show (`summon`).
    Shown(std::time::Duration),
    /// Something on it moved and is still now: how smoothly its frames went.
    Frames(Frames),
    /// A panel put itself away, as something else was clicked or came in front.
    PutAway,
    /// A window that isn't a panel lost the keyboard: its person is using something else.
    Unfocused,
    /// Its person asked to quit the app from it: Ctrl-Q on Linux.
    Quit,
}

/// Where an app's window was left on the screen and how big, in device pixels, and whether it was maximized, as its
/// `frame` file keeps it.
#[cfg(all(any(target_os = "windows", target_os = "linux"), not(feature = "fake")))]
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub maximized: bool,
}

#[cfg(all(any(target_os = "windows", target_os = "linux"), not(feature = "fake")))]
impl Frame {
    pub fn read(file: &std::path::Path) -> Option<Frame> {
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    }

    pub fn write(&self, file: &std::path::Path) {
        let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(file, serde_json::to_vec(self).unwrap_or_default()));
        if let Err(error) = written {
            log::warn!("where the window was left isn't kept: {error}");
        }
    }
}

/// A field's menu for text, Cut, Copy, Paste and Select All, as muda makes a menu each system shows its own way
/// (`Display::text_menu`), for window `window`: each item's id names the window and its key (`menu_chosen`).
#[cfg(all(any(target_os = "macos", target_os = "windows", target_os = "linux"), not(feature = "fake")))]
fn text_menu_of(menu: crate::view::TextMenu, window: WindowId) -> muda::Menu {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuItem, PredefinedMenuItem};
    let command = if cfg!(target_os = "macos") { Modifiers::META } else { Modifiers::CONTROL };
    let item = |title: &str, key: &str, code: Code, enabled: bool| MenuItem::with_id(format!("one-text:{window}:{key}"), title, enabled, Some(Accelerator::new(command, code)));
    let (cut, copy, paste, all) = (
        item("Cu&t", "x", Code::KeyX, menu.cut),
        item("&Copy", "c", Code::KeyC, menu.copy),
        item("&Paste", "v", Code::KeyV, menu.paste),
        item("Select &All", "a", Code::KeyA, menu.select_all),
    );
    Menu::with_items(&[&cut, &copy, &paste, &PredefinedMenuItem::separator(), &all]).unwrap_or_default()
}

/// What was chosen in a menu of a window's own (`text_menu_of`), by muda's id, which the app's menu events hand on:
/// its window is told its key, as pressed there. → whether it was one of those.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn menu_chosen(id: &str) -> bool {
    let Some((window, key)) = id.strip_prefix("one-text:").and_then(|rest| rest.split_once(':')) else { return false };
    let (Ok(window), Some(native)) = (window.parse::<WindowId>(), Native::started()) else { return true };
    let mac = cfg!(target_os = "macos");
    let key = crate::inspect::Key { key: key.to_owned(), shift: false, alt: false, ctrl: !mac, meta: mac };
    native.run(move |windows| windows.input(window, 0, Input::Key(key)));
    true
}

/// What a computer does for a page itself: its own picker of files to open (`open_files`, each system's).
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
struct Computer;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl crate::view::Device for Computer {
    fn pick(&mut self, _: crate::tree::Id, _: crate::inspect::Rect, _: &serde_json::Value, _: Option<&str>) -> bool {
        false
    }
    fn share(&mut self, _: &str) -> bool {
        false
    }
    fn haptic(&mut self, _: &str) -> bool {
        false
    }
    fn open_files(&mut self, accept: &[String], multiple: bool, chosen: Box<dyn FnOnce(Vec<std::path::PathBuf>) + Send>) -> bool {
        open_files(accept, multiple, chosen);
        true
    }
}

/// The smallest an app's window can be made, in points, as every desktop app has one: its title row's window
/// controls, its title and its page's Back and Forward and title still fit.
pub const SMALLEST: Size<Pixels> = Size { width: Pixels(480.0), height: Pixels(320.0) };

pub struct WindowOptions {
    pub title: String,
    /// The window's class, which the desktop groups windows by (X11's `WM_CLASS`, Wayland's app id).
    pub app_id: String,
    pub size: Size<Pixels>,
    /// What whoever opened it says it is, which the inspector's `windows` lists with it: its app.
    pub tag: serde_json::Value,
    /// A panel, as a launcher's (Spotlight's, Raycast's) is: no frame, over every other window, out of the taskbar
    /// and the window switcher, with rounded corners of this many points where the system draws them see-through.
    /// It's opened hidden, shown only as it's summoned (`Windows::summon`), and goes as it loses the keyboard.
    pub panel: Option<f32>,
    /// A panel's file of where it was dragged to on each screen, which it's summoned at there again, as Raycast's is.
    pub places: Option<std::path::PathBuf>,
    /// An app window's file of where it was left and how big (`Frame`), which it opens as again, where the system
    /// keeps none itself (Windows, Linux; a Mac keeps it in its defaults).
    pub frame: Option<std::path::PathBuf>,
    /// The window under every other, as a phone's launcher (its Home Screen) is: the system's Back doesn't close it,
    /// and the apps' windows open over it. It's hidden while something else covers the screen (`Windows::hide`).
    pub root: bool,
    /// Where on the screen it opened from, in points: its app's icon on the Home Screen, which pulled home by the
    /// rail's line it goes back into (`Windows::line`).
    pub icon: Option<crate::geometry::Bounds<f32>>,
    /// The window it's a sheet of, as the system attaches one (`Display::attach`): over it, and modal to it, until it
    /// closes. A phone's covers the app it's over already.
    pub owner: Option<WindowId>,
    /// Where its frame is kept as it's left (it loses the keyboard, the app goes to the background, it closes), which
    /// it shows as it opens again, while its page starts and hears back from what it first asks, as iOS shows an app's
    /// last snapshot as it launches (`snapshot`), a new build of its page's too. Whoever opens it names it by whose it
    /// is, so it never shows another person's.
    pub snapshot: Option<std::path::PathBuf>,
    /// Its page draws its title row, with the window's buttons, where the system's frame has its title bar, as an
    /// app's window on Windows and Linux does: so it's made unframed from the start (`Display::unframe`).
    pub title_row: bool,
}

/// Where a panel was dragged to on each screen, by the screen's name as its platform gives it (`Input::Placed`): its
/// top left, in points from the screen's.
pub type Places = HashMap<String, Point<f32>>;

/// Where a page asked for the system's own material (a material token that's `system`): in the page's points, with its
/// corners' radius and how opaque it's drawn. On a panel each is a window of its own, the system's material its
/// background (`Display::arrange`); elsewhere it's drawn as any other material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub bounds: crate::geometry::Bounds<f32>,
    pub radius: f32,
    pub opacity: f32,
}

/// Where to send the windows' thread a job.
#[derive(Clone)]
pub struct Native(Sender);

static STARTED: OnceLock<Result<Native, String>> = OnceLock::new();

#[cfg(all(any(target_os = "linux", target_os = "macos"), not(feature = "fake")))]
type Sender = calloop::channel::Sender<Job>;
#[cfg(all(target_os = "windows", not(feature = "fake")))]
type Sender = windows::Sender;
#[cfg(feature = "fake")]
type Sender = std::sync::mpsc::Sender<Job>;
#[cfg(target_os = "android")]
type Sender = android::Sender;
#[cfg(target_os = "ios")]
type Sender = ios::Sender;
#[cfg(target_arch = "wasm32")]
type Sender = web::Sender;

impl Native {
    /// The windows' thread, started the first time it's asked for.
    pub fn get() -> anyhow::Result<Native> {
        STARTED.get_or_init(|| start().map_err(|error| format!("{error:#}"))).clone().map_err(|error| anyhow::anyhow!(error))
    }

    /// The windows' thread, if it's been started.
    pub fn started() -> Option<Native> {
        STARTED.get()?.clone().ok()
    }

    /// Runs `job` on the windows' thread, where every window and its screen are.
    pub fn run(&self, job: impl FnOnce(&mut Windows) + Send + 'static) {
        let _ = self.0.send(Box::new(job));
    }
}

#[cfg(all(target_os = "windows", not(feature = "fake")))]
use windows::start;
/// The fake platform's windows for an app that asks for them as it would a system's, as the app does built with it
/// (`clients/app`'s `fake`, the app harness): a thread of their own, as a host's, on real time, whose display refreshes
/// at 60 Hz while something moves, its pages' posts and its jobs taken in between, as each host's loop does. With a
/// seed (`ONE_SEED`), it's as busy as a loaded device's: one job in four waits up to 2 s for the thread, as a Surface's
/// launcher waited 4.5 s to be made as the app started (21 Nov). A harness that drives them itself, frame by frame, has
/// them by `Windows::fake` instead (`examples/screenshot.rs`).
#[cfg(feature = "fake")]
fn start() -> anyhow::Result<Native> {
    let (sender, jobs) = std::sync::mpsc::channel::<Job>();
    let (ready_to, ready) = std::sync::mpsc::channel();
    let seed: u64 = std::env::var("ONE_SEED").ok().and_then(|seed| seed.parse().ok()).unwrap_or(0);
    std::thread::Builder::new().name("native".into()).spawn(move || {
        let mut windows = match Windows::fake(1.0, 60.0, crate::js::Clock::real()) {
            Ok(windows) => windows,
            Err(error) => return drop(ready_to.send(Err(error))),
        };
        windows.seed(seed);
        let _ = ready_to.send(Ok(()));
        let mut ticked = web_time::Instant::now();
        loop {
            match jobs.recv_timeout(windows.due().unwrap_or(FRAME).min(FRAME)) {
                Ok(job) => {
                    if seed != 0 && windows.random(4) == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(windows.random(2000)));
                    }
                    job(&mut windows)
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(job) = jobs.try_recv() {
                job(&mut windows);
            }
            windows.wait();
            if windows.wanted() && ticked.elapsed() >= FRAME {
                ticked = web_time::Instant::now();
                windows.tick();
            }
            windows.turn();
        }
    })?;
    ready.recv()??;
    Ok(Native(sender))
}
#[cfg(target_os = "android")]
use android::start;
#[cfg(target_os = "ios")]
use ios::start;
#[cfg(target_arch = "wasm32")]
use web::start;

#[cfg(all(any(target_os = "linux", target_os = "macos"), not(feature = "fake")))]
fn start() -> anyhow::Result<Native> {
    let (sender, channel) = calloop::channel::channel::<Job>();
    let (ready_to, ready) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("native".into()).spawn(move || {
        // It takes the person's input and draws what they see, so it runs ahead of work nobody's waiting on, as an
        // app's main thread does: as far as the system lets it.
        let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max);
        let mut event_loop = match calloop::EventLoop::<Windows>::try_new() {
            Ok(event_loop) => event_loop,
            Err(error) => return drop(ready_to.send(Err(anyhow::anyhow!("{error}")))),
        };
        let windows = Display::connect(&event_loop.handle()).and_then(Windows::new);
        let mut windows = match windows {
            Ok(windows) => windows,
            Err(error) => return drop(ready_to.send(Err(error))),
        };
        let inserted = event_loop.handle().insert_source(channel, |event, _, windows: &mut Windows| {
            if let calloop::channel::Event::Msg(job) = event {
                job(windows);
            }
        });
        if let Err(error) = inserted {
            return drop(ready_to.send(Err(anyhow::anyhow!("{error}"))));
        }
        let _ = ready_to.send(Ok(()));
        loop {
            let timeout = windows.due();
            if let Err(error) = event_loop.dispatch(timeout, &mut windows) {
                log::error!("native windows: {error}");
            }
            windows.turn();
        }
    })?;
    ready.recv()??;
    Ok(Native(sender))
}

/// How long a window's snapshot covers its page while its page hasn't heard back from what it first asked, so the page
/// shows with what it asked for, or as it is, as its page turns out to need.
const SHOWN_WITHIN: std::time::Duration = std::time::Duration::from_millis(1000);
/// How long a window with no snapshot shows its bare canvas while its page starts, before the page shows as it is,
/// its own loading state, so a quick page shows no spinner (Andre, 20 Nov), and how long what covers a page takes to
/// fade into it.
const QUIET: std::time::Duration = std::time::Duration::from_millis(150);
const FADES_IN: std::time::Duration = std::time::Duration::from_millis(200);

/// What covers a window's page as it opens, as iOS shows an app's snapshot as it launches: its snapshot (`picture`),
/// or with none its canvas (the page's background, under its title row), from the moment it shows until its page is
/// ready, made as it's first drawn; then fading into it. A page shown still starting, its own loading state, fades
/// into what it shows once it's ready the same way, what was on the screen its cover.
enum Cover {
    Over(Option<crate::render::scene::Scene>),
    Fading(crate::render::scene::Scene, web_time::Instant),
    Gone,
}

impl Cover {
    /// How opaque it's drawn now, easing in and out as it fades.
    fn opacity(&self) -> f32 {
        match self {
            Cover::Over(_) => 1.0,
            Cover::Fading(_, began) => {
                let t = (since(*began).as_secs_f32() / FADES_IN.as_secs_f32()).min(1.0);
                1.0 - t * t * (3.0 - 2.0 * t)
            }
            Cover::Gone => 0.0,
        }
    }

    fn scene(&self) -> Option<&crate::render::scene::Scene> {
        match self {
            Cover::Over(scene) => scene.as_ref(),
            Cover::Fading(scene, _) => Some(scene),
            Cover::Gone => None,
        }
    }
}

/// The whole of a window `size` device pixels big.
fn whole(size: Size<DevicePixels>) -> crate::geometry::Bounds<crate::geometry::ScaledPixels> {
    let px = crate::geometry::ScaledPixels;
    crate::geometry::Bounds { origin: Point { x: px(0.0), y: px(0.0) }, size: Size { width: px(size.width.0 as f32), height: px(size.height.0 as f32) } }
}

/// One of a window's native windows, each with a surface of its own: a window's only one, or each part of a panel's
/// page that has the system's material (`Glass`), its search and its card, the first the panel itself and the rest its
/// child windows, which move, hide and stack with it.
struct Part {
    native: NativeWindow,
    /// What draws it, once it has a surface: at once on a computer, as its view's is made on Android.
    renderer: Option<Renderer>,
    /// Its box of the page, in points.
    bounds: crate::geometry::Bounds<f32>,
    radius: f32,
}

struct Window {
    /// Its native windows, the first the window itself.
    parts: Vec<Part>,
    /// How big its page is laid out, in points: a panel's as its page asks (`resize`), the rest their window's.
    room: Size<Pixels>,
    /// A panel summoned or showing that's to be placed, as it's summoned (`Display::summon`), before its next frame
    /// shows: summoned before it had drawn, or showing as its page asked for another room.
    placing: bool,
    /// When it was opened, hidden, and whether it's shown: a panel once its page has started, which it's summoned with.
    opened: web_time::Instant,
    shown: bool,
    /// A panel's: whether it's showing, and since when it was asked to, until its frame is drawn.
    panel: bool,
    visible: bool,
    summoned: Option<web_time::Instant>,
    /// When a panel last went as something else took the keyboard (`showing`).
    left: Option<web_time::Instant>,
    /// The Colour Mode its system's own parts were last drawn in (`Display::set_appearance`).
    chrome: Option<Option<crate::Appearance>>,
    /// What it shows: its app's screen, or why its app didn't open; nothing yet while its page starts (`opening`),
    /// until it's given it (`attach`), which its title row is made for (`WindowOptions::title_row`).
    screen: Result<Screen, String>,
    pending: bool,
    title_row: bool,
    title: String,
    scene: crate::render::scene::Scene,
    /// Whether what it shows changed, and so is drawn on the loop's turn, and whether something on it moves by
    /// itself, and so is drawn again on the display's next frame.
    dirty: bool,
    moving: bool,
    drawn: bool,
    /// Whether a frame of its page is drawn, not its snapshot (`Live`).
    live: bool,
    /// What it painted isn't on the screen yet: painted while it wasn't shown, or not drawn since.
    unshown: bool,
    /// Its page is loading again in its place (`reload`), until it's drawn.
    reloading: bool,
    /// How many frames running it had nothing to draw on (`Window::missed`).
    missed: u32,
    /// What its text input plugin was last told.
    text_input: crate::view::TextInputConnection,
    /// The system's menu for text and its magnifier as its host was last asked to show them, on a phone
    /// (`Screen::text_menu_shown`, `Screen::magnifier`).
    text_menu: Option<crate::view::TextMenu>,
    magnifier: Option<crate::view::Magnifier>,
    on_event: Box<dyn FnMut(Event)>,
    tag: serde_json::Value,
    /// A panel's file of where it was dragged to on each screen (`WindowOptions::places`).
    places: Option<std::path::PathBuf>,
    /// It's the root window (`WindowOptions::root`).
    root: bool,
    /// Where it opened from (`WindowOptions::icon`), and the finger on its rail's line, while there's one, and the
    /// move home it drives.
    #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
    icon: Option<crate::geometry::Bounds<f32>>,
    pull: Option<rail::Pull>,
    /// Its spring from a swipe back from the left edge let go (`view::touch`), where the host leaves that to its app.
    #[cfg_attr(not(any(target_os = "ios", feature = "fake")), allow(dead_code))]
    slide: Option<Slide>,
    /// Where its frame is kept (`WindowOptions::snapshot`), and the one kept, as RGBA rows its size, which covers its
    /// page until its page has started and heard back, and whether its renderer has it yet; what covers its page now,
    /// and whether its page shows still starting, its own loading state.
    snapshot: Option<std::path::PathBuf>,
    picture: Option<Vec<u8>>,
    pictured: bool,
    cover: Cover,
    loading: bool,
    /// What covered its page went: its page is drawn afresh once more, without it.
    uncovered: bool,
    /// Its frames while something on it moves.
    timing: frame_timing::Timing,
    /// Whether it's handed the display a frame since the display's last tick (`Windows::turned`).
    presented: bool,
    /// How many frames it's presented, all told.
    frames: u64,
    /// What the person did that waits, in order, for its page to catch up with what they did before (`Windows::input`).
    waiting: VecDeque<Waiting>,
}

/// Input that waits for its window's page (`Windows::input`): part of the window it's on, or a finger, and when it
/// happened, where its system says.
enum Waiting {
    Input(usize, Input),
    Touch { finger: u64, at: Point<f32>, phase: crate::view::touch::Phase, happened: Option<web_time::Instant>, fingers: crate::view::touch::Fingers },
}

/// A window let go of from a swipe back, springing back to where it was or off to the right (`target`, in points), from
/// where it is and how fast it's going (points a second), and when it last stepped.
#[cfg_attr(not(any(target_os = "ios", feature = "fake")), allow(dead_code))]
struct Slide {
    across: f32,
    velocity: f32,
    target: f32,
    stepped: web_time::Instant,
}

/// What a panel's file of places says, or nothing where it hasn't one yet.
fn places(file: Option<&std::path::Path>) -> Places {
    let read = file.and_then(|file| std::fs::read(file).ok()).and_then(|bytes| serde_json::from_slice::<HashMap<String, [f32; 2]>>(&bytes).ok());
    read.unwrap_or_default().into_iter().map(|(screen, [x, y])| (screen, Point { x, y })).collect()
}

/// Every native window, on the windows' thread.
pub struct Windows {
    display: Display,
    context: Context,
    atlas: Arc<Atlas>,
    text: TextSystem,
    windows: HashMap<WindowId, Window>,
    next: WindowId,
    /// When the display last showed a frame, as its platform says, or the loop's own clock stood in for it.
    framed: web_time::Instant,
    /// Whether the display's frames were asked for as the loop last turned, and whether one has come since they were
    /// (`ticking`).
    wanted: bool,
    ticked: bool,
    /// The inspector's `host` check under way, and what the host has done since it started.
    check: Option<conformance::Check>,
    record: conformance::Record,
    /// The inspector's `record` under way.
    recording: Option<conformance::Recording>,
    /// Whether the last frames met the display's refresh, and how pressed the device is, as that and its system say.
    deadlines: pressure::Deadlines,
    pressure: Pressure,
    /// What the system last said of its pressure, and when it was asked, which is once a second.
    system: (web_time::Instant, Pressure),
    /// The app is in the background, where its system refuses its GPU's work (`background`).
    background: bool,
}

/// How often frames come where a platform says nothing of its display's: a 60 Hz display's.
const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

impl Windows {
    #[cfg(not(any(target_arch = "wasm32", feature = "fake")))]
    fn new(display: Display) -> anyhow::Result<Windows> {
        Self::made(display, Context::new())
    }

    /// Its windows, drawn with `context`, the GPU, where there is one.
    fn made(display: Display, context: anyhow::Result<Context>) -> anyhow::Result<Windows> {
        let made = (|| {
            let context = context?;
            #[cfg(not(target_arch = "wasm32"))]
            context.prepare();
            let atlas = Arc::new(Atlas::new(&context));
            // The system's fonts where ours lack a character: a fake platform has only those the harness's host is
            // given for it (`fake::fallback_fonts`), so it draws the same everywhere.
            #[allow(unused_mut)]
            let mut fonts = crate::fonts()?;
            #[cfg(feature = "fake")]
            fonts.extend(fake::fallback_fonts()?);
            let text = TextSystem::new(crate::FONT, fonts, !cfg!(feature = "fake"));
            Ok(Windows { display, context, atlas, text, windows: HashMap::new(), next: 1, framed: now(), wanted: false, ticked: false, check: None, record: Default::default(), recording: None, deadlines: Default::default(), pressure: Pressure::Nominal, system: (now(), Pressure::Nominal), background: false })
        })();
        if let Err(error) = &made {
            *UNSTARTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!("{error:#}"));
        }
        made
    }

    /// Opens a window showing `screen`, which tells `on_event` what happens to it. It's shown once its app's JS
    /// has started and heard back from what it first asked, or has waited long enough (`SHOWN_WITHIN`), and drawn
    /// as that posts what it changed.
    /// Opens a window of `screen`, or one that says only why its page isn't there, as one whose page didn't start does.
    pub fn open(&mut self, options: WindowOptions, screen: Result<Screen, String>, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        self.create(options, Some(screen), on_event)
    }

    /// Opens a window whose page is still starting, as a browser opens a tab before its page has loaded: its system's
    /// window and renderer are made while its page starts elsewhere, and it shows its snapshot, where it has one, until
    /// its page is given it (`attach`) and has started and heard back, as `open`'s does.
    pub fn opening(&mut self, options: WindowOptions, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        self.create(options, None, on_event)
    }

    /// Window `id`, opened while its page started (`opening`), shows its page now, `screen`. → whether it was such a
    /// window and is still open.
    pub fn attach(&mut self, id: WindowId, mut screen: Screen) -> bool {
        let Some(window) = self.windows.get_mut(&id).filter(|window| window.pending) else { return false };
        fit(&mut screen, &window.parts[0].native, id, window.room(), window.root, window.title_row, self.pressure);
        (window.screen, window.pending, window.dirty) = (Ok(screen), false, true);
        true
    }

    fn create(&mut self, options: WindowOptions, screen: Option<Result<Screen, String>>, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let asked = web_time::Instant::now();
        let native = self.display.open(id, &options)?;
        let windowed = asked.elapsed();
        if let Some(owner) = options.owner.and_then(|owner| self.windows.get(&owner)) {
            self.display.attach(&native, &owner.parts[0].native);
        }
        // An app's window on Linux is unframed, its title row a GNOME header bar with the desktop's window buttons, and
        // on Windows its title row has Windows' caption buttons, drawn by the window: so from the start, before its page
        // is there, so the system's frame never shows first.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if options.title_row {
            self.display.unframe(&native);
        }
        let pending = screen.is_none();
        let mut screen = screen.unwrap_or_else(|| Err(String::new()));
        if let Ok(screen) = &mut screen {
            // Its page is laid out in the room its window has from its first frame, which only the window knows: a
            // phone's is its screen whatever it was asked to be, and a computer's app opens where it was left.
            let room = if options.panel.is_some() { options.size } else { native.size() };
            fit(screen, &native, id, room, options.root, options.title_row, self.pressure);
        }
        let rendering = web_time::Instant::now();
        let renderer = self.renderer(&native)?;
        // Where a window's opening went, which a page's own (`Opening` in clients/app/src/native.rs) has as one step.
        eprintln!("{} opened in {} ms: the system's window {} ms, its renderer {} ms", options.title, asked.elapsed().as_millis(), windowed.as_millis(), rendering.elapsed().as_millis());
        let opened = now();
        let device = native.device_size();
        let picture = options.snapshot.as_deref().filter(|_| pending || screen.is_ok()).and_then(|stem| snapshot::read(stem, (device.width.0 as u32, device.height.0 as u32)));
        let screen_failed = screen.is_err() && !pending;
        let panel = options.panel.is_some();
        let whole = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: Size { width: options.size.width.0, height: options.size.height.0 } };
        self.windows.insert(
            id,
            Window {
                parts: vec![Part { native, renderer, bounds: whole, radius: options.panel.unwrap_or(0.0) }],
                room: options.size,
                placing: false,
                opened,
                shown: false,
                panel,
                visible: false,
                summoned: None,
                left: None,
                chrome: None,
                screen,
                pending,
                title_row: options.title_row,
                title: options.title,
                scene: Default::default(),
                dirty: true,
                moving: false,
                drawn: false,
                live: false,
                reloading: false,
                missed: 0,
                unshown: false,
                text_input: Default::default(),
                text_menu: None,
                magnifier: None,
                on_event: Box::new(on_event),
                tag: options.tag,
                places: options.places,
                root: options.root,
                icon: options.icon,
                pull: None,
                slide: None,
                snapshot: options.snapshot,
                picture,
                pictured: false,
                // A panel is summoned once its page has drawn; one that says why it didn't open has nothing to cover.
                cover: if panel || screen_failed { Cover::Gone } else { Cover::Over(None) },
                loading: false,
                uncovered: false,
                presented: false,
                frames: 0,
                timing: Default::default(),
                waiting: VecDeque::new(),
            },
        );
        Ok(id)
    }

    /// Window `id`'s app's JS posted what it changed: it's applied, and drawn on the loop's turn.
    fn posted(&mut self, id: WindowId) {
        let started = web_time::Instant::now();
        if let Some(window) = self.windows.get_mut(&id)
            && let Ok(screen) = &mut window.screen
        {
            if screen.pump() {
                window.dirty = true;
            }
            screen.unused();
            window.timing.applied(started.elapsed());
        }
        self.catch_up(id);
    }

    /// Whether what the person does to window `id` now waits, behind what already does, or for its page: what's
    /// `discrete` (a press, a key) waits until the page has run what they did before (`Screen::caught_up`), as a
    /// browser runs each input event once its page has run the one before, so a Tab or a press acts on what the page
    /// made of the key or press before it, never on a frame from before it. What's continuous (the pointer or a finger
    /// moving) waits only behind what does, so it comes after it.
    fn waits(&mut self, id: WindowId, discrete: bool) -> bool {
        let Some(window) = self.windows.get(&id) else { return false };
        let Ok(screen) = &window.screen else { return false };
        if !window.waiting.is_empty() || (discrete && !screen.caught_up()) {
            return true;
        }
        if discrete {
            self.lay_out(id);
        }
        false
    }

    /// Window `id`'s page laid out as it is now, where what it changed isn't yet, for what the person does next to hit,
    /// as a browser lays its page out before it hit-tests a click: it's painted now, and its next frame shows that.
    fn lay_out(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let Ok(screen) = &mut window.screen else { return };
        if !screen.laid_out() {
            screen.paint(room, scale, &mut self.text, &self.atlas, &mut window.scene);
            window.dirty = true;
        }
    }

    /// What waited for window `id`'s page is done, in order, while its page has caught up.
    fn catch_up(&mut self, id: WindowId) {
        loop {
            #[cfg(feature = "fake")]
            self.wait();
            let Some(window) = self.windows.get_mut(&id) else { return };
            let Ok(screen) = &window.screen else { return window.waiting.clear() };
            let discrete = match window.waiting.front() {
                None => return,
                Some(Waiting::Input(_, input)) => input.waits() == Some(true),
                Some(Waiting::Touch { finger, phase, .. }) => phase.discrete(screen, *finger),
            };
            if discrete && !screen.caught_up() {
                return;
            }
            if discrete {
                self.lay_out(id);
            }
            let Some(window) = self.windows.get_mut(&id) else { return };
            match window.waiting.pop_front().expect("there's one") {
                Waiting::Input(part, input) => self.act(id, part, input),
                Waiting::Touch { finger, at, phase, happened, fingers } => {
                    let ago = happened.map(|happened| since(happened).as_secs_f64() * 1000.0);
                    self.touched(id, finger, at, phase, ago, fingers);
                }
            }
        }
    }

    /// The app goes to the background, or comes back, as an iPhone's does. While it's there, its system refuses its
    /// GPU's work: Metal fails each command buffer it's given, and what the queue uploads with one, as the glyphs and
    /// pictures a frame added to the atlas, which the atlas holds as there from then on (letters missing from an
    /// iPhone's labels, each where a frame drawn in the background first drew it). So its windows' renderers go, as
    /// a lost surface's do, and what changes meanwhile is drawn once it's back.
    pub fn background(&mut self, background: bool) {
        if self.background == background {
            return;
        }
        self.background = background;
        // Its pages aren't seen meanwhile, as a browser's page in a tab behind isn't, so what each does only while it's
        // seen waits (a toast's time, a refresh).
        for window in self.windows.values_mut() {
            let put_away = window.panel && !window.visible;
            if let Ok(screen) = &mut window.screen {
                screen.seen(!background && !put_away);
            }
        }
        let parts: Vec<(WindowId, usize)> = self.windows.iter().flat_map(|(id, window)| (0..window.parts.len()).map(|part| (*id, part))).collect();
        for (id, part) in parts {
            self.input(id, part, if background { Input::SurfaceLost } else { Input::Surface });
        }
    }

    /// What draws `native`, where it has a surface to draw on yet, the app isn't in the background, and the pipelines
    /// it draws with are compiled: until they are, it has none, so the windows' thread never waits for them, and is
    /// given one as they are (`compiled`).
    fn renderer(&self, native: &NativeWindow) -> anyhow::Result<Option<Renderer>> {
        if self.background || !native.has_surface() || !drawable(&self.context, native) {
            return Ok(None);
        }
        drawn_on(&self.context, self.atlas.clone(), native).map(Some)
    }

    /// The pipelines a kind of window draws with are compiled (`Context::prepare`): each window waiting for them has
    /// its renderer now, and is drawn.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn compiled(&mut self) {
        let waiting: Vec<WindowId> = self.windows.iter().filter(|(_, window)| window.parts[0].renderer.is_none()).map(|(id, _)| *id).collect();
        for id in waiting {
            let made = match self.renderer(&self.windows[&id].parts[0].native) {
                Ok(made) => made,
                Err(error) => {
                    log::error!("native window {id} can't draw: {error:#}");
                    continue;
                }
            };
            if let Some(window) = self.windows.get_mut(&id) {
                window.parts[0].renderer = made;
                window.dirty = true;
            }
        }
    }

    /// Whether a window holds something its person typed that its app hasn't taken (`Screen::typed`): any app's, and
    /// a panel while it's showing.
    pub fn typed(&self) -> bool {
        self.windows.values().any(|window| (!window.panel || window.visible) && window.screen.as_ref().is_ok_and(Screen::typed))
    }

    /// Window `id` shows `page`, a new version of its page, in that one's place (`Screen::reload`), as a browser's tab
    /// loads again: its frame stays on screen until the new page has heard back from what it first asked, or has waited
    /// long enough, as a new window shows (`open`), and it says it's drawn again then (`Drawn`). → whether it's open.
    pub fn reload(&mut self, id: WindowId, page: Screen) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        let Ok(screen) = &mut window.screen else { return false };
        screen.reload(page);
        window.waiting.clear();
        window.opened = now();
        (window.drawn, window.live, window.reloading, window.dirty) = (false, false, true, true);
        true
    }

    /// Does `f` to window `id`'s screen, if it's open, and draws what it changed.
    pub fn with<R>(&mut self, id: WindowId, f: impl FnOnce(&mut Screen) -> R) -> Option<R> {
        let window = self.windows.get_mut(&id)?;
        window.dirty = true;
        window.screen.as_mut().ok().map(f)
    }

    /// Every window, for the inspector's `windows`: what it says it is, its id (`window`), and its `window` op's
    /// answer.
    pub fn list(&mut self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for (id, window) in &mut self.windows {
            let mut listed = window.tag.clone();
            listed["window"] = serde_json::json!(id);
            if let Ok(screen) = &mut window.screen
                && let Ok(request) = crate::inspect::Request::parse(r#"{"v":1,"id":0,"op":"window"}"#)
            {
                let answer = std::rc::Rc::new(std::cell::RefCell::new(serde_json::Value::Null));
                let to = answer.clone();
                screen.inspect(request, Box::new(move |answered| *to.borrow_mut() = answered));
                if let Some(result) = answer.borrow_mut()["result"].as_object() {
                    for (name, value) in result {
                        listed[name] = value.clone();
                    }
                }
            }
            out.push(listed);
        }
        out
    }

    /// Answers an inspector's request of window `id`: `snap` and `profile`, whose frames are drawn again offscreen,
    /// and read back, and the rest by its screen.
    pub fn inspect(&mut self, id: WindowId, request: crate::inspect::Request, reply: crate::Reply) {
        let Some(window) = self.windows.get_mut(&id) else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("there's no window {id} open"))));
        };
        // Closed as its system closes it, as its close button, ⌘W or Alt-F4 do: a panel's put away.
        if request.op == "close" {
            let closed = !window.panel;
            self.input(id, 0, Input::Close);
            return reply(crate::inspect::answer(&request.id, Ok(serde_json::json!({"closed": closed}))));
        }
        // Every input the window gets for `ms`, as a trace's events.
        if request.op == "record" {
            if self.recording.is_some() {
                return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("a recording is under way: ask again once it's said what came"))));
            }
            let started = web_time::Instant::now();
            let until = started + std::time::Duration::from_secs_f64(request.f64("ms").unwrap_or(5000.0).clamp(0.0, 60_000.0) / 1000.0);
            self.recording = Some(conformance::Recording { window: id, started, until, events: Vec::new(), id: request.id, reply });
            return;
        }
        // Whether the host keeps its contract: its window kept moving, then still, what it does meanwhile recorded.
        if request.op == "host" {
            if self.check.is_some() {
                return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("a check is under way: ask again once it's said what held"))));
            }
            self.record = Default::default();
            self.check = Some(conformance::Check { window: id, started: web_time::Instant::now(), id: request.id, reply });
            return;
        }
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let Ok(screen) = &mut window.screen else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("its app didn't open"))));
        };
        window.dirty = true;
        if !matches!(request.op.as_str(), "snap" | "profile" | "replay") {
            return screen.inspect(request, reply);
        }
        // Painted afresh, as it is now: a window with no surface (Android's view while it's hidden) has drawn nothing,
        // and the scene it last drew may be older than what it shows. Each of a profile's or a replay's frames after the
        // first is painted as the window paints one, from what changed.
        let device = Size { width: DevicePixels((room.width.0 * scale).round() as i32), height: DevicePixels((room.height.0 * scale).round() as i32) };
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
        let mut scene = crate::render::scene::Scene::default();
        let (text, atlas) = (&mut self.text, &self.atlas);
        let snaps = crate::inspect::snaps(&request);
        let mut afresh = true;
        let mut draw = |screen: &mut Screen| -> anyhow::Result<serde_json::Value> {
            if std::mem::take(&mut afresh) {
                screen.paint_afresh(room, scale, text, atlas, &mut scene);
            } else {
                screen.paint(room, scale, text, atlas, &mut scene);
            }
            renderer.draw(&scene, [0.0, 0.0]);
            renderer.finish();
            let mut frame = screen.cost(&renderer);
            if snaps {
                let (width, height, rgba) = renderer.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn"))?;
                for (name, value) in crate::inspect::png(width, height, rgba)?.as_object().into_iter().flatten() {
                    frame[name] = value.clone();
                }
            }
            Ok(frame)
        };
        let answered = match request.op.as_str() {
            "snap" => draw(screen),
            "replay" => screen.replay(&request, &mut draw),
            _ => screen.profile(&request, &mut draw),
        };
        renderer.destroy();
        reply(crate::inspect::answer(&request.id, answered))
    }

    /// Brings window `id` forward.
    pub fn activate(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get(&id) {
            self.display.activate(&window.parts[0].native);
        }
    }

    /// Shows panel `id`, centred near the top of the screen the pointer is on, with the keyboard, its frame drawn as
    /// it shows; the window that had the keyboard is kept, to give it back (`put_away`). `asked` is when it was asked
    /// to, which its `Shown` says how long after.
    /// `key` is the key its shortcut ends with, where its shortcut summoned it (`Screen::summoned`).
    pub fn summon(&mut self, id: WindowId, asked: web_time::Instant, key: Option<&str>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        // Showing, it has the keyboard already: a panel never brings the One App in front.
        if window.visible {
            return;
        }
        window.visible = true;
        window.dirty = true;
        window.summoned = Some(asked);
        if let Ok(screen) = &mut window.screen {
            screen.shown(true);
            screen.summoned(key);
        }
        let queued = asked.elapsed();
        // On the screen with what it last drew, where it has; else once its page has drawn its first frame (`turn`), so
        // a launcher summoned as the app starts never shows empty.
        match window.drawn {
            true => self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room),
            false => window.placing = true,
        }
        // Where the time to show it goes: waiting for this thread, then for the system's to place it, then drawing.
        let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
        eprintln!("the launcher's summon waited {:.1} ms for its thread and {:.1} ms to be placed", ms(queued), ms(asked.elapsed() - queued));
        // Drawn now, not on the loop's next turn, so its first frame is the one it shows with.
        self.turn();
    }

    /// Puts panel `id` away, and with `give_back`, gives the keyboard back to the window that had it before.
    pub fn put_away(&mut self, id: WindowId, give_back: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        if !window.visible {
            return;
        }
        window.visible = false;
        window.summoned = None;
        window.placing = false;
        // Painted once more, put away, so what was on its way is where it was going (`Screen::shown`).
        window.dirty = true;
        // Its surfaces let go of their images, or on a Mac its next frame's taken while it's still on the screen, so
        // it's drawn at once as it's summoned (`Renderer::put_away`).
        for renderer in window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()) {
            renderer.put_away();
        }
        if let Some(frames) = window.timing.end(self.display.refresh()) {
            (window.on_event)(Event::Frames(frames));
        }
        if let Ok(screen) = &mut window.screen {
            screen.shown(false);
        }
        self.display.put_away(&window.parts[0].native, give_back);
    }

    /// The device's Back, on window `id`: what's open over its page shuts, or an app's window closes. The root window
    /// leaves Back with nothing open over it to the system. → whether it took it.
    pub fn back(&mut self, id: WindowId) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        if window.screen.as_mut().is_ok_and(Screen::back) {
            window.dirty = true;
            return true;
        }
        if window.root {
            return false;
        }
        self.close(id);
        true
    }

    /// Whether something's open over window `id`'s page (a sheet, a menu), which Back shuts first.
    pub fn covered(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.screen.as_ref().is_ok_and(|screen| screen.covered().is_some()))
    }

    /// Where its memory is, in bytes: the GPU's allocator (`Context::memory`, where its backend says), the atlas,
    /// and each window's, by its entrypoint, whether it's a panel (the launcher) and on screen, what its renderers hold
    /// (`Renderer::memory`) and what its JS held as it was last collected (`Screen::js_memory`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn memory(&self) -> serde_json::Value {
        let gpu = self.context.memory();
        let windows: Vec<serde_json::Value> = self
            .windows
            .values()
            .map(|window| {
                let renderers: Vec<serde_json::Value> = window.parts.iter().filter_map(|part| part.renderer.as_ref().map(Renderer::memory)).collect();
                let screen = window.screen.as_ref().ok();
                serde_json::json!({
                    "entrypoint": window.tag["entrypoint"],
                    "panel": window.panel,
                    "on_screen": window.on_screen(),
                    "renderers": renderers,
                    "js": screen.map_or(0, Screen::js_memory),
                })
            })
            .collect();
        serde_json::json!({"gpu": gpu, "atlas": self.atlas.usage()["bytes"], "windows": windows})
    }

    /// Hides window `id`, or shows it again, as a phone's root window is while the page over it shows an app it
    /// draws itself.
    pub fn hide(&mut self, id: WindowId, hidden: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        if hidden {
            self.display.put_away(&window.parts[0].native, false);
        } else {
            window.dirty = true;
            self.display.show(&window.parts[0].native);
        }
    }

    /// A finger on window `id`, as its platform says, which the rail's line takes where it lands on it (`rail`): it
    /// drives the window's move home from then on, and the first to land keeps it, another on the line meanwhile
    /// ignored. → whether it's the line's, which the window's page doesn't hear.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn line(&mut self, id: WindowId, finger: u64, at: crate::geometry::Point<f32>, phase: crate::view::touch::Phase) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        let (room, icon) = (window.room(), window.icon);
        let Ok(screen) = &mut window.screen else { return false };
        // A finger on the line takes an app opening out of its icon from where it's got to.
        if let Some(pull) = window.pull.as_mut().filter(|pull| pull.opens())
            && phase == crate::view::touch::Phase::Down
            && screen.on_line(at)
        {
            pull.take(finger, at);
            return true;
        }
        let Some(pull) = window.pull.as_mut().filter(|pull| !pull.opens()) else {
            if phase != crate::view::touch::Phase::Down || window.root || !screen.on_line(at) {
                return false;
            }
            window.pull = Some(rail::Pull::new(finger, at));
            return true;
        };
        if pull.finger() != finger {
            return phase == crate::view::touch::Phase::Down && screen.on_line(at);
        }
        match phase {
            crate::view::touch::Phase::Down => {}
            crate::view::touch::Phase::Moved => {
                let was = pull.pulling();
                match pull.moved(at, Size { width: room.width.0, height: room.height.0 }, icon, screen.reduced_motion) {
                    rail::Pulled::Kept(Some(moved)) => {
                        self.display.set_moved(&window.parts[0].native, moved);
                        if !was {
                            // Its line goes with it, and its page is the card's, which no other window shows behind.
                            screen.pulled = true;
                            window.dirty = true;
                            for (_, other) in self.windows.iter().filter(|(other, under)| **other != id && !under.root) {
                                self.display.set_moved(&other.parts[0].native, rail::Moved::HIDDEN);
                            }
                        }
                    }
                    rail::Pulled::Kept(None) => {}
                    rail::Pulled::Dropped => window.pull = None,
                }
            }
            crate::view::touch::Phase::Up | crate::view::touch::Phase::Cancelled => {
                if !pull.let_go(phase == crate::view::touch::Phase::Cancelled) {
                    window.pull = None;
                }
            }
        }
        true
    }

    /// Finger `finger` on window `id` at `at`, in its points, as its system says, `ago` ms before now where the system
    /// says when: the rail's line's where it lands on it, and otherwise its page's (`view::touch`), the window swiped
    /// back from its left edge where the host leaves that to its app, as an iPhone's apps draw it. `fingers` is how the
    /// system differs. → whether it pans, which a host that flings flings as it lifts.
    /// A finger that lands, or lifts to press, waits for its page, and its moves behind it (`waits`).
    pub(crate) fn touch(&mut self, id: WindowId, finger: u64, at: Point<f32>, phase: crate::view::touch::Phase, ago: Option<f64>, fingers: crate::view::touch::Fingers) -> bool {
        let happened = ago.and_then(|ago| now().checked_sub(std::time::Duration::from_secs_f64(ago / 1000.0)));
        if self.check.is_some() {
            self.record.input("touch", happened);
        }
        if let Some(recording) = &mut self.recording {
            recording.input(id, serde_json::json!({"kind": "touch", "phase": format!("{phase:?}").to_lowercase(), "finger": finger, "x": at.x, "y": at.y}), happened);
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if self.line(id, finger, at, phase) {
            return false;
        }
        let discrete = self.windows.get(&id).and_then(|window| window.screen.as_ref().ok()).is_some_and(|screen| phase.discrete(screen, finger));
        if self.waits(id, discrete) {
            let happened = ago.and_then(|ago| now().checked_sub(std::time::Duration::from_secs_f64(ago / 1000.0)));
            if let Some(window) = self.windows.get_mut(&id) {
                window.waiting.push_back(Waiting::Touch { finger, at, phase, happened, fingers });
            }
            return false;
        }
        self.touched(id, finger, at, phase, ago, fingers)
    }

    /// Finger `finger` on window `id`, its page caught up (`touch`).
    fn touched(&mut self, id: WindowId, finger: u64, at: Point<f32>, phase: crate::view::touch::Phase, ago: Option<f64>, fingers: crate::view::touch::Fingers) -> bool {
        use crate::view::touch::Touched;
        #[cfg_attr(not(any(target_os = "ios", feature = "fake")), allow(unused_mut))]
        let mut fingers = fingers;
        let Some(window) = self.windows.get_mut(&id) else { return false };
        window.dirty = true;
        let native = &window.parts[0].native;
        #[cfg(any(target_os = "ios", feature = "fake"))]
        if !window.root {
            fingers.edge = Some(crate::view::touch::Edge { width: native.size().width.0, slid: window.slide.is_some().then(|| native.slid()) });
        }
        let Ok(screen) = &mut window.screen else { return false };
        screen.happened(ago);
        let time = screen.input_time();
        let touched = screen.touch(&fingers, finger, at, phase, time);
        screen.happened(None);
        match touched {
            // Lifted from a pan, on a host that flings it itself (Android's `OverScroller`), as the lift is taken here:
            // at once, or once its page caught up, where it waited (`waits`).
            #[cfg(target_os = "android")]
            Touched::Panning if phase == crate::view::touch::Phase::Up => native.fling(),
            Touched::Panning => return true,
            Touched::Keyboard => self.display.text_input(native, &[crate::editing::TextInput::Show]),
            #[cfg(any(target_os = "ios", feature = "fake"))]
            Touched::Slide(across) => {
                window.slide = None;
                native.slide(across);
            }
            #[cfg(any(target_os = "ios", feature = "fake"))]
            Touched::Slid { velocity, out } => {
                let target = if out { native.size().width.0 } else { 0.0 };
                window.slide = Some(Slide { across: native.slid(), velocity, target, stepped: now() });
            }
            _ => {}
        }
        false
    }

    /// Steps each window springing from a swipe back let go, with the rail's spring, to now: one slid off closes.
    #[cfg(any(target_os = "ios", feature = "fake"))]
    fn slide(&mut self) {
        let at = now();
        let mut off = Vec::new();
        for (id, window) in &mut self.windows {
            let Some(slide) = window.slide.as_mut() else { continue };
            let native = &window.parts[0].native;
            let width = native.size().width.0.max(1.0);
            let dt = at.saturating_duration_since(slide.stepped).as_secs_f32();
            slide.stepped = at;
            let (across, velocity, still) = rail::spring(slide.across / width, slide.velocity / width, slide.target / width, dt);
            (slide.across, slide.velocity) = (across * width, velocity * width);
            native.slide(slide.across);
            window.dirty = true;
            if still {
                if slide.target > 0.0 {
                    off.push(*id);
                }
                window.slide = None;
            }
        }
        for id in off {
            self.close(id);
        }
    }

    /// Steps each window's pull let go, and each opening: home, every app's window closes; back, it's as it was.
    #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
    fn spring(&mut self) {
        let mut home = false;
        let mut back = false;
        for window in self.windows.values_mut() {
            let (room, icon) = (window.room(), window.icon);
            let reduced = window.screen.as_ref().is_ok_and(|screen| screen.reduced_motion);
            // An opening waits on its icon for its window's first frame, which a slow GPU takes a while to show.
            let Some(pull) = window.pull.as_mut().filter(|pull| pull.springing() && !(pull.opens() && window.frames == 0)) else { continue };
            let (moved, still) = pull.step(Size { width: room.width.0, height: room.height.0 }, icon, reduced);
            match still {
                Some(1.0) => home = true,
                Some(_) => {
                    window.pull = None;
                    if let Ok(screen) = &mut window.screen {
                        screen.pulled = false;
                    }
                    window.dirty = true;
                    back = true;
                    self.display.set_moved(&window.parts[0].native, rail::Moved::STILL);
                }
                None => self.display.set_moved(&window.parts[0].native, moved),
            }
        }
        if home {
            let apps: Vec<WindowId> = self.windows.iter().filter(|(_, window)| !window.root).map(|(id, _)| *id).collect();
            for id in apps {
                self.close(id);
            }
        } else if back {
            for window in self.windows.values().filter(|window| !window.root && window.pull.is_none()) {
                self.display.set_moved(&window.parts[0].native, rail::Moved::STILL);
            }
        }
    }

    /// Whether panel `id` is showing, or was until something else took the keyboard just now: a click on Windows'
    /// notification-area icon, which toggles it, first gives the keyboard to the taskbar, so the click that would put
    /// it away would show it again.
    pub fn showing(&self, id: WindowId) -> bool {
        const CLICK: std::time::Duration = std::time::Duration::from_millis(300);
        self.windows.get(&id).is_some_and(|window| window.visible || window.left.is_some_and(|left| since(left) < CLICK))
    }

    /// Lays panel `id`'s page out `height` points tall, and `width` wide where it's said, as its page asks: its parts'
    /// windows follow its page (`Display::arrange`).
    pub fn resize(&mut self, id: WindowId, width: Option<f32>, height: f32) {
        if let Some(window) = self.windows.get_mut(&id) {
            let size = window.room;
            let width = width.unwrap_or(size.width.0);
            if (size.height.0 - height).abs() >= 0.5 || (size.width.0 - width).abs() >= 0.5 {
                window.room = Size { width: Pixels(width), height: Pixels(height) };
                window.dirty = true;
                window.placing = window.visible;
            }
        }
    }

    /// Closes window `id`.
    pub fn close(&mut self, id: WindowId) {
        self.keep_snapshot(id);
        if let Some(mut window) = self.windows.remove(&id) {
            self.let_go(window.parts);
            (window.on_event)(Event::Closed);
        }
    }

    /// Every window's renderers, each part's that has one.
    fn renderers(&self) -> impl Iterator<Item = &Renderer> {
        self.windows.values().flat_map(|window| window.parts.iter().filter_map(|part| part.renderer.as_ref()))
    }

    /// How long until any window's screen has something to do by itself, a window waiting to show has waited long
    /// enough, or a renderer has drawn nothing for a while (`Renderer::idle`).
    pub fn due(&self) -> Option<std::time::Duration> {
        let screens = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due());
        let showing = self.windows.values().filter(|window| !window.shown || matches!(window.cover, Cover::Over(_)) || window.reloading).map(|window| (if window.picture.is_some() { SHOWN_WITHIN } else { QUIET }).saturating_sub(since(window.opened)));
        // Where the display says nothing of its frames, or hasn't yet since they were asked for, the next is a 60 Hz
        // display's.
        let frame = (self.framing() && !self.ticking()).then(|| FRAME.saturating_sub(since(self.framed)));
        // A pull's spring steps at a 120 Hz display's frames.
        let pulled = self.windows.values().any(|window| window.pull.as_ref().is_some_and(rail::Pull::springing)).then_some(std::time::Duration::from_micros(8_333));
        let check = self.check.as_ref().map(conformance::Check::due).into_iter().chain(self.recording.as_ref().map(|recording| recording.until.saturating_duration_since(now()))).min();
        let idle = self.renderers().filter_map(Renderer::idle_in);
        screens.chain(showing).chain(frame).chain(pulled).chain(check).chain(idle).min()
    }

    /// Whether its windows are a phone's: each the whole screen, an app's over the Home Screen, which it opens out of.
    #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
    fn on_phone(&self) -> bool {
        #[cfg(feature = "fake")]
        return self.display.is_phone();
        #[cfg(not(feature = "fake"))]
        cfg!(any(target_os = "android", target_os = "ios"))
    }

    /// Whether the display says when it shows each frame, and has since its frames were last asked for: a display link
    /// just started takes a while to tick (a Mac's two or three refreshes), and the loop's own clock stands in for it
    /// till then, so what started moving isn't held on its first frame.
    fn ticking(&self) -> bool {
        self.display.paced() && self.ticked
    }

    /// Whether a window on screen wants the display's next frame: something on it moves by itself, or its page asked
    /// for one (`requestAnimationFrame`).
    fn framing(&self) -> bool {
        self.check.as_ref().is_some_and(conformance::Check::moving)
            || self.windows.values().any(|window| window.on_screen() && (window.moving || window.slide.is_some() || window.pull.as_ref().is_some_and(rail::Pull::springing) || matches!(window.cover, Cover::Fading(..)) || window.screen.as_ref().is_ok_and(Screen::framing)))
    }

    /// Makes what draws again, on a new GPU device, where the one it drew with was lost, as a browser makes a lost
    /// WebGPU device's: the atlas and every window's renderer, each window drawn afresh (`Screen::gpu_lost`).
    fn recover(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.recovered(Context::new());
        // The web's is asked for with a promise, and drawn on once it's come.
        #[cfg(target_arch = "wasm32")]
        if let Some(made) = web::recover() {
            self.recovered(made);
        }
    }

    fn recovered(&mut self, made: anyhow::Result<Context>) {
        let made = made.map(|context| {
            let atlas = Arc::new(Atlas::new(&context));
            (context, atlas)
        });
        let (context, atlas) = match made {
            Ok(made) => made,
            Err(error) => return log::error!("no GPU to draw with again yet: {error:#}"),
        };
        for window in self.windows.values_mut() {
            for part in &mut window.parts {
                part.renderer = None;
                if !self.background && part.native.has_surface() {
                    match drawn_on(&context, atlas.clone(), &part.native) {
                        Ok(renderer) => part.renderer = Some(renderer),
                        Err(error) => log::error!("a native window can't draw: {error:#}"),
                    }
                }
            }
            if let Ok(screen) = &mut window.screen {
                screen.gpu_lost();
            }
            window.dirty = true;
        }
        (self.context, self.atlas) = (context, atlas);
        log::info!("drawing again on a new GPU device");
    }

    /// After each wait for what's due, a turn of the loop (`turned`), whose time each window's frame timing hears.
    pub fn turn(&mut self) {
        let started = web_time::Instant::now();
        self.turned();
        for window in self.windows.values_mut() {
            window.timing.turned(started.elapsed());
        }
    }

    /// A turn of the loop: what the display said, timers that are due, then each window that changed drawn.
    fn turned(&mut self) {
        let turned = now();
        if self.context.lost() {
            self.recover();
        }
        // Each window waiting for its pipelines has its renderer once they're compiled, on whatever turns the loop: a
        // harness's own turns too, as well as the windows' thread woken as they are (`Context::prepare`).
        #[cfg(not(target_arch = "wasm32"))]
        self.compiled();
        for (id, part, input) in self.display.events() {
            // The fake platform's pages are as quick as what comes, the same each time: each has run what came before it
            // (`wait`), unless a seed has it busy (`Thread::seed`).
            #[cfg(feature = "fake")]
            self.wait();
            self.input(id, part, input);
        }
        for window in self.windows.values_mut() {
            for renderer in window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()) {
                if renderer.idle_in().is_some_and(|due| due.is_zero()) {
                    renderer.idle();
                }
            }
        }
        #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
        self.spring();
        #[cfg(any(target_os = "ios", feature = "fake"))]
        self.slide();
        // The display showed a frame: what moves on a window on screen is drawn again, and its page's
        // `requestAnimationFrame` runs, told when, as a browser's does once a frame.
        let framing = self.framing();
        let ticked = self.display.frame();
        self.ticked |= ticked.is_some();
        let shown = ticked.or_else(|| (framing && !self.ticking() && since(self.framed) >= FRAME).then(now));
        if let Some(at) = shown {
            self.framed = at;
            if let Some(check) = &self.check {
                self.record.tick(at);
                // The window it checks moves on each tick while it's kept moving.
                if check.moving()
                    && let Some(window) = self.windows.get_mut(&check.window)
                {
                    window.moving = true;
                }
            }
            for window in self.windows.values_mut().filter(|window| window.on_screen()) {
                window.dirty |= std::mem::take(&mut window.moving);
                if let Ok(screen) = &mut window.screen {
                    screen.frame(at);
                }
            }
        }
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen
                && screen.due().is_some_and(|due| due.is_zero())
            {
                screen.fire();
                window.dirty = true;
            }
        }
        // A window whose page's module didn't run says only that its app didn't open.
        for window in self.windows.values_mut() {
            if let Some(why) = window.screen.as_ref().ok().and_then(Screen::didnt_start).map(str::to_owned) {
                window.screen = Err(format!("{} didn't open", window.title));
                window.dirty = true;
                (window.on_event)(Event::Failed(why));
            }
            // One whose page crashed shows nothing of it, never what it drew last as if it were alive.
            if let Some(why) = window.screen.as_ref().ok().and_then(Screen::crashed) {
                window.screen = Err(format!("{} stopped working", window.title));
                window.dirty = true;
                (window.on_event)(Event::Crashed(why));
            }
        }
        // The system's own parts of a window (a Mac's glass and title bar, Windows' frame, a Linux window manager's)
        // are drawn light or dark as its Colour Mode says, or as the system is where it says nothing.
        for window in self.windows.values_mut() {
            let theme = window.screen.as_ref().ok().and_then(|screen| screen.theme);
            if window.chrome != Some(theme) {
                window.chrome = Some(theme);
                for part in &window.parts {
                    self.display.set_appearance(&part.native, theme);
                }
            }
        }
        // A window whose page uses what this host can't draw says so, and why.
        for window in self.windows.values_mut() {
            if let Some(why) = window.screen.as_ref().ok().and_then(|screen| screen.mismatched.clone()) {
                window.screen = Err(format!("{} didn't open: {why}", window.title));
                window.dirty = true;
                (window.on_event)(Event::Mismatched(why));
            }
        }
        // While the display's link ticks, a window hands it at most one frame a refresh: what changes after it has drawn
        // since the last tick is drawn on the next, as a browser draws on its frames. Drawn as each input or change
        // came, frames queued ahead of the display, each shown a refresh late, and the thread waited for an image to
        // draw on every frame (an iPhone's main thread, 120 times a second). A change before it has is drawn at once,
        // so what the page answers just after a tick isn't held a refresh; idle, every change is.
        let ticking = self.ticking();
        #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
        let phone = self.on_phone();
        if shown.is_some() {
            for window in self.windows.values_mut() {
                window.presented = false;
            }
        }
        for window in self.windows.values_mut() {
            // A new window shows at once, its page covered (`Cover`) by its snapshot until its page has heard back from
            // what it asked, or has waited long enough, or with no snapshot by its canvas, until its page is ready or a
            // moment has gone by; then what covers it fades into it.
            let starting = (window.pending || window.screen.as_ref().is_ok_and(Screen::starting)) && since(window.opened) < SHOWN_WITHIN;
            let device = window.parts[0].native.device_size();
            if window.picture.as_ref().is_some_and(|picture| picture.len() != (device.width.0 * device.height.0 * 4) as usize) {
                window.picture = None;
            }
            if matches!(window.cover, Cover::Over(_)) && (!starting || (window.picture.is_none() && since(window.opened) >= QUIET)) {
                window.loading = starting;
                window.cover = match std::mem::replace(&mut window.cover, Cover::Gone) {
                    Cover::Over(Some(cover)) => Cover::Fading(cover, now()),
                    _ => Cover::Gone,
                };
            }
            // A page shown as it started is ready: what was on the screen, as it was last drawn, fades into it.
            if window.loading && !starting {
                window.loading = false;
                let mut shown = crate::render::scene::Scene::default();
                shown.splice(&window.scene, 1.0, [0.0, 0.0], 1.0, whole(device));
                shown.finish();
                window.cover = Cover::Fading(shown, now());
            }
            if let Cover::Fading(_, began) = window.cover {
                window.dirty = true;
                if since(began) >= FADES_IN {
                    (window.cover, window.picture, window.uncovered) = (Cover::Gone, None, true);
                }
            }
            if !window.shown {
                // Not before it can be drawn, so it never shows with nothing drawn on it: as its pipelines compile
                // (`compiled`).
                if window.parts[0].native.has_surface() && window.parts[0].renderer.is_none() && !self.background {
                    continue;
                }
                window.shown = true;
                // On a phone, an app's window grows out of the icon it was opened from, from the frame it first shows
                // in, as the reverse of its move home (`rail::Pull::opening`).
                #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
                if phone && !window.root && !window.panel && window.icon.is_some() && window.pull.is_none() {
                    let mut opening = rail::Pull::opening();
                    let reduced = window.screen.as_ref().is_ok_and(|screen| screen.reduced_motion);
                    let room = window.room();
                    let (moved, _) = opening.step(Size { width: room.width.0, height: room.height.0 }, window.icon, reduced);
                    self.display.set_moved(&window.parts[0].native, moved);
                    window.pull = Some(opening);
                }
                // A panel shows as it's summoned.
                if !window.panel {
                    self.display.show(&window.parts[0].native);
                }
            }
            // Its page loading again in its place (`reload`) is drawn as a new window shows, the frame before staying
            // on screen until then.
            if window.reloading && starting {
                continue;
            }
            // A panel put away is painted as its page changes, never shown, so its text is shaped and its glyphs and
            // pictures are in the atlas, and drawn once, so its surfaces have had a frame, by the time it's summoned,
            // and the frame it shows with is as quick as any after (26 to 52 ms on Linux's first summon after the app started, where
            // later ones take 7). What moves asks for no more frames while it's away. A window with no surface yet
            // (Android's view, until it's laid out) is drawn once it has one.
            // Its first frame drawn so is when it's opened (`Drawn`), as it's ready to show: when it's first summoned is
            // its person's (`Shown`).
            if window.panel && !window.visible {
                if std::mem::take(&mut window.dirty)
                    && let Ok(screen) = &mut window.screen
                    && window.parts[0].renderer.is_some()
                {
                    window.unshown |= screen.paint(window.room, window.parts[0].native.scale(), &mut self.text, &self.atlas, &mut window.scene);
                    // Drawn once, so its surfaces have had a frame; after that it's painted alone, and the image it
                    // took as it went (`put_away`) is kept for the frame it's summoned with.
                    if !window.drawn {
                        let glass = screen.glass().to_vec();
                        window.arrange(&self.display, &self.context, &self.atlas, &glass);
                        (window.drawn, window.live) = (true, true);
                        (window.on_event)(Event::Drawn);
                        (window.on_event)(Event::Live);
                    }
                }
                continue;
            }
            let (width, height) = (device.width.0 as u32, device.height.0 as u32);
            if ticking && window.presented && window.summoned.is_none() {
                continue;
            }
            if !window.dirty || window.parts[0].renderer.is_none() {
                if window.timing.idle()
                    && let Some(frames) = window.timing.end(self.display.refresh())
                {
                    (window.on_event)(Event::Frames(frames));
                }
                continue;
            }
            window.dirty = false;
            let began = web_time::Instant::now();
            let (room, scale) = (window.room(), window.parts[0].native.scale());
            let mut changed = true;
            {
                match &mut window.screen {
                    Ok(screen) => {
                        // In its first second, its page is laid out in the room its window has, never in another it
                        // was told of first, which it's drawn at for a frame as it's told the real one.
                        #[cfg(feature = "fake")]
                        if since(window.opened) < std::time::Duration::from_secs(1)
                            && let Some(told) = screen.told_room()
                            && (told.width != room.width.0 || told.height != room.height.0)
                        {
                            self.display.wrong(format!("{} was laid out {}×{} in its first second, in a window {}×{}", window.title, told.width, told.height, room.width.0, room.height.0));
                        }
                        // Under what covers it, drawn afresh, as what covers it is drawn into the same scene.
                        if !matches!(window.cover, Cover::Gone) || std::mem::take(&mut window.uncovered) {
                            screen.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut window.scene);
                        } else {
                            changed = screen.paint(room, scale, &mut self.text, &self.atlas, &mut window.scene);
                        }
                        window.moving = screen.take_again();
                        let calls = screen.text_input(&mut window.text_input);
                        if !calls.is_empty() {
                            self.display.text_input(&window.parts[0].native, &calls);
                        }
                        // The system's menu for what a finger selected, and its magnifier, where they are now.
                        let menu = screen.text_menu_shown();
                        if menu != window.text_menu {
                            window.text_menu = menu;
                            self.display.text_menu(&window.parts[0].native, menu);
                        }
                        let magnifier = screen.magnifier();
                        if magnifier != window.magnifier {
                            window.magnifier = magnifier;
                            self.display.magnifier(&window.parts[0].native, magnifier);
                        }
                        self.display.set_title_row(&window.parts[0].native, screen.title_row());
                    }
                    Err(why) => crate::view::paint_message(why, appearance(), room, scale, &mut self.text, &self.atlas, &mut window.scene),
                }
                // What covers it as it opens, made as it's first drawn: its snapshot, or its canvas, which a window whose
                // page isn't there yet (`opening`) has drawn bare.
                if let Cover::Over(cover @ None) = &mut window.cover {
                    let mut made = crate::render::scene::Scene::default();
                    match (&window.picture, &window.screen) {
                        (Some(picture), _) => {
                            if !std::mem::replace(&mut window.pictured, true)
                                && let Some(renderer) = &mut window.parts[0].renderer
                            {
                                renderer.picture(snapshot::RASTER, width, height, picture);
                            }
                            made.picture(snapshot::RASTER, whole(device), 1.0);
                        }
                        (None, Ok(screen)) => screen.paint_canvas(room, scale, 1.0, &mut made),
                        (None, Err(_)) => made.splice(&window.scene, 1.0, [0.0, 0.0], 1.0, whole(device)),
                    }
                    made.finish();
                    *cover = Some(made);
                }
            }
            // What it shows is its page with what covers it over it, as opaque as it's faded to.
            if let Some(cover) = window.cover.scene() {
                window.scene.splice(cover, 1.0, [0.0, 0.0], window.cover.opacity(), whole(device));
                window.scene.finish();
            }
            // A character no font had draws as a font's missing glyph: a gap where a symbol or an emoji goes.
            for character in self.text.undrawn() {
                let what = format!("{} drew {character:?} (U+{:04X}) as a missing glyph: no font has it", window.title, u32::from(character));
                #[cfg(feature = "fake")]
                self.display.wrong(what);
                #[cfg(not(feature = "fake"))]
                log::warn!("{what}");
            }
            let painted = web_time::Instant::now();
            let glass = window.screen.as_ref().map(|screen| screen.glass().to_vec()).unwrap_or_default();
            // Placed for the room it's drawn in before that frame shows, so it shows where it stays.
            if window.drawn && std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
            }
            // Nothing it shows changed (its page asked for a frame, or something moved that ended where it was): the
            // frame on the screen is it, so none is drawn or presented, as a compositor draws none without damage.
            // On the Surface, each was half a ms of this thread and 0.7 to 2 of its GPU, for every part of a panel.
            if !changed && !window.unshown && window.drawn && window.summoned.is_none() && window.missed == 0 {
                continue;
            }
            if !window.arrange(&self.display, &self.context, &self.atlas, &glass) {
                window.missed(&self.context, &self.atlas);
                continue;
            }
            window.unshown = false;
            window.missed = 0;
            window.presented = true;
            window.frames += 1;
            // Its opening out of its icon starts from the frame it first shows something in.
            if window.frames == 1
                && let Some(opening) = window.pull.as_mut().filter(|pull| pull.opens())
            {
                opening.shown();
            }
            if self.check.is_some() {
                self.record.present();
            }
            let moving = window.moving || window.screen.as_ref().is_ok_and(Screen::framing);
            if window.screen.as_ref().is_ok_and(Screen::held) {
                window.timing.input("drag");
            }
            let refresh = self.display.refresh();
            let drawn = window.parts.iter().filter_map(|part| part.renderer.as_ref());
            let (acquire, submit, present, gpu) = drawn.fold((std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO, None), |(acquire, submit, present, gpu), renderer| {
                let (all, blur) = gpu.unwrap_or_default();
                let (submitted, presented) = renderer.submitted();
                (acquire + renderer.acquired(), submit + submitted, present + presented, renderer.gpu().map(|(part, blurs)| (all + part, blur + blurs)).or(gpu))
            });
            let frame = frame_timing::Frame { at: now(), paint: painted - began, draw: painted.elapsed(), acquire, submit, present, gpu: gpu.map(|(all, _)| all), blur: gpu.map(|(_, blur)| blur), applied: Default::default(), turned: Default::default() };
            let (interval, frames) = window.timing.drawn(frame, moving, refresh);
            if let Some(interval) = interval {
                self.deadlines.frame(frame_timing::Timing::long(interval, refresh.unwrap_or(FRAME)));
            }
            if let Some(frames) = frames {
                (window.on_event)(Event::Frames(frames));
            }
            // Summoned before it had drawn, it's on the screen now it has, arranged again so its parts show with it.
            if std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
                window.arrange(&self.display, &self.context, &self.atlas, &glass);
            }
            if !window.drawn {
                window.drawn = true;
                window.reloading = false;
                (window.on_event)(Event::Drawn);
            }
            if !window.pending && !matches!(window.cover, Cover::Over(_)) && !std::mem::replace(&mut window.live, true) {
                (window.on_event)(Event::Live);
            }
            if let Some(asked) = window.summoned.take() {
                let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
                // Where its time went: to the turn it's drawn on, on that turn before it (input, timers, other windows
                // drawn first), painting it and drawing it.
                let (before, on) = (turned.saturating_duration_since(asked), began.saturating_duration_since(turned));
                eprintln!("the launcher's first frame as it was summoned: {:.1} ms before its turn, {:.1} ms on it before it was painted, {:.1} ms to paint and {:.1} ms to draw", ms(before), ms(on), ms(painted - began), ms(painted.elapsed()));
                (window.on_event)(Event::Shown(asked.elapsed()));
            }
        }
        // What waited for a page that's caught up now is done.
        let waiting: Vec<WindowId> = self.windows.iter().filter(|(_, window)| !window.waiting.is_empty()).map(|(id, _)| *id).collect();
        for id in waiting {
            self.catch_up(id);
        }
        self.pressed();
        // The display's frames are wanted while something moves, and while a change waits for its next tick; a link
        // just started, or stopped, hasn't ticked.
        let waiting = ticking && self.windows.values().any(|window| window.on_screen() && window.dirty);
        let wanted = self.framing() || waiting;
        self.ticked &= wanted && self.wanted;
        self.wanted = wanted;
        self.display.request_frame(wanted);
        self.display.flush();
        if let Some(recording) = self.recording.take_if(|recording| recording.until <= now()) {
            (recording.reply)(crate::inspect::answer(&recording.id, Ok(serde_json::json!({"events": recording.events}))));
        }
        if let Some(check) = self.check.take_if(|check| check.over()) {
            let said = check.said(&self.record, self.display.paced());
            (check.reply)(crate::inspect::answer(&check.id, Ok(said)));
        }
    }

    /// How pressed the device is, the worse of what its system and its frames say, given to each window's screen as
    /// it changes, which draws as it says (`Screen::pressure`).
    fn pressed(&mut self) {
        if since(self.system.0) >= std::time::Duration::from_secs(1) {
            self.system = (now(), self.display.pressure());
        }
        let pressure = self.system.1.max(self.deadlines.pressure());
        if pressure == self.pressure {
            return;
        }
        log::info!("pressure is {} now, where it was {}", pressure.name(), self.pressure.name());
        self.pressure = pressure;
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen {
                screen.pressure = pressure;
                window.dirty = true;
            }
        }
    }

    /// Keeps window `id`'s frame as its snapshot (`WindowOptions::snapshot`), drawn afresh offscreen as it is now,
    /// where it shows what its page drew: not its snapshot, nor a page still starting.
    pub(crate) fn keep_snapshot(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let (Some(stem), Ok(screen)) = (&window.snapshot, &mut window.screen) else { return };
        if self.background || !window.drawn || !matches!(window.cover, Cover::Gone) || window.loading || screen.starting() {
            return;
        }
        let device = Size { width: DevicePixels((room.width.0 * scale).round() as i32), height: DevicePixels((room.height.0 * scale).round() as i32) };
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
        let mut scene = crate::render::scene::Scene::default();
        screen.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut scene);
        renderer.draw(&scene, [0.0, 0.0]);
        if let Some((width, height, rgba)) = renderer.read() {
            snapshot::keep(stem, width, height, rgba);
        }
        renderer.destroy();
    }

    /// Lets go of a window's parts: their renderers, then their native windows, the children first.
    fn let_go(&self, parts: Vec<Part>) {
        for mut part in parts.into_iter().rev() {
            if let Some(mut renderer) = part.renderer.take() {
                renderer.destroy();
            }
            self.display.close(&part.native);
        }
    }

    /// What the display said happened to part `part` of window `id`, where it is in its page.
    fn input(&mut self, id: WindowId, part: usize, input: Input) {
        if let Input::Touch(finger, at, phase) = input {
            self.touch(id, finger, at, phase, None, crate::view::touch::Fingers::PHONE);
            return;
        }
        if self.check.is_some() {
            self.record.input(input.kind(), None);
        }
        if let Some((recording, event)) = self.recording.as_mut().zip(input.traced()) {
            recording.input(id, event, None);
        }
        if let Some(discrete) = input.waits()
            && self.waits(id, discrete)
        {
            if let Some(window) = self.windows.get_mut(&id) {
                window.waiting.push_back(Waiting::Input(part, input));
            }
            return;
        }
        self.act(id, part, input);
    }

    /// What the display said happened to part `part` of window `id`, its page caught up where it waits (`input`).
    fn act(&mut self, id: WindowId, part: usize, input: Input) {
        // Ctrl-W closes the window, as a Linux or Windows app's does, and Ctrl-Q on Linux quits the app (a Mac's are
        // ⌘W and ⌘Q, its menu bar's).
        let (closes, quits) = match &input {
            Input::Close => (true, false),
            Input::Key(key) if !cfg!(target_os = "macos") && key.ctrl && !key.alt && !key.shift && !key.meta => (key.key == "w", cfg!(target_os = "linux") && key.key == "q"),
            _ => (false, false),
        };
        if quits {
            if let Some(window) = self.windows.get_mut(&id) {
                (window.on_event)(Event::Quit);
            }
            return;
        }
        if closes {
            // A panel's put away, as Raycast's is by ⌘W or Alt-F4: it's summoned again, never closed.
            if !self.windows.get(&id).is_some_and(|window| window.panel) {
                return self.close(id);
            }
            self.put_away(id, true);
            if let Some(window) = self.windows.get_mut(&id) {
                (window.on_event)(Event::PutAway);
            }
            return;
        }
        let background = self.background;
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(at) = window.parts.get(part).map(|part| part.bounds.origin) else { return };
        let input = input.moved_by(at);
        window.dirty = true;
        match input {
            Input::Resized(size) => return window.parts[part].native.resized(size),
            // Its surface came, or a new one did: drawn on from now, a renderer for one that's gone let go first.
            Input::Surface => {
                let part = &mut window.parts[part];
                if let Some(mut renderer) = part.renderer.take() {
                    renderer.destroy();
                }
                if !background && part.native.has_surface() {
                    match drawn_on(&self.context, self.atlas.clone(), &part.native) {
                        Ok(renderer) => part.renderer = Some(renderer),
                        Err(error) => log::error!("native window {id} can't draw: {error:#}"),
                    }
                }
                return;
            }
            Input::SurfaceLost => {
                if let Some(mut renderer) = window.parts[part].renderer.take() {
                    renderer.destroy();
                }
                return;
            }
            _ => {}
        }
        let Ok(screen) = &mut window.screen else { return };
        match input {
            Input::Resized(_) | Input::Surface | Input::SurfaceLost | Input::Touch(..) => {}
            Input::Expose => {}
            // The system went light or dark: so do its own parts of the window, where its Colour Mode says nothing.
            Input::FullScreen(full) => screen.full_screen = full,
            Input::Dropped(at, files) => screen.dropped(at, &files),
            Input::Appearance(appearance) => {
                screen.appearance = appearance;
                window.chrome = None;
            }
            Input::Moved(at) => {
                screen.pointer_moved(at);
                let cursor = screen.cursor(at);
                self.display.set_cursor(&window.parts[part].native, cursor);
            }
            Input::Predicted(at) => screen.predicted(at),
            Input::Left => screen.pointer_left(),
            Input::Down(at, button, shift) => {
                screen.pointer_down(at, button, shift);
                // The system's own menu for text, where it was clicked in this part.
                if let Some(mut menu) = screen.take_text_menu() {
                    let origin = window.parts[part].bounds.origin;
                    menu.rect.origin = crate::geometry::point(menu.rect.origin.x - origin.x, menu.rect.origin.y - origin.y);
                    self.display.text_menu(&window.parts[part].native, Some(menu));
                }
            }
            Input::Up(at, button) => {
                screen.pointer_up(at, button);
                // Its own window buttons do what they say (an unframed Linux window's).
                #[cfg(target_os = "linux")]
                match screen.take_window_button() {
                    Some(crate::WindowButton::Close) => return self.close(id),
                    Some(crate::WindowButton::Minimize) => self.display.minimize(&window.parts[0].native),
                    Some(crate::WindowButton::Maximize) => self.display.toggle_maximized(window.parts[0].native.x_window()),
                    None => {}
                }
            }
            Input::Wheel(at, delta, phase, notches) => {
                window.timing.input("scroll");
                screen.scroll(at, delta, phase, notches);
            }
            Input::Key(key) => drop(screen.key(&key)),
            Input::Ime(ime) => screen.ime(ime),
            Input::TextInputClient(said) => screen.text_input_client(&mut window.text_input, said),
            // A panel goes as something else takes the keyboard, as Spotlight does.
            Input::Focus(false) if window.panel && window.visible => {
                window.left = Some(now());
                screen.activate(false);
                self.put_away(id, false);
                if let Some(window) = self.windows.get_mut(&id) {
                    (window.on_event)(Event::PutAway);
                }
                // Its frame as it was put away is what it shows as it opens again (`WindowOptions::snapshot`).
                return self.keep_snapshot(id);
            }
            Input::Focus(active) => {
                screen.activate(active);
                if !active {
                    (window.on_event)(Event::Unfocused);
                    return self.keep_snapshot(id);
                }
            }
            // Dragged somewhere on a screen, a panel is summoned there on that screen from now on.
            Input::Placed(on, at) => {
                if let Some(file) = &window.places {
                    let mut kept = places(Some(file));
                    kept.insert(on, at);
                    let kept: HashMap<&String, [f32; 2]> = kept.iter().map(|(screen, at)| (screen, [at.x, at.y])).collect();
                    if let Err(error) = serde_json::to_vec(&kept).map_err(anyhow::Error::from).and_then(|bytes| Ok(std::fs::write(file, bytes)?)) {
                        log::warn!("where the panel was dragged to isn't kept: {error}");
                    }
                }
            }
            Input::Picked(node, value) => screen.picked(node, &value),
            Input::Close => {}
        }
        screen.unused();
        // What it changed reaches the text input plugin at once, before its next key, not with the frame.
        let calls = screen.text_input(&mut window.text_input);
        if !calls.is_empty() {
            self.display.text_input(&window.parts[0].native, &calls);
        }
    }
}

/// Fits `screen` to window `id`, its `native` window: the `room` its page is laid out in, what it's told of its system
/// and device and of its window's title row (`WindowOptions::title_row`), and the windows' thread woken as its page
/// posts.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(unused_variables))]
fn fit(screen: &mut Screen, native: &NativeWindow, id: WindowId, room: Size<Pixels>, root: bool, title_row: bool, pressure: Pressure) {
    screen.room = Size { width: room.width.0, height: room.height.0 };
    // A phone's app window has the rail's home line; its Home Screen doesn't.
    screen.rail = cfg!(any(target_os = "android", target_os = "ios")) && !root;
    screen.appearance = appearance();
    screen.reduced_motion = reduced_motion();
    screen.pressure = pressure;
    if let Some(clipboard) = clipboard() {
        screen.clipboard = clipboard;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        screen.device = native.device(id);
        screen.text_size = Some(text_size());
    }
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let _ = native;
        screen.device = Box::new(Computer);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if title_row {
        #[cfg(any(target_os = "linux", feature = "fake"))]
        {
            screen.window_buttons = button_layout();
        }
        #[cfg(all(target_os = "windows", not(feature = "fake")))]
        {
            screen.window_buttons = (Vec::new(), vec![crate::WindowButton::Minimize, crate::WindowButton::Maximize, crate::WindowButton::Close]);
        }
    }
    if let Some(native) = Native::started() {
        let native = std::sync::Mutex::new(native);
        screen.on_post(Arc::new(move || native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).run(move |windows| windows.posted(id))));
    }
}

impl Window {
    /// Whether it's on screen: shown, and for a panel, summoned.
    fn on_screen(&self) -> bool {
        self.shown && (!self.panel || self.visible)
    }

    /// How big its page is laid out: a panel's as its page asked, the rest as big as their window.
    fn room(&self) -> Size<Pixels> {
        if self.panel { self.room } else { self.parts[0].native.size() }
    }

    /// Draws its scene into its parts, a panel's first made to follow its page: one part for each box its page asked
    /// the system's material for (`glass`), or one for all of it until it has, each window's frame and what's drawn in
    /// it put on screen together (`Display::arrange`). A display with no windows of a panel's own keeps one, as big
    /// as its page.
    /// It had nothing to draw on this frame, as when its surface is lost (the computer slept, its display changed,
    /// its GPU was reset): it's drawn again on the next, and after a few such, on a surface made again from its native
    /// window's, as when the system gives it a new one (`Input::Surface`).
    fn missed(&mut self, context: &Context, atlas: &Arc<Atlas>) {
        self.dirty = true;
        self.moving = true;
        self.missed += 1;
        if self.missed < 3 {
            return;
        }
        self.missed = 0;
        for part in &mut self.parts {
            if let Some(mut renderer) = part.renderer.take() {
                renderer.destroy();
            }
            match drawn_on(context, atlas.clone(), &part.native) {
                Ok(renderer) => part.renderer = Some(renderer),
                Err(error) => log::error!("a native window can't draw: {error:#}"),
            }
        }
    }

    /// → whether each of its parts was drawn, which isn't so where one had nothing to draw on (`missed`).
    fn arrange(&mut self, display: &Display, context: &Context, atlas: &Arc<Atlas>, glass: &[Glass]) -> bool {
        let whole = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: Size { width: self.room().width.0, height: self.room().height.0 } };
        let corner = self.parts[0].radius;
        let mut boxes: Vec<(crate::geometry::Bounds<f32>, f32)> = match self.panel {
            true if !glass.is_empty() => glass.iter().map(|glass| (whole_points(glass.bounds), glass.radius)).collect(),
            _ => vec![(whole, if self.panel { corner } else { 0.0 })],
        };
        while self.panel && self.parts.len() < boxes.len() {
            let made = display.open_part(&self.parts[0].native, self.parts.len()).and_then(|native| {
                let renderer = drawn_on(context, atlas.clone(), &native)?;
                Ok(Part { native, renderer: Some(renderer), bounds: whole, radius: 0.0 })
            });
            match made {
                Ok(part) => self.parts.push(part),
                Err(error) => {
                    log::debug!("a panel's parts are one window here: {error:#}");
                    boxes = vec![(whole, corner)];
                }
            }
        }
        while self.parts.len() > boxes.len() {
            let mut part = self.parts.pop().expect("there's one");
            if let Some(mut renderer) = part.renderer.take() {
                renderer.destroy();
            }
            display.close(&part.native);
        }
        for (part, (bounds, radius)) in self.parts.iter_mut().zip(&boxes) {
            part.bounds = *bounds;
            part.radius = *radius;
        }
        let scale = self.parts[0].native.scale();
        let placed: Vec<(Handle, crate::geometry::Bounds<f32>, f32)> = self.parts.iter().map(|part| (part.native.handle(), part.bounds, part.radius)).collect();
        let (parts, scene, panel) = (&mut self.parts, &self.scene, self.panel);
        let mut drawn = true;
        let mut draw = || {
            for part in parts.iter_mut() {
                let Some(renderer) = part.renderer.as_mut() else { continue };
                // A panel's part is as big as it was just made (`Display::arrange`), which its window says later.
                let size = |length: f32| DevicePixels((length * scale).round() as i32);
                renderer.update_drawable_size(if panel { Size { width: size(part.bounds.size.width), height: size(part.bounds.size.height) } } else { part.native.device_size() });
                drawn &= renderer.draw(scene, [(part.bounds.origin.x * scale).round(), (part.bounds.origin.y * scale).round()]);
            }
        };
        if self.panel {
            display.arrange(&placed, &mut draw);
        } else {
            draw();
        }
        drawn
    }
}

/// What a harness drives the fake platform's windows by (`fake`), as GPUI's `TestAppContext` drives its test windows:
/// it owns them, on its own thread, and says when their pages have run, when the display shows a frame and what input
/// comes, and the loop (`turn`) does the rest as it does on every host.
#[cfg(feature = "fake")]
impl Windows {
    /// The windows of a display of `scale` device pixels a point, refreshing `hz` times a second, whose time is
    /// `clock`'s, which their pages' JS reads too: paused, it moves only as the driver moves it.
    pub fn fake(scale: f32, hz: f64, clock: crate::js::Clock) -> anyhow::Result<Windows> {
        fake::keep_time(clock);
        Self::made(Display::new(scale, hz), Context::new())
    }

    /// Waits for the pipelines its windows draw with, compiled as it started (`Context::prepare`), as a harness that
    /// turns its loop itself on a paused clock does, so its first frame is drawn on its first turn, as every run's is.
    #[cfg(feature = "fake")]
    pub fn prepared(&self) {
        self.context.wait_compiled();
    }

    /// Whether the loop wants the display's next frame: something on a window moves.
    pub fn wanted(&self) -> bool {
        self.display.wanted()
    }

    /// The display's refreshes come late as `seed` decides (`fake::Display::seed`), and `random` draws from it too.
    pub fn seed(&self, seed: u64) {
        self.display.seed(seed);
    }

    /// A number from 0 to below `below`, as the seed decides: always 0 with none.
    pub fn random(&self, below: u64) -> u64 {
        self.display.random(below)
    }

    /// Each frame takes `ms` to draw from now on, the windows' thread busy meanwhile and the display's refreshes going
    /// by unseen, as on a device others load (the Home Screen on a loaded Mac); 0 for none.
    pub fn set_frame_ms(&self, ms: f64) {
        self.display.set_frame_ms(ms);
    }

    /// How pressed the windows are, by Compute Pressure's names, as their frames and the system say.
    pub fn pressure(&self) -> &'static str {
        self.pressure.name()
    }

    /// The display refreshes `hz` times a second from now on.
    pub fn set_refresh(&self, hz: f64) {
        self.display.set_refresh(hz);
    }

    /// Each window's page has run all it's been sent, and what it posted is applied, as `posted` applies it once its
    /// JS wakes the loop. → whether anything came.
    pub fn wait(&mut self) -> bool {
        let mut came = false;
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen
                && screen.idle()
            {
                window.dirty = true;
                came = true;
            }
        }
        came
    }

    /// The display shows a frame now, which the loop hears on its turn. A panel just put on the screen with nothing
    /// drawn on it (a page that hadn't drawn yet) is wrong (`wrong`), as a host's compositor shows its window then,
    /// its backdrop an empty box, until the next frame.
    pub fn tick(&mut self) {
        self.display.tick(now());
        for id in self.display.summoned() {
            let Some(window) = self.windows.get_mut(&id) else { continue };
            let drawn = window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()?.read()).any(|(_, _, rgba)| rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
            if !drawn {
                self.display.wrong(format!("{} was put on the screen with nothing drawn on it", window.title));
            }
        }
    }

    /// What was wrong with what the display showed, since it was last asked.
    pub fn wrong(&self) -> Vec<String> {
        self.display.wrongs()
    }

    /// What window `id`'s input with the keyboard asked the platform's keyboard for (Flutter's
    /// `TextInputConfiguration`), if one has it.
    pub fn keyboard(&self, id: WindowId) -> Option<serde_json::Value> {
        self.display.keyboard(id)
    }

    /// What window `id`'s system was last asked to show of a finger's selection: its menu for text, what of it can be
    /// done, what it's for and where the system put it; its magnifier; and the handles the window draws, each in its
    /// points, with its safe area.
    pub fn text_selection(&self, id: WindowId) -> serde_json::Value {
        use serde_json::json;
        let rect = |rect: crate::geometry::Bounds<f32>| json!({"x": rect.origin.x, "y": rect.origin.y, "width": rect.size.width, "height": rect.size.height});
        let (menu, magnifier) = self.display.text_selection(id);
        let screen = self.windows.get(&id).and_then(|window| window.screen.as_ref().ok());
        let menu = menu.map(|(menu, placed)| {
            let items: Vec<&str> = [("cut", menu.cut), ("copy", menu.copy), ("paste", menu.paste), ("select", menu.select), ("selectAll", menu.select_all)].into_iter().filter(|(_, shown)| *shown).map(|(item, _)| item).collect();
            json!({"items": items, "for": rect(menu.rect), "rect": rect(placed)})
        });
        let handles: Vec<serde_json::Value> = screen.map(|screen| screen.handles().iter().map(|handle| json!({"end": format!("{:?}", handle.end).to_lowercase(), "rect": rect(handle.bounds())})).collect()).unwrap_or_default();
        let insets = screen.map(|screen| screen.insets).unwrap_or_default();
        json!({
            "menu": menu,
            "magnifier": magnifier.map(|magnifier| json!({"x": magnifier.at.x, "y": magnifier.at.y})),
            "handles": handles,
            "safeArea": {"top": insets.top, "right": insets.right, "bottom": insets.bottom, "left": insets.left},
        })
    }

    /// Window `id` is drawn again on the next turn, though nothing on it changed.
    pub fn redraw(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.dirty = true;
        }
    }

    /// Whether window `id`'s page is still under what covers it as it opens (`Cover`).
    pub fn revealing(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| !matches!(window.cover, Cover::Gone))
    }

    /// It's a phone's screen, `screen` points big: every window but a panel is the whole of it, whatever it was asked
    /// to be, as an iPhone's and an Android phone's are.
    pub fn phone(&self, screen: Size<Pixels>) {
        self.display.phone(screen);
    }

    /// The windows on the screen, the root window first and the rest as they opened, each with how it's drawn over
    /// what's under it (`rail::Moved`).
    fn stacked(&self) -> Vec<WindowId> {
        let mut ids: Vec<WindowId> = self.windows.iter().filter(|(_, window)| window.parts[0].native.composed().1).map(|(id, _)| *id).collect();
        ids.sort_by_key(|id| (!self.windows[id].root, *id));
        ids
    }

    /// Each window on the screen, bottom first: its id, its title, how it's drawn over what's under it, whether it
    /// shows its snapshot, and the room its page was last told and the room it has, in points.
    pub fn composed(&self) -> serde_json::Value {
        let listed: Vec<serde_json::Value> = self
            .stacked()
            .into_iter()
            .map(|id| {
                let window = &self.windows[&id];
                let (moved, _) = window.parts[0].native.composed();
                let told = window.screen.as_ref().ok().and_then(Screen::told_room);
                let room = window.room();
                serde_json::json!({
                    "window": id,
                    "title": window.title,
                    "moved": {"scale": moved.scale, "x": moved.x, "y": moved.y, "opacity": moved.opacity, "dim": moved.dim},
                    "snapshot": window.picture.is_some(),
                    "covered": window.cover.opacity(),
                    "starting": window.screen.as_ref().is_ok_and(Screen::starting),
                    "told": told.map(|told| serde_json::json!({"width": told.width, "height": told.height})),
                    "room": {"width": room.width.0, "height": room.height.0},
                })
            })
            .collect();
        serde_json::json!(listed)
    }

    /// The screen as its compositor shows it: each window on it what it last presented, over those under it, grown and
    /// moved about its middle, as opaque and with corners as round as it's moved to, what's under it dimmed by its
    /// `dim` (`rail::Moved`), as an iPhone's Core Animation draws its card: its width, height and pixels.
    pub fn screen(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let stacked = self.stacked();
        let first = *stacked.first()?;
        let (width, height, mut rgba) = self.presented(first)?;
        for id in stacked.into_iter().skip(1) {
            let scale = self.windows[&id].parts[0].native.scale();
            let (moved, _) = self.windows[&id].parts[0].native.composed();
            let Some((w, h, drawn)) = self.presented(id) else { continue };
            for pixel in rgba.chunks_exact_mut(4) {
                for channel in &mut pixel[..3] {
                    *channel = (f32::from(*channel) * (1.0 - moved.dim)).round() as u8;
                }
            }
            // Where each pixel of the screen comes from on the window: back through its transform, about its middle.
            let (middle_x, middle_y) = (w as f32 / 2.0, h as f32 / 2.0);
            let corner = moved.corner * scale;
            for (y, x) in (0..height).flat_map(|y| (0..width).map(move |x| (y, x))) {
                let from_x = middle_x + (x as f32 + 0.5 - middle_x - moved.x * scale) / moved.scale;
                let from_y = middle_y + (y as f32 + 0.5 - middle_y - moved.y * scale) / moved.scale;
                if from_x < 0.0 || from_y < 0.0 || from_x >= w as f32 || from_y >= h as f32 {
                    continue;
                }
                // Outside its round corners.
                let (dx, dy) = ((corner - from_x).max(from_x - (w as f32 - corner)), (corner - from_y).max(from_y - (h as f32 - corner)));
                if dx > 0.0 && dy > 0.0 && dx.hypot(dy) > corner {
                    continue;
                }
                let (from, to) = (((from_y as u32 * w + from_x as u32) * 4) as usize, ((y * width + x) * 4) as usize);
                let alpha = f32::from(drawn[from + 3]) / 255.0 * moved.opacity;
                for channel in 0..3 {
                    rgba[to + channel] = (f32::from(drawn[from + channel]) * alpha + f32::from(rgba[to + channel]) * (1.0 - alpha)).round() as u8;
                }
            }
        }
        Some((width, height, rgba))
    }

    /// What window `id` last presented, as its compositor shows it: its width, height and pixels (`composed`).
    pub fn presented(&mut self, id: WindowId) -> Option<(u32, u32, Vec<u8>)> {
        let window = self.windows.get_mut(&id)?;
        let drawn = window.parts.iter_mut().map(|part| part.renderer.as_mut().and_then(Renderer::read)).collect();
        composed(window, drawn)
    }

    /// Window `id` drawn afresh as it is now, as its compositor would show it (`presented`): its page painted afresh,
    /// and each of its parts drawn where it is by a renderer of its own, so nothing a frame before kept (a layer, a
    /// blur, a surface's last image) is in it. A frame drawn on what was kept must be the same.
    pub fn afresh(&mut self, id: WindowId) -> Option<(u32, u32, Vec<u8>)> {
        let window = self.windows.get_mut(&id)?;
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let mut scene = crate::render::scene::Scene::default();
        window.screen.as_mut().ok()?.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut scene);
        let size = |length: f32| DevicePixels((length * scale).round() as i32);
        let drawn = window
            .parts
            .iter()
            .map(|part| {
                let device = if window.panel { Size { width: size(part.bounds.size.width), height: size(part.bounds.size.height) } } else { part.native.device_size() };
                let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
                renderer.draw(&scene, [(part.bounds.origin.x * scale).round(), (part.bounds.origin.y * scale).round()]);
                let read = renderer.read();
                renderer.destroy();
                read
            })
            .collect();
        composed(window, drawn)
    }

    /// The fake platform's time moves on by `ms`, its pages' clock with it: each of their timers due meanwhile runs at
    /// its own time, and while the loop wants frames the display ticks each refresh, the loop taking a turn for each,
    /// as a host's do. What each page sent is run before each turn (`wait`), and what a turn sent one (input that
    /// waited for it) is run and drawn at once, on a turn of its own, as its post wakes a host's loop (`posted`).
    pub fn run(&mut self, ms: f64) {
        let refresh = self.display.refresh().unwrap_or(FRAME).as_secs_f64() * 1000.0;
        let started = fake::time();
        let mut late = 0;
        loop {
            self.wait();
            let drawn: u64 = self.windows.values().map(|window| window.frames).sum();
            self.turn();
            if self.wait() {
                self.turn();
            }
            // A frame drawn on a device others load keeps its thread busy, the display's refreshes going by unseen.
            let busy = self.display.frame_ms();
            if busy > 0.0 && self.windows.values().map(|window| window.frames).sum::<u64>() > drawn {
                fake::advance(busy);
                for window in self.windows.values_mut() {
                    if let Ok(screen) = &mut window.screen {
                        window.dirty |= screen.advance(0.0);
                    }
                }
            }
            // What the turn set off decoding has decoded, so when it's there is known before time moves on past it.
            self.wait();
            let now = fake::time();
            let left = started + ms - now;
            // The display's next refresh, at least a microsecond on, so a clock a hair short of one isn't moved by nothing.
            // The display ticks while the loop wants frames, or a window has what came meanwhile to draw (a picture
            // that's there at its time).
            let tick = (self.display.wanted() || self.windows.values().any(|window| window.dirty && window.on_screen())).then(|| refresh - now.rem_euclid(refresh)).map(|next| if next < 0.001 { next + refresh } else { next });
            let timer = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.timer_due()).reduce(f64::min).map(|due| due.max(0.0));
            // A refresh a hair past the end, as a driver's own refresh, `1000 / hz` ms, is a hair short of the
            // display's (a `Duration`, whole nanoseconds), is the one it ends on, where it was never shown.
            let next = tick.into_iter().chain(timer).reduce(f64::min).filter(|next| *next <= left + 0.001);
            // The platform's time moves on once, its pages' clock with it, then each page runs what's due by then.
            let by = next.unwrap_or(left.max(0.0));
            fake::advance(by);
            for window in self.windows.values_mut() {
                if let Ok(screen) = &mut window.screen {
                    window.dirty |= screen.advance(0.0);
                }
            }
            if next.is_none() {
                break;
            }
            if next == tick {
                // A late refresh shows nothing new: what's waiting is shown on the next.
                match self.display.late(late) {
                    true => late += 1,
                    false => {
                        late = 0;
                        self.tick();
                    }
                }
            }
        }
        self.wait();
        self.turn();
    }

    /// Where `x`, `y` of window `id`'s page is to its system: in the uppermost of its native windows there (each of a
    /// panel's surfaces of glass is one), at that point of it, as each says the pointer's in its own.
    fn part_at(&self, id: WindowId, x: f32, y: f32) -> (usize, crate::geometry::Point<f32>) {
        let at = crate::geometry::point(x, y);
        let parts = self.windows.get(&id).map(|window| &window.parts[..]).unwrap_or_default();
        let part = parts.iter().rposition(|part| part.bounds.contains(&at)).unwrap_or(0);
        let origin = parts.get(part).map_or(crate::geometry::point(0.0, 0.0), |part| part.bounds.origin);
        (part, crate::geometry::point(x - origin.x, y - origin.y))
    }

    /// Finger `finger` at `x`, `y` on window `id`, as a phone's screen says it.
    pub fn finger(&mut self, id: WindowId, finger: u64, x: f32, y: f32, phase: crate::view::touch::Phase) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Touch(finger, at, phase));
    }

    /// The pointer moved to `x`, `y` on window `id`, as a mouse does.
    pub fn pointer_moved(&mut self, id: WindowId, x: f32, y: f32) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Moved(at));
    }

    /// A button went down at `x`, `y` on window `id`: 0 the main one, 1 the middle, 2 the secondary, as the DOM's.
    pub fn pointer_down(&mut self, id: WindowId, x: f32, y: f32, button: u8, shift: bool) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Down(at, button, shift));
    }

    pub fn pointer_up(&mut self, id: WindowId, x: f32, y: f32, button: u8) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Up(at, button));
    }

    /// A wheel's or a trackpad's scroll at `at` on window `id`, by `by` in points, in a trackpad's phase where it says
    /// one (its fingers' by AppKit's names, `crate::view::Phase::FINGERS`, and whether it's the momentum after), or a
    /// wheel's `notches` where it turned by clicks.
    pub fn wheel(&mut self, id: WindowId, at: (f32, f32), by: (f32, f32), fingers: Option<&str>, momentum: bool, notches: Option<(f32, f32)>) {
        let fingers = fingers.and_then(|named| crate::view::Phase::FINGERS.into_iter().find(|one| *one == named));
        let phase = (fingers.is_some() || momentum).then_some(crate::view::Phase { fingers, momentum });
        let notches = notches.map(|(x, y)| crate::geometry::point(x, y));
        let (part, at) = self.part_at(id, at.0, at.1);
        self.display.push_to(id, part, Input::Wheel(at, crate::geometry::point(by.0, by.1), phase, notches));
    }

    /// A key on window `id`, as an inspector's `key` request says it.
    pub fn key(&mut self, id: WindowId, request: &crate::inspect::Request) -> anyhow::Result<()> {
        self.display.push(id, Input::Key(crate::inspect::Key::of(request)?));
        Ok(())
    }

    /// The app goes to the background for `ms` of its time and comes back, as an iPhone's does: its windows lose the
    /// keyboard, then iOS has them drawn in the other appearance and back, for the app switcher's snapshot of each
    /// (WWDC 2019's "Implementing Dark Mode on iOS"), while their pages run; then they have it back. → how many frames
    /// its windows presented while it was there, each lost on an iPhone, its GPU's work refused.
    pub fn away(&mut self, ms: f64) -> u64 {
        let frames = |windows: &Windows| windows.windows.values().map(|window| window.frames).sum::<u64>();
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let other = match appearance() {
            crate::Appearance::Light => crate::Appearance::Dark,
            crate::Appearance::Dark => crate::Appearance::Light,
        };
        for &id in &ids {
            self.display.push(id, Input::Focus(false));
        }
        self.run(0.0);
        self.background(true);
        let before = frames(self);
        for appearance in [other, appearance()] {
            for &id in &ids {
                self.display.push(id, Input::Appearance(appearance));
            }
            self.run(ms / 2.0);
        }
        let drawn = frames(self) - before;
        self.background(false);
        for &id in &ids {
            self.display.push(id, Input::Focus(true));
        }
        self.run(0.0);
        drawn
    }

    /// How many frames window `id` has presented, all told.
    pub fn frames(&self, id: WindowId) -> u64 {
        self.windows.get(&id).map_or(0, |window| window.frames)
    }

    /// Whether window `id` is on screen (`Window::on_screen`): not a panel put away.
    pub fn on_screen(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(Window::on_screen)
    }

    /// The system's appearance becomes `appearance`, as a person switching between Light and Dark makes it (or their
    /// schedule does): every window is told, as its host tells it.
    pub fn appearance(&mut self, appearance: crate::Appearance) {
        fake::appear(appearance);
        for id in self.windows.keys().copied().collect::<Vec<_>>() {
            self.display.push(id, Input::Appearance(appearance));
        }
    }

    /// Whether input to window `id` waits for its page to catch up (`waits`), not handled yet.
    pub fn waiting(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| !window.waiting.is_empty())
    }

    /// What window `id`'s frames cost since this was last asked, as the inspector's `profile` says it (`Screen::cost`),
    /// its GPU waited for, to say its time for the last.
    pub fn cost(&mut self, id: WindowId) -> Option<serde_json::Value> {
        let window = self.windows.get_mut(&id)?;
        let renderer = window.parts[0].renderer.as_mut()?;
        renderer.finish();
        Some(window.screen.as_mut().ok()?.cost(renderer))
    }

    /// What window `id` last drew, the GPU it drew it with and its atlas.
    pub fn drawn(&self, id: WindowId) -> Option<(&crate::render::scene::Scene, &Context, &Arc<Atlas>)> {
        Some((&self.windows.get(&id)?.scene, &self.context, &self.atlas))
    }
}

/// Now, as the windows' loop tells the time a frame or an input came: the system's, or on the fake platform its
/// driver's, which moves only as it's moved (`fake::now`). How long work took is the system's everywhere.
#[cfg(not(feature = "fake"))]
pub(crate) fn now() -> web_time::Instant {
    web_time::Instant::now()
}
#[cfg(feature = "fake")]
pub(crate) use fake::now;
/// The fake platform's time, in ms on its pages' clock.
#[cfg(feature = "fake")]
pub use fake::time as fake_time;

/// How long since `at`, by `now`.
pub(crate) fn since(at: web_time::Instant) -> std::time::Duration {
    now().saturating_duration_since(at)
}

/// What `window`'s parts drew (`drawn`, each part's, where it drew), as its compositor shows them: a window's one part
/// as it is, and a panel's put together where each is, over the system's glass, a flat grey of its Colour Mode here.
#[cfg(feature = "fake")]
fn composed(window: &Window, drawn: Vec<Option<(u32, u32, Vec<u8>)>>) -> Option<(u32, u32, Vec<u8>)> {
    if window.parts.len() == 1 && !window.panel {
        return drawn.into_iter().next().flatten();
    }
    let scale = window.parts[0].native.scale();
    let (width, height) = ((window.room.width.0 * scale).round() as u32, (window.room.height.0 * scale).round() as u32);
    let glass = glass(window);
    let mut rgba: Vec<u8> = (0..width * height).flat_map(|_| [glass[0], glass[1], glass[2], 255]).collect();
    for (part, drawn) in window.parts.iter().zip(drawn) {
        let (left, top) = ((part.bounds.origin.x * scale).round() as i64, (part.bounds.origin.y * scale).round() as i64);
        let Some((w, h, drawn)) = drawn else { continue };
        for (y, x) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))) {
            let (to_x, to_y) = (i64::from(x) + left, i64::from(y) + top);
            if to_x < 0 || to_y < 0 || to_x >= i64::from(width) || to_y >= i64::from(height) {
                continue;
            }
            let (from, to) = (((y * w + x) * 4) as usize, ((to_y as u32 * width + to_x as u32) * 4) as usize);
            let alpha = u32::from(drawn[from + 3]);
            for channel in 0..3 {
                rgba[to + channel] = ((u32::from(drawn[from + channel]) * alpha + u32::from(rgba[to + channel]) * (255 - alpha)) / 255) as u8;
            }
        }
    }
    Some((width, height, rgba))
}

/// The system's glass a panel is shown over, a flat grey of its Colour Mode here.
#[cfg(feature = "fake")]
fn glass(window: &Window) -> [u8; 3] {
    match window.screen.as_ref().ok().and_then(|screen| screen.theme).unwrap_or(appearance()) {
        crate::Appearance::Dark => [30, 30, 32],
        crate::Appearance::Light => [236, 236, 238],
    }
}

/// Whether what draws on `native` would draw at once: the pipelines its surface draws with are compiled
/// (`Context::compiled`).
fn drawable(context: &Context, native: &NativeWindow) -> bool {
    let (format, premultiplied) = crate::render::context::surface_pipelines(native.transparent());
    context.compiled(format, premultiplied)
}

/// What draws on `native`: its surface, which the system shows.
#[cfg(not(feature = "fake"))]
fn drawn_on(context: &Context, atlas: Arc<Atlas>, native: &NativeWindow) -> anyhow::Result<Renderer> {
    Renderer::new(context, atlas, native.surface()?, native.device_size(), native.transparent())
}

/// What draws on `native` on the fake platform: an image offscreen, as its compositor would show it, which its driver
/// reads back (`Windows::presented`).
#[cfg(feature = "fake")]
fn drawn_on(context: &Context, atlas: Arc<Atlas>, native: &NativeWindow) -> anyhow::Result<Renderer> {
    Ok(Renderer::offscreen(context, atlas, native.device_size()))
}

/// `bounds` with its edges on whole points, as a window's frame is.
fn whole_points(bounds: crate::geometry::Bounds<f32>) -> crate::geometry::Bounds<f32> {
    let (left, top) = (bounds.origin.x.round(), bounds.origin.y.round());
    let (right, bottom) = ((bounds.origin.x + bounds.size.width).round(), (bounds.origin.y + bounds.size.height).round());
    crate::geometry::Bounds { origin: crate::geometry::point(left, top), size: Size { width: right - left, height: bottom - top } }
}

impl crate::view::touch::Phase {
    /// Whether a finger in this phase is discrete input (`Windows::waits`): it lands, or lifts to press.
    fn discrete(self, screen: &Screen, finger: u64) -> bool {
        use crate::view::touch::Phase;
        match self {
            Phase::Down => true,
            Phase::Moved => false,
            Phase::Up | Phase::Cancelled => screen.lift_presses(finger),
        }
    }
}

impl Input {
    /// Whether it's the page's, and waits for it (`Windows::waits`): a press, a key, what's typed or picked, which do,
    /// or the pointer moving, which only follows what does. The wheel scrolls at once, as a browser's compositor
    /// scrolls, and what's the window's or the system's is the window's.
    fn waits(&self) -> Option<bool> {
        match self {
            Input::Down(..) | Input::Up(..) | Input::Key(_) | Input::Ime(_) | Input::TextInputClient(_) | Input::Picked(..) | Input::Dropped(..) => Some(true),
            Input::Moved(_) | Input::Predicted(_) | Input::Left => Some(false),
            _ => None,
        }
    }

    /// What it is, as the inspector's `host` check names it.
    fn kind(&self) -> &'static str {
        match self {
            Input::Moved(_) => "moved",
            Input::Predicted(_) => "predicted",
            Input::Down(..) => "down",
            Input::Up(..) => "up",
            Input::Wheel(..) => "wheel",
            Input::Key(_) => "key",
            Input::Left => "left",
            _ => "other",
        }
    }

    /// It as a trace's event (`{kind, x, y, …}`, as the headless harness's `trace` plays it), where it's one a person
    /// gives: the pointer, a button, the wheel or a key.
    fn traced(&self) -> Option<serde_json::Value> {
        use serde_json::json;
        Some(match self {
            Input::Moved(at) => json!({"kind": "move", "x": at.x, "y": at.y}),
            Input::Down(at, button, shift) => json!({"kind": "down", "x": at.x, "y": at.y, "button": button, "shift": shift}),
            Input::Up(at, button) => json!({"kind": "up", "x": at.x, "y": at.y, "button": button}),
            Input::Wheel(at, by, phase, notches) => {
                let mut wheel = json!({"kind": "wheel", "x": at.x, "y": at.y, "dx": by.x, "dy": by.y, "phase": phase.as_ref().and_then(|phase| phase.fingers), "momentum": phase.as_ref().is_some_and(|phase| phase.momentum)});
                if let Some(notches) = notches {
                    (wheel["notchesX"], wheel["notchesY"]) = (json!(notches.x), json!(notches.y));
                }
                wheel
            }
            Input::Key(key) => json!({"kind": "key", "key": key.key, "shift": key.shift, "alt": key.alt, "ctrl": key.ctrl, "meta": key.meta}),
            _ => return None,
        })
    }

    /// The same, at `by` further along and down: a part's point, in its page.
    fn moved_by(self, by: crate::geometry::Point<f32>) -> Input {
        let moved = |at: crate::geometry::Point<f32>| crate::geometry::point(at.x + by.x, at.y + by.y);
        match self {
            Input::Moved(at) => Input::Moved(moved(at)),
            Input::Predicted(at) => Input::Predicted(moved(at)),
            Input::Down(at, button, shift) => Input::Down(moved(at), button, shift),
            Input::Up(at, button) => Input::Up(moved(at), button),
            Input::Dropped(at, files) => Input::Dropped(moved(at), files),
            Input::Wheel(at, delta, phase, notches) => Input::Wheel(moved(at), delta, phase, notches),
            input => input,
        }
    }
}

/// What the display says happened to a window.
#[cfg_attr(any(target_os = "android", target_os = "ios", target_arch = "wasm32", feature = "fake"), allow(dead_code))]
pub(crate) enum Input {
    Resized(Size<crate::geometry::DevicePixels>),
    /// It has a surface to draw on, or a new one (Android's view's), or the app's back from the background.
    Surface,
    /// Its surface is going, as its view is hidden or gone, or the app to the background (`Windows::background`):
    /// nothing draws on it.
    SurfaceLost,
    Expose,
    /// The system went light or dark.
    Appearance(crate::Appearance),
    /// Files were dropped on it from the system, at a point in it.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), allow(dead_code))]
    Dropped(crate::geometry::Point<f32>, Vec<std::path::PathBuf>),
    /// It went full screen, or came out of it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FullScreen(bool),
    Close,
    Moved(crate::geometry::Point<f32>),
    /// A finger, as a phone's screen says it, in the window's points (`Windows::touch`).
    #[cfg_attr(not(feature = "fake"), allow(dead_code))]
    Touch(u64, crate::geometry::Point<f32>, crate::view::touch::Phase),
    /// Where the device predicts the pointer or finger down is going, a little after its last move
    /// (`Screen::predicted`).
    Predicted(crate::geometry::Point<f32>),
    Left,
    /// A button went down, with Shift held or not, or up: 0 the main one, 1 the middle, 2 the secondary, as the DOM
    /// numbers them.
    Down(crate::geometry::Point<f32>, u8, bool),
    Up(crate::geometry::Point<f32>, u8),
    /// Scrolled along and down by so many pixels (left and up negative), in the trackpad's phase where the system
    /// says it, and by so many notches along and down where a wheel turned by clicks, as a mouse's does: what tells
    /// it from a trackpad's or a smooth wheel's scroll, which has none.
    Wheel(crate::geometry::Point<f32>, crate::geometry::Point<f32>, Option<crate::view::Phase>, Option<crate::geometry::Point<f32>>),
    /// A key the page hears first, as Flutter's keyboard manager gives its framework each key.
    Key(crate::inspect::Key),
    /// What X11's input method composes or commits, which has no text input plugin.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Ime(crate::editing::Ime),
    /// What its text input plugin says.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    TextInputClient(crate::editing::TextInputClient),
    /// A panel was dragged to `at` on the screen named the first, in points from its top left (`Places`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Placed(String, crate::geometry::Point<f32>),
    /// The window got the keyboard, or lost it.
    Focus(bool),
    /// The device's picker for picker node `0` chose value `1` (`Device::pick`).
    #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
    Picked(crate::tree::Id, String),
}

/// How long this thread has run on a CPU, as its system counts it (POSIX's `CLOCK_THREAD_CPUTIME_ID`): what its JS
/// cost, whatever else the machine ran meanwhile, which its time by the clock isn't, so a busy machine's doesn't read as
/// a slow page. On the web, which can't ask, and on Windows, its time by the clock: Windows counts a thread's time
/// (`GetThreadTimes`) in its scheduler's ticks of 15.6 ms, longer than a frame's work, which read as 0 or 15.6 ms.
#[cfg(unix)]
pub fn on_cpu() -> std::time::Duration {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // Safety: it writes only the timespec it's given.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    std::time::Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

#[cfg(any(windows, target_arch = "wasm32"))]
pub fn on_cpu() -> std::time::Duration {
    thread_local!(static SINCE: web_time::Instant = web_time::Instant::now());
    SINCE.with(web_time::Instant::elapsed)
}
