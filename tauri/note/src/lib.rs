//! `one_open_note(secret, pushed)`: a note pushed to this device (its `n`, base64url), opened with the device's key (32
//! bytes) → JSON `{ title, body, app, screen, by }`, or `{ error }`; free it with `one_free`.

use std::ffi::{CStr, CString, c_char};

/// # Safety
/// `secret` points at 32 bytes; `pushed` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_open_note(secret: *const u8, pushed: *const c_char) -> *mut c_char {
    let secret: [u8; 32] = unsafe { std::slice::from_raw_parts(secret, 32) }.try_into().unwrap();
    let pushed = unsafe { CStr::from_ptr(pushed) }.to_string_lossy();
    let out = match one_client::open_note(secret, &pushed) {
        Ok((n, by)) => serde_json::json!({ "title": n.title, "body": n.body, "app": n.app, "screen": n.screen, "by": by }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    CString::new(out.to_string()).unwrap().into_raw()
}

/// # Safety
/// `s` came from `one_open_note`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
