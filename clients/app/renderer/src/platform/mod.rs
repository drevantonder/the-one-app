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

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::render::{atlas::Atlas, context::Context, renderer::Renderer};
use crate::text::TextSystem;
use crate::Screen;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{Display, Handle, NativeWindow, appearance, clipboard, open_files, reduced_motion};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::{Display, Handle, NativeWindow, appearance, clipboard, open_files, reduced_motion};
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::{Display, Handle, NativeWindow, appearance, clipboard, open_files, reduced_motion};
#[cfg(target_os = "android")]
mod android;
/// A native screen's semantic tree as AccessKit's, for each phone's screen reader.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) mod a11y;
/// The rail's home line over a phone's app windows, pulled home (`Windows::line`).
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub(crate) mod rail;
mod conformance;
mod snapshot;
mod frame_timing;
pub use frame_timing::Frames;
mod pressure;
pub use pressure::Pressure;
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
    /// It was closed: by the person, or by `close`.
    Closed,
    /// Its page uses what this host can't draw, which it says in its place: why.
    Mismatched(String),
    /// Its page's module didn't run, so it says only that its app didn't open: why.
    Failed(String),
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
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub maximized: bool,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
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
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
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
    /// last snapshot as it launches (`snapshot`). Whoever opens it names it by whose it is and its app's build, so it
    /// never shows another person's, or another build's.
    pub snapshot: Option<std::path::PathBuf>,
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
type Sender = calloop::channel::Sender<Job>;
#[cfg(target_os = "windows")]
type Sender = windows::Sender;
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

#[cfg(target_os = "windows")]
use windows::start;
#[cfg(target_os = "android")]
use android::start;
#[cfg(target_os = "ios")]
use ios::start;
#[cfg(target_arch = "wasm32")]
use web::start;

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

/// How long a window waits to show for its page to hear back from what it first asked, so it shows its first frame
/// with what it asked for, or says why it didn't open, as its page turns out to need.
const SHOWN_WITHIN: std::time::Duration = std::time::Duration::from_millis(1000);

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
    /// A panel showing as its page asked for another room, which it's placed again for once it's drawn in it, as it
    /// would have been summoned: its page asks as it first opens, which is after it first shows.
    placing: bool,
    /// When it was opened, hidden, and whether it's shown.
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
    /// What it shows: its app's screen, or why its app didn't open.
    screen: Result<Screen, String>,
    title: String,
    scene: crate::render::scene::Scene,
    /// Whether what it shows changed, and so is drawn on the loop's turn, and whether something on it moves by
    /// itself, and so is drawn again on the display's next frame.
    dirty: bool,
    moving: bool,
    drawn: bool,
    /// How many frames running it had nothing to draw on (`Window::missed`).
    missed: u32,
    /// What its text input plugin was last told.
    text_input: crate::view::TextInputConnection,
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
    /// Where its frame is kept (`WindowOptions::snapshot`), and the one kept, as RGBA rows its size, shown until its
    /// page has started and heard back, and whether its renderer has it yet.
    snapshot: Option<std::path::PathBuf>,
    picture: Option<Vec<u8>>,
    pictured: bool,
    /// Its frames while something on it moves.
    timing: frame_timing::Timing,
    /// Whether it's handed the display a frame since the display's last tick (`Windows::turned`).
    presented: bool,
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
    /// Whether the last frames met the display's refresh, and how pressed the device is, as that and its system say.
    deadlines: pressure::Deadlines,
    pressure: Pressure,
    /// What the system last said of its pressure, and when it was asked, which is once a second.
    system: (web_time::Instant, Pressure),
}

/// How often frames come where a platform says nothing of its display's: a 60 Hz display's.
const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

impl Windows {
    #[cfg(not(target_arch = "wasm32"))]
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
            let text = TextSystem::new(crate::FONT, crate::fonts()?, true);
            Ok(Windows { display, context, atlas, text, windows: HashMap::new(), next: 1, framed: web_time::Instant::now(), wanted: false, ticked: false, check: None, record: Default::default(), deadlines: Default::default(), pressure: Pressure::Nominal, system: (web_time::Instant::now(), Pressure::Nominal) })
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
    pub fn open(&mut self, options: WindowOptions, mut screen: Result<Screen, String>, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let asked = web_time::Instant::now();
        let native = self.display.open(id, &options)?;
        let windowed = asked.elapsed();
        if let Some(owner) = options.owner.and_then(|owner| self.windows.get(&owner)) {
            self.display.attach(&native, &owner.parts[0].native);
        }
        if let Ok(screen) = &mut screen {
            // A phone's app window has the rail's home line; its Home Screen doesn't.
            screen.rail = cfg!(any(target_os = "android", target_os = "ios")) && !options.root;
            screen.appearance = appearance();
            screen.reduced_motion = reduced_motion();
            screen.pressure = self.pressure;
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
                screen.device = Box::new(Computer);
            }
            // An app's window on Linux is unframed, its title row a GNOME header bar with the desktop's window buttons.
            #[cfg(target_os = "linux")]
            if screen.title.is_some() {
                screen.window_buttons = linux::button_layout();
                self.display.unframe(&native);
            }
            if let Some(native) = Native::started() {
                let native = std::sync::Mutex::new(native);
                screen.on_post(Arc::new(move || native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).run(move |windows| windows.posted(id))));
            }
        }
        let rendering = web_time::Instant::now();
        let renderer = self.renderer(&native)?;
        // Where a window's opening went, which a page's own (`Opening` in clients/app/src/native.rs) has as one step.
        eprintln!("{} opened in {} ms: the system's window {} ms, its renderer {} ms", options.title, asked.elapsed().as_millis(), windowed.as_millis(), rendering.elapsed().as_millis());
        let opened = web_time::Instant::now();
        let device = native.device_size();
        let picture = options.snapshot.as_deref().filter(|_| screen.is_ok()).and_then(|stem| snapshot::read(stem, (device.width.0 as u32, device.height.0 as u32)));
        let panel = options.panel.is_some();
        let whole = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: Size { width: options.size.width.0, height: options.size.height.0 } };
        self.windows.insert(
            id,
            Window {
                parts: vec![Part { native, renderer, bounds: whole, radius: options.panel.unwrap_or(0.0) }],
                room: options.size,
                placing: false,
                opened,
                shown: panel,
                panel,
                visible: false,
                summoned: None,
                left: None,
                chrome: None,
                screen,
                title: options.title,
                scene: Default::default(),
                dirty: true,
                moving: false,
                drawn: false,
                missed: 0,
                text_input: Default::default(),
                on_event: Box::new(on_event),
                tag: options.tag,
                places: options.places,
                root: options.root,
                icon: options.icon,
                pull: None,
                snapshot: options.snapshot,
                picture,
                pictured: false,
                presented: false,
                timing: Default::default(),
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
            window.timing.applied(started.elapsed());
        }
    }

    /// What draws `native`, where it has a surface to draw on yet.
    fn renderer(&self, native: &NativeWindow) -> anyhow::Result<Option<Renderer>> {
        if !native.has_surface() {
            return Ok(None);
        }
        Renderer::new(&self.context, self.atlas.clone(), native.surface()?, native.device_size(), native.transparent()).map(Some)
    }

    /// Whether a window holds something its person typed that its app hasn't taken (`Screen::typed`): any app's, and
    /// a panel while it's showing.
    pub fn typed(&self) -> bool {
        self.windows.values().any(|window| (!window.panel || window.visible) && window.screen.as_ref().is_ok_and(Screen::typed))
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
        self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
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
        // Its next frame's images taken while it's still on the screen, so it's drawn at once as it's summoned.
        for renderer in window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()) {
            renderer.reserve();
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
        let gpu = self.context.memory().map(|(reserved, allocated)| serde_json::json!({"reserved": reserved, "allocated": allocated}));
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
    pub(crate) fn line(&mut self, id: WindowId, finger: u64, at: crate::geometry::Point<f32>, phase: rail::Phase) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        let (room, icon) = (window.room(), window.icon);
        let Ok(screen) = &mut window.screen else { return false };
        let Some(pull) = window.pull.as_mut() else {
            if phase != rail::Phase::Down || window.root || !screen.on_line(at) {
                return false;
            }
            window.pull = Some(rail::Pull::new(finger, at));
            return true;
        };
        if pull.finger() != finger {
            return phase == rail::Phase::Down && screen.on_line(at);
        }
        match phase {
            rail::Phase::Down => {}
            rail::Phase::Moved => {
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
            rail::Phase::Up | rail::Phase::Cancelled => {
                if !pull.let_go(phase == rail::Phase::Cancelled) {
                    window.pull = None;
                }
            }
        }
        true
    }

    /// Steps each window's pull let go: home, every app's window closes; back, it's as it was.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn spring(&mut self) {
        let mut home = false;
        let mut back = false;
        for window in self.windows.values_mut() {
            let (room, icon) = (window.room(), window.icon);
            let reduced = window.screen.as_ref().is_ok_and(|screen| screen.reduced_motion);
            let Some(pull) = window.pull.as_mut().filter(|pull| pull.springing()) else { continue };
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
        self.windows.get(&id).is_some_and(|window| window.visible || window.left.is_some_and(|left| left.elapsed() < CLICK))
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

    /// How long until any window's screen has something to do by itself, or a window waiting to show has waited long
    /// enough.
    fn due(&self) -> Option<std::time::Duration> {
        let screens = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due());
        let showing = self.windows.values().filter(|window| !window.shown || window.picture.is_some()).map(|window| SHOWN_WITHIN.saturating_sub(window.opened.elapsed()));
        // Where the display says nothing of its frames, or hasn't yet since they were asked for, the next is a 60 Hz
        // display's.
        let frame = (self.framing() && !self.ticking()).then(|| FRAME.saturating_sub(self.framed.elapsed()));
        // A pull's spring steps at a 120 Hz display's frames.
        let pulled = self.windows.values().any(|window| window.pull.as_ref().is_some_and(rail::Pull::springing)).then_some(std::time::Duration::from_micros(8_333));
        let check = self.check.as_ref().map(conformance::Check::due);
        screens.chain(showing).chain(frame).chain(pulled).chain(check).min()
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
            || self.windows.values().any(|window| window.on_screen() && (window.moving || window.screen.as_ref().is_ok_and(Screen::framing)))
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
                if part.native.has_surface() {
                    match part.native.surface().and_then(|surface| Renderer::new(&context, atlas.clone(), surface, part.native.device_size(), part.native.transparent())) {
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
    fn turn(&mut self) {
        let started = web_time::Instant::now();
        self.turned();
        for window in self.windows.values_mut() {
            window.timing.turned(started.elapsed());
        }
    }

    /// A turn of the loop: what the display said, timers that are due, then each window that changed drawn.
    fn turned(&mut self) {
        let turned = web_time::Instant::now();
        if self.context.lost() {
            self.recover();
        }
        for (id, part, input) in self.display.events() {
            self.input(id, part, input);
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.spring();
        // The display showed a frame: what moves on a window on screen is drawn again, and its page's
        // `requestAnimationFrame` runs, told when, as a browser's does once a frame.
        let framing = self.framing();
        let ticked = self.display.frame();
        self.ticked |= ticked.is_some();
        let shown = ticked.or_else(|| (framing && !self.ticking() && self.framed.elapsed() >= FRAME).then(web_time::Instant::now));
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
        if shown.is_some() {
            for window in self.windows.values_mut() {
                window.presented = false;
            }
        }
        for window in self.windows.values_mut() {
            // A new window shows once its page has heard back from what it asked, or has waited long enough: at once
            // where it has a snapshot, which it shows until then.
            let starting = window.screen.as_ref().is_ok_and(Screen::starting) && window.opened.elapsed() < SHOWN_WITHIN;
            if !window.shown {
                if starting && window.picture.is_none() {
                    continue;
                }
                window.shown = true;
                self.display.show(&window.parts[0].native);
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
                    screen.paint(window.room, window.parts[0].native.scale(), &mut self.text, &self.atlas, &mut window.scene);
                    // Drawn once, so its surfaces have had a frame; after that it's painted alone, and the image it
                    // took as it went (`put_away`) is kept for the frame it's summoned with.
                    if !window.drawn {
                        let glass = screen.glass().to_vec();
                        window.arrange(&self.display, &self.context, &self.atlas, &glass);
                        window.drawn = true;
                        (window.on_event)(Event::Drawn);
                    }
                }
                continue;
            }
            // Its snapshot, while it fits the window, until its page has started; then what its page draws.
            let device = window.parts[0].native.device_size();
            let (width, height) = (device.width.0 as u32, device.height.0 as u32);
            if window.picture.as_ref().is_some_and(|picture| !starting || picture.len() != (width * height * 4) as usize) {
                window.picture = None;
                window.dirty = true;
            }
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
            if let Some(picture) = &window.picture {
                if !std::mem::replace(&mut window.pictured, true)
                    && let Some(renderer) = &mut window.parts[0].renderer
                {
                    renderer.picture(snapshot::RASTER, width, height, picture);
                }
                window.scene.clear();
                let px = crate::geometry::ScaledPixels;
                let whole = crate::geometry::Bounds { origin: Point { x: px(0.0), y: px(0.0) }, size: Size { width: px(width as f32), height: px(height as f32) } };
                window.scene.picture(snapshot::RASTER, whole);
                window.scene.finish();
            } else {
                match &mut window.screen {
                    Ok(screen) => {
                        screen.paint(room, scale, &mut self.text, &self.atlas, &mut window.scene);
                        window.moving = screen.take_again();
                        let calls = screen.text_input(&mut window.text_input);
                        if !calls.is_empty() {
                            self.display.text_input(&window.parts[0].native, &calls);
                        }
                        self.display.set_title_row(&window.parts[0].native, screen.title_row());
                    }
                    Err(why) => crate::view::paint_message(why, appearance(), room, scale, &mut self.text, &self.atlas, &mut window.scene),
                }
            }
            let painted = web_time::Instant::now();
            let glass = window.screen.as_ref().map(|screen| screen.glass().to_vec()).unwrap_or_default();
            if !window.arrange(&self.display, &self.context, &self.atlas, &glass) {
                window.missed(&self.context, &self.atlas);
                continue;
            }
            window.missed = 0;
            window.presented = true;
            if self.check.is_some() {
                self.record.present();
            }
            let moving = window.moving || window.screen.as_ref().is_ok_and(Screen::framing);
            if window.screen.as_ref().is_ok_and(Screen::held) {
                window.timing.input("drag");
            }
            let refresh = self.display.refresh();
            let drawn = window.parts.iter().filter_map(|part| part.renderer.as_ref());
            let (acquire, gpu) = drawn.fold((std::time::Duration::ZERO, None), |(acquire, gpu), renderer| {
                let (all, blur) = gpu.unwrap_or_default();
                (acquire + renderer.acquired(), renderer.gpu().map(|(part, blurs)| (all + part, blur + blurs)).or(gpu))
            });
            let frame = frame_timing::Frame { at: web_time::Instant::now(), paint: painted - began, draw: painted.elapsed(), acquire, gpu: gpu.map(|(all, _)| all), blur: gpu.map(|(_, blur)| blur), applied: Default::default(), turned: Default::default() };
            let (interval, frames) = window.timing.drawn(frame, moving, refresh);
            if let Some(interval) = interval {
                self.deadlines.frame(frame_timing::Timing::long(interval, refresh.unwrap_or(FRAME)));
            }
            if let Some(frames) = frames {
                (window.on_event)(Event::Frames(frames));
            }
            if std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
            }
            if !window.drawn {
                window.drawn = true;
                (window.on_event)(Event::Drawn);
            }
            if let Some(asked) = window.summoned.take() {
                let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
                // Where its time went: to the turn it's drawn on, on that turn before it (input, timers, other windows
                // drawn first), painting it and drawing it.
                let (before, on) = (turned.saturating_duration_since(asked), began - turned);
                eprintln!("the launcher's first frame as it was summoned: {:.1} ms before its turn, {:.1} ms on it before it was painted, {:.1} ms to paint and {:.1} ms to draw", ms(before), ms(on), ms(painted - began), ms(painted.elapsed()));
                (window.on_event)(Event::Shown(asked.elapsed()));
            }
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
        if let Some(check) = self.check.take_if(|check| check.over()) {
            let said = check.said(&self.record, self.display.paced());
            (check.reply)(crate::inspect::answer(&check.id, Ok(said)));
        }
    }

    /// How pressed the device is, the worse of what its system and its frames say, given to each window's screen as
    /// it changes, which draws as it says (`Screen::pressure`).
    fn pressed(&mut self) {
        if self.system.0.elapsed() >= std::time::Duration::from_secs(1) {
            self.system = (web_time::Instant::now(), self.display.pressure());
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
        if !window.drawn || window.picture.is_some() || screen.starting() {
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
        if self.check.is_some() {
            self.record.input(input.kind(), None);
        }
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
                if part.native.has_surface() {
                    match part.native.surface().and_then(|surface| Renderer::new(&self.context, self.atlas.clone(), surface, part.native.device_size(), part.native.transparent())) {
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
            Input::Resized(_) | Input::Surface | Input::SurfaceLost => {}
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
                    menu.at = crate::geometry::point(menu.at.x - origin.x, menu.at.y - origin.y);
                    self.display.text_menu(&window.parts[part].native, menu);
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
            Input::Wheel(at, delta, phase) => {
                window.timing.input("scroll");
                screen.scroll(at, delta, phase);
            }
            Input::Key(key) => drop(screen.key(&key)),
            Input::Ime(ime) => screen.ime(ime),
            Input::TextInputClient(said) => screen.text_input_client(&mut window.text_input, said),
            // A panel goes as something else takes the keyboard, as Spotlight does.
            Input::Focus(false) if window.panel && window.visible => {
                window.left = Some(web_time::Instant::now());
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
        // What it changed reaches the text input plugin at once, before its next key, not with the frame.
        let calls = screen.text_input(&mut window.text_input);
        if !calls.is_empty() {
            self.display.text_input(&window.parts[0].native, &calls);
        }
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
            match part.native.surface().and_then(|surface| Renderer::new(context, atlas.clone(), surface, part.native.device_size(), part.native.transparent())) {
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
                let renderer = Renderer::new(context, atlas.clone(), native.surface()?, native.device_size(), native.transparent())?;
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

/// `bounds` with its edges on whole points, as a window's frame is.
fn whole_points(bounds: crate::geometry::Bounds<f32>) -> crate::geometry::Bounds<f32> {
    let (left, top) = (bounds.origin.x.round(), bounds.origin.y.round());
    let (right, bottom) = ((bounds.origin.x + bounds.size.width).round(), (bounds.origin.y + bounds.size.height).round());
    crate::geometry::Bounds { origin: crate::geometry::point(left, top), size: Size { width: right - left, height: bottom - top } }
}

impl Input {
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

    /// The same, at `by` further along and down: a part's point, in its page.
    fn moved_by(self, by: crate::geometry::Point<f32>) -> Input {
        let moved = |at: crate::geometry::Point<f32>| crate::geometry::point(at.x + by.x, at.y + by.y);
        match self {
            Input::Moved(at) => Input::Moved(moved(at)),
            Input::Predicted(at) => Input::Predicted(moved(at)),
            Input::Down(at, button, shift) => Input::Down(moved(at), button, shift),
            Input::Up(at, button) => Input::Up(moved(at), button),
            Input::Dropped(at, files) => Input::Dropped(moved(at), files),
            Input::Wheel(at, delta, phase) => Input::Wheel(moved(at), delta, phase),
            input => input,
        }
    }
}

/// What the display says happened to a window.
#[cfg_attr(any(target_os = "android", target_os = "ios", target_arch = "wasm32"), allow(dead_code))]
pub(crate) enum Input {
    Resized(Size<crate::geometry::DevicePixels>),
    /// It has a surface to draw on, or a new one (Android's view's).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    Surface,
    /// Its surface is going, as its view is hidden or gone: nothing draws on it.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
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
    /// Where the device predicts the pointer or finger down is going, a little after its last move
    /// (`Screen::predicted`).
    Predicted(crate::geometry::Point<f32>),
    Left,
    /// A button went down, with Shift held or not, or up: 0 the main one, 1 the middle, 2 the secondary, as the DOM
    /// numbers them.
    Down(crate::geometry::Point<f32>, u8, bool),
    Up(crate::geometry::Point<f32>, u8),
    /// Scrolled along and down by so many pixels (left and up negative), in the trackpad's phase where the system
    /// says it.
    Wheel(crate::geometry::Point<f32>, crate::geometry::Point<f32>, Option<crate::view::Phase>),
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
