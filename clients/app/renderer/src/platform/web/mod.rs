//! The web's window: an app's page drawn into a canvas by wgpu on the browser's WebGPU, in the page's own frame, with
//! its JS the browser's own. It's every other platform's window (`Windows`, `Screen`), its display the canvas and
//! the DOM's events on it (`clients/web/renderer/renderer.js`), and its windows' thread the browser's event loop: a job
//! runs as a microtask (`Sender`), a frame as `requestAnimationFrame` calls back, and a timer as `setTimeout`.
//!
//! What the page's JS calls (the renderer's `globalThis.mutations` and `host.post`) only queues what it says, and what
//! the screen sends the page (`ui.event`, `ui.message`) is called once the windows are let go of, so neither ever runs
//! inside the other, as a native host's JS runs on a thread of its own.

mod display;
mod draw;
mod frames;
mod host;
mod input;
mod page;
mod thread;
mod video;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

pub use display::{Display, Handle, NativeWindow, appearance, clipboard, mac_keys, reduced_motion};
pub(crate) use draw::{decode, glyph, measure, path};
pub use thread::Thread;
use thread::deliver;
pub(crate) use video::video;

use super::{Input, Job, Native, WindowId, Windows};
use crate::Screen;

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
    /// The timer set for when the windows next have something to do, and when that is.
    static TIMER: Cell<Option<(i32, f64)>> = const { Cell::new(None) };
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

/// Calls the frame's own JS: `calls[name](argument)`, or with several, `calls[name]([...arguments])`.
fn call_with(name: &str, arguments: &[JsValue]) {
    call(name, &arguments.iter().collect::<js_sys::Array>());
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

/// A turn of the windows' loop, as a native one's (`platform::native::start`): the jobs that came, then what the display said
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
