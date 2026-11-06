//! What's the iPhone's own, beside the window, in Swift (iphone/Sources/OneNative, a Tauri plugin of this app's own):
//! signing in through the system's sign-in sheet, which shares Safari's passkeys; pushes (APNs), and what a tapped one
//! opens or answers; the icon's count; haptics, the document camera, the share sheet and Quick Look; touch and hold
//! lifting what's held out of the page with its menu, as the Home Screen does; and the App Group, where the widgets
//! read what the app keeps for them. Rust asks Swift (`ask`); Swift tells Rust what happened (`one_ios`).

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use serde_json::{Value, json};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};

tauri::ios_plugin_binding!(init_plugin_one);

static PLUGIN: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

pub fn plugin() -> TauriPlugin<tauri::Wry> {
    Builder::new("one")
        .setup(|_, api| {
            let _ = PLUGIN.set(api.register_ios_plugin(init_plugin_one)?);
            Ok(())
        })
        .build()
}

/// Asks the Swift side and waits for its answer, which it gives on the main thread: so never from there.
pub fn ask(method: &str, args: Value) -> Result<Value, String> {
    PLUGIN.get().ok_or("The app hasn't started yet.")?.run_mobile_plugin(method, args).map_err(|e| e.to_string())
}

pub async fn ask_off(method: &'static str, args: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || ask(method, args)).await.map_err(|e| e.to_string())?
}

/// Tells the Swift side, not waiting.
pub fn tell(method: &'static str, args: Value) {
    std::thread::spawn(move || {
        if let Err(e) = ask(method, args) {
            eprintln!("{method}: {e}");
        }
    });
}

/// The App Group's folder (Info.plist's `AppGroup`), which the widgets and the notification extension share.
pub fn group() -> Option<PathBuf> {
    let group = NSString::from_str(&crate::apple::info("AppGroup")?);
    let manager: Retained<AnyObject> = unsafe { msg_send![class!(NSFileManager), defaultManager] };
    let url: Option<Retained<AnyObject>> = unsafe { msg_send![&manager, containerURLForSecurityApplicationGroupIdentifier: &*group] };
    let path: Option<Retained<NSString>> = unsafe { msg_send![&url?, path] };
    Some(PathBuf::from(path?.to_string()))
}

/// What the person calls this iPhone (UIDevice's name, which is "iPhone" unless the app may know theirs).
pub fn name() -> Option<String> {
    let device: Retained<AnyObject> = unsafe { msg_send![class!(UIDevice), currentDevice] };
    let name: Option<Retained<NSString>> = unsafe { msg_send![&device, name] };
    name.map(|n| n.to_string()).filter(|n| !n.is_empty())
}

/// How many times usual the person has made text in Settings (Dynamic Type): body text's size as they've set it, over its
/// size at the default, for the page to zoom screens by.
pub fn text_scale() -> f64 {
    let metrics: Retained<AnyObject> = unsafe { msg_send![class!(UIFontMetrics), defaultMetrics] };
    let scaled: f64 = unsafe { msg_send![&metrics, scaledValueForValue: 17.0f64] };
    (scaled / 17.0).clamp(0.5, 3.0)
}

/// Signs in in the system's sign-in sheet (ASWebAuthenticationSession, sharing Safari's passkeys and sign-ins) at
/// `url` → where it came back to, the app's own `<cli>://signed-in/…`.
pub fn sign_in(url: String) -> Result<String, String> {
    let out = ask("signIn", json!({ "url": url, "scheme": one::config::CLI }))?;
    out["url"].as_str().map(str::to_string).ok_or_else(|| "Sign in again.".into())
}

/// Which APNs service this build's pushes come through: what its provisioning profile says, as a build installed
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

/// In a build with pushes (Info.plist's `Push`), once it's let in: asks the person the first time, and gives the push
/// Worker this iPhone's token, each start, as a token can change. Its notification extension opens each one.
pub fn push(device: std::sync::Arc<one_client::Device>) {
    if !crate::apple::info_true("Push") {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let token = match ask_off("push", json!({})).await {
            Ok(out) => out["token"].as_str().map(str::to_string),
            Err(e) => return eprintln!("push: no token: {e}"),
        };
        let Some(token) = token else { return };
        let address = one::push::Address { via: "apns".into(), address: token, env: environment().into(), topic: crate::identifier().into(), keys: None };
        if let Err(e) = device.register_push(&address).await {
            eprintln!("push: can't register: {e}");
        }
    });
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

/// What the Swift side tells Rust (`kind`, with `json`): "open", a link a tapped notification opens (`{ link }`);
/// "join", its Approve or Not Me (`{ code, approve }`); "call", a widget's button or toggle (`{ app, tool, input }`), after
/// which the widgets are drawn again. → `{ ok }` or `{ error }`, freed with `one_ios_free`. It waits: never call it on
/// the main thread.
///
/// # Safety
/// Both are C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_ios(kind: *const c_char, json: *const c_char) -> *mut c_char {
    let kind = unsafe { CStr::from_ptr(kind) }.to_string_lossy().into_owned();
    let asked: Value = serde_json::from_str(&unsafe { CStr::from_ptr(json) }.to_string_lossy()).unwrap_or_default();
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let out = match (kind.as_str(), started()) {
        (_, None) => json!({ "error": "The app hasn't started." }),
        ("open", Some(_)) => {
            crate::opened(&text(&asked["link"]));
            json!({ "ok": null })
        }
        ("join", Some(_)) => {
            let how = if asked["approve"] == true { "approve" } else { "refuse" };
            tauri::async_runtime::block_on(crate::headless::run_here("--answer-join", &[text(&asked["code"]), how.into()])).unwrap_or_else(|e| json!({ "error": e }))
        }
        ("call", Some(_)) => {
            let args = [text(&asked["app"]), text(&asked["tool"]), asked["input"].to_string()];
            let out = tauri::async_runtime::block_on(crate::headless::run_here("--call", &args)).unwrap_or_else(|e| json!({ "error": e }));
            crate::changed();
            out
        }
        _ => json!({ "error": "Unknown" }),
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
