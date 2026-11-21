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
//!   `requestAnimationFrame`. A display that says nothing (`Display::paced` false), or whose link hasn't ticked for
//!   a refresh and a quarter, is ticked by the loop's own clock, at the display's refresh (`Display::refresh`), or 60 Hz
//!   where it doesn't say.
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
mod cover;
#[cfg(feature = "fake")]
mod harness;
mod input;
mod native;
mod open;
mod pages;
mod panel;
mod touch;
mod turn;
mod window;
use cover::{BLANK, FADES_IN, QUIET, SHOWN_WITHIN, Cover, covered, whole};
use window::fit;
pub(crate) use input::Input;
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
    /// It's shown nothing of its page `BLANK` after it opened, its page loaded again, or a panel put away was summoned:
    /// why, as a browser's tab left blank. It's said once each time, and it may yet draw.
    Blank(String),
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

/// One of a window's native windows, each with a surface of its own: a window's only one, or each part of a panel's
/// page that has the system's material (`Glass`), its search and its card, the first the panel itself and the rest its
/// child windows, which move, hide and stack with it.
struct Part {
    native: NativeWindow,
    /// Its box of the page, in points.
    bounds: crate::geometry::Bounds<f32>,
    radius: f32,
}

struct Window {
    /// Its native windows, the first the window itself.
    parts: Vec<Part>,
    /// What draws its parts, each one of its targets, all in one submit, once its first has a surface: at once on a
    /// computer, as its view's is made on Android (`draws_on`).
    renderer: Option<Renderer>,
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
    /// Hidden while the page over it shows an app (`hide`), as a phone's root window is: not drawn, its images let go of.
    hidden: bool,
    summoned: Option<web_time::Instant>,
    /// When a panel last went as something else took the keyboard (`showing`).
    left: Option<web_time::Instant>,
    /// The Colour Mode its system's own parts were last drawn in (`Display::set_appearance`).
    chrome: Option<Option<crate::Appearance>>,
    /// What it shows: its app's screen, or why its app didn't open; nothing yet while its page starts (`opening`),
    /// until it's given it (`attach`), which its title row is made for (`WindowOptions::title_row`).
    screen: Result<Screen, String>,
    pending: bool,
    /// What waits for its page not to be in use (`Windows::when_unused`) that came while it was still opening: its
    /// page has it as it's attached.
    when_unused: Vec<Box<dyn FnOnce() + Send>>,
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
    /// Whether it's been looked at for being blank since it opened (`Blank`), and since when it's been looked at for
    /// that: it opened, its page loaded again, or, for a panel put away, which nobody sees, now.
    looked: bool,
    looking: web_time::Instant,
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
    /// What covered its page went: its page is presented once more, without it.
    uncovered: bool,
    /// Its frames while something on it moves.
    timing: frame_timing::Timing,
    /// When it last handed the display a frame, since the display's last tick (`Windows::turned`).
    presented: Option<web_time::Instant>,
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
    /// When the display last showed a frame, as its platform says, or the loop's own clock stood in for it (`showed`),
    /// or a window last drew where its link doesn't tick: what the loop's own clock counts its next frame from.
    framed: web_time::Instant,
    showed: web_time::Instant,
    /// Whether the display's frames were asked for as the loop last turned, and when its link last ticked since they
    /// were (`ticking`).
    wanted: bool,
    ticked: Option<web_time::Instant>,
    /// The refresh its link last said the display showed, which its others are whole refreshes from, so the loop's own
    /// clock stands in on them (`turned`).
    vsync: Option<web_time::Instant>,
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

/// A 60 Hz display's refresh: the fake platform's own thread's (`native`), and where a system has no display to ask
/// (a Mac's with none attached, a browser before it's shown two frames in a row).
const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

/// Now, as the windows' loop tells the time a frame or an input came: the system's, or on the fake platform its
/// driver's, which moves only as it's moved (`fake::now`). How long work took is the system's everywhere.
#[cfg(not(feature = "fake"))]
pub(crate) fn now() -> web_time::Instant {
    web_time::Instant::now()
}
#[cfg(feature = "fake")]
pub(crate) use fake::now;
/// The fake platform's Reduce Motion setting.
#[cfg(feature = "fake")]
pub use fake::reduce as fake_reduce;
/// The fake platform's time, in ms on its pages' clock.
#[cfg(feature = "fake")]
pub use fake::time as fake_time;

/// When `at`, an instant the windows' loop told (a frame's), was on `clock`, a page's: on the fake platform, whose
/// instants are its clock's (`fake::now`), exactly as long before its time; elsewhere as the clock says (`Clock::at`).
#[cfg(not(feature = "fake"))]
pub(crate) fn time_at(clock: &crate::js::Clock, at: web_time::Instant) -> f64 {
    clock.at(at)
}
#[cfg(feature = "fake")]
pub(crate) fn time_at(clock: &crate::js::Clock, at: web_time::Instant) -> f64 {
    clock.now() - since(at).as_secs_f64() * 1000.0
}

/// How long since `at`, by `now`.
pub(crate) fn since(at: web_time::Instant) -> std::time::Duration {
    now().saturating_duration_since(at)
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

/// `renderer` draws on `native` too, a panel's part: its surface, after those it draws on.
#[cfg(not(feature = "fake"))]
fn also_on(renderer: &mut Renderer, context: &Context, native: &NativeWindow) -> anyhow::Result<()> {
    renderer.add(context, native.surface()?, native.device_size(), native.transparent())
}

/// `renderer` draws on `native` too on the fake platform: an image offscreen.
#[cfg(feature = "fake")]
fn also_on(renderer: &mut Renderer, context: &Context, native: &NativeWindow) -> anyhow::Result<()> {
    renderer.add_offscreen(context, native.device_size());
    Ok(())
}

/// What draws on `parts`, a window's: one renderer, each part one of its targets.
fn draws_on(context: &Context, atlas: Arc<Atlas>, parts: &[Part]) -> anyhow::Result<Renderer> {
    let mut renderer = drawn_on(context, atlas, &parts[0].native)?;
    for part in &parts[1..] {
        also_on(&mut renderer, context, &part.native)?;
    }
    Ok(renderer)
}

/// `bounds` with its edges on whole points, as a window's frame is.
fn whole_points(bounds: crate::geometry::Bounds<f32>) -> crate::geometry::Bounds<f32> {
    let (left, top) = (bounds.origin.x.round(), bounds.origin.y.round());
    let (right, bottom) = ((bounds.origin.x + bounds.size.width).round(), (bounds.origin.y + bounds.size.height).round());
    crate::geometry::Bounds { origin: crate::geometry::point(left, top), size: Size { width: right - left, height: bottom - top } }
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
