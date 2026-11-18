//! Every strict app's window on a computer, drawn by one thread of one process: one connection to the display, one GPU
//! context, one glyph atlas and one font set, and a QuickJS sandbox per app (its `Screen`), so another open app costs
//! a few MB, as Zed's and Raycast's windows do. What the rest of the program asks of them (open one, tell one's app
//! something) is a job run on that thread (`Strict::run`), as GPUI's foreground executor runs its work.

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
    /// Its app sent a version of the strict protocol this host doesn't speak, so it's closed, to open in a web view.
    Mismatched,
}

pub struct WindowOptions {
    pub title: String,
    /// The window's class, which the desktop groups windows by (X11's `WM_CLASS`, Wayland's app id).
    pub app_id: String,
    pub size: Size<Pixels>,
    /// What whoever opened it says it is, which the inspector's `windows` lists with it: its app.
    pub tag: serde_json::Value,
}

/// Where to send the windows' thread a job.
#[derive(Clone)]
pub struct Strict(Sender);

#[cfg(target_os = "linux")]
type Sender = calloop::channel::Sender<Job>;
#[cfg(target_os = "windows")]
type Sender = windows::Sender;

impl Strict {
    /// The windows' thread, started the first time it's asked for.
    pub fn get() -> anyhow::Result<Strict> {
        static STARTED: OnceLock<Result<Strict, String>> = OnceLock::new();
        STARTED.get_or_init(|| start().map_err(|error| format!("{error:#}"))).clone().map_err(|error| anyhow::anyhow!(error))
    }

    /// Runs `job` on the windows' thread, where every window and its screen are.
    pub fn run(&self, job: impl FnOnce(&mut Windows) + Send + 'static) {
        let _ = self.0.send(Box::new(job));
    }
}

#[cfg(target_os = "windows")]
use windows::start;

#[cfg(target_os = "linux")]
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

struct Window {
    native: NativeWindow,
    /// What it shows: its app's screen, or why its app didn't open.
    screen: Result<Screen, String>,
    renderer: Renderer,
    scene: crate::render::scene::Scene,
    dirty: bool,
    drawn: bool,
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

    /// Opens a window showing `screen`, which tells `on_event` what happens to it.
    pub fn open(&mut self, options: WindowOptions, mut screen: Screen, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let native = self.display.open(id, &options)?;
        screen.appearance = appearance();
        screen.reduced_motion = reduced_motion();
        if let Some(clipboard) = clipboard() {
            screen.clipboard = clipboard;
        }
        let renderer = Renderer::new(&self.context, self.atlas.clone(), &native, native.device_size(), false)?;
        self.windows.insert(id, Window { native, screen: Ok(screen), renderer, scene: Default::default(), dirty: true, drawn: false, on_event: Box::new(on_event), tag: options.tag });
        Ok(id)
    }

    /// Opens a window that says only `why`, centred: its app didn't open.
    pub fn open_failed(&mut self, options: WindowOptions, why: &str, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let native = self.display.open(id, &options)?;
        let renderer = Renderer::new(&self.context, self.atlas.clone(), &native, native.device_size(), false)?;
        self.windows.insert(id, Window { native, screen: Err(why.to_owned()), renderer, scene: Default::default(), dirty: true, drawn: false, on_event: Box::new(on_event), tag: options.tag });
        Ok(id)
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

    /// Closes window `id`.
    pub fn close(&mut self, id: WindowId) {
        if let Some(mut window) = self.windows.remove(&id) {
            window.renderer.destroy();
            self.display.close(&window.native);
            (window.on_event)(Event::Closed);
        }
    }

    /// How long until any window's screen has something to do by itself.
    fn due(&self) -> Option<std::time::Duration> {
        self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due()).min()
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
        let mismatched: Vec<WindowId> =
            self.windows.iter().filter(|(_, window)| window.screen.as_ref().is_ok_and(|screen| screen.mismatched)).map(|(id, _)| *id).collect();
        for id in mismatched {
            if let Some(mut window) = self.windows.remove(&id) {
                window.renderer.destroy();
                self.display.close(&window.native);
                (window.on_event)(Event::Mismatched);
            }
        }
        for window in self.windows.values_mut() {
            if !window.dirty {
                continue;
            }
            window.dirty = false;
            let (size, scale) = (window.native.size(), window.native.scale());
            window.renderer.update_drawable_size(window.native.device_size());
            match &mut window.screen {
                Ok(screen) => {
                    screen.paint(size, scale, &mut self.text, &self.atlas, &mut window.scene);
                    window.dirty = screen.take_again();
                    self.display.set_ime(&window.native, screen.caret_rect());
                }
                Err(why) => crate::view::paint_message(why, appearance(), size, scale, &mut self.text, &self.atlas, &mut window.scene),
            }
            window.renderer.draw(&window.scene);
            if !window.drawn {
                window.drawn = true;
                (window.on_event)(Event::Drawn);
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
        if let Input::Resized(size) = input {
            return window.native.resized(size);
        }
        let Ok(screen) = &mut window.screen else { return };
        match input {
            Input::Resized(_) => {}
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
                // Ctrl-W and Ctrl-Q close the window, as a Linux app's do.
                if key.ctrl && !key.alt && matches!(key.key.as_str(), "w" | "q") {
                    return self.close(id);
                }
                screen.key(&key);
            }
            Input::Ime(ime) => screen.ime(ime),
            Input::Focus(active) => screen.activate(active),
            Input::Close => {}
        }
    }
}

/// What the display says happened to a window.
pub(crate) enum Input {
    Resized(Size<crate::geometry::DevicePixels>),
    Expose,
    /// The system went light or dark.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
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
    Key(crate::inspect::Key),
    /// What the input method composes or commits.
    Ime(crate::editing::Ime),
    /// The window got the keyboard, or lost it.
    Focus(bool),
}
