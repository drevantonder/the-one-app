//! What's the Mac's own, beside the window: opening at login (SMAppService's main app), this Mac's node (oned, the
//! bundle's launch agent `app.<cli>.mac.oned`, kept running), and "Let Agents Use This Mac", which Computer Use's helper
//! reads from this app's defaults (`computerUse`).

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;

use crate::apple::defaults;

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

/// Whether the person lets their agents use this Mac (Computer Use's helper reads it here).
pub fn computer_use() -> bool {
    defaults().boolForKey(&NSString::from_str("computerUse"))
}

pub fn set_computer_use(on: bool) {
    defaults().setBool_forKey(on, &NSString::from_str("computerUse"));
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

/// The app's menus, macOS's own with the Mac app's keys, as items of them so they work wherever the focus is (a key
/// pressed in a screen's frame never reaches the page): View's Fold Sidebar (⌘\), File's Pair a Computer… (⌘⇧I, signed
/// in) and Join with a Pairing Code… (⌘⇧J, while this Mac's node is in no network).
pub fn menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let pair = MenuItem::with_id(app, "pair", "Pair a Computer…", false, Some("CmdOrCtrl+Shift+I"))?;
    let join = MenuItem::with_id(app, "join", "Join with a Pairing Code…", false, Some("CmdOrCtrl+Shift+J"))?;
    let about = AboutMetadata { name: Some(one::config::NAME.into()), ..Default::default() };
    let menu = Menu::with_items(
        app,
        &[
            &Submenu::with_items(app, one::config::NAME, true, &[&P::about(app, None, Some(about))?, &P::separator(app)?, &P::services(app, None)?, &P::separator(app)?, &P::hide(app, None)?, &P::hide_others(app, None)?, &P::separator(app)?, &P::quit(app, None)?])?,
            &Submenu::with_items(app, "File", true, &[&pair, &join, &P::separator(app)?, &P::close_window(app, None)?])?,
            &Submenu::with_items(app, "Edit", true, &[&P::undo(app, None)?, &P::redo(app, None)?, &P::separator(app)?, &P::cut(app, None)?, &P::copy(app, None)?, &P::paste(app, None)?, &P::select_all(app, None)?])?,
            &Submenu::with_items(app, "View", true, &[&MenuItem::with_id(app, "sidebar", "Fold Sidebar", true, Some("CmdOrCtrl+\\"))?, &P::separator(app)?, &P::fullscreen(app, None)?])?,
            &Submenu::with_items(app, "Window", true, &[&P::minimize(app, None)?, &P::maximize(app, None)?, &P::separator(app)?, &P::close_window(app, None)?])?,
            &Submenu::with_items(app, "Help", true, &[])?,
        ],
    )?;
    let _ = MENU.set((pair, join));
    Ok(menu)
}

static MENU: std::sync::OnceLock<(tauri::menu::MenuItem<tauri::Wry>, tauri::menu::MenuItem<tauri::Wry>)> = std::sync::OnceLock::new();

/// Pair a Computer… while someone's signed in here; Join with a Pairing Code… while this Mac's node is in no network.
pub fn menu_now(signed_in: bool, unpaired: bool) {
    if let Some((pair, join)) = MENU.get() {
        let _ = pair.set_enabled(signed_in);
        let _ = join.set_enabled(unpaired);
    }
}
