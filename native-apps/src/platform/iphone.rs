//! What's the iPhone's own, beside the window, in Swift (iphone/Sources/OneNative, a Tauri plugin of this app's own):
//! signing in through the system's sign-in sheet, which shares Safari's passkeys; notifications (APNs), and what a tapped one
//! opens or answers; the icon's count; haptics, the document camera, the share sheet and Quick Look; touch and hold
//! lifting what's held out of the page with its menu, as the Home Screen does; and the App Group, where the widgets
//! read what the app keeps for them. Rust asks Swift (`ask`); Swift tells Rust what happened (`one_ios`).

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use serde_json::{Value, json};
use tauri::Manager;
use tauri::plugin::{Builder, TauriPlugin};
use tauri_plugin_opener::OpenerExt;

pub use super::mobile::*;

tauri::ios_plugin_binding!(init_plugin_one);

/// What a call says it came from (as the iPhone app's did), and the platform as users say it.
pub const CLIENT: &str = "phone";
pub const LABEL: &str = "iPhone";

/// The Swift plugin, and the system's file dialogs and opener.
pub fn plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let plugin: TauriPlugin<tauri::Wry> = Builder::new("one")
        .setup(|_, api| {
            super::mobile::registered(api.register_ios_plugin(init_plugin_one)?);
            Ok(())
        })
        .build();
    builder.plugin(plugin).plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init())
}

/// Every command comes from the window's own page: WKWebView gives a screen's frame none of the app's.
pub fn ours(_: &tauri::ipc::Invoke<tauri::Wry>) -> bool {
    true
}

/// An id of its own in its Keychain, which outlives signing out (as the iPhone app's was), hashed.
pub fn this_device() -> Option<String> {
    use sha2::{Digest, Sha256};
    crate::secrets::this_device().ok().map(|id| data_encoding::HEXLOWER.encode(&Sha256::digest(id)))
}

pub fn data(app: &tauri::App) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}

/// Tauri's app local data folder, found with no window.
pub fn data_without_window() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support").join(crate::identifier()))
}

/// The App Group, which the widgets share.
pub fn widgets_dir() -> PathBuf {
    group().unwrap_or_else(super::mobile::widgets_dir)
}

/// The home-screen widgets, drawn again.
pub fn redraw_widgets() {
    tell("reload", json!({}));
}

/// The count on the app's icon; none at 0.
pub fn badge(count: u32) {
    tell("badge", json!({ "count": count }));
}

pub fn can_scan() -> bool {
    true
}

/// Signs in in the system's sign-in sheet, which comes back to the app's own link; a test's prints the link, to the
/// loopback.
pub fn sign_in(signing: &crate::signin::SignIn, join: &str, print: bool) -> Result<(), onet::sign_in::Failed> {
    if print {
        return signing.sign_in(join, |u| Ok(println!("Sign in: {u}")));
    }
    signing.sign_in_here(join, |u| signed_in_at(u, true))
}

/// In a build with notifications (Info.plist's `Notifications`), once it's let in: asks the user the first time, and gives the push
/// Worker this iPhone's token, each start, as a token can change. Its notification extension opens each one.
pub fn let_in(device: Arc<onet::Device>) {
    if super::apple::info_true("Notifications") {
        register_notifications(device, |token| notifications::Address { via: "apns".into(), address: token, env: environment().into(), topic: crate::identifier().into(), keys: None });
    }
}

/// A screen's menu while a touch and hold has lifted what it's for out of the page: shown with it, as the Home Screen's
/// → `{ index }` (null if none was chosen); else `{ lifted: false }`, and the page shows its own.
pub async fn menu(items: Value) -> Value {
    ask_off("menu", json!({ "items": items })).await.unwrap_or_else(|_| json!({ "lifted": false }))
}

pub async fn notify(ask: bool) -> Option<String> {
    Some(ask_off("notify", json!({ "ask": ask })).await.ok().and_then(|v| v["state"].as_str().map(str::to_string)).unwrap_or_else(|| "default".into()))
}

pub fn open(url: String) -> Result<(), String> {
    crate::app().handle.opener().open_url(url, None::<&str>).map_err(|_| "Couldn’t open your browser.".to_string())
}

/// Shared from the app's own temporary folder (Quick Look shows it).
pub async fn save(name: String, data: Vec<u8>, preview: bool) -> Result<(), String> {
    give(std::env::temp_dir().join("screen-files"), name, data, preview).await
}

pub fn start(_: &tauri::App) {}

/// The App Group's folder (Info.plist's `AppGroup`), which the widgets and the notification extension share.
fn group() -> Option<PathBuf> {
    let group = NSString::from_str(&super::apple::info("AppGroup")?);
    let manager: Retained<AnyObject> = unsafe { msg_send![class!(NSFileManager), defaultManager] };
    let url: Option<Retained<AnyObject>> = unsafe { msg_send![&manager, containerURLForSecurityApplicationGroupIdentifier: &*group] };
    let path: Option<Retained<NSString>> = unsafe { msg_send![&url?, path] };
    Some(PathBuf::from(path?.to_string()))
}

/// What the user calls this iPhone (UIDevice's name, which is "iPhone" unless the app may know theirs).
pub fn name() -> Option<String> {
    let device: Retained<AnyObject> = unsafe { msg_send![class!(UIDevice), currentDevice] };
    let name: Option<Retained<NSString>> = unsafe { msg_send![&device, name] };
    name.map(|n| n.to_string()).filter(|n| !n.is_empty())
}

/// The window isn't zoomed by Dynamic Type: WebKit lays out a screen's frame put into a CSS-zoomed page at no size (a
/// black window), and its page zoom keeps the layout at the screen's width, so a larger text size runs off it.
pub fn text_scale() -> f64 {
    1.0
}

/// Which APNs service this build's notifications come through: what its provisioning profile says, as a build installed
/// from Xcode is signed for development; with none, the App Store's.
fn environment() -> &'static str {
    let profile = std::env::current_exe().ok().and_then(|e| std::fs::read(e.with_file_name("embedded.mobileprovision")).ok());
    let text = profile.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let value = text.split("<key>aps-environment</key>").nth(1).and_then(|t| t.split("<string>").nth(1)).and_then(|t| t.split("</string>").next());
    match value {
        Some("development") => "sandbox",
        Some(_) => "production",
        None if cfg!(debug_assertions) => "sandbox",
        None => "production",
    }
}

/// The app, once it's started: the system may start it in the background for a widget's button.
fn started() -> Option<&'static crate::App> {
    for _ in 0..100 {
        if let Some(app) = crate::APP.get() {
            return Some(app);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// What the Swift side tells Rust: one of the things the app answers with no window (`headless`, `kind` with `json`):
/// a tapped notification's link (`open`), its Approve or Not Me (`join`), a widget's button or toggle (`call`). → its
/// answer, freed with `one_ios_free`. It waits: never call it on the main thread.
///
/// # Safety
/// Both are C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_ios(kind: *const c_char, json: *const c_char) -> *mut c_char {
    let kind = unsafe { CStr::from_ptr(kind) }.to_string_lossy().into_owned();
    let asked: Value = serde_json::from_str(&unsafe { CStr::from_ptr(json) }.to_string_lossy()).unwrap_or_default();
    let out = match started() {
        None => json!({ "error": "The app hasn't started." }),
        Some(_) => tauri::async_runtime::block_on(crate::headless::run_here(&kind, &asked)),
    };
    CString::new(out.to_string()).unwrap_or_default().into_raw()
}

/// # Safety
/// `s` came from `one_ios`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_ios_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
