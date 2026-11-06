//! What's Windows' own, beside the window: the taskbar button's count, opening at login (the package's startup task,
//! Package.appxmanifest's `login`), the machine's own id, and the package's helper
//! (`windows/`, the Windows App SDK's), which registers this device's push channel. All but the id need the package's
//! identity, which the app has installed (windows/build.sh); run from a folder, they do nothing.

use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{BadgeNotification, BadgeUpdateManager};
use windows::core::HSTRING;

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
