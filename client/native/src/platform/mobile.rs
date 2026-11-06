//! What both phones do the same way, through the app's own Tauri plugin `one` (Swift's on an iPhone, Kotlin's on
//! Android): asking it, and what's asked of it by name, the same on both (signing in, a push token, haptics, the
//! scanner, the share sheet, light or dark).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use tauri::plugin::PluginHandle;

use crate::app;

static PLUGIN: OnceLock<PluginHandle<tauri::Wry>> = OnceLock::new();

/// The plugin, once Tauri has registered it.
pub(super) fn registered(plugin: PluginHandle<tauri::Wry>) {
    let _ = PLUGIN.set(plugin);
}

/// Asks the plugin and waits for its answer, which it gives on the main thread: so never from there.
pub fn ask(method: &str, args: Value) -> Result<Value, String> {
    PLUGIN.get().ok_or("The app hasn't started yet.")?.run_mobile_plugin(method, args).map_err(|e| e.to_string())
}

pub async fn ask_off(method: &'static str, args: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || ask(method, args)).await.map_err(|e| e.to_string())?
}

/// Tells the plugin, not waiting.
pub fn tell(method: &'static str, args: Value) {
    std::thread::spawn(move || {
        if let Err(e) = ask(method, args) {
            eprintln!("{method}: {e}");
        }
    });
}

/// Signs in in the system's sign-in sheet (an iPhone's ASWebAuthenticationSession, Android's Chrome Custom Tab, which
/// share the browser's passkeys and sign-ins) at `url`, or with `open` false (a test's), only waiting for the link →
/// where it came back to, the app's own `<cli>://signed-in/…`.
pub fn signed_in_at(url: String, open: bool) -> Result<String, String> {
    let out = ask("signIn", json!({ "url": url, "scheme": one::config::CLI, "open": open }))?;
    out["url"].as_str().map(str::to_string).ok_or_else(|| "Sign in again.".into())
}

/// Gives the push Worker this phone's token from the plugin, as `address` makes it its push address.
pub fn push(device: Arc<one_client::Device>, address: impl FnOnce(String) -> one::push::Address + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        let token = match ask_off("push", json!({})).await {
            Ok(out) => out["token"].as_str().map(str::to_string),
            Err(e) => return eprintln!("push: no token: {e}"),
        };
        let Some(token) = token else { return };
        if let Err(e) = device.register_push(&address(token)).await {
            eprintln!("push: can't register: {e}");
        }
    });
}

pub fn widgets_dir() -> PathBuf {
    crate::headless::data().join("widgets")
}

/// "selection" (a press), "medium" (a long press), "success" or "error".
pub fn haptic(style: String) {
    tell("haptic", json!({ "style": style }));
}

/// The document camera: each page found, straightened and cropped, as a JPEG `{ name, type, size, base64 }`; none if the
/// person cancels.
pub async fn scan() -> Result<Value, String> {
    ask_off("scan", json!({})).await.map(|v| v["files"].clone())
}

/// A page's own menu, but where something lifts what's held (an iPhone's).
pub async fn menu(_: Value) -> Value {
    json!({ "lifted": false })
}

/// The system bars follow it.
pub fn appearance(value: String) {
    tell("appearance", json!({ "value": value }));
}

/// A file a screen gives, shared (the share sheet, which saves to Files too) or shown with `preview` (what views its
/// type), from a folder of its own in `base`.
pub async fn give(base: PathBuf, name: String, data: Vec<u8>, preview: bool) -> Result<(), String> {
    let dir = base.join(app().next.fetch_add(1, Ordering::Relaxed).to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&name);
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    ask_off("give", json!({ "path": path, "preview": preview })).await.map(drop)
}

/// A phone has no login to open at, no menu bar, nothing to let agents use, and no window to bring in front.
pub fn login() -> Option<bool> {
    None
}

pub fn set_login(_: bool) {}

pub fn menubar(_: String, _: bool) -> Vec<String> {
    Vec::new()
}

pub fn menubar_hidden() -> Option<Vec<String>> {
    None
}

pub fn computer_use() -> Option<bool> {
    None
}

pub fn set_computer_use(_: bool) {}

pub fn show() {}

pub fn claim(_: Option<String>) {}

pub fn unpaired(_: Option<&Value>) -> bool {
    false
}

pub fn menus(_: bool, _: bool) {}

pub fn approve_here(_: String) {}
