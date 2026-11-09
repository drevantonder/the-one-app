//! What's Android's own, beside the window, in Kotlin (android/app/src/main/kotlin/app/client): signing in in a Chrome
//! Custom Tab, the Keystore this device's keys are sealed by, pushes (FCM) and what a tapped
//! one opens or answers, the document scanner, the share sheet, haptics, the system bars, and the home-screen widgets.
//!
//! Two ways across, both JNI into this library, which the app's process loads whether or not its window is open:
//! - Kotlin calls in (`Native.call`, `Java_app_client_Native_call`): what the system starts with no window (a widget's
//!   button, the refresh WorkManager makes, a push) is answered here, by the same code the window uses (headless.rs).
//! - Rust calls out, to `Host` (what needs no window: the Keystore, the phone's name, the widgets drawn again) or to the
//!   Tauri plugin `OnePlugin` (what needs the window's activity: the Custom Tab, the scanner, the share sheet).

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use jni::objects::{GlobalRef, JClass, JString, JValue};
use jni::sys::{JNI_VERSION_1_6, jint, jstring};
use jni::{JNIEnv, JavaVM};
use serde_json::{Value, json};
use tauri::plugin::{Builder, TauriPlugin};

pub use super::mobile::*;

/// What a call says it came from, and the platform as users say it.
pub const CLIENT: &str = "android";
pub const LABEL: &str = "Android";

static VM: OnceLock<JavaVM> = OnceLock::new();
static HOST: OnceLock<GlobalRef> = OnceLock::new();

/// Found as the library loads, on a thread that sees the app's classes: a thread of Rust's own can't find them.
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut c_void) -> jint {
    if let Ok(mut env) = vm.get_env()
        && let Ok(class) = env.find_class("app/client/Host")
        && let Ok(global) = env.new_global_ref(class)
    {
        let _ = HOST.set(global);
    }
    let _ = VM.set(vm);
    JNI_VERSION_1_6
}

/// Kotlin's plugin. A phone's notifications are its own (Push.kt), which Tauri's plugin would take from it, and it saves
/// and opens what screens give with its own share sheet.
pub fn plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let plugin: TauriPlugin<tauri::Wry> = Builder::new("one")
        .setup(|_, api| {
            super::mobile::registered(api.register_android_plugin("app.client", "OnePlugin")?);
            Ok(())
        })
        .build();
    builder.plugin(plugin)
}

/// Asks `Host`, which needs no window: `{ "ok": … }` or `{ "error": "…" }`, as one string.
pub fn host(method: &str, args: Value) -> Result<Value, String> {
    let vm = VM.get().ok_or("The library isn't loaded by an app.")?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let class = HOST.get().ok_or("Host isn't found.")?;
    // SAFETY: a global reference to the class `Host`, found as the library loaded.
    let class = unsafe { JClass::from_raw(class.as_obj().as_raw()) };
    let (method, args) = (env.new_string(method).map_err(|e| e.to_string())?, env.new_string(args.to_string()).map_err(|e| e.to_string())?);
    let out = env
        .call_static_method(&class, "call", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&method), JValue::Object(&args)])
        .and_then(|v| v.l())
        .map_err(|e| {
            let _ = env.exception_clear();
            e.to_string()
        })?;
    let text: String = env.get_string(&JString::from(out)).map_err(|e| e.to_string())?.into();
    let said: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    match said["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(said["ok"].clone()),
    }
}

/// What only the page's own frame is given (Kotlin's `Host.token`, handed it by a web message listener that answers the
/// main frame at the app's own origin alone): every one of the app's commands carries it, as `x-one`, or is refused. Android's
/// web view gives its Tauri script and `ipc` to every frame, a screen's sandboxed one too, which could otherwise call them.
pub fn ours(invoke: &tauri::ipc::Invoke<tauri::Wry>) -> bool {
    static TOKEN: OnceLock<String> = OnceLock::new();
    let token = TOKEN.get_or_init(|| host("token", json!({})).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default());
    !token.is_empty() && invoke.message.headers().get("x-one").and_then(|v| v.to_str().ok()) == Some(token.as_str())
}

/// The app's own folder, where it keeps what isn't secret (Kotlin's `filesDir`): the same for the window and for what
/// runs with none.
pub fn data_without_window() -> Option<PathBuf> {
    host("files", json!({})).ok()?.as_str().map(Into::into)
}

pub fn data(_: &tauri::App) -> Result<PathBuf, String> {
    data_without_window().ok_or_else(|| "Kotlin gave no folder for the app.".into())
}

/// Where what screens give is shared from, which its share sheet is let read (android/.../res/xml/files.xml).
fn given() -> PathBuf {
    crate::app().data.join("given")
}

/// What screens gave to be shared, shown or saved is gone with the last run.
pub fn start(_: &tauri::App) {
    let _ = std::fs::remove_dir_all(given());
}

pub async fn save(name: String, data: Vec<u8>, preview: bool) -> Result<(), String> {
    give(given(), name, data, preview).await
}

/// The home-screen widgets, drawn again.
pub fn redraw_widgets() {
    let _ = host("reload", json!({}));
}

/// An Android launcher counts what notifications it shows, not what the app says.
pub fn badge(_: u32) {}

/// Whether this build shows notifications, asking the user first if `ask`; none where it has no pushes to show (no
/// Firebase project in its config.toml), so Settings has no row for it.
pub async fn notify(ask: bool) -> Option<String> {
    ask_off("notify", json!({ "ask": ask })).await.ok().and_then(|v| v["state"].as_str().map(str::to_string))
}

/// The network's own pages open in a Custom Tab, as signed in there as the browser is; others in the app for them.
pub fn open(url: String) -> Result<(), String> {
    let own = crate::app().address().is_some_and(|a| url.starts_with(&a));
    ask("open", json!({ "url": url, "own": own })).map(drop).map_err(|_| "Couldn’t open your browser.".to_string())
}

/// Signs in in a Chrome Custom Tab, which comes back to the app's own link; a test's prints the link and waits for it
/// to be opened.
pub fn sign_in(signing: &crate::signin::SignIn, join: &str, print: bool) -> Result<(), one_client::sign_in::Failed> {
    signing.sign_in_here(join, |u| {
        if print {
            println!("Sign in: {u}");
        }
        signed_in_at(u, !print)
    })
}

/// What the user calls this phone: its name in Settings, else its model.
pub fn name() -> Option<String> {
    host("name", json!({})).ok()?.as_str().map(str::to_string).filter(|n| !n.is_empty())
}

/// The phone's own id for this app, hashed (its Android ID, which outlives reinstalling), which signing in again replaces
/// its sign-in under, as the Kotlin app's was.
pub fn this_device() -> Option<String> {
    host("device", json!({})).ok()?.as_str().map(str::to_string)
}

/// Whether Google's document scanner is here to use (Play services, and memory enough).
pub fn can_scan() -> bool {
    host("canScan", json!({})).ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

/// How much bigger than usual the user has made text in Settings (1 for usual), for screens to be zoomed by.
pub fn text_scale() -> f64 {
    host("textScale", json!({})).ok().and_then(|v| v.as_f64()).unwrap_or(1.0)
}

/// Once let in, in a build with pushes (a Firebase project in its config.toml): gives the push Worker this phone's FCM
/// token, as it starts and whenever it changes (`register-push`); and asks the user, once, whether it may show
/// notifications. Notes are opened with the device's key by Kotlin's `PushService`.
pub fn let_in(device: Arc<one_client::Device>) {
    push(device, |token| one::push::Address { via: "fcm".into(), address: token, ..Default::default() });
    tell("notify", json!({ "ask": true, "once": true }));
}

/// # Safety
/// Called by the JVM, with its own environment and strings.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_client_Native_call<'a>(mut env: JNIEnv<'a>, _: JClass<'a>, kind: JString<'a>, json: JString<'a>) -> jstring {
    let read = |env: &mut JNIEnv, s: &JString| -> String { env.get_string(s).map(Into::into).unwrap_or_default() };
    let (kind, json) = (read(&mut env, &kind), read(&mut env, &json));
    // One of the things the app answers with no window (`headless`): a widget's button (`call`), WorkManager's refresh
    // (`widgets`), a push (`open-note`), its Approve or Not Me (`join`), a new FCM token (`register-push`).
    let out = tauri::async_runtime::block_on(crate::headless::run_here(&kind, &serde_json::from_str(&json).unwrap_or_default()));
    env.new_string(out.to_string()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
