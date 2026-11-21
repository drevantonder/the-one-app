//! The canvas as a window's display (`Display`, `NativeWindow`), and what the browser says of the device: its
//! appearance, its CPU's pressure, its clipboard, its keys.

use std::cell::Cell;

use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use super::{EVENTS, HOST, SCALE, call, soon};
use crate::Appearance;
use crate::geometry::{DevicePixels, Pixels, Size};
use crate::platform::{Input, WindowId, WindowOptions};

thread_local! {
    /// A frame asked for (`request_frame`), and when the browser last showed one.
    static ASKED: Cell<bool> = const { Cell::new(false) };
    static FRAMED: Cell<Option<web_time::Instant>> = const { Cell::new(None) };
}

thread_local! {
    /// What the browser's `PressureObserver` last said of its CPU.
    static PRESSURE: Cell<crate::platform::Pressure> = const { Cell::new(crate::platform::Pressure::Nominal) };
}

/// Hears the browser's own pressure, its CPU's, where it has the Compute Pressure API (Chromium's): one that hasn't, or
/// whose page may not use it (a permissions policy), leaves it nominal.
pub(super) fn observe_pressure() {
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
    pub fn summon(&self, _: &NativeWindow, _: &crate::platform::Places, _: Size<Pixels>) {}
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
