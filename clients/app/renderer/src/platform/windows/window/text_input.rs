// Each window's text input plugin, Flutter's (`text_input.cc`), and what the screen tells it.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_void};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{HIMC, IACE_DEFAULT, ImmAssociateContextEx};

use crate::editing::{TextInput, TextInputClient, Value};
use crate::platform::Input;
use super::{Display, NativeWindow, State};

impl Display {
    /// Tells window `window`'s text input plugin what the screen says (`editing::TextInput`), its method calls decoded.
    /// Its input method is on while an input takes typing, and off while none does.
    pub fn text_input(&self, window: &NativeWindow, calls: &[TextInput]) {
        let plugin = window.state.text_input.get();
        for call in calls {
            unsafe {
                match call {
                    TextInput::SetClient(id, configuration) => {
                        let (kind, action) = (cstring(configuration["inputType"]["name"].as_str().unwrap_or_default()), cstring(configuration["inputAction"].as_str().unwrap_or_default()));
                        text_input_set_client(plugin, *id as i32, kind.as_ptr(), action.as_ptr());
                        let _ = ImmAssociateContextEx(window.hwnd, HIMC::default(), IACE_DEFAULT);
                    }
                    TextInput::ClearClient => {
                        text_input_clear_client(plugin);
                        let _ = ImmAssociateContextEx(window.hwnd, HIMC::default(), 0);
                    }
                    TextInput::SetEditingState(value) => {
                        let state = value.flutter();
                        let at = |key: &str| state[key].as_i64().unwrap_or(-1) as i32;
                        let text = cstring(&value.text);
                        let (base, extent) = (at("selectionBase"), at("selectionExtent"));
                        text_input_set_editing_state(plugin, text.as_ptr(), base, extent, at("composingBase"), at("composingExtent"));
                    }
                    TextInput::SetMarkedTextRect(rect) => {
                        let (x, y, width, height) = (rect.origin.x.into(), rect.origin.y.into(), rect.size.width.into(), rect.size.height.into());
                        text_input_set_marked_text_rect(plugin, x, y, width, height);
                    }
                    TextInput::SetEditableSizeAndTransform { transform, .. } => {
                        let transform = transform.map(f64::from);
                        text_input_set_editable_transform(plugin, transform.as_ptr());
                    }
                    TextInput::SetCaretRect(_) | TextInput::Show | TextInput::Hide => {}
                }
            }
        }
    }
}

/// A window's text input plugin (`text_input.cc`): Flutter's, as a C interface.
pub(super) struct Plugin(pub(super) Cell<*mut c_void>);

impl Plugin {
    pub(super) fn get(&self) -> *mut c_void {
        self.0.get()
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        unsafe { text_input_free(self.0.get()) };
    }
}

/// Flutter's TextInputClient calls, as the plugin makes them (`flutter/.../windows/text_input_channel.h`).
#[repr(C)]
pub(super) struct TextInputChannel {
    pub(super) context: *mut c_void,
    pub(super) update_editing_state: extern "C" fn(*mut c_void, i32, *const c_char, i32, i32, i32, i32),
    pub(super) perform_action: extern "C" fn(*mut c_void, i32, *const c_char),
}

unsafe extern "C" {
    pub(super) fn text_input_new(window: HWND, channel: TextInputChannel) -> *mut c_void;
    pub(super) fn text_input_free(plugin: *mut c_void);
    pub(super) fn text_input_has_client(plugin: *mut c_void) -> bool;
    pub(super) fn text_input_set_client(plugin: *mut c_void, client: i32, input_type: *const c_char, input_action: *const c_char);
    pub(super) fn text_input_clear_client(plugin: *mut c_void);
    pub(super) fn text_input_set_editing_state(plugin: *mut c_void, text: *const c_char, base: i32, extent: i32, composing_base: i32, composing_extent: i32);
    pub(super) fn text_input_set_marked_text_rect(plugin: *mut c_void, x: f64, y: f64, width: f64, height: f64);
    pub(super) fn text_input_set_editable_transform(plugin: *mut c_void, transform: *const f64);
    pub(super) fn text_input_text(plugin: *mut c_void, text: *const u16, length: usize);
    pub(super) fn text_input_ime(plugin: *mut c_void, message: u32, wparam: WPARAM, lparam: *mut LPARAM) -> bool;
}

/// The plugin says input `client`'s whole editing state, for the window's screen.
pub(super) extern "C" fn update_editing_state(state: *mut c_void, client: i32, text: *const c_char, base: i32, extent: i32, composing_base: i32, composing_extent: i32) {
    // SAFETY: the window's, which outlives its plugin (`State`).
    let state = unsafe { &*state.cast::<State>() };
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let value = Value::from_flutter(&serde_json::json!({
        "text": text, "selectionBase": base, "selectionExtent": extent, "composingBase": composing_base, "composingExtent": composing_extent,
    }));
    state.queue.borrow_mut().push((state.id, state.part, Input::TextInputClient(TextInputClient::UpdateEditingState(client as u64, value))));
}

/// The plugin says input `client`'s action key was pressed.
pub(super) extern "C" fn perform_action(state: *mut c_void, client: i32, action: *const c_char) {
    let state = unsafe { &*state.cast::<State>() };
    let action = unsafe { CStr::from_ptr(action) }.to_string_lossy().into_owned();
    state.queue.borrow_mut().push((state.id, state.part, Input::TextInputClient(TextInputClient::PerformAction(client as u64, action))));
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).expect("no NUL in it")
}
