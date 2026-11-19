//! The web's window: an app's page drawn into a canvas by wgpu on the browser's WebGPU, in the page's own frame, with
//! its JS the browser's own. It's every other platform's window (`Windows`, `Screen`), its display the canvas and
//! the DOM's events on it (`clients/web/renderer/renderer.js`), and its windows' thread the browser's event loop: a job
//! runs as a microtask (`Sender`), a frame as `requestAnimationFrame` calls back, and a timer as `setTimeout`.
//!
//! What the page's JS calls (the renderer's `globalThis.mutations` and `host.post`) only queues what it says, and what
//! the screen sends the page (`ui.event`, `ui.message`) is called once the windows are let go of, so neither ever runs
//! inside the other, as a native host's JS runs on a thread of its own.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use super::{Input, Job, Native, WindowId, WindowOptions, Windows};
use crate::geometry::{DevicePixels, Pixels, Size, point};
use crate::js::{Clock, Posted};
use crate::tree::{Kind, Mutation};
use crate::{Appearance, Screen};

/// Where a job is sent: the queue the next microtask runs.
#[derive(Clone)]
pub struct Sender;

impl Sender {
    pub fn send(&self, job: Job) -> Result<(), std::convert::Infallible> {
        JOBS.with(|jobs| jobs.borrow_mut().push_back(job));
        soon();
        Ok(())
    }
}

thread_local! {
    /// The windows, once the page's canvas has its GPU (`open`).
    static WINDOWS: RefCell<Option<Windows>> = const { RefCell::new(None) };
    static JOBS: RefCell<VecDeque<Job>> = const { RefCell::new(VecDeque::new()) };
    /// A microtask is on its way to run what's queued.
    static SOON: Cell<bool> = const { Cell::new(false) };
    /// What the DOM said happened to the canvas, for the windows' next turn (`Display::events`).
    static EVENTS: RefCell<Vec<(WindowId, usize, Input)>> = const { RefCell::new(Vec::new()) };
    /// A frame asked for (`request_frame`), and when the browser last showed one.
    static ASKED: Cell<bool> = const { Cell::new(false) };
    static FRAMED: Cell<Option<web_time::Instant>> = const { Cell::new(None) };
    /// The timer set for when the windows next have something to do, and when that is.
    static TIMER: Cell<Option<(i32, f64)>> = const { Cell::new(None) };
    /// What the page's JS gave its host since the screen last took it, how many of the calls the screen sent it it's
    /// run, whether its module ran (or why not), and what wakes the screen as more comes.
    static POSTED: RefCell<Vec<Posted>> = const { RefCell::new(Vec::new()) };
    static RAN: Cell<u64> = const { Cell::new(0) };
    static STARTED: RefCell<Option<Result<(), String>>> = const { RefCell::new(None) };
    static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    static WOKEN: Cell<bool> = const { Cell::new(false) };
    /// What the screen sent the page that it hasn't been called with yet: `ui.event` or `ui.message`.
    static OUTBOX: RefCell<VecDeque<(&'static str, Value)>> = const { RefCell::new(VecDeque::new()) };
    /// The page's frame, as `open` was given it: its canvas, how dark the device is and whether it asked for less
    /// motion, and what tells the frame's own JS what the screen says of itself (its cursor, its client's messages).
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

struct Host {
    canvas: web_sys::HtmlCanvasElement,
    calls: JsValue,
    dark: bool,
    reduced_motion: bool,
}

/// The one window a frame has: its page's.
const WINDOW: WindowId = 1;

/// The windows' thread is the browser's event loop, which is always there once a canvas is open.
pub fn start() -> anyhow::Result<Native> {
    Ok(Native(Sender))
}

pub fn appearance() -> Appearance {
    HOST.with_borrow(|host| if host.as_ref().is_some_and(|host| host.dark) { Appearance::Dark } else { Appearance::Light })
}

pub fn reduced_motion() -> bool {
    HOST.with_borrow(|host| host.as_ref().is_some_and(|host| host.reduced_motion))
}

/// The browser's clipboard is asked for with a promise and a person's gesture, which a screen's copy doesn't wait on:
/// its own, for now.
pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    None
}

/// Calls the frame's own JS: `calls[name](argument)`.
fn call(name: &str, argument: &JsValue) {
    HOST.with_borrow(|host| {
        let Some(host) = host else { return };
        if let Ok(function) = js_sys::Reflect::get(&host.calls, &name.into()).and_then(|function| function.dyn_into::<js_sys::Function>()) {
            let _ = function.call1(&host.calls, argument);
        }
    })
}

/// Runs what's queued in a microtask, if one isn't on its way already.
fn soon() {
    if !SOON.replace(true) {
        wasm_bindgen_futures::spawn_local(async { turn() });
    }
}

/// A turn of the windows' loop, as a native one's (`platform::start`): the jobs that came, then what the display said
/// and what's due (`Windows::turn`), then what the screen sent the page, and the timer for its next.
fn turn() {
    SOON.set(false);
    let due = WINDOWS.with_borrow_mut(|windows| {
        let windows = windows.as_mut()?;
        while let Some(job) = JOBS.with_borrow_mut(VecDeque::pop_front) {
            job(windows);
        }
        windows.turn();
        windows.due()
    });
    deliver();
    if let Some(due) = due {
        let at = js_sys::Date::now() + due.as_secs_f64() * 1000.0;
        if TIMER.get().is_none_or(|(_, set)| set > at + 1.0) {
            let window = web_sys::window().expect("a frame has a window");
            if let Some((handle, _)) = TIMER.take() {
                window.clear_timeout_with_handle(handle);
            }
            let fired = Closure::once_into_js(|| {
                TIMER.set(None);
                soon();
            });
            let delay = (due.as_secs_f64() * 1000.0).ceil() as i32;
            if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(fired.unchecked_ref(), delay) {
                TIMER.set(Some((handle, at)));
            }
        }
    }
}

/// Calls the page with what the screen sent it, in order, once its module has run, as a native host's JS thread runs
/// each call it's sent.
fn deliver() {
    if !matches!(STARTED.with_borrow(Clone::clone), Some(Ok(()))) {
        return;
    }
    let ran = RAN.get();
    while let Some((to, message)) = OUTBOX.with_borrow_mut(VecDeque::pop_front) {
        let ui = js_sys::Reflect::get(&js_sys::global(), &"ui".into()).unwrap_or(JsValue::UNDEFINED);
        if let Ok(function) = js_sys::Reflect::get(&ui, &to.into()).and_then(|function| function.dyn_into::<js_sys::Function>()) {
            let message = js_sys::JSON::parse(&message.to_string()).unwrap_or(JsValue::NULL);
            if let Err(error) = function.call1(&ui, &message) {
                POSTED.with_borrow_mut(|posted| posted.push(Posted::Threw(format!("{error:?}"))));
            }
        }
        RAN.set(RAN.get() + 1);
    }
    if RAN.get() != ran {
        wake();
    }
}

/// Wakes the screen to take what the page gave it and what it's run, once until it has (`Thread::take`).
fn wake() {
    if !WOKEN.replace(true) {
        WAKE.with_borrow(|wake| wake.as_ref().map(|wake| wake()));
    }
}

/// The page's JS, as its screen reads it: the browser's own, which `globalThis.mutations` and `host.post` queue
/// what it gives, and which is called with what it's sent between the windows' turns (`deliver`). Its timers and
/// frames are the browser's.
pub struct Thread {
    clock: Clock,
    sent: u64,
}

impl Thread {
    pub fn new(clock: Clock) -> Thread {
        Thread { clock, sent: 0 }
    }

    pub fn send(&mut self, to: &'static str, message: Value) {
        OUTBOX.with_borrow_mut(|outbox| outbox.push_back((to, message)));
        self.sent += 1;
        soon();
    }

    pub fn fire(&mut self) {}

    pub fn frame(&mut self, _: f64) {}

    pub fn framing(&self) -> bool {
        false
    }

    pub fn take(&self) -> (Vec<Posted>, u64) {
        WOKEN.set(false);
        (POSTED.with_borrow_mut(std::mem::take), RAN.get())
    }

    pub fn sent(&self) -> u64 {
        self.sent
    }

    pub fn on_post(&self, wake: impl Fn() + Send + 'static) {
        WAKE.set(Some(Box::new(wake)));
        WOKEN.set(false);
        self::wake();
    }

    pub fn started(&self) -> Option<Result<(), String>> {
        STARTED.with_borrow(Clone::clone)
    }

    pub fn busy(&self) -> bool {
        STARTED.with_borrow(Option::is_none) || RAN.get() < self.sent
    }

    /// A browser's JS can't be waited on: what it was sent runs once this turn ends.
    pub fn wait(&self) {}

    pub fn due(&self) -> Option<f64> {
        None
    }

    pub fn timers(&self) -> Vec<Value> {
        Vec::new()
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }
}

/// The canvas, as a window's display: its events as the DOM gave them, and its frames as the browser shows them.
pub struct Display;

impl Display {
    pub fn open(&self, _: WindowId, _: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let (canvas, size, scale) = HOST.with_borrow(|host| {
            let canvas = host.as_ref().map(|host| host.canvas.clone()).ok_or_else(|| anyhow::anyhow!("no canvas"))?;
            let size = Size { width: DevicePixels(canvas.width() as i32), height: DevicePixels(canvas.height() as i32) };
            anyhow::Ok((canvas, size, web_sys::window().map_or(1.0, |window| window.device_pixel_ratio() as f32)))
        })?;
        Ok(NativeWindow { canvas, device: Cell::new(size), scale })
    }

    pub fn close(&self, _: &NativeWindow) {}
    pub fn show(&self, _: &NativeWindow) {}
    pub fn activate(&self, _: &NativeWindow) {}

    pub fn set_cursor(&self, _: &NativeWindow, cursor: crate::Cursor) {
        let name = match cursor {
            crate::Cursor::Arrow => "default",
            crate::Cursor::Pointer => "pointer",
            crate::Cursor::Text => "text",
        };
        call("cursor", &name.into());
    }

    // A computer's launcher panel, which a frame has none of.
    pub fn summon(&self, _: &NativeWindow, _: &super::Places, _: Size<Pixels>) {}
    pub fn put_away(&self, _: &NativeWindow, _: bool) {}
    pub fn open_part(&self, _: &NativeWindow, _: usize) -> anyhow::Result<NativeWindow> {
        anyhow::bail!("a panel's parts aren't windows of their own on the web")
    }
    pub fn arrange(&self, _: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        draw();
    }
    pub fn set_title_row(&self, _: &NativeWindow, _: Option<crate::view::TitleRow>) {}

    /// Typing comes with the web's text input plugin, which the frame's JS will be told Flutter's calls for.
    pub fn text_input(&self, _: &NativeWindow, _: &[crate::editing::TextInput]) {}

    pub fn flush(&self) {}

    /// `requestAnimationFrame` says when each frame is shown.
    pub fn paced(&self) -> bool {
        true
    }

    pub fn request_frame(&self, wanted: bool) {
        if wanted && !ASKED.replace(true) {
            let framed = Closure::once_into_js(|| {
                ASKED.set(false);
                FRAMED.set(Some(web_time::Instant::now()));
                soon();
            });
            if let Some(window) = web_sys::window() {
                let _ = window.request_animation_frame(framed.unchecked_ref());
            }
        }
    }

    pub fn frame(&self) -> Option<web_time::Instant> {
        FRAMED.take()
    }

    pub(crate) fn events(&self) -> Vec<(WindowId, usize, Input)> {
        EVENTS.with_borrow_mut(std::mem::take)
    }
}

/// The page's canvas: its size in device pixels, as the frame last said it, and how many of them to a CSS pixel.
pub struct NativeWindow {
    canvas: web_sys::HtmlCanvasElement,
    device: Cell<Size<DevicePixels>>,
    scale: f32,
}

#[derive(Clone, Copy)]
pub struct Handle;

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle
    }

    pub fn surface(&self) -> anyhow::Result<crate::render::SurfaceTarget> {
        Ok(wgpu::SurfaceTarget::Canvas(self.canvas.clone()))
    }

    pub fn has_surface(&self) -> bool {
        true
    }

    pub fn transparent(&self) -> bool {
        false
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.device.get()
    }

    pub fn size(&self) -> Size<Pixels> {
        let device = self.device.get();
        Size { width: Pixels(device.width.0 as f32 / self.scale), height: Pixels(device.height.0 as f32 / self.scale) }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn resized(&self, size: Size<DevicePixels>) {
        self.canvas.set_width(size.width.0 as u32);
        self.canvas.set_height(size.height.0 as u32);
        self.device.set(size);
    }
}

// What the frame's JS calls: `renderer.js`, as it opens the page and as the DOM tells it what happens on the canvas.

/// Opens the page's window on `canvas`, drawn with the design tokens and styles in `tables` (the UI library's tokens
/// and styles, and its app's styles), in `theme` (`dark`, `light`, or as the device is), as the frame's JS (`calls`)
/// is told what it says of itself. The page's module is imported after, and says it ran (`started`).
#[wasm_bindgen]
pub async fn open(canvas: web_sys::HtmlCanvasElement, tables: Vec<String>, theme: Option<String>, touch: bool, calls: JsValue) -> Result<(), JsError> {
    console_log();
    let media = |query: &str| web_sys::window().and_then(|window| window.match_media(query).ok().flatten()).is_some_and(|list| list.matches());
    HOST.set(Some(Host { canvas, calls, dark: media("(prefers-color-scheme: dark)"), reduced_motion: media("(prefers-reduced-motion: reduce)") }));
    let page = load(&tables).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let context = crate::render::Context::create().await.map_err(|error| JsError::new(&format!("{error:#}")))?;
    let mut windows = Windows::made(Display, Ok(context)).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let mut screen = Screen::new(Thread::new(Clock::real()), Some(Box::new(asked)), Size { width: 0.0, height: 0.0 }, Default::default());
    screen.page = page;
    screen.touch = touch;
    screen.theme = match theme.as_deref() {
        Some("dark") => Some(Appearance::Dark),
        Some("light") => Some(Appearance::Light),
        _ => None,
    };
    let _ = Native::get();
    let options = WindowOptions { title: String::new(), app_id: String::new(), size: Size::default(), tag: Value::Null, panel: None, places: None, root: false, icon: None };
    // Its first frame is told, for how long it took, and a page this can't draw is drawn as its own page instead.
    let on_event = |event| match event {
        super::Event::Drawn => call("drawn", &JsValue::UNDEFINED),
        super::Event::Mismatched(why) | super::Event::Failed(why) => call("unopened", &why.into()),
        _ => {}
    };
    windows.open(options, Ok(screen), on_event).map_err(|error| JsError::new(&format!("{error:#}")))?;
    WINDOWS.set(Some(windows));
    soon();
    Ok(())
}

/// The runtime version this renderer draws (`crate::RUNTIME_VERSION`), which a page's entrypoint names.
#[wasm_bindgen]
pub fn runtime_version() -> u32 {
    crate::RUNTIME_VERSION
}

/// Loads the design tokens and styles, as a native host does (`clients/app/src/native.rs`): → the number of its page's
/// own tokens.
fn load(tables: &[String]) -> anyhow::Result<u32> {
    if let [tokens, styles @ ..] = tables {
        crate::tokens::load_table(tokens)?;
        for styles in styles {
            crate::load_styles(styles)?;
        }
    }
    Ok(match tables {
        [_, _, own] => crate::tokens::load_page(own)?,
        _ => 0,
    })
}

/// What the page asks its client goes to the frame's client (`calls.post`), which answers through `message`.
fn asked(message: Value, _: Appearance) -> Option<Value> {
    call("post", &js_sys::JSON::parse(&message.to_string()).unwrap_or(JsValue::NULL));
    None
}

/// Logs go to the browser's console.
fn console_log() {
    struct Console;
    impl log::Log for Console {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record) {
            web_sys::console::warn_1(&format!("{}: {}", record.level(), record.args()).into());
        }
        fn flush(&self) {}
    }
    static CONSOLE: Console = Console;
    if log::set_logger(&CONSOLE).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&info.to_string().into())));
}

/// The page's module ran, or why it didn't.
#[wasm_bindgen]
pub fn started(error: Option<String>) {
    STARTED.set(Some(error.map_or(Ok(()), Err)));
    soon();
    wake();
}

/// A message from the page's client: an answer to what it asked, or a notification. What it says of where the page is
/// (its Colour Mode, the insets its frame reaches) the screen draws with too.
#[wasm_bindgen]
pub fn message(message: JsValue) {
    let Ok(message) = data(&message, &mut Vec::new()) else { return };
    let _ = Native::get().map(|native| {
        native.run(move |windows| {
            windows.with(WINDOW, |screen| {
                let context = if message["result"]["hostContext"].is_object() { &message["result"]["hostContext"] } else { &message["params"] };
                if message["method"] == "ui/notifications/host-context-changed" || message["result"]["hostContext"].is_object() {
                    if let Some(theme) = context["theme"].as_str() {
                        screen.theme = Some(if theme == "dark" { Appearance::Dark } else { Appearance::Light });
                    }
                    if let Some(insets) = context["safeAreaInsets"].as_object() {
                        let edge = |name: &str| insets.get(name).and_then(Value::as_f64).unwrap_or_default() as f32;
                        screen.insets = crate::geometry::Edges { top: edge("top"), right: edge("right"), bottom: edge("bottom"), left: edge("left") };
                    }
                }
                screen.message(&message);
            });
        })
    });
}

/// What the DOM said happened on the canvas, for the windows' next turn.
fn input(input: Input) {
    EVENTS.with_borrow_mut(|events| events.push((WINDOW, 0, input)));
    soon();
}

/// The canvas is `width` by `height` device pixels now.
#[wasm_bindgen]
pub fn resized(width: u32, height: u32) {
    input(Input::Resized(Size { width: DevicePixels(width as i32), height: DevicePixels(height as i32) }));
}

/// The pointer moved, went down or up (`button`, as the DOM numbers them), or left, at `x`, `y` in CSS pixels.
#[wasm_bindgen]
pub fn pointer(kind: &str, x: f32, y: f32, button: u8, shift: bool) {
    let at = point(x, y);
    input(match kind {
        "down" => Input::Down(at, button, shift),
        "up" => Input::Up(at, button),
        "leave" => Input::Left,
        _ => Input::Moved(at),
    });
}

/// A wheel or trackpad scrolled by `dx`, `dy` CSS pixels.
#[wasm_bindgen]
pub fn wheel(x: f32, y: f32, dx: f32, dy: f32) {
    input(Input::Wheel(point(x, y), point(dx, dy), None));
}

/// A key, as the DOM's `KeyboardEvent.key` names it, with the modifiers held.
#[wasm_bindgen]
pub fn key(key: String, shift: bool, alt: bool, ctrl: bool, meta: bool) {
    input(Input::Key(crate::inspect::Key { key, shift, alt, ctrl, meta }));
}

/// The frame got the keyboard, or lost it.
#[wasm_bindgen]
pub fn focused(focused: bool) {
    input(Input::Focus(focused));
}

/// The device went light or dark.
#[wasm_bindgen]
pub fn appearance_changed(dark: bool) {
    HOST.with_borrow_mut(|host| host.as_mut().map(|host| host.dark = dark));
    input(Input::Appearance(if dark { Appearance::Dark } else { Appearance::Light }));
}

// What the page's JS calls, as a native host's sandbox has them (`globalThis.mutations`, `host.post`), each queued for
// the screen to apply after its turn.

fn posted(posted: Posted) {
    POSTED.with_borrow_mut(|queue| queue.push(posted));
    wake();
}

#[wasm_bindgen(js_name = create)]
pub fn create(id: f64, kind: &str, value: Option<String>) -> Result<(), JsError> {
    let kind = Kind::deserialize(kind.into_deserializer()).map_err(|error: serde::de::value::Error| JsError::new(&error.to_string()))?;
    posted(Posted::Mutation(Mutation::Create { id: id as u64, kind, value }));
    Ok(())
}

/// A value that holds itself, or is nested past what's read, is the page's error, thrown to it, as JSON.stringify throws
/// one, as a native host's sandbox throws it.
#[wasm_bindgen(js_name = set)]
pub fn set(id: f64, name: String, value: JsValue) -> Result<(), JsValue> {
    let value = data(&value, &mut Vec::new()).map_err(|why| JsValue::from(js_sys::TypeError::new(why)))?;
    posted(Posted::Mutation(Mutation::Set { id: id as u64, name, value }));
    Ok(())
}

#[wasm_bindgen(js_name = insert)]
pub fn insert(parent: f64, id: f64, before: Option<f64>) {
    posted(Posted::Mutation(Mutation::Insert { parent: parent as u64, id: id as u64, before: before.map(|before| before as u64) }));
}

#[wasm_bindgen(js_name = remove)]
pub fn remove(parent: f64, id: f64) {
    posted(Posted::Mutation(Mutation::Remove { parent: parent as u64, id: id as u64 }));
}

#[wasm_bindgen(js_name = drop)]
pub fn drop_node(id: f64) {
    posted(Posted::Mutation(Mutation::Drop { id: id as u64 }));
}

#[wasm_bindgen(js_name = unsupported)]
pub fn unsupported(what: String) {
    posted(Posted::Mutation(Mutation::Unsupported { what }));
}

/// `host.post`: a JSON-RPC message for the page's client, as JSON.
#[wasm_bindgen(js_name = post)]
pub fn post(json: String) {
    posted(Posted::Message(json));
}

/// How deep a value's arrays and objects are read at most, as each is read on the stack.
const NESTED_MOST: usize = 512;

/// A JS value as JSON's data model, read from it directly, as a native host's sandbox reads one (`js::quickjs`): a
/// function, a symbol or `undefined` is `null`, and a number that's whole is an integer. `within` is the arrays and
/// objects it's in. → why it can't be: it holds itself, or it's nested too deep.
fn data(value: &JsValue, within: &mut Vec<JsValue>) -> Result<Value, &'static str> {
    if let Some(bool) = value.as_bool() {
        return Ok(Value::Bool(bool));
    }
    if let Some(number) = value.as_f64() {
        return Ok(if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 { Value::from(number as i64) } else { serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number) });
    }
    if let Some(text) = value.as_string() {
        return Ok(Value::String(text));
    }
    if !value.is_object() || value.is_function() {
        return Ok(Value::Null);
    }
    if within.iter().any(|outer| js_sys::Object::is(outer, value)) {
        return Err("cyclic object value");
    }
    if within.len() >= NESTED_MOST {
        return Err("object nested too deeply");
    }
    within.push(value.clone());
    let read = if let Some(array) = value.dyn_ref::<js_sys::Array>() {
        array.iter().map(|item| data(&item, within)).collect::<Result<_, _>>().map(Value::Array)
    } else {
        js_sys::Object::entries(value.unchecked_ref())
            .iter()
            .filter_map(|entry| {
                let entry: js_sys::Array = entry.unchecked_into();
                let field = entry.get(1);
                (!field.is_undefined() && !field.is_function()).then(|| Ok((entry.get(0).as_string().unwrap_or_default(), data(&field, within)?)))
            })
            .collect::<Result<_, _>>()
            .map(Value::Object)
    };
    within.pop();
    read
}
