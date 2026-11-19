//! Every strict app's window, drawn by one thread of one process: one connection to the display, one GPU context, one
//! glyph atlas and one font set, and a QuickJS sandbox per app (its `Screen`'s) on a thread of its own (`js::Thread`),
//! so another open app costs a few MB, as Zed's and Raycast's windows do, and one busy in its JS holds up no window's
//! frames. What the rest of the program asks of them (open one, tell one's app something) is a job run on that thread
//! (`Strict::run`), as GPUI's foreground executor runs its work: on a computer a thread of its own, and on Android the
//! app's UI thread, where its views are (`android`).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::geometry::{Pixels, Size};
use crate::render::{Atlas, Context, Renderer};
use crate::text::{PlatformTextSystem, TextSystem};
use crate::Screen;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{Display, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::{Display, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::{Display, NativeWindow, appearance, clipboard, reduced_motion};
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android::{Display, NativeWindow, appearance, clipboard, reduced_motion};

/// A window, as the windows' thread names it.
pub type WindowId = u64;

/// A job for the windows' thread.
pub type Job = Box<dyn FnOnce(&mut Windows) + Send>;

/// What a window tells whoever opened it.
pub enum Event {
    /// Its first frame is on screen.
    Drawn,
    /// It was closed: by the person, or by `close`.
    Closed,
    /// Its page uses what this host can't draw, so it's closed, to open in a web view: why.
    Mismatched(String),
    /// Its page's module didn't run, so it says only that its app didn't open: why.
    Failed(String),
    /// A panel was summoned and its frame is on screen: how long after it was asked to show (`summon`).
    Shown(std::time::Duration),
    /// A panel put itself away, as something else was clicked or came in front.
    PutAway,
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
}

/// Where a page asked for the system's own material, as a panel's glass (a material token that's `system`): in the
/// window's points, with its corners' radius and how opaque it's drawn. A host with that material puts it there,
/// under the page, and none under the rest; one without has drawn it as any other material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub bounds: crate::geometry::Bounds<f32>,
    pub radius: f32,
    pub opacity: f32,
}

/// Where to send the windows' thread a job.
#[derive(Clone)]
pub struct Strict(Sender);

static STARTED: OnceLock<Result<Strict, String>> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
type Sender = calloop::channel::Sender<Job>;
#[cfg(target_os = "windows")]
type Sender = windows::Sender;
#[cfg(target_os = "android")]
type Sender = android::Sender;

impl Strict {
    /// The windows' thread, started the first time it's asked for.
    pub fn get() -> anyhow::Result<Strict> {
        STARTED.get_or_init(|| start().map_err(|error| format!("{error:#}"))).clone().map_err(|error| anyhow::anyhow!(error))
    }

    /// The windows' thread, if it's been started.
    pub fn started() -> Option<Strict> {
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start() -> anyhow::Result<Strict> {
    let (sender, channel) = calloop::channel::channel::<Job>();
    let (ready_to, ready) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("strict".into()).spawn(move || {
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
                log::error!("strict windows: {error}");
            }
            windows.turn();
        }
    })?;
    ready.recv()??;
    Ok(Strict(sender))
}

/// How long a window waits to show for its page to hear back from what it first asked, so it shows its first frame
/// with what it asked for, or opens in a web view instead without having shown, as its page turns out to need.
const SHOWN_WITHIN: std::time::Duration = std::time::Duration::from_millis(1000);

struct Window {
    native: NativeWindow,
    /// When it was opened, hidden, and whether it's shown.
    opened: std::time::Instant,
    shown: bool,
    /// A panel's: whether it's showing, and since when it was asked to, until its frame is drawn.
    panel: bool,
    visible: bool,
    summoned: Option<std::time::Instant>,
    /// What it shows: its app's screen, or why its app didn't open.
    screen: Result<Screen, String>,
    title: String,
    /// What draws it, once it has a surface: at once on a computer, as its view's is made on Android.
    renderer: Option<Renderer>,
    scene: crate::render::scene::Scene,
    dirty: bool,
    drawn: bool,
    /// Where the system's material was put under it last.
    glass: Vec<Glass>,
    on_event: Box<dyn FnMut(Event)>,
    tag: serde_json::Value,
}

/// Every strict window, on the windows' thread.
pub struct Windows {
    display: Display,
    context: Context,
    atlas: Arc<Atlas>,
    text: TextSystem,
    windows: HashMap<WindowId, Window>,
    next: WindowId,
}

impl Windows {
    fn new(display: Display) -> anyhow::Result<Windows> {
        let context = Context::new(true)?;
        let atlas = Arc::new(Atlas::new(&context));
        let text = TextSystem::new(PlatformTextSystem::new(crate::FONT, crate::fonts()?, true));
        Ok(Windows { display, context, atlas, text, windows: HashMap::new(), next: 1 })
    }

    /// Opens a window showing `screen`, which tells `on_event` what happens to it. It's shown once its app's JS
    /// has started and heard back from what it first asked, or has waited long enough (`SHOWN_WITHIN`), and drawn
    /// as that posts what it changed.
    pub fn open(&mut self, options: WindowOptions, mut screen: Screen, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let native = self.display.open(id, &options)?;
        screen.appearance = appearance();
        screen.reduced_motion = reduced_motion();
        if let Some(clipboard) = clipboard() {
            screen.clipboard = clipboard;
        }
        if let Some(strict) = Strict::started() {
            screen.on_post(move || strict.run(move |windows| windows.posted(id)));
        }
        let renderer = self.renderer(&native)?;
        let opened = std::time::Instant::now();
        let panel = options.panel.is_some();
        self.windows.insert(
            id,
            Window {
                native,
                opened,
                shown: panel,
                panel,
                visible: false,
                summoned: None,
                screen: Ok(screen),
                title: options.title,
                renderer,
                scene: Default::default(),
                dirty: true,
                drawn: false,
                glass: Vec::new(),
                on_event: Box::new(on_event),
                tag: options.tag,
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
        Renderer::new(&self.context, self.atlas.clone(), native, native.device_size(), native.transparent()).map(Some)
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
        let Ok(screen) = &mut window.screen else {
            return reply(crate::inspect::answer(&request.id, Err(anyhow::anyhow!("its app didn't open"))));
        };
        window.dirty = true;
        if request.op != "snap" {
            return screen.inspect(request, reply);
        }
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), window.native.device_size());
        let mut scene = crate::render::scene::Scene::default();
        screen.paint(window.native.size(), window.native.scale(), &mut self.text, &self.atlas, &mut scene);
        renderer.draw(&scene);
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
            self.display.activate(&window.native);
        }
    }

    /// Shows panel `id`, centred near the top of the screen the pointer is on, with the keyboard, its frame drawn as
    /// it shows; the window that had the keyboard is kept, to give it back (`put_away`). `asked` is when it was asked
    /// to, which its `Shown` says how long after.
    /// `key` is the key its shortcut ends with, where its shortcut summoned it (`Screen::summoned`).
    pub fn summon(&mut self, id: WindowId, asked: std::time::Instant, key: Option<&str>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        if window.visible {
            return self.display.activate(&window.native);
        }
        window.visible = true;
        window.dirty = true;
        window.summoned = Some(asked);
        if let Ok(screen) = &mut window.screen {
            screen.shown(true);
            screen.summoned(key);
        }
        self.display.summon(&window.native);
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
        self.display.put_away(&window.native, give_back);
    }

    /// Whether panel `id` is showing.
    pub fn showing(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.visible)
    }

    /// Makes window `id` `height` points tall, and `width` wide where it's said, its top and its middle where they are,
    /// as a launcher grows and shrinks with what it shows.
    pub fn resize(&mut self, id: WindowId, width: Option<f32>, height: f32) {
        if let Some(window) = self.windows.get_mut(&id) {
            let size = window.native.size();
            let width = width.unwrap_or(size.width.0);
            if (size.height.0 - height).abs() >= 0.5 || (size.width.0 - width).abs() >= 0.5 {
                self.display.resize(&window.native, Size { width: Pixels(width), height: Pixels(height) });
                window.dirty = true;
            }
        }
    }

    /// Closes window `id`.
    pub fn close(&mut self, id: WindowId) {
        if let Some(mut window) = self.windows.remove(&id) {
            if let Some(mut renderer) = window.renderer.take() {
                renderer.destroy();
            }
            self.display.close(&window.native);
            (window.on_event)(Event::Closed);
        }
    }

    /// How long until any window's screen has something to do by itself, or a window waiting to show has waited long
    /// enough.
    fn due(&self) -> Option<std::time::Duration> {
        let screens = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due());
        let showing = self.windows.values().filter(|window| !window.shown).map(|window| SHOWN_WITHIN.saturating_sub(window.opened.elapsed()));
        screens.chain(showing).min()
    }

    /// After each turn of the loop: what the display said, timers that are due, then each window that changed drawn.
    fn turn(&mut self) {
        for (id, input) in self.display.events() {
            self.input(id, input);
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
        let mismatched: Vec<(WindowId, String)> =
            self.windows.iter().filter_map(|(id, window)| Some((*id, window.screen.as_ref().ok()?.mismatched.clone()?))).collect();
        for (id, why) in mismatched {
            if let Some(mut window) = self.windows.remove(&id) {
                if let Some(mut renderer) = window.renderer.take() {
                    renderer.destroy();
                }
                self.display.close(&window.native);
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
                self.display.show(&window.native);
            }
            // A panel put away is drawn as its page changes, never shown, so its text is shaped, its glyphs and
            // pictures are in the atlas and its surface has had a frame by the time it's summoned, and the frame it
            // shows with is as quick as any after (26 to 52 ms on Linux's first summon after the app started, where
            // later ones take 7). What moves asks for no more frames while it's away. A window with no surface yet
            // (Android's view, until it's laid out) is drawn once it has one.
            if window.panel && !window.visible {
                if std::mem::take(&mut window.dirty)
                    && let (Ok(screen), Some(renderer)) = (&mut window.screen, window.renderer.as_mut())
                {
                    renderer.update_drawable_size(window.native.device_size());
                    screen.paint(window.native.size(), window.native.scale(), &mut self.text, &self.atlas, &mut window.scene);
                    renderer.draw(&window.scene);
                }
                continue;
            }
            let Some(renderer) = window.renderer.as_mut().filter(|_| window.dirty) else { continue };
            window.dirty = false;
            let (size, scale) = (window.native.size(), window.native.scale());
            renderer.update_drawable_size(window.native.device_size());
            match &mut window.screen {
                Ok(screen) => {
                    screen.paint(size, scale, &mut self.text, &self.atlas, &mut window.scene);
                    window.dirty = screen.take_again();
                    if screen.glass != window.glass {
                        window.glass = screen.glass.clone();
                        self.display.set_glass(&window.native, &window.glass);
                    }
                    self.display.set_ime(&window.native, screen.caret_rect());
                    self.display.set_title_row(&window.native, screen.title_row());
                }
                Err(why) => crate::view::paint_message(why, appearance(), size, scale, &mut self.text, &self.atlas, &mut window.scene),
            }
            renderer.draw(&window.scene);
            if !window.drawn {
                window.drawn = true;
                (window.on_event)(Event::Drawn);
            }
            if let Some(asked) = window.summoned.take() {
                (window.on_event)(Event::Shown(asked.elapsed()));
            }
        }
        self.display.flush();
    }

    fn input(&mut self, id: WindowId, input: Input) {
        if let Input::Close = input {
            return self.close(id);
        }
        let Some(window) = self.windows.get_mut(&id) else { return };
        window.dirty = true;
        match input {
            Input::Resized(size) => return window.native.resized(size),
            // Its surface came, or a new one did: drawn on from now, a renderer for one that's gone let go first.
            Input::Surface => {
                if let Some(mut renderer) = window.renderer.take() {
                    renderer.destroy();
                }
                if window.native.has_surface() {
                    match Renderer::new(&self.context, self.atlas.clone(), &window.native, window.native.device_size(), window.native.transparent()) {
                        Ok(renderer) => window.renderer = Some(renderer),
                        Err(error) => log::error!("strict window {id} can't draw: {error:#}"),
                    }
                }
                return;
            }
            Input::SurfaceLost => {
                if let Some(mut renderer) = window.renderer.take() {
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
                self.display.set_cursor(&window.native, cursor);
            }
            Input::Left => screen.pointer_left(),
            Input::Down(at, button, shift) => screen.pointer_down(at, button, shift),
            Input::Up(at, button) => screen.pointer_up(at, button),
            Input::Wheel(at, delta) => drop(screen.scroll(at, delta)),
            Input::Key(key) => {
                // Ctrl-W and Ctrl-Q close the window, as a Linux or Windows app's do (a Mac's is ⌘W, its own).
                if !cfg!(target_os = "macos") && key.ctrl && !key.alt && matches!(key.key.as_str(), "w" | "q") {
                    return self.close(id);
                }
                screen.key(&key);
            }
            Input::Ime(ime) => screen.ime(ime),
            // A panel goes as something else takes the keyboard, as Spotlight does.
            Input::Focus(false) if window.panel && window.visible => {
                screen.activate(false);
                self.put_away(id, false);
                if let Some(window) = self.windows.get_mut(&id) {
                    (window.on_event)(Event::PutAway);
                }
            }
            Input::Focus(active) => screen.activate(active),
            Input::Close => {}
        }
    }
}

/// What the display says happened to a window.
#[cfg_attr(target_os = "android", allow(dead_code))]
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
    /// Scrolled along and down by so many pixels (left and up negative).
    Wheel(crate::geometry::Point<f32>, crate::geometry::Point<f32>),
    /// A key, and what an input method does: a Mac's go to the screen as AppKit asks (`mac::window`).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Key(crate::inspect::Key),
    /// What the input method composes or commits.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Ime(crate::editing::Ime),
    /// The window got the keyboard, or lost it.
    Focus(bool),
}
