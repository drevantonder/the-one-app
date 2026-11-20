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
use crate::view::touch::{Fingers, Phase};
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
    /// A microtask is on its way to run what's queued, and a turn is running now.
    static SOON: Cell<bool> = const { Cell::new(false) };
    static TURNING: Cell<bool> = const { Cell::new(false) };
    /// The semantic tree the frame's ARIA tree was last given (`semantics`), and what's selected of a text that isn't a
    /// field, as the frame was last told (`selected`).
    static SPOKEN: RefCell<String> = const { RefCell::new(String::new()) };
    static SELECTED: RefCell<Option<String>> = const { RefCell::new(None) };
    /// What the DOM said happened to the canvas, for the windows' next turn (`Display::events`).
    static EVENTS: RefCell<Vec<(WindowId, usize, Input)>> = const { RefCell::new(Vec::new()) };
    /// How many device pixels the canvas has to a CSS pixel, as the frame last said it (`resized`): the display's, and
    /// the page's zoom.
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
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
    /// The page's place in itself it was opened at, a link's fragment, which a native host tells it in its host
    /// context (`link`).
    static LINK: RefCell<String> = const { RefCell::new(String::new()) };
    /// Each video's latest frame, as the browser played it, and where it was last drawn (`video`).
    static VIDEOS: RefCell<std::collections::HashMap<crate::tree::Id, Video>> = RefCell::new(std::collections::HashMap::new());
}

struct Host {
    canvas: web_sys::HtmlCanvasElement,
    calls: JsValue,
    dark: bool,
    reduced_motion: bool,
}

thread_local! {
    /// What the browser's `PressureObserver` last said of its CPU.
    static PRESSURE: Cell<crate::platform::Pressure> = const { Cell::new(crate::platform::Pressure::Nominal) };
}

/// Hears the browser's own pressure, its CPU's, where it has the Compute Pressure API (Chromium's): one that hasn't, or
/// whose page may not use it (a permissions policy), leaves it nominal.
fn observe_pressure() {
    use crate::platform::Pressure;
    let Ok(observing) = js_sys::Reflect::get(&js_sys::global(), &"PressureObserver".into()).and_then(|class| class.dyn_into::<js_sys::Function>()) else { return };
    let heard = Closure::<dyn FnMut(js_sys::Array)>::new(|records: js_sys::Array| {
        let last = records.at(-1);
        let state = js_sys::Reflect::get(&last, &"state".into()).ok().and_then(|state| state.as_string());
        PRESSURE.set(match state.as_deref() {
            Some("critical") => Pressure::Critical,
            Some("serious") => Pressure::Serious,
            Some("fair") => Pressure::Fair,
            _ => Pressure::Nominal,
        });
    });
    let Ok(observer) = js_sys::Reflect::construct(&observing, &js_sys::Array::of1(heard.as_ref())) else { return };
    heard.forget();
    let observe = js_sys::Reflect::get(&observer, &"observe".into()).and_then(|observe| observe.dyn_into::<js_sys::Function>());
    if let Ok(promise) = observe.and_then(|observe| observe.call1(&observer, &"cpu".into())).and_then(|promise| promise.dyn_into::<js_sys::Promise>()) {
        let refused = Closure::<dyn FnMut(JsValue)>::new(|_| {});
        let _ = promise.catch(&refused);
        refused.forget();
    }
}

/// The one window a frame has: its page's.
const WINDOW: WindowId = 1;

/// A new GPU device, where the one drawn with was lost (`Windows::recover`): → it, once the browser has given it, having
/// been asked for the first time this is asked.
pub fn recover() -> Option<anyhow::Result<crate::render::context::Context>> {
    thread_local! {
        static ASKING: Cell<bool> = const { Cell::new(false) };
        static CAME: RefCell<Option<anyhow::Result<crate::render::context::Context>>> = const { RefCell::new(None) };
    }
    if let Some(made) = CAME.take() {
        return Some(made);
    }
    if !ASKING.replace(true) {
        wasm_bindgen_futures::spawn_local(async {
            CAME.set(Some(crate::render::context::Context::create().await));
            ASKING.set(false);
            soon();
        });
    }
    None
}

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

/// The browser's clipboard, which what's copied is written to (`navigator.clipboard`, in the key's own gesture). What's
/// pasted comes through the field typed in (text-input.js), as the browser reads its clipboard only with a promise.
struct Clipboard;

impl crate::editing::Clipboard for Clipboard {
    fn read(&mut self, _: bool) -> Option<String> {
        None
    }

    fn write(&mut self, primary: bool, text: &str) {
        if !primary && let Some(window) = web_sys::window() {
            let _ = window.navigator().clipboard().write_text(text);
        }
    }
}

pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    Some(Box::new(Clipboard))
}

/// Whether the browser is a Mac's, an iPhone's or an iPad's, whose keys are a Mac's (`platform::mac_keys`).
pub fn mac_keys() -> bool {
    thread_local! {
        static MAC: bool = web_sys::window().and_then(|window| window.navigator().platform().ok()).is_some_and(|platform| ["Mac", "iPhone", "iPad"].iter().any(|name| platform.contains(name)));
    }
    MAC.with(|mac| *mac)
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
    // What a turn calls in the frame (a field's focus) may tell it something at once: that's the next turn's.
    if TURNING.replace(true) {
        return soon();
    }
    let (due, semantics, selected) = WINDOWS.with_borrow_mut(|windows| {
        let Some(windows) = windows.as_mut() else { return (None, None, None) };
        while let Some(job) = JOBS.with_borrow_mut(VecDeque::pop_front) {
            job(windows);
        }
        windows.turn();
        let screen = windows.windows.get(&WINDOW).and_then(|window| window.screen.as_ref().ok());
        let (semantics, selected) = (screen.map(semantics), screen.and_then(Screen::selection_text));
        (windows.due(), semantics, selected)
    });
    TURNING.set(false);
    // The ARIA tree follows the semantic tree, where it changed.
    if let Some(semantics) = semantics.filter(|semantics| SPOKEN.with_borrow(|spoken| spoken != semantics)) {
        call("semantics", &js_sys::JSON::parse(&semantics).unwrap_or(JsValue::NULL));
        SPOKEN.set(semantics);
    }
    // What's selected of a text, mirrored in the page as it changes, for the browser's own Copy.
    if SELECTED.with_borrow(|was| *was != selected) {
        call("selected", &selected.as_deref().map_or(JsValue::NULL, JsValue::from));
        SELECTED.set(selected);
    }
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

/// The screen's semantic tree (`Screen::semantics`), each node with where it was drawn (`rect`), as JSON.
fn semantics(screen: &Screen) -> String {
    fn placed(screen: &Screen, node: &mut Value) {
        if let Some(rect) = node["id"].as_u64().and_then(|id| screen.rect(id)) {
            node["rect"] = serde_json::json!([rect.x, rect.y, rect.width, rect.height]);
        }
        for child in node["children"].as_array_mut().into_iter().flatten() {
            placed(screen, child);
        }
    }
    let mut tree = screen.semantics();
    for node in tree.as_array_mut().into_iter().flatten() {
        placed(screen, node);
    }
    tree.to_string()
}

/// Runs what's queued now, not in a microtask: a press that gives a field the keyboard focuses its field in the
/// press's own event, which is when a phone's browser brings up its keyboard.
#[wasm_bindgen]
pub fn flush() {
    turn();
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

    /// Nothing: the browser runs the page's JS, beside the renderer, and its own Performance panel times it.
    pub fn script(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    pub fn take(&self) -> (Vec<Posted>, u64) {
        WOKEN.set(false);
        (POSTED.with_borrow_mut(std::mem::take), RAN.get())
    }

    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Whether it's posted what its window hasn't taken yet (`take`).
    pub fn has_posted(&self) -> bool {
        POSTED.with_borrow(|posted| !posted.is_empty())
    }

    /// Nothing seeds a browser's page's thread.
    pub fn holding(&self) -> Option<f64> {
        None
    }

    /// What it's sent always tells it something.
    pub fn told(&self) -> u64 {
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

    /// Never: a page the browser runs that crashes is its browser's to say (Chrome's "Aw, Snap!").
    pub fn crashed(&self) -> Option<String> {
        None
    }

    pub fn busy(&self) -> bool {
        STARTED.with_borrow(Option::is_none) || RAN.get() < self.sent
    }

    /// A browser's JS never ends.
    pub fn ended(&self) -> bool {
        false
    }

    /// A browser's JS can't be waited on: what it was sent runs once this turn ends.
    pub fn wait(&self) {}

    /// A browser collects its JS itself, as it sees fit.
    pub fn collect(&mut self) {}

    /// None that it says: what a browser's JS holds is its own (`performance.memory` is Chrome's alone).
    pub fn memory(&self) -> usize {
        0
    }

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
    /// A page's canvas is its one window.
    pub fn attach(&self, _sheet: &NativeWindow, _owner: &NativeWindow) {}

    pub fn open(&self, _: WindowId, _: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let (canvas, size) = HOST.with_borrow(|host| {
            let canvas = host.as_ref().map(|host| host.canvas.clone()).ok_or_else(|| anyhow::anyhow!("no canvas"))?;
            let size = Size { width: DevicePixels(canvas.width() as i32), height: DevicePixels(canvas.height() as i32) };
            anyhow::Ok((canvas, size))
        })?;
        Ok(NativeWindow { canvas, device: Cell::new(size) })
    }

    pub fn close(&self, _: &NativeWindow) {}
    pub fn show(&self, _: &NativeWindow) {}
    pub fn activate(&self, _: &NativeWindow) {}

    /// A browser has its own menu for text.
    /// A phone's browser's menu for text and its magnifier are its own field's, which a canvas has none of.
    pub fn text_menu(&self, _: &NativeWindow, _: Option<crate::view::TextMenu>) {}
    pub fn magnifier(&self, _: &NativeWindow, _: Option<crate::view::Magnifier>) {}

    /// A page has none of the system's own parts to draw light or dark.
    pub fn set_appearance(&self, _: &NativeWindow, _: Option<crate::Appearance>) {}

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

    /// Tells the frame's text input plugin (`text-input.js`) Flutter's method calls, as each is `[method, arguments]`.
    pub fn text_input(&self, _: &NativeWindow, calls: &[crate::editing::TextInput]) {
        let calls: Vec<Value> = calls.iter().map(|call| serde_json::json!(call.call())).collect();
        call("textInput", &js_sys::JSON::parse(&Value::Array(calls).to_string()).unwrap_or(JsValue::NULL));
    }

    pub fn flush(&self) {}

    /// How pressed the device is, as the browser's own Compute Pressure API last said, where it has one
    /// (`observe_pressure`).
    pub fn pressure(&self) -> crate::platform::Pressure {
        PRESSURE.get()
    }

    /// The display's refresh interval, where it says: it doesn't here, so frame timing takes its frames' median.
    pub fn refresh(&self) -> Option<std::time::Duration> {
        None
    }

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

/// The page's canvas: its size in device pixels, as the frame last said it.
pub struct NativeWindow {
    canvas: web_sys::HtmlCanvasElement,
    device: Cell<Size<DevicePixels>>,
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
        Size { width: Pixels(device.width.0 as f32 / self.scale()), height: Pixels(device.height.0 as f32 / self.scale()) }
    }

    pub fn scale(&self) -> f32 {
        SCALE.get()
    }

    pub fn resized(&self, size: Size<DevicePixels>) {
        self.canvas.set_width(size.width.0 as u32);
        self.canvas.set_height(size.height.0 as u32);
        self.device.set(size);
    }
}

// What the browser draws for the renderer: a page's paths and its pictures.

/// A path, SVG's `d` in its `viewBox` stretched to `device` pixels, filled and stroked (`stroke_width` device pixels
/// wide however it's stretched), drawn by the browser's canvas, as its own SVG would draw it: in BGRA, straight.
pub(crate) fn path(d: &str, view_box: &str, device: Size<u32>, fill: Option<String>, stroke: Option<String>, stroke_width: f32) -> Option<crate::view::Decoded> {
    let [x, y, width, height] = <[f64; 4]>::try_from(view_box.split([' ', ',']).filter(|part| !part.is_empty()).map(|part| part.parse().unwrap_or(0.0)).collect::<Vec<f64>>()).ok()?;
    let canvas = web_sys::OffscreenCanvas::new(device.width, device.height).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let (sx, sy) = (f64::from(device.width) / width.max(f64::EPSILON), f64::from(device.height) / height.max(f64::EPSILON));
    let shape = web_sys::Path2d::new_with_path_string(d).ok()?;
    // In device pixels, so its stroke is as wide however it's stretched (`vector-effect: non-scaling-stroke`).
    let placed = web_sys::Path2d::new().ok()?;
    let matrix = web_sys::DomMatrix2dInit::new();
    (matrix.set_a(sx), matrix.set_d(sy), matrix.set_e(-x * sx), matrix.set_f(-y * sy));
    // `addPath` takes any DOMMatrix2DInit, where web-sys binds only an SVGMatrix.
    let add: js_sys::Function = js_sys::Reflect::get(&placed, &"addPath".into()).ok()?.dyn_into().ok()?;
    add.call2(&placed, &shape, &matrix).ok()?;
    if let Some(fill) = fill {
        context.set_fill_style_str(&fill);
        context.fill_with_path_2d(&placed);
    }
    if let Some(stroke) = stroke {
        context.set_stroke_style_str(&stroke);
        context.set_line_width(f64::from(stroke_width));
        context.set_line_cap("round");
        context.set_line_join("round");
        context.stroke_with_path(&placed);
    }
    let pixels = context.get_image_data(0.0, 0.0, f64::from(device.width), f64::from(device.height)).ok()?.data().0;
    Some(bgra(device, pixels))
}

/// The face the browser draws what no face of the renderer's has in: its UI's, which falls back to its emoji's.
const BROWSER_FONT: &str = "system-ui, sans-serif";

/// A 2D canvas of `width` by `height`, its text in the browser's face at `size` pixels.
fn canvas(width: u32, height: u32, size: f32) -> Option<web_sys::OffscreenCanvasRenderingContext2d> {
    let canvas = web_sys::OffscreenCanvas::new(width.max(1), height.max(1)).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.set_font(&format!("{size}px {BROWSER_FONT}"));
    Some(context)
}

/// How wide `text` is in the browser's face, in ems, and whether the browser draws it in colour (an emoji's) or in
/// one, as a symbol, which is then drawn in its text's colour (`text::browser`).
pub(crate) fn measure(text: &str) -> (f32, bool) {
    const SIZE: f32 = 100.0;
    let Some(context) = canvas(1, 1, SIZE) else { return (0.0, false) };
    let advance = context.measure_text(text).map_or(0.0, |metrics| metrics.width() as f32 / SIZE);
    // Drawn in black and in red: one that's the same both times has colours of its own, as an emoji (a grey one too).
    let colored = match (glyph_in(text, 32.0, "#000"), glyph_in(text, 32.0, "#f00")) {
        (Some(black), Some(red)) => !black.4.is_empty() && black.4 == red.4,
        _ => false,
    };
    (advance, colored)
}

/// `text` as the browser's canvas draws it at `size` pixels, in black where it has no colour of its own: where it is
/// from its pen (left, and top above the baseline), its size, and its pixels, RGBA, straight.
pub(crate) fn glyph(text: &str, size: f32) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    glyph_in(text, size, "#000")
}

/// `text` drawn as `glyph` says, in `colour` where it has none of its own.
fn glyph_in(text: &str, size: f32, colour: &str) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    let metrics = canvas(1, 1, size)?.measure_text(text).ok()?;
    let left = (-metrics.actual_bounding_box_left()).floor() as i32;
    let right = metrics.actual_bounding_box_right().ceil() as i32;
    let (ascent, descent) = (metrics.actual_bounding_box_ascent().ceil() as i32, metrics.actual_bounding_box_descent().ceil() as i32);
    let (width, height) = ((right - left).max(0) as u32, (ascent + descent).max(0) as u32);
    if width == 0 || height == 0 {
        return Some((0, 0, 0, 0, Vec::new()));
    }
    let context = canvas(width, height, size)?;
    context.set_fill_style_str(colour);
    context.fill_text(text, f64::from(-left), f64::from(ascent)).ok()?;
    let pixels = context.get_image_data(0.0, 0.0, f64::from(width), f64::from(height)).ok()?.data().0;
    Some((left, ascent, width, height, pixels))
}

/// The browser's straight RGBA as the atlas's BGRA.
fn bgra(size: Size<u32>, mut pixels: Vec<u8>) -> crate::view::Decoded {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    crate::view::Decoded { picture: crate::view::Picture { width: size.width, height: size.height }, bgra: pixels }
}

/// A `data:` address's picture, decoded by the browser for a box of `device` pixels, as a native host's are
/// (`view::paint::decode`): an SVG drawn at that size, and any other at the smallest of its size halved again and again
/// that still covers what of the box it fills.
pub(crate) async fn decode(src: &str, device: Size<u32>) -> Option<crate::view::Decoded> {
    let image = web_sys::HtmlImageElement::new().ok()?;
    image.set_src(src);
    wasm_bindgen_futures::JsFuture::from(image.decode()).await.ok()?;
    let svg = src.starts_with("data:image/svg+xml");
    let size = if svg {
        device
    } else {
        let (mut width, mut height) = (image.natural_width().max(1), image.natural_height().max(1));
        let fit = (device.width as f32 / width as f32).min(device.height as f32 / height as f32);
        let shown = (width as f32 * fit, height as f32 * fit);
        while (width / 2) as f32 >= shown.0 && (height / 2) as f32 >= shown.1 && width > 1 && height > 1 {
            (width, height) = (width / 2, height / 2);
        }
        Size { width, height }
    };
    let canvas = web_sys::OffscreenCanvas::new(size.width, size.height).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    if svg {
        // Fitted in its box, keeping its shape, as resvg draws one natively.
        let (natural_width, natural_height) = (f64::from(image.natural_width().max(1)), f64::from(image.natural_height().max(1)));
        let fit = (width / natural_width).min(height / natural_height);
        context.draw_image_with_html_image_element_and_dw_and_dh(&image, 0.0, 0.0, natural_width * fit, natural_height * fit).ok()?;
    } else {
        context.draw_image_with_html_image_element_and_dw_and_dh(&image, 0.0, 0.0, width, height).ok()?;
    }
    Some(bgra(size, context.get_image_data(0.0, 0.0, width, height).ok()?.data().0))
}

/// A picture a page names by an address, fetched as one of its app's files by the browser, which the screen then has
/// as a `data:` address (`Screen::fetched`).
fn fetch(page: &url::Url, src: &str) {
    let (src, address) = (src.to_owned(), page.join(src).map(String::from).unwrap_or_default());
    wasm_bindgen_futures::spawn_local(async move {
        let fetched = async {
            let window = web_sys::window()?;
            let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&address)).await.ok()?.dyn_into().ok()?;
            if !response.ok() {
                return None;
            }
            let kind = response.headers().get("content-type").ok().flatten().unwrap_or_default();
            let bytes = js_sys::Uint8Array::new(&wasm_bindgen_futures::JsFuture::from(response.array_buffer().ok()?).await.ok()?).to_vec();
            Some(format!("data:{kind};base64,{}", data_encoding::BASE64.encode(&bytes)))
        };
        let data = fetched.await;
        let _ = Native::get().map(|native| native.run(move |windows| {
            windows.with(WINDOW, |screen| screen.fetched(&src, data));
        }));
    });
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

// What the frame's JS calls: `renderer.js`, as it opens the page and as the DOM tells it what happens on the canvas.

/// Opens the page at `page` (where what it names by an address is from), at `link` in it, on `canvas`, `scale` device pixels to a CSS
/// pixel, drawn with the design tokens and styles in `tables` (the UI library's tokens
/// and styles, and its app's styles), in `theme` (`dark`, `light`, or as the device is), as the frame's JS (`calls`)
/// is told what it says of itself. The page's module is imported after, and says it ran (`started`).
#[wasm_bindgen]
pub async fn open(canvas: web_sys::HtmlCanvasElement, scale: f32, page: String, link: String, tables: Vec<String>, theme: Option<String>, touch: bool, calls: JsValue) -> Result<(), JsError> {
    console_log();
    SCALE.set(scale);
    LINK.set(link);
    let media = |query: &str| web_sys::window().and_then(|window| window.match_media(query).ok().flatten()).is_some_and(|list| list.matches());
    HOST.set(Some(Host { canvas, calls, dark: media("(prefers-color-scheme: dark)"), reduced_motion: media("(prefers-reduced-motion: reduce)") }));
    observe_pressure();
    let tokens = load(&tables).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let context = crate::render::context::Context::create().await.map_err(|error| JsError::new(&format!("{error:#}")))?;
    let mut windows = Windows::made(Display, Ok(context)).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let mut screen = Screen::new(Thread::new(Clock::real()), Some(Box::new(asked)));
    screen.page = tokens;
    let page = url::Url::parse(&page).map_err(|error| JsError::new(&format!("{error}")))?;
    screen.fetch = Some(Box::new(move |src: &str| fetch(&page, src)));
    screen.touch = touch;
    screen.theme = match theme.as_deref() {
        Some("dark") => Some(Appearance::Dark),
        Some("light") => Some(Appearance::Light),
        _ => None,
    };
    let _ = Native::get();
    let options = WindowOptions { title: String::new(), app_id: String::new(), size: Size::default(), tag: Value::Null, panel: None, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None, title_row: false };
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
    let Ok(mut message) = data(&message, &mut Vec::new()) else { return };
    // Where in itself it was opened, which the web client says in the frame's address and a native host in this.
    if message["result"]["hostContext"].is_object() && message["result"]["hostContext"].get("link").is_none() {
        message["result"]["hostContext"]["link"] = Value::String(LINK.with_borrow(Clone::clone));
    }
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

/// The canvas is `width` by `height` device pixels now, `scale` of them to a CSS pixel.
#[wasm_bindgen]
pub fn resized(width: u32, height: u32, scale: f32) {
    SCALE.set(scale);
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

/// Where the browser predicts the pointer or finger down is going (`getPredictedEvents`), at `x`, `y` in CSS pixels, after
/// its last move.
#[wasm_bindgen]
pub fn predicted(x: f32, y: f32) {
    input(Input::Predicted(point(x, y)));
}

/// A wheel or trackpad scrolled by `dx`, `dy` CSS pixels, and by `nx`, `ny` notches where the browser says a wheel
/// turned by clicks (its `deltaMode` in lines, as Firefox's is), or 0.
#[wasm_bindgen]
pub fn wheel(x: f32, y: f32, dx: f32, dy: f32, nx: f32, ny: f32) {
    input(Input::Wheel(point(x, y), point(dx, dy), None, (nx != 0.0 || ny != 0.0).then(|| point(nx, ny))));
}

/// A key, as the DOM's `KeyboardEvent.key` names it, with the modifiers held.
#[wasm_bindgen]
pub fn key(key: String, shift: bool, alt: bool, ctrl: bool, meta: bool) {
    input(Input::Key(crate::inspect::Key { key, shift, alt, ctrl, meta }));
}

/// What the frame's text input plugin says, as Flutter's engine calls its framework's `TextInputClient`.
#[wasm_bindgen]
pub fn text_input_client(method: &str, arguments: JsValue) {
    if let Some(said) = data(&arguments, &mut Vec::new()).ok().and_then(|arguments| crate::editing::TextInputClient::from_call(method, &arguments)) {
        input(Input::TextInputClient(said));
    }
}

/// A screen reader pressed node `id`, gave it the keyboard, or moved to it, which shows it, as VoiceOver and
/// TalkBack do on a phone (`platform::ios::a11y`).
#[wasm_bindgen]
pub fn act(id: f64, action: &str) {
    let action = action.to_owned();
    let _ = Native::get().map(|native| {
        native.run(move |windows| {
            windows.with(WINDOW, |screen| match action.as_str() {
                "press" => drop(screen.press(id as u64)),
                "focus" => screen.set_focused(Some(id as u64)),
                _ => drop(screen.reveal(id as u64)),
            });
        })
    });
}

/// A finger at `x`, `y` (CSS pixels on the canvas), as a phone's own app has it (`view::touch`): `kind` is the touch
/// event's, `down`, `move`, `up` or `cancel`. One on an input gives it the keyboard as it lifts, in the lift's own event
/// (`flush`), which is when a phone brings up its keyboard. The browser's own back swipe is its own.
#[wasm_bindgen]
pub fn touch(kind: &str, x: f32, y: f32, finger: i32) {
    let phase = match kind {
        "down" => Phase::Down,
        "move" => Phase::Moved,
        "up" => Phase::Up,
        _ => Phase::Cancelled,
    };
    let _ = Native::get().map(|native| {
        native.run(move |windows| {
            windows.touch(WINDOW, finger as u64, point(x, y), phase, None, Fingers::PHONE);
        })
    });
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
