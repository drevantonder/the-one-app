//! What's Linux's own: opening at login (an XDG autostart entry), the app in the desktop's launcher as what opens
//! `<cli>://` links, and setting up `<cli>` where it's missing, with oned run by the user's own systemd (`set_up`).

use std::path::PathBuf;

use tauri::Manager;

pub use super::desktop::*;
pub use super::flyout::{fit, start_tray};

use crate::app;

/// What a call says it came from, and the platform as users say it.
pub const CLIENT: &str = "linux";
pub const LABEL: &str = "Linux";
/// How far, in points, the tray's flyouts keep from the bar and the edges (a gap from its panel), and whether its tray is at
/// the screen's foot when nothing says.
pub const FLYOUT_GAP: f64 = 8.0;
pub const TRAY_AT_BOTTOM: bool = false;

/// Sets up `<cli>` on this computer, from the network at `address`, with no root: the newest for it where install.sh
/// puts it (`~/.local/bin`; a throwaway network's in its home, as its programs put theirs), and its oned run by the
/// user's own systemd, which starts it as they sign in and again whenever it stops, as `<cli> update` has it. A program
/// already there is only started. One installed for the whole machine (install.sh, by root) is root's to start.
pub fn set_up(address: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let cli = custom_build::config::CLI;
    let oned = format!("{cli}d");
    // A throwaway network's computer is its own, beside the machine's.
    let throwaway = crate::env("THROWAWAY").is_some();
    if !throwaway && std::path::Path::new("/etc/systemd/system").join(format!("{oned}.service")).exists() {
        return Err(format!("This computer's {oned} isn't running: start it with sudo systemctl start {oned}"));
    }
    let bin = if throwaway { platform::home().join("bin") } else { PathBuf::from(std::env::var_os("HOME").ok_or("This user has no home folder.")?).join(".local/bin") };
    let program = bin.join(cli);
    if !program.exists() {
        let bytes = super::desktop::program(address)?;
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let part = bin.join(format!(".{cli}.part"));
        std::fs::write(&part, bytes).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &program).map_err(|e| e.to_string())?;
        for name in [oned.clone(), format!("git-remote-{cli}")] {
            let _ = std::fs::remove_file(bin.join(&name));
            std::os::unix::fs::symlink(cli, bin.join(name)).map_err(|e| e.to_string())?;
        }
        device_logs::say(&format!("set up {} from {address}", program.display()));
    }
    // A throwaway network's oned is a unit of its own beside the user's real one, named for its home.
    let unit = match throwaway {
        true => format!("{oned}-throwaway-{}", &data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(platform::home().to_string_lossy().as_bytes()))[..8]),
        false => oned.clone(),
    };
    // What says which network and home it's for: the environment `one-dev throwaway` gives its programs, or a custom
    // build's home, as the window was started with.
    let env: String = ["HOME", "ADDR", "THROWAWAY", "RELEASE_KEY"].iter().filter_map(|k| Some(format!("Environment=\"{}={}\"\n", custom_build::config::env(k), crate::env(k)?))).collect();
    let file = platform::home().join(format!("{unit}.service"));
    std::fs::create_dir_all(platform::home()).map_err(|e| e.to_string())?;
    std::fs::write(
        &file,
        format!("[Unit]\nDescription={}, this device\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=2\n{env}\n[Install]\nWantedBy=default.target\n", custom_build::config::NAME, bin.join(&oned).display()),
    )
    .map_err(|e| e.to_string())?;
    let systemctl = |args: &[&str]| -> Result<(), String> {
        let done = std::process::Command::new("systemctl").arg("--user").args(args).output().map_err(|_| format!("This computer has no systemd to keep {oned} running: run {} yourself.", bin.join(&oned).display()))?;
        if done.status.success() { Ok(()) } else { Err(format!("systemctl --user {}: {}", args.join(" "), String::from_utf8_lossy(&done.stderr).trim())) }
    };
    systemctl(&["link", &file.to_string_lossy()])?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &unit])
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
    let _ = std::fs::write(entry, format!("[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nTerminal=false\n", custom_build::config::NAME, exe.display()));
}

pub fn start(app: &tauri::App) {
    super::desktop::start(app);
    if let Err(e) = launcher(app) {
        device_logs::say(&format!("the launcher's entry: {e}"));
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
    if let Some(bytes) = ui::web::files().into_iter().find_map(|(path, bytes)| (path == "icon.png").then_some(bytes)) {
        std::fs::write(&icon, bytes).map_err(|e| e.to_string())?;
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exe}\" %u\nIcon={id}\nTerminal=false\nCategories=Office;Network;\nMimeType=x-scheme-handler/{cli};\nStartupWMClass={cli}\n",
        name = custom_build::config::NAME,
        exe = exe.display(),
        cli = custom_build::config::CLI,
    );
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    std::fs::write(apps.join(format!("{id}.desktop")), entry).map_err(|e| e.to_string())?;
    // And it's what opens `<cli>://`, in mimeapps.list where XDG_CONFIG_HOME says.
    let _ = std::process::Command::new("xdg-mime").args(["default", &format!("{id}.desktop"), &format!("x-scheme-handler/{}", custom_build::config::CLI)]).status();
    Ok(())
}

/// The count on the app's icon (the launcher's); none at 0.
pub fn badge(count: u32) {
    use tauri::Manager;
    if let Some(w) = crate::app().handle.get_webview_window("main") {
        let _ = w.set_badge_count((count > 0).then_some(count as i64));
    }
}

/// Notifications come only to computers that register a channel (Windows).
pub fn let_in() {}

/// Links to the network's address open only the browser.
pub fn claim(_: Option<String>) {}

/// Its windows are all they look: none reaches past what shows, and their corners are the system's.
pub fn unseen(_: &tauri::WebviewWindow) -> (i32, i32, i32, i32) {
    (0, 0, 0, 0)
}

pub fn round(_: &tauri::WebviewWindow) {}
