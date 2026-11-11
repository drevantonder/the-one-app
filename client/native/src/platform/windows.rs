//! What's Windows' own, beside the window: the taskbar button's count, opening at login (the package's startup task,
//! Package.appxmanifest's `login`), the links to the network's address, the package's helper (`windows/`, the Windows
//! App SDK's), which registers this computer's push channel, and setting up `<cli>` where it's missing (`set_up`). All but
//! that need the package's identity, which the app has installed (windows/build.sh); run from a folder, they do nothing.

use std::path::PathBuf;

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Data::Xml::Dom::XmlDocument;
use windows::System::{AppUriHandlerHost, AppUriHandlerRegistrationManager};
use windows::UI::Notifications::{BadgeNotification, BadgeUpdateManager};
use windows::core::HSTRING;
use windows_collections::IIterable;

pub use super::desktop::*;
pub use super::flyout::{fit, start_tray};

/// What a call says it came from, and the platform as users say it.
pub const CLIENT: &str = "windows";
pub const LABEL: &str = "Windows";
/// How far, in points, the tray's flyouts keep from the taskbar and the edges (Windows 11's own), and its taskbar at the
/// screen's foot when nothing says.
pub const FLYOUT_GAP: f64 = 12.0;
pub const TRAY_AT_BOTTOM: bool = true;

/// Tauri's app local data folder, found with no window.
pub fn data_without_window() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(crate::identifier()))
}

/// The count on the taskbar button; none at 0.
pub fn badge(count: u32) {
    let set = || -> windows::core::Result<()> {
        let updater = BadgeUpdateManager::CreateBadgeUpdaterForApplication()?;
        if count == 0 {
            return updater.Clear();
        }
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(format!("<badge value=\"{count}\"/>")))?;
        updater.Update(&BadgeNotification::CreateBadgeNotification(&xml)?)
    };
    if let Err(e) = set() {
        device_logs::say(&format!("badge: {e}"));
    }
}

/// Links to the network's address open this app, from other apps (Edge and Chrome keep their own): the package's
/// `links` names the one host the build was made for, and this makes it the address signed in to, or none. Windows asks
/// the address for its `/.well-known/windows-app-web-link`, which names this package (the gateway serves it).
pub fn claim(address: Option<String>) {
    std::thread::spawn(move || claim_now(address.as_deref()));
}

fn claim_now(address: Option<&str>) {
    let set = || -> windows::core::Result<()> {
        let Some(links) = AppUriHandlerRegistrationManager::GetDefault()?.TryGetRegistration(&HSTRING::from("links")).ok() else { return Ok(()) };
        let host = address.and_then(|a| url::Url::parse(a).ok()).filter(|u| u.scheme() == "https").and_then(|u| u.host_str().map(str::to_string));
        let hosts = host.map(|h| AppUriHandlerHost::CreateInstance(&HSTRING::from(h)).map(Some)).transpose()?;
        links.SetAppAddedHostsAsync(&IIterable::<AppUriHandlerHost>::from(hosts.into_iter().collect::<Vec<_>>()))?.join()
    };
    if let Err(e) = set() {
        device_logs::say(&format!("links: {e}"));
    }
}

/// The title bar in the page's own background (@one/ui's --background), so it reads as the window's top, not a strip across it.
/// Windows 11 draws it; Windows 10 has no such attribute and says no. The bar stays the system's: its caption buttons and
/// Snap Layouts on Maximize can't be had from a page, which sits in a child window over them.
pub fn caption(window: &tauri::WebviewWindow, dark: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DwmSetWindowAttribute};
    let Ok(hwnd) = window.hwnd() else { return };
    let colour: u32 = if dark { 0x000a_0a0a } else { 0x00ff_ffff };
    // SAFETY: the window's own handle, and a value of the attribute's type and size.
    let _ = unsafe { DwmSetWindowAttribute(windows::Win32::Foundation::HWND(hwnd.0), DWMWA_CAPTION_COLOR, (&raw const colour).cast(), size_of_val(&colour) as u32) };
}

/// A flyout (the panel) with rounded corners, as Windows 11 draws its own; Windows 10 has none, and says no.
pub fn round(window: &tauri::WebviewWindow) {
    use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    let Ok(hwnd) = window.hwnd() else { return };
    let corner = DWMWCP_ROUND;
    // SAFETY: the window's own handle, and a value of the attribute's type and size.
    let _ = unsafe { DwmSetWindowAttribute(windows::Win32::Foundation::HWND(hwnd.0), DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const corner).cast(), size_of_val(&corner) as u32) };
}

/// How far a window reaches past what shows of it, on each side (left, top, right, bottom), physical: Windows keeps
/// invisible borders around it for resizing, which its place and size include.
pub fn unseen(window: &tauri::WebviewWindow) -> (i32, i32, i32, i32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    let (Ok(hwnd), Ok(at), Ok(size)) = (window.hwnd(), window.outer_position(), window.outer_size()) else { return (0, 0, 0, 0) };
    let mut seen = RECT::default();
    // SAFETY: the window's own handle, and a RECT of the attribute's size to fill.
    let got = unsafe { DwmGetWindowAttribute(windows::Win32::Foundation::HWND(hwnd.0), DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut seen).cast(), size_of::<RECT>() as u32) };
    if got.is_err() || seen.right <= seen.left {
        return (0, 0, 0, 0);
    }
    (seen.left - at.x, seen.top - at.y, at.x + size.width as i32 - seen.right, at.y + size.height as i32 - seen.bottom)
}

/// The package's helper, beside this program.
fn helper() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let helper = exe.with_file_name(format!("{}Helper.exe", exe.file_stem()?.to_string_lossy()));
    helper.exists().then_some(helper)
}

/// Let in: this computer's WNS channel, registered with the push Worker by the helper (`register-push` back to this
/// program, src/headless.rs, which its onet registers), each start, since a channel lasts 30 days; a custom build with no
/// Microsoft Entra app for WNS has none, and the helper ends.
pub fn let_in() {
    let Some(helper) = helper() else { return };
    use std::os::windows::process::CommandExt;
    // No console window for it.
    if let Err(e) = std::process::Command::new(helper).arg("--register-push").creation_flags(0x0800_0000).spawn() {
        device_logs::say(&format!("push: {e}"));
    }
}

/// Sets up `<cli>` on this computer, from the network at `address`, as install.ps1 does by hand and with this very
/// script: the newest for it, checked, put in place, and its oned a task of the user's, started at sign-in and looked at
/// every minute, with no administrator. One already there is only started.
pub fn set_up(address: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let cli = custom_build::config::CLI;
    let powershell = |args: &[&str]| -> Result<(), String> {
        // No console window for it.
        let done = std::process::Command::new("powershell.exe").args(["-NoProfile", "-ExecutionPolicy", "Bypass"]).args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
        if done.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&done.stderr).trim().to_string()) }
    };
    if platform::home().join("bin").join(format!("{cli}.exe")).exists() {
        return powershell(&["-Command", &format!("Start-ScheduledTask -TaskName {cli}d")]);
    }
    let dir = std::env::temp_dir().join(format!("{cli}-set-up-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (program, script) = (dir.join(format!("{cli}.exe")), dir.join("install.ps1"));
    let done = (|| {
        std::fs::write(&program, super::desktop::program(address)?).map_err(|e| e.to_string())?;
        std::fs::write(&script, include_str!("../../../../install.ps1")).map_err(|e| e.to_string())?;
        powershell(&["-File", &script.to_string_lossy(), "-Binary", &program.to_string_lossy()])
    })();
    let _ = std::fs::remove_dir_all(&dir);
    done
}

fn startup() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from("login"))?.join()
}

/// Whether it opens at login.
pub fn login() -> Option<bool> {
    Some(startup().and_then(|t| t.State()).is_ok_and(|s| s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy))
}

/// Opens at login, or doesn't; one the user turned off in Windows' own settings stays off until they turn it on there.
pub fn set_login(on: bool) {
    let set = || -> windows::core::Result<()> {
        let task = startup()?;
        if on { task.RequestEnableAsync()?.join().map(drop) } else { task.Disable() }
    };
    if let Err(e) = set() {
        device_logs::say(&format!("open at login: {e}"));
    }
}

/// Windows learns the app opens `<cli>://` as it runs (Linux by its launcher entry; elsewhere its bundle says so).
pub fn start(app: &tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    super::desktop::start(app);
    let _ = app.deep_link().register_all();
}
