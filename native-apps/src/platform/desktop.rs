//! What every computer does the same way: Linux, the Mac and Windows each take these, but what they do their own way.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use crate::app;

/// What the user calls this computer: its host name (Windows names it in COMPUTERNAME).
pub fn name() -> Option<String> {
    std::env::var("HOSTNAME").ok().or_else(|| std::env::var("COMPUTERNAME").ok()).or_else(|| std::fs::read_to_string("/etc/hostname").ok()).map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

/// The newest `<cli>` for this computer, as the network at `address` publishes it (`oned_update::latest`, the signed record a
/// device updates by), checked against the release key: a throwaway network's own, as it tells its programs.
pub fn program(address: &str) -> Result<Vec<u8>, String> {
    let key = if crate::env("THROWAWAY").is_some() { crate::env("RELEASE_KEY").unwrap_or_default() } else { custom_build::config::RELEASE_KEY.to_string() };
    if key.is_empty() {
        return Err("This build has no release key, so nothing can be set up from it.".into());
    }
    let text = oned_update::latest::fetch(address).map_err(|e| format!("{e:#}"))?.ok_or_else(|| format!("Nothing has been published at {address} yet."))?;
    let record = oned_update::latest::Latest::read(&key, &text).map_err(|e| format!("The record at {address}: {e:#}"))?;
    let thing = record.to_update_to(custom_build::config::TARGET).ok_or_else(|| format!("{address} has published nothing for {}.", custom_build::config::TARGET))?;
    oned_update::latest::download(address, thing).map_err(|e| format!("{e:#}"))
}

/// Its local folder, not one a domain profile roams (Windows): the client's is this device's alone.
pub fn data(app: &tauri::App) -> Result<PathBuf, String> {
    app.path().app_local_data_dir().map_err(|e| e.to_string())
}

pub fn widgets_dir() -> PathBuf {
    crate::headless::data().join("widgets")
}

/// The widgets are only the page's.
pub fn redraw_widgets() {}

/// A computer has no haptics.
pub fn haptic(_: String) {}

pub fn can_scan() -> bool {
    false
}

pub async fn scan() -> Result<Value, String> {
    Err("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.".into())
}

pub fn text_scale() -> f64 {
    1.0
}

/// Signs in in the system's browser, which comes back to this machine's loopback; a test's prints the link.
pub fn sign_in(signing: &crate::signin::SignIn, join: &str, print: bool) -> Result<(), onet::sign_in::Failed> {
    let handle = app().handle.clone();
    signing.sign_in(join, |u| {
        if print {
            println!("Sign in: {u}");
            return Ok(());
        }
        handle.opener().open_url(u, None::<&str>).map_err(|e| e.to_string())
    })
}

/// A page's own menu: nothing here lifts what's held.
pub async fn menu(_: Value) -> Value {
    json!({ "lifted": false })
}

/// Kept, so its next start draws in it from its first frame.
pub fn appearance(value: String) {
    app().keep("appearance", Some(&value).filter(|v| *v != "automatic").map(String::as_str));
    app().handle.webview_windows().values().for_each(appear);
}

/// A window in the user's light or dark (the system's, for automatic): its title bar, and the colour it shows before
/// its page draws, so a start in dark has no white frame.
pub fn appear(window: &tauri::WebviewWindow) {
    let theme = match app().kept("appearance").as_deref() {
        Some("dark") => Some(tauri::Theme::Dark),
        Some("light") => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);
    // @one/ui's --background.
    let dark = theme.or_else(|| window.theme().ok()) == Some(tauri::Theme::Dark);
    #[cfg(windows)]
    super::caption(window, dark);
    let _ = window.set_background_color(Some(if dark { tauri::window::Color(10, 10, 10, 255) } else { tauri::window::Color(255, 255, 255, 255) }));
}

/// Whether it shows notifications, asking the user first if `ask`.
pub async fn notify(ask: bool) -> Option<String> {
    let n = app().handle.notification();
    let state = match n.permission_state() {
        Ok(PermissionState::Granted) => PermissionState::Granted,
        Ok(s) if !ask => s,
        _ => n.request_permission().unwrap_or(PermissionState::Denied),
    };
    Some(match state {
        PermissionState::Granted => "granted".into(),
        PermissionState::Denied => "denied".into(),
        _ => "default".into(),
    })
}

pub fn open(url: String) -> Result<(), String> {
    app().handle.opener().open_url(url, None::<&str>).map_err(|_| "Couldn’t open your browser.".to_string())
}

/// A file a screen gives: saved where the user chooses, or with `preview`, opened in the app for it.
pub async fn save(name: String, data: Vec<u8>, preview: bool) -> Result<(), String> {
    if preview {
        let dir = std::env::temp_dir().join(format!("{}-{}", custom_build::config::CLI, app().next.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(&name);
        std::fs::write(&path, data).map_err(|e| e.to_string())?;
        return app().handle.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string());
    }
    let handle = app().handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = handle.dialog().file().set_file_name(&name).blocking_save_file() else { return Ok(()) };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(path, data).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The window, in front.
pub fn show() {
    if let Some(w) = app().handle.get_webview_window("main") {
        // Only when the window comes back from being closed: never under a page in use.
        if !w.is_visible().unwrap_or(true) {
            crate::bundle::reopened(&app().handle);
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn menus(_: bool, _: bool) {}

/// What's kept of a window between starts. Not whether it was visible: closing only hides it, and the app shows it at each start.
const PLACE: StateFlags = StateFlags::all().difference(StateFlags::VISIBLE);

/// Its size and place kept now, as the window closes: the app may next stop with no chance to (an update, the machine going down).
pub fn keep_place() {
    let _ = app().handle.save_window_state(PLACE);
}

/// One app at a time (a second start, with a link or none, shows the first), as big and where it was left; the system's
/// notifications, file dialogs and opener.
pub fn plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        // First, so a second start goes to the first before anything else runs.
        .plugin(tauri_plugin_single_instance::init(|_, argv, _| {
            show();
            argv.iter().for_each(|a| crate::opened(a));
        }))
        // The panel shows only when its item is clicked.
        .plugin(tauri_plugin_window_state::Builder::new().with_denylist(&["panel"]).with_state_flags(PLACE).build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
}

/// Every command comes from the app's own windows (their capability says so).
pub fn ours(_: &tauri::ipc::Invoke<tauri::Wry>) -> bool {
    true
}

/// What runs beside the window asks it, while it runs, rather than starting a client of its own.
pub fn start(_: &tauri::App) {
    crate::headless::serve();
}
