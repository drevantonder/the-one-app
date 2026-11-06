//! What's Windows' own, beside the window: the taskbar button's count, opening at login (the package's startup task,
//! Package.appxmanifest's `login`), the links to the network's address, the machine's own id, and the package's helper
//! (`windows/`, the Windows App SDK's), which registers this device's push channel. All but the id need the package's
//! identity, which the app has installed (windows/build.sh); run from a folder, they do nothing.

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Data::Xml::Dom::XmlDocument;
use windows::System::{AppUriHandlerHost, AppUriHandlerRegistrationManager};
use windows::UI::Notifications::{BadgeNotification, BadgeUpdateManager};
use windows::core::HSTRING;
use windows_collections::IIterable;

/// Windows' own id for this machine, which outlives reinstalling the app.
pub fn machine_guid() -> Option<String> {
    windows_registry::LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Cryptography").ok()?.get_string("MachineGuid").ok()
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
        eprintln!("badge: {e}");
    }
}

/// Links to the network's address open this app, from other apps (Edge and Chrome keep their own): the package's
/// `links` names the one host the build was made for, and this makes it the address signed in to, or none. Windows asks
/// the address for its `/.well-known/windows-app-web-link`, which names this package (the gateway serves it).
pub fn claim(address: Option<&str>) {
    let set = || -> windows::core::Result<()> {
        let Some(links) = AppUriHandlerRegistrationManager::GetDefault()?.TryGetRegistration(&HSTRING::from("links")).ok() else { return Ok(()) };
        let host = address.and_then(|a| url::Url::parse(a).ok()).filter(|u| u.scheme() == "https").and_then(|u| u.host_str().map(str::to_string));
        let hosts = host.map(|h| AppUriHandlerHost::CreateInstance(&HSTRING::from(h)).map(Some)).transpose()?;
        links.SetAppAddedHostsAsync(&IIterable::<AppUriHandlerHost>::from(hosts.into_iter().collect::<Vec<_>>()))?.join()
    };
    if let Err(e) = set() {
        eprintln!("links: {e}");
    }
}

/// The package's helper, beside this program.
fn helper() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let helper = exe.with_file_name(format!("{}Helper.exe", exe.file_stem()?.to_string_lossy()));
    helper.exists().then_some(helper)
}

/// This device's WNS channel, registered with the push Worker by the helper (`--register-push wns <channel>` back to
/// this program, src/headless.rs); a custom build with no Microsoft Entra app for WNS has none, and the helper ends.
pub fn register_push() {
    let Some(helper) = helper() else { return };
    use std::os::windows::process::CommandExt;
    // No console window for it.
    if let Err(e) = std::process::Command::new(helper).arg("--register-push").creation_flags(0x0800_0000).spawn() {
        eprintln!("push: {e}");
    }
}

fn startup() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from("login"))?.join()
}

/// Whether it opens at login.
pub fn login() -> bool {
    startup().and_then(|t| t.State()).is_ok_and(|s| s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy)
}

/// Opens at login, or doesn't; one the person turned off in Windows' own settings stays off until they turn it on there.
pub fn set_login(on: bool) {
    let set = || -> windows::core::Result<()> {
        let task = startup()?;
        if on { task.RequestEnableAsync()?.join().map(drop) } else { task.Disable() }
    };
    if let Err(e) = set() {
        eprintln!("open at login: {e}");
    }
}
