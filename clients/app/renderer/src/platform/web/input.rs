//! What the DOM says happened on the canvas: its pointer, wheel, keys and fingers, its field, its screen reader.

use wasm_bindgen::prelude::*;

use super::page::data;
use super::{EVENTS, HOST, SCALE, WINDOW, soon};
use crate::Appearance;
use crate::geometry::{DevicePixels, Size, point};
use crate::platform::{Input, Native};
use crate::view::touch::{Fingers, Phase};

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
