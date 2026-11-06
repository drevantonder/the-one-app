//! What's Linux's own: the machine's id, opening at login (an XDG autostart entry), and the app in the desktop's
//! launcher as what opens `<cli>://` links.

use std::path::PathBuf;

use tauri::Manager;

pub use super::desktop::*;

use crate::app;

/// What a call says it came from, and the platform as people say it.
pub const CLIENT: &str = "linux";
pub const LABEL: &str = "Linux";
/// How far, in points, the tray's flyouts keep from the bar and the edges (a gap from its panel), and whether its tray is at
/// the screen's foot when nothing says.
pub const FLYOUT_GAP: f64 = 8.0;
pub const TRAY_AT_BOTTOM: bool = false;

pub fn this_device() -> Option<String> {
    this_device_or(["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|p| std::fs::read_to_string(p).ok()))
}

/// Tauri's app local data folder, found with no window: where XDG_DATA_HOME says.
pub fn data_without_window() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join(crate::identifier()))
}

/// Its autostart entry, where XDG_CONFIG_HOME says.
fn entry() -> Option<PathBuf> {
    let handle = &app().handle;
    Some(handle.path().config_dir().ok()?.join("autostart").join(format!("{}.desktop", handle.config().identifier)))
}

pub fn login() -> Option<bool> {
    Some(entry().is_some_and(|e| e.exists()))
}

pub fn set_login(on: bool) {
    let (Some(entry), Ok(exe)) = (entry(), std::env::current_exe()) else { return };
    if !on {
        let _ = std::fs::remove_file(entry);
        return;
    }
    let _ = std::fs::create_dir_all(entry.parent().unwrap());
    let _ = std::fs::write(entry, format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nTerminal=false\n", one::config::NAME, exe.display()));
}

pub fn start(app: &tauri::App) {
    super::desktop::start(app);
    if let Err(e) = launcher(app) {
        eprintln!("the launcher's entry: {e}");
    }
}

/// The app in the desktop's launcher, with its icon, and as what opens `<cli>://` links: written each start, so it names
/// the program where it is now.
fn launcher(app: &tauri::App) -> Result<(), String> {
    let id = &app.config().identifier;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = app.path().data_dir().map_err(|e| e.to_string())?;
    let icon = data.join("icons/hicolor/1024x1024/apps").join(format!("{id}.png"));
    std::fs::create_dir_all(icon.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Some(bytes) = one::web::files().into_iter().find_map(|(path, bytes)| (path == "icon.png").then_some(bytes)) {
        std::fs::write(&icon, bytes).map_err(|e| e.to_string())?;
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exe}\" %u\nIcon={id}\nTerminal=false\nCategories=Office;Network;\nMimeType=x-scheme-handler/{cli};\nStartupWMClass={cli}\n",
        name = one::config::NAME,
        exe = exe.display(),
        cli = one::config::CLI,
    );
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    std::fs::write(apps.join(format!("{id}.desktop")), entry).map_err(|e| e.to_string())?;
    // And it's what opens `<cli>://`, in mimeapps.list where XDG_CONFIG_HOME says.
    let _ = std::process::Command::new("xdg-mime").args(["default", &format!("{id}.desktop"), &format!("x-scheme-handler/{}", one::config::CLI)]).status();
    Ok(())
}

/// The count on the app's icon (the launcher's); none at 0.
pub fn badge(count: u32) {
    use tauri::Manager;
    if let Some(w) = crate::app().handle.get_webview_window("main") {
        let _ = w.set_badge_count((count > 0).then_some(count as i64));
    }
}

/// Pushes come only to computers that register a channel (Windows).
pub fn let_in(_: std::sync::Arc<one_client::Device>) {}

/// Links to the network's address open only the browser.
pub fn claim(_: Option<String>) {}

/// Its windows are all they look: none reaches past what shows, and their corners are the system's.
pub fn unseen(_: &tauri::WebviewWindow) -> (i32, i32, i32, i32) {
    (0, 0, 0, 0)
}

pub fn round(_: &tauri::WebviewWindow) {}
