//! Every app's native window, drawn by one thread of one process: one connection to the display, one GPU context, one
//! glyph atlas and one font set, and a QuickJS sandbox per app (its `Screen`'s) on a thread of its own (`js::Thread`),
//! so another open app costs a few MB, as Zed's and Raycast's windows do, and one busy in its JS holds up no window's
//! frames. What the rest of the program asks of them (open one, tell one's app something) is a job run on that thread
//! (`Native::run`), as GPUI's foreground executor runs its work: on a computer a thread of its own, and on a phone the
//! app's UI thread, where its views are (`android`, `ios`).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::render::{Atlas, Context, Renderer};
use crate::text::TextSystem;
use crate::Screen;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "android")]
mod android;
/// The rail's home line over a phone's app windows, pulled home (`Windows::line`).
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub(crate) mod rail;
#[cfg(target_os = "android")]
use android::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "ios")]
pub mod ios;
#[cfg(target_os = "ios")]
use ios::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(target_arch = "wasm32")]
use web::{Display, Handle, NativeWindow, appearance, clipboard, reduced_motion};

/// A window, as the windows' thread names it.
pub type WindowId = u64;

/// Whether its windows can be a root window, under every other (`WindowOptions::root`): a phone's whose launcher is
/// drawn natively. Android's are; an iPhone's are once its views stack so.
pub const ROOTS: bool = cfg!(target_os = "android");

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
    /// Its first frame is on screen.
    Drawn,
    /// It was closed: by the person, or by `close`.
    Closed,
    /// Its page uses what this host can't draw, which it says in its place: why.
    Mismatched(String),
    /// Its page's module didn't run, so it says only that its app didn't open: why.
    Failed(String),
    /// A panel was summoned and its frame is on screen: how long after it was asked to show (`summon`).
    Shown(std::time::Duration),
    /// A panel put itself away, as something else was clicked or came in front.
    PutAway,
    /// A window that isn't a panel lost the keyboard: its person is using something else.
    Unfocused,
}

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
    /// The window under every other, as a phone's launcher (its Home Screen) is: the system's Back doesn't close it,
    /// and the apps' windows open over it. It's hidden while something else covers the screen (`Windows::hide`).
    pub root: bool,
    /// Where on the screen it opened from, in points: its app's icon on the Home Screen, which pulled home by the
    /// rail's line it goes back into (`Windows::line`).
    pub icon: Option<crate::geometry::Bounds<f32>>,
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
            let atlas = Arc::new(Atlas::new(&context));
            let text = TextSystem::new(crate::FONT, crate::fonts()?, true);
            Ok(Windows { display, context, atlas, text, windows: HashMap::new(), next: 1, framed: web_time::Instant::now() })
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
        let native = self.display.open(id, &options)?;
        if let Ok(screen) = &mut screen {
            // A phone's app window has the rail's home line; its Home Screen doesn't.
            screen.rail = cfg!(target_os = "ios") && !options.root;
            screen.appearance = appearance();
            screen.reduced_motion = reduced_motion();
            if let Some(clipboard) = clipboard() {
                screen.clipboard = clipboard;
            }
            if let Some(native) = Native::started() {
                let native = std::sync::Mutex::new(native);
                screen.on_post(Arc::new(move || native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).run(move |windows| windows.posted(id))));
            }
        }
        let renderer = self.renderer(&native)?;
        let opened = web_time::Instant::now();
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
            },
        );
        Ok(id)
    }

    /// Window `id`'s app's JS posted what it changed: it's applied, and drawn on the loop's turn.
    fn posted(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get_mut(&id)
            && let Ok(screen) = &mut window.screen
            && screen.pump()
        {
            window.dirty = true;
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

    /// Answers an inspector's request of window `id`: `snap` drawn again offscreen, as it was last drawn, and read
    /// back, and the rest by its screen.
    pub fn inspect(&mut self, id: WindowId, request: crate::inspect::Request, reply: crate::Reply) {
        let Some(window) = self.windows.get_mut(&id) else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("there's no window {id} open"))));
        };
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let Ok(screen) = &mut window.screen else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("its app didn't open"))));
        };
        window.dirty = true;
        if request.op != "snap" {
            return screen.inspect(request, reply);
        }
        // Painted afresh, as it is now: a window with no surface (Android's view while it's hidden) has drawn nothing,
        // and the scene it last drew may be older than what it shows.
        let device = Size { width: DevicePixels((room.width.0 * scale).round() as i32), height: DevicePixels((room.height.0 * scale).round() as i32) };
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
        let mut scene = crate::render::scene::Scene::default();
        screen.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut scene);
        renderer.draw(&scene, [0.0, 0.0]);
        let snapped = renderer.read().ok_or_else(|| anyhow::anyhow!("nothing was drawn")).and_then(|(width, height, rgba)| {
            let mut png = Vec::new();
            image::RgbaImage::from_raw(width, height, rgba)
                .ok_or_else(|| anyhow::anyhow!("its pixels don't fill it"))?
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
            Ok(serde_json::json!({"png": data_encoding::BASE64.encode(&png), "width": width, "height": height}))
        });
        renderer.destroy();
        reply(crate::inspect::answer(&request.id, snapped))
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
        if window.visible {
            return self.display.activate(&window.parts[0].native);
        }
        window.visible = true;
        window.dirty = true;
        window.summoned = Some(asked);
        if let Ok(screen) = &mut window.screen {
            screen.shown(true);
            screen.summoned(key);
        }
        self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
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

    /// Whether panel `id` is showing.
    pub fn showing(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.visible)
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
        if let Some(mut window) = self.windows.remove(&id) {
            self.let_go(window.parts);
            (window.on_event)(Event::Closed);
        }
    }

    /// How long until any window's screen has something to do by itself, or a window waiting to show has waited long
    /// enough.
    fn due(&self) -> Option<std::time::Duration> {
        let screens = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due());
        let showing = self.windows.values().filter(|window| !window.shown).map(|window| SHOWN_WITHIN.saturating_sub(window.opened.elapsed()));
        // Where the display says nothing of its frames, the next is a 60 Hz display's.
        let frame = (self.framing() && !self.display.paced()).then(|| FRAME.saturating_sub(self.framed.elapsed()));
        // A pull's spring steps at a 120 Hz display's frames.
        let pulled = self.windows.values().any(|window| window.pull.as_ref().is_some_and(rail::Pull::springing)).then_some(std::time::Duration::from_micros(8_333));
        screens.chain(showing).chain(frame).chain(pulled).min()
    }

    /// Whether a window on screen wants the display's next frame: something on it moves by itself, or its page asked
    /// for one (`requestAnimationFrame`).
    fn framing(&self) -> bool {
        self.windows.values().any(|window| window.on_screen() && (window.moving || window.screen.as_ref().is_ok_and(Screen::framing)))
    }

    /// Makes what draws again, on a new GPU device, where the one it drew with was lost, as a browser makes a lost
    /// WebGPU device's: the atlas and every window's renderer, each window drawn afresh (`Screen::gpu_lost`).
    fn recover(&mut self) {
        let made = Context::new().map(|context| {
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

    /// After each turn of the loop: what the display said, timers that are due, then each window that changed drawn.
    fn turn(&mut self) {
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
        let shown = self.display.frame().or_else(|| (framing && !self.display.paced() && self.framed.elapsed() >= FRAME).then(web_time::Instant::now));
        if let Some(at) = shown {
            self.framed = at;
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
        // A window whose page uses what this host can't draw says so, and why.
        for window in self.windows.values_mut() {
            if let Some(why) = window.screen.as_ref().ok().and_then(|screen| screen.mismatched.clone()) {
                window.screen = Err(format!("{} didn't open: {why}", window.title));
                window.dirty = true;
                (window.on_event)(Event::Mismatched(why));
            }
        }
        for window in self.windows.values_mut() {
            // A new window shows once its page has heard back from what it asked, or has waited long enough.
            if !window.shown {
                if window.screen.as_ref().is_ok_and(Screen::waiting) && window.opened.elapsed() < SHOWN_WITHIN {
                    continue;
                }
                window.shown = true;
                self.display.show(&window.parts[0].native);
            }
            // A panel put away is drawn as its page changes, never shown, so its text is shaped, its glyphs and
            // pictures are in the atlas and its surfaces have had a frame by the time it's summoned, and the frame it
            // shows with is as quick as any after (26 to 52 ms on Linux's first summon after the app started, where
            // later ones take 7). What moves asks for no more frames while it's away. A window with no surface yet
            // (Android's view, until it's laid out) is drawn once it has one.
            if window.panel && !window.visible {
                if std::mem::take(&mut window.dirty)
                    && let Ok(screen) = &mut window.screen
                    && window.parts[0].renderer.is_some()
                {
                    screen.paint(window.room, window.parts[0].native.scale(), &mut self.text, &self.atlas, &mut window.scene);
                    let glass = screen.glass().to_vec();
                    window.arrange(&self.display, &self.context, &self.atlas, &glass);
                }
                continue;
            }
            if !window.dirty || window.parts[0].renderer.is_none() {
                continue;
            }
            window.dirty = false;
            let (room, scale) = (window.room(), window.parts[0].native.scale());
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
            let glass = window.screen.as_ref().map(|screen| screen.glass().to_vec()).unwrap_or_default();
            if !window.arrange(&self.display, &self.context, &self.atlas, &glass) {
                window.missed(&self.context, &self.atlas);
                continue;
            }
            window.missed = 0;
            if std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
            }
            if !window.drawn {
                window.drawn = true;
                (window.on_event)(Event::Drawn);
            }
            if let Some(asked) = window.summoned.take() {
                (window.on_event)(Event::Shown(asked.elapsed()));
            }
        }
        self.display.request_frame(self.framing());
        self.display.flush();
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
        if let Input::Close = input {
            return self.close(id);
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
            Input::Appearance(appearance) => screen.appearance = appearance,
            Input::Moved(at) => {
                screen.pointer_moved(at);
                let cursor = screen.cursor(at);
                self.display.set_cursor(&window.parts[part].native, cursor);
            }
            Input::Left => screen.pointer_left(),
            Input::Down(at, button, shift) => screen.pointer_down(at, button, shift),
            Input::Up(at, button) => screen.pointer_up(at, button),
            Input::Wheel(at, delta, phase) => drop(screen.scroll(at, delta, phase)),
            Input::Key(key) => {
                // Ctrl-W and Ctrl-Q close the window, as a Linux or Windows app's do (a Mac's is ⌘W, its own).
                if !cfg!(target_os = "macos") && key.ctrl && !key.alt && matches!(key.key.as_str(), "w" | "q") {
                    return self.close(id);
                }
                screen.key(&key);
            }
            Input::Ime(ime) => screen.ime(ime),
            Input::TextInputClient(said) => screen.text_input_client(&mut window.text_input, said),
            // A panel goes as something else takes the keyboard, as Spotlight does.
            Input::Focus(false) if window.panel && window.visible => {
                screen.activate(false);
                self.put_away(id, false);
                if let Some(window) = self.windows.get_mut(&id) {
                    (window.on_event)(Event::PutAway);
                }
                return;
            }
            Input::Focus(active) => {
                screen.activate(active);
                if !active {
                    (window.on_event)(Event::Unfocused);
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
    /// The same, at `by` further along and down: a part's point, in its page.
    fn moved_by(self, by: crate::geometry::Point<f32>) -> Input {
        let moved = |at: crate::geometry::Point<f32>| crate::geometry::point(at.x + by.x, at.y + by.y);
        match self {
            Input::Moved(at) => Input::Moved(moved(at)),
            Input::Down(at, button, shift) => Input::Down(moved(at), button, shift),
            Input::Up(at, button) => Input::Up(moved(at), button),
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
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Appearance(crate::Appearance),
    Close,
    Moved(crate::geometry::Point<f32>),
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
}
