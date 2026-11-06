//! What's the Mac's own, beside the window: opening at login (SMAppService's main app), this Mac's node (oned, the
//! bundle's launch agent `app.<cli>.mac.oned`, kept running), the defaults the old Mac app kept that this one takes over
//! once (its address and sign-in), and "Let Agents Use This Mac", which Computer Use's helper reads from this app's
//! defaults (`computerUse`).

use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSString, NSUserDefaults};

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// SMAppServiceStatus's enabled.
const ENABLED: isize = 1;

fn status(service: &AnyObject) -> isize {
    unsafe { msg_send![service, status] }
}

fn register(service: &AnyObject) -> bool {
    unsafe { msg_send![service, registerAndReturnError: std::ptr::null_mut::<*mut AnyObject>()] }
}

fn unregister(service: &AnyObject) -> bool {
    unsafe { msg_send![service, unregisterAndReturnError: std::ptr::null_mut::<*mut AnyObject>()] }
}

fn main_app() -> Retained<AnyObject> {
    unsafe { msg_send![class!(SMAppService), mainAppService] }
}

/// Whether it opens at login.
pub fn login() -> bool {
    status(&main_app()) == ENABLED
}

pub fn set_login(on: bool) {
    let app = main_app();
    if on { register(&app) } else { unregister(&app) };
}

fn defaults() -> Retained<NSUserDefaults> {
    NSUserDefaults::standardUserDefaults()
}

fn string(key: &str) -> Option<String> {
    defaults().stringForKey(&NSString::from_str(key)).map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// Whether the person lets their agents use this Mac (Computer Use's helper reads it here).
pub fn computer_use() -> bool {
    defaults().boolForKey(&NSString::from_str("computerUse"))
}

pub fn set_computer_use(on: bool) {
    defaults().setBool_forKey(on, &NSString::from_str("computerUse"));
}

/// What the old Mac app kept in its defaults, taken over the first time (`data` has no address yet), so a signed-in
/// Mac stays signed in: the address, an invite's secret, its sign-in's client, and the client's own folder; and its Open at
/// Login and the widgets hidden from the menu bar.
pub fn take_over(data: &Path) {
    if data.join("address").exists() {
        return;
    }
    let Some(address) = string("address") else { return };
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(data.join("address"), &address);
    if let Some(invite) = string("invite") {
        let _ = std::fs::write(data.join("invite"), invite);
    }
    if let (Some(client), Some("http://127.0.0.1/callback")) = (string("client"), string("clientRedirect").as_deref()) {
        let _ = std::fs::write(data.join("sign-in"), format!("{address}\n{client}"));
    }
    // Open at login was set the first time it opened; the person's choice since stays (lib.rs `tray`).
    if defaults().boolForKey(&NSString::from_str("loginItemSet")) {
        let _ = std::fs::write(data.join("opened-at-login"), "");
    }
    // The widgets they hid from the menu bar (src/menubar.rs).
    if let Some(hidden) = defaults().stringArrayForKey(&NSString::from_str("hiddenWidgets")) {
        let kinds: Vec<String> = hidden.iter().map(|k| k.to_string()).collect();
        let _ = std::fs::write(data.join("menubar-hidden"), kinds.join("\n"));
    }
    if let Some(support) = std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support")) {
        let old = support.join(one::config::NAME).join(one::config::CLI).join("client");
        if old.is_dir() && !data.join("client").exists() {
            let _ = std::fs::rename(old, data.join("client"));
        }
    }
}

/// The node's launch agent's label, in this bundle's Contents/Library/LaunchAgents.
fn label() -> String {
    format!("app.{}.mac.oned", one::config::CLI)
}

fn launchctl(args: &[&str]) -> String {
    std::process::Command::new("/bin/launchctl").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// Asks launchd to run the node's agent now → whether it runs within a few seconds.
fn kickstarted() -> bool {
    let job = format!("gui/{}/{}", unsafe { getuid() }, label());
    launchctl(&["kickstart", &job]);
    (0..10).any(|_| {
        let running = launchctl(&["print", &job]).contains("state = running");
        if !running {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        running
    })
}

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Keeps oned, this Mac as a device, running: the bundle's launch agent, registered (again, if launchd won't run it, as
/// once the app's been rebuilt since it was registered, which records this build); or, refused, a child of the app's.
/// Off the main thread: launchctl takes seconds.
pub fn start_node() {
    std::thread::spawn(|| {
        let agent: Retained<AnyObject> = unsafe { msg_send![class!(SMAppService), agentServiceWithPlistName: &*NSString::from_str(&format!("{}.plist", label()))] };
        if status(&agent) == ENABLED {
            if kickstarted() {
                return;
            }
            unregister(&agent);
        }
        if register(&agent) && status(&agent) == ENABLED && kickstarted() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let node = exe.with_file_name(format!("{}d", one::config::CLI));
        let running = std::process::Command::new("/usr/bin/pgrep").args(["-f", &format!("{}$", node.display())]).output().is_ok_and(|o| o.status.success());
        if node.exists() && !running {
            let _ = std::process::Command::new(node).spawn();
        }
    });
}
