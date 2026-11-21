//! A frame's page, in a frame of its own (renderer.js): what it gives and what it hears, by its frame's node.

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use super::draw::fetch;
use super::page::data;
use super::thread::{Thread, channel, wake};
use super::{WINDOW, call_with, soon};
use crate::js::{Clock, Posted};
use crate::platform::Native;
use crate::tree::{Kind, Mutation};
use crate::Screen;

// A frame's page, in a frame of its own (renderer.js): what it gives and what it hears, by its frame's node.

/// The page frame `node` shows, `src`, has started in its own frame, where what it names is from `page`, drawn with its
/// app's `styles`: its screen is made, and drawn in the frame from now on, as a native host's is (`Screen::framed`).
#[wasm_bindgen]
pub fn framed(node: f64, src: String, page: String, styles: String) -> Result<(), JsError> {
    let node = node as crate::tree::Id;
    let page = url::Url::parse(&page).map_err(|error| JsError::new(&format!("{error}")))?;
    crate::load_styles(&styles).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let tokens = crate::tokens::load_page(&styles).map_err(|error| JsError::new(&format!("{error:#}")))?;
    let _ = Native::get().map(|native| {
        native.run(move |windows| {
            let asked: crate::view::Asked = Box::new(move |message, _| {
                call_with("frameAsk", &[JsValue::from(node as f64), js_sys::JSON::parse(&message.to_string()).unwrap_or(JsValue::NULL)]);
                None
            });
            let mut screen = Screen::new(Thread::of(Clock::real(), Some(node)), Some(asked));
            screen.page = tokens;
            screen.fetch = Some(Box::new(move |src: &str| fetch(&page, src, Some(node))));
            windows.with(WINDOW, |parent| parent.framed(node, &src, screen));
        })
    });
    Ok(())
}

/// The page in frame `node` ran its module, or why it didn't.
#[wasm_bindgen]
pub fn frame_started(node: f64, error: Option<String>) {
    let frame = Some(node as crate::tree::Id);
    channel(frame, |channel| channel.started = Some(error.map_or(Ok(()), Err)));
    soon();
    wake(frame);
}

/// What the page in frame `node` gave its host since it last said, each as its call was (`["create", id, kind,
/// value]`, `["set", …]`, `["insert", …]`, `["remove", …]`, `["drop", id]`, `["post", json]`, `["threw", why]`), how
/// many of the calls it was sent it's run, and whether it asks for a frame.
#[wasm_bindgen]
pub fn frame_posted(node: f64, calls: js_sys::Array, ran: f64, framing: bool) {
    let frame = Some(node as crate::tree::Id);
    let number = |value: JsValue| value.as_f64().unwrap_or_default() as u64;
    let posted: Vec<Posted> = calls
        .iter()
        .filter_map(|call| {
            let call: js_sys::Array = call.dyn_into().ok()?;
            Some(match call.get(0).as_string()?.as_str() {
                "create" => Posted::Mutation(Mutation::Create { id: number(call.get(1)), kind: Kind::deserialize(call.get(2).as_string()?.as_str().into_deserializer()).map_err(|_: serde::de::value::Error| ()).ok()?, value: call.get(3).as_string() }),
                "set" => Posted::Mutation(Mutation::Set { id: number(call.get(1)), name: call.get(2).as_string()?, value: data(&call.get(3), &mut Vec::new()).ok()? }),
                "insert" => Posted::Mutation(Mutation::Insert { parent: number(call.get(1)), id: number(call.get(2)), before: call.get(3).as_f64().map(|before| before as u64) }),
                "remove" => Posted::Mutation(Mutation::Remove { parent: number(call.get(1)), id: number(call.get(2)) }),
                "drop" => Posted::Mutation(Mutation::Drop { id: number(call.get(1)) }),
                "post" => Posted::Message(call.get(1).as_string()?),
                _ => Posted::Threw(call.get(1).as_string().unwrap_or_default()),
            })
        })
        .collect();
    channel(frame, |channel| {
        channel.posted.extend(posted);
        channel.ran = ran as u64;
        channel.framing = framing;
    });
    wake(frame);
    soon();
}

/// A message from its client for the page in frame `node`, which it was sent there: its screen hears it as its page's.
#[wasm_bindgen]
pub fn frame_message(node: f64, message: JsValue) {
    let Ok(message) = data(&message, &mut Vec::new()) else { return };
    let _ = Native::get().map(|native| {
        native.run(move |windows| {
            windows.with(WINDOW, |screen| screen.frame_mut(node as crate::tree::Id).map(|screen| screen.message(&message)));
        })
    });
}
