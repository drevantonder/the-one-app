//! What's the Mac's own, beside the window: opening at login (SMAppService's main app), this Mac's node (oned, the
//! bundle's launch agent `app.<cli>.mac.oned`, kept running, which joins a network with a pairing code and approves
//! the window), "Let Agents Use This Mac", which Computer Use's helper reads from this app's defaults (`computerUse`),
//! the app's menus, and the widgets' own items in the menu bar (`menubar`).

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSString, NSUserDefaults};

use serde_json::{Value, json};

pub use super::desktop::*;
use super::menubar;

/// What a call says it came from, and the platform as users say it.
pub const CLIENT: &str = "mac";
pub const LABEL: &str = "Mac";
/// How far, in points, the tray's flyouts keep from the bar and the edges (a popover's, just under the menu bar), and whether its tray is at
/// the screen's foot when nothing says.
pub const FLYOUT_GAP: f64 = 5.0;
pub const TRAY_AT_BOTTOM: bool = false;

/// What the user calls this Mac, as it's named in its settings ("Andre's MacBook Air"), as the Mac app named it.
pub fn name() -> Option<String> {
    let named = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    named.filter(|n| !n.is_empty()).or_else(super::desktop::name)
}

/// A Mac is the device its own node is.
pub fn this_device() -> Option<String> {
    this_device_or(None)
}

/// Tauri's app local data folder, found with no window.
pub fn data_without_window() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support").join(crate::identifier()))
}

/// The widgets' own items in the menu bar, drawn again from what's kept.
pub fn redraw_widgets() {
    menubar::update();
}

/// A widget's own menu bar item shown again, or hidden → the kinds hidden now.
pub fn menubar(kind: String, show: bool) -> Vec<String> {
    menubar::set_hidden(&kind, !show);
    crate::tray_menu(&crate::app().handle);
    menubar::hidden()
}

pub fn menubar_hidden() -> Option<Vec<String>> {
    Some(menubar::hidden())
}

pub fn hidden_widgets() -> Vec<(String, String)> {
    let hidden = menubar::hidden();
    menubar::widgets().into_iter().filter(|w| hidden.contains(&w.0)).map(|(kind, title, ..)| (kind, title)).collect()
}

/// The menu bar's mark: the custom build's (build.rs), a template macOS tints for the menu bar.
pub fn tray_icon(_: &tauri::App, tray: tauri::tray::TrayIconBuilder<tauri::Wry>) -> tauri::Result<tauri::tray::TrayIconBuilder<tauri::Wry>> {
    Ok(tray.icon(tauri::image::Image::from_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/menubar.png")))?).icon_as_template(true))
}

/// Whether this Mac's own node is in no network yet, so joins one with a pairing code (`pair`), as the Mac app did.
pub fn unpaired(local: Option<&Value>) -> bool {
    local.is_some_and(|s| s["network"].is_null())
}

/// Its own node approves the window by its code, as one of the Mac's own programs, if it's a device of the same user
/// (as the Mac app's did): else the code waits for another of theirs.
pub fn approve_here(code: String) {
    std::thread::spawn(move || crate::local("/v1/devices/approve", Some(json!({ "code": crate::device_code(&code), "confirm": true })), 30));
}

/// This Mac's node kept running; the Mac app's menus, with its keys; the widgets' own items, as last kept, until the
/// node's answer redraws them.
pub fn start(app: &tauri::App) {
    super::desktop::start(app);
    start_node();
    match app_menu(app.handle()) {
        Ok(menu) => drop(app.set_menu(menu)),
        Err(e) => one::logs::say(&format!("the app's menu: {e}")),
    }
    app.on_menu_event(|_, e| match e.id().as_ref() {
        "sidebar" | "join" => {
            show();
            command(e.id().as_ref());
        }
        "pair" => crate::main(Some(format!("{}://settings/devices?add=1", one::config::CLI))),
        _ => {}
    });
    menubar::update();
}

/// One of the app menu's commands, to the window's page, which does it: "sidebar" (fold it or unfold it), "join".
fn command(name: &str) {
    use tauri::Manager;
    if let Some(w) = crate::app().handle.get_webview_window("main") {
        let _ = w.eval(format!("window.__oneNative?.command({})", json!(name)));
    }
}

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

fn defaults() -> Retained<NSUserDefaults> {
    NSUserDefaults::standardUserDefaults()
}

/// Whether it opens at login.
pub fn login() -> Option<bool> {
    Some(status(&main_app()) == ENABLED)
}

pub fn set_login(on: bool) {
    let app = main_app();
    if on { register(&app) } else { unregister(&app) };
}

/// Whether the user lets their agents use this Mac (Computer Use's helper reads it here).
pub fn computer_use() -> Option<bool> {
    Some(defaults().boolForKey(&NSString::from_str("computerUse")))
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
fn start_node() {
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
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
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
pub fn menus(signed_in: bool, unpaired: bool) {
    if let Some((pair, join)) = MENU.get() {
        let _ = pair.set_enabled(signed_in);
        let _ = join.set_enabled(unpaired);
    }
}

/// The count on the app's icon (the dock's); none at 0.
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
