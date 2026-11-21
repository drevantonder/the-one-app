//! What the frame's JS calls to open the page and to ask of it: `renderer.js`, as it opens the page, and a harness, as
//! it waits for it to settle.

use std::cell::RefCell;

use serde_json::Value;
use wasm_bindgen::prelude::*;

use super::display::{Display, observe_pressure};
use super::draw::fetch;
use super::page::data;
use super::thread::{Thread, channel, wake};
use super::{HOST, Host, SCALE, WINDOW, WINDOWS, call, call_with, soon};
use crate::geometry::Size;
use crate::js::Clock;
use crate::platform::{Event, Native, WindowOptions, Windows};
use crate::{Appearance, Screen};

thread_local! {
    /// What each `settle` waiting resolves, by its number, and the last number given.
    static SETTLING: RefCell<(u32, std::collections::HashMap<u32, js_sys::Function>)> = RefCell::new((0, std::collections::HashMap::new()));
    /// Each `settle` answered, with what resolves it, and the frame it waits for the GPU to finish (`shown`): the
    /// frames its window had submitted at the end of the turn it was answered in.
    static SHOWING: RefCell<Vec<(js_sys::Function, JsValue, Option<u64>)>> = const { RefCell::new(Vec::new()) };
    /// The page's place in itself it was opened at, a link's fragment, which a native host tells it in its host
    /// context (`link`).
    static LINK: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Opens the page at `page` (where what it names by an address is from), at `link` in it, on `canvas`, `scale` device pixels to a CSS
/// pixel, drawn with the design tokens and styles in `tables` (the UI library's tokens
/// and styles, and its app's styles), in `theme` (`dark`, `light`, or as the device is), for a finger where `touch`,
/// at the text size its person chose where it's a phone's or a tablet's (`text_size`, as their app has it,
/// `Screen::text_size`), as the frame's JS (`calls`) is told what it says of itself. The page's module is imported
/// after, and says it ran (`started`).
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub async fn open(canvas: web_sys::HtmlCanvasElement, scale: f32, page: String, link: String, tables: Vec<String>, theme: Option<String>, touch: bool, text_size: Option<f32>, calls: JsValue) -> Result<(), JsError> {
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
    screen.fetch = Some(Box::new(move |src: &str| fetch(&page, src, None)));
    // Another app's page, in a frame of its own of the frame's, which its client says where it is (renderer.js).
    screen.make_frame = Some(Box::new(|node, src, subject| call_with("frame", &[JsValue::from(node as f64), src.into(), subject.map_or(JsValue::NULL, JsValue::from)])));
    screen.touch = touch;
    screen.text_size = text_size;
    screen.theme = match theme.as_deref() {
        Some("dark") => Some(Appearance::Dark),
        Some("light") => Some(Appearance::Light),
        _ => None,
    };
    let _ = Native::get();
    let options = WindowOptions { title: String::new(), app_id: String::new(), size: Size::default(), tag: Value::Null, panel: None, places: None, frame: None, root: false, icon: None, owner: None, snapshot: None, title_row: false };
    // Its first frame is told, for how long it took, and a page this can't draw is drawn as its own page instead.
    let on_event = |event| match event {
        Event::Drawn => call("drawn", &JsValue::UNDEFINED),
        Event::Mismatched(why) | Event::Failed(why) => call("unopened", &why.into()),
        _ => {}
    };
    windows.open(options, Ok(screen), on_event).map_err(|error| JsError::new(&format!("{error:#}")))?;
    WINDOWS.set(Some(windows));
    soon();
    Ok(())
}

/// The styles another app's component is drawn with (its `styles.json`), which the browser's loader doesn't load as
/// it loads the component's module, as a native host's does (`js::quickjs`'s `Modules::style`).
#[wasm_bindgen]
pub fn component(styles: String) -> Result<(), JsError> {
    crate::tokens::load_component(&styles).map_err(|error| JsError::new(&format!("{error:#}")))?;
    crate::load_styles(&styles).map_err(|error| JsError::new(&format!("{error:#}")))?;
    Ok(())
}

/// Resolves once nothing on the page, or its frames' pages, is pending (what it asked, a picture fetched or decoding,
/// what moves), with what settled says, as the inspector's `settle` answers natively (`Screen::inspect`), or at
/// `deadline` ms with what still is: what a harness waits for before it takes a picture (core/ui's scenes.js). The
/// picture is the canvas, so it resolves once the frame that drew what settled is on it too, which the GPU finished
/// (`shown`), where a native harness paints its picture afresh: on SwiftShader, two of the browser's frames after the
/// page settled still showed the frame before it, without the shapes a widget draws once it's laid out.
#[wasm_bindgen]
pub fn settle(deadline: f64) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve, _| {
        let number = SETTLING.with_borrow_mut(|(next, waiting)| {
            *next += 1;
            waiting.insert(*next, resolve);
            *next
        });
        let _ = Native::get().map(|native| {
            native.run(move |windows| {
                let request = crate::inspect::Request { id: Value::Null, op: "settle".into(), inputs: serde_json::Map::from_iter([("deadline".to_owned(), Value::from(deadline))]) };
                windows.with(WINDOW, |screen| {
                    screen.inspect(
                        request,
                        Box::new(move |answer| {
                            if let Some(resolve) = SETTLING.with_borrow_mut(|(_, waiting)| waiting.remove(&number)) {
                                let answer = js_sys::JSON::parse(&answer["result"].to_string()).unwrap_or(JsValue::NULL);
                                SHOWING.with_borrow_mut(|showing| showing.push((resolve, answer, None)));
                                soon();
                            }
                        }),
                    )
                });
            })
        });
    })
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
    channel(None, |channel| channel.started = Some(error.map_or(Ok(()), Err)));
    soon();
    wake(None);
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

/// After a turn of the windows' loop, where its window had submitted `frames` and the GPU had finished `finished` of
/// them: each `settle` answered waits for the frames submitted by the end of the turn it was answered in, and is
/// resolved once they're finished.
pub(super) fn shown(frames: u64, finished: u64) {
    let done: Vec<_> = SHOWING.with_borrow_mut(|showing| {
        for (.., frame) in showing.iter_mut() {
            frame.get_or_insert(frames);
        }
        showing.extract_if(.., |(.., frame)| frame.is_some_and(|frame| frame <= finished)).collect()
    });
    for (resolve, answer, _) in done {
        let _ = resolve.call1(&JsValue::NULL, &answer);
    }
}
