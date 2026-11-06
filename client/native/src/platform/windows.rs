//! What's Windows' own, beside the window: the taskbar button's count, opening at login (the package's startup task,
//! Package.appxmanifest's `login`), the links to the network's address, the machine's own id, and the package's helper
//! (`windows/`, the Windows App SDK's), which registers this device's push channel. All but the id need the package's
//! identity, which the app has installed (windows/build.sh); run from a folder, they do nothing.

use std::path::PathBuf;
use std::sync::Arc;

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Data::Xml::Dom::XmlDocument;
use windows::System::{AppUriHandlerHost, AppUriHandlerRegistrationManager};
use windows::UI::Notifications::{BadgeNotification, BadgeUpdateManager};
use windows::core::HSTRING;
use windows_collections::IIterable;

pub use super::desktop::*;

/// What a call says it came from, and the platform as people say it.
pub const CLIENT: &str = "windows";
pub const LABEL: &str = "Windows";
/// How far, in points, the tray's flyouts keep from the taskbar and the edges (Windows 11's own), and its taskbar at the
/// screen's foot when nothing says.
pub const FLYOUT_GAP: f64 = 12.0;
pub const TRAY_AT_BOTTOM: bool = true;

/// Windows' own id for this machine, MachineGuid, which outlives reinstalling Windows' apps (as the Windows app's was).
pub fn this_device() -> Option<String> {
    this_device_or(windows_registry::LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Cryptography").ok().and_then(|k| k.get_string("MachineGuid").ok()))
}

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
        one::logs::say(&format!("badge: {e}"));
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
        one::logs::say(&format!("links: {e}"));
    }
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

/// Let in: this device's WNS channel, registered with the push Worker by the helper (`register-push` back to this
/// program, src/headless.rs), each start, since a channel lasts 30 days; a custom build with no Microsoft Entra app for
/// WNS has none, and the helper ends.
pub fn let_in(_: Arc<one_client::Device>) {
    let Some(helper) = helper() else { return };
    use std::os::windows::process::CommandExt;
    // No console window for it.
    if let Err(e) = std::process::Command::new(helper).arg("--register-push").creation_flags(0x0800_0000).spawn() {
        one::logs::say(&format!("push: {e}"));
    }
}

fn startup() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from("login"))?.join()
}

/// Whether it opens at login.
pub fn login() -> Option<bool> {
    Some(startup().and_then(|t| t.State()).is_ok_and(|s| s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy))
}

/// Opens at login, or doesn't; one the person turned off in Windows' own settings stays off until they turn it on there.
pub fn set_login(on: bool) {
    let set = || -> windows::core::Result<()> {
        let task = startup()?;
        if on { task.RequestEnableAsync()?.join().map(drop) } else { task.Disable() }
    };
    if let Err(e) = set() {
        one::logs::say(&format!("open at login: {e}"));
    }
}

/// Windows learns the app opens `<cli>://` as it runs (Linux by its launcher entry; elsewhere its bundle says so).
pub fn start(app: &tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;
    super::desktop::start(app);
    let _ = app.deep_link().register_all();
}
