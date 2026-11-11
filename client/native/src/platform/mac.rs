//! What's the Mac's own, beside the window: opening at login (SMAppService's main app), setting up `<cli>` where it's
//! missing, with oned a launch agent of the user's (`set_up`), whose onet the window is a view of, "Let Agents Use This
//! Mac", which Computer Use's helper reads from this app's defaults (`computerUse`), the app's menus, and the menu bar
//! (`menubar`).

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
/// What the user calls this Mac, as it's named in its settings ("Andre's MacBook Air"), as the Mac app named it.
pub fn name() -> Option<String> {
    let named = std::process::Command::new("/usr/sbin/scutil").args(["--get", "ComputerName"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    named.filter(|n| !n.is_empty()).or_else(super::desktop::name)
}

/// Tauri's app local data folder, found with no window.
pub fn data_without_window() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support").join(crate::identifier()))
}

/// The widgets' own items in the menu bar, drawn again from what's kept.
pub fn redraw_widgets() {
    menubar::update();
}

/// The menu bar's items, AppKit's (`menubar`).
/// Where the window's buttons sit: points from its left, and how far tao lowers them, which centres them in the 40-point
/// title bar `title_bar_area` gives the page.
pub const LIGHTS_AT: (f64, f64) = (16.0, 22.5);

/// What the page's told of the title bar lying over its top, by the names of the web's Window Controls Overlay
/// (`titlebar-area-x` and the rest, which WKWebView doesn't give): where the room beside the window's buttons starts, none
/// taken in full screen, when they're hidden; and that it's there at all (`.wco`).
pub fn title_bar_area(full_screen: bool) -> String {
    let x = if full_screen { 0 } else { 79 };
    format!(
        "(() => {{ const root = document.documentElement; root.classList.add('wco'); for (const [k, v] of Object.entries({{ x: '{x}px', y: '0px', width: 'calc(100vw - {x}px)', height: '40px' }})) root.style.setProperty(`--titlebar-area-${{k}}`, v); }})();"
    )
}

/// A double-click on the window's empty top, as System Settings says (Desktop & Dock, "Double-click a window's title bar to").
pub fn title_bar_double_clicked(window: &tauri::WebviewWindow) {
    match defaults().stringForKey(&NSString::from_str("AppleActionOnDoubleClick")).map(|s| s.to_string()).as_deref() {
        Some("Minimize") => drop(window.minimize()),
        Some("None") => {}
        _ if window.is_maximized().unwrap_or(false) => drop(window.unmaximize()),
        _ => drop(window.maximize()),
    }
}

/// Tells the page when the window goes full screen and back (`title_bar_area`).
pub fn watch_full_screen(window: &tauri::WebviewWindow) {
    let last = std::sync::atomic::AtomicBool::new(window.is_fullscreen().unwrap_or(false));
    let w = window.clone();
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::Resized(_) = e {
            let now = w.is_fullscreen().unwrap_or(false);
            if last.swap(now, std::sync::atomic::Ordering::Relaxed) != now {
                let _ = w.eval(title_bar_area(now));
            }
        }
    });
}

pub fn start_tray(_: &tauri::App) -> tauri::Result<()> {
    menubar::start();
    Ok(())
}

/// This Mac's `<cli>` set up, if the window knows its network; the Mac app's menus, with its keys; the widgets' own
/// items, as last kept, until onet's answer redraws them.
pub fn start(app: &tauri::App) {
    super::desktop::start(app);
    if let Some(address) = crate::app().address() {
        // Off the main thread: a download, and launchctl, take seconds.
        std::thread::spawn(move || {
            if let Err(e) = set_up(&address) {
                device_logs::say(&format!("setting up {}: {e}", custom_build::config::CLI));
            }
        });
    }
    match app_menu(app.handle()) {
        Ok(menu) => drop(app.set_menu(menu)),
        Err(e) => device_logs::say(&format!("the app's menu: {e}")),
    }
    app.on_menu_event(|_, e| match e.id().as_ref() {
        "sidebar" | "join" => {
            show();
            command(e.id().as_ref());
        }
        "pair" => crate::main(Some(format!("{}://settings/devices?add=1", custom_build::config::CLI))),
        _ => {}
    });
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

/// SMAppServiceStatus's not registered and enabled.
const NOT_REGISTERED: isize = 0;
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
pub fn computer_use() -> bool {
    defaults().boolForKey(&NSString::from_str("computerUse"))
}

pub fn set_computer_use(on: bool) {
    defaults().setBool_forKey(on, &NSString::from_str("computerUse"));
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let done = std::process::Command::new("/bin/launchctl").args(args).output().map_err(|e| e.to_string())?;
    if done.status.success() { Ok(()) } else { Err(format!("launchctl {}: {}", args.join(" "), String::from_utf8_lossy(&done.stderr).trim())) }
}

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Sets up `<cli>` on this Mac, from the network at `address`, with no root: the newest for it where install.sh puts it
/// on Linux (`~/.local/bin`; a throwaway network's computer's in its home, as its programs put theirs), and its oned a
/// launch agent of the user's, which launchd starts as they log in and again whenever it stops, as `<cli> update` has
/// it. A program there already is only started. A link there is what builds before the window carried no background
/// program made, to the copy inside the app: it's replaced, and their launch agent, the app's own, goes.
pub fn set_up(address: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let cli = custom_build::config::CLI;
    let oned = format!("{cli}d");
    // A throwaway network's computer is its own, beside the Mac's.
    let throwaway = crate::env("THROWAWAY").is_some();
    let user = PathBuf::from(std::env::var_os("HOME").ok_or("This user has no home folder.")?);
    let bin = if throwaway { platform::home().join("bin") } else { user.join(".local/bin") };
    let program = bin.join(cli);
    if !std::fs::symlink_metadata(&program).is_ok_and(|m| m.is_file()) {
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
    let uid = unsafe { getuid() };
    // The launch agent the app carried before, by its id: its job and its record in Login Items go.
    let before = format!("{}.{oned}", crate::identifier());
    let agent: Retained<AnyObject> = unsafe { msg_send![class!(SMAppService), agentServiceWithPlistName: &*NSString::from_str(&format!("{before}.plist"))] };
    if status(&agent) != NOT_REGISTERED {
        unregister(&agent);
    }
    let _ = launchctl(&["bootout", &format!("gui/{uid}/{before}")]);
    // A throwaway network's oned is a launch agent of its own beside the Mac's, named for its home.
    let label = match throwaway {
        true => format!("app.{cli}.{oned}.throwaway-{}", &data_encoding::HEXLOWER.encode(&<sha2::Sha256 as sha2::Digest>::digest(platform::home().to_string_lossy().as_bytes()))[..8]),
        false => format!("app.{cli}.{oned}"),
    };
    let plist = if throwaway { platform::home() } else { user.join("Library/LaunchAgents") }.join(format!("{label}.plist"));
    // What says which network and home it's for: the environment `one-dev throwaway` gives its programs, or a custom
    // build's home, as the window was started with.
    let env: String = ["HOME", "ADDR", "THROWAWAY", "RELEASE_KEY"].iter().filter_map(|k| Some(format!("<key>{}</key><string>{}</string>", custom_build::config::env(k), crate::env(k)?))).collect();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array><string>{}</string></array><key>EnvironmentVariables</key><dict>{env}</dict><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ProcessType</key><string>Background</string></dict></plist>\n",
        bin.join(&oned).display()
    );
    let job = format!("gui/{uid}/{label}");
    if std::fs::read_to_string(&plist).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(plist.parent().unwrap_or(&plist)).map_err(|e| e.to_string())?;
        std::fs::write(&plist, &text).map_err(|e| e.to_string())?;
        let _ = launchctl(&["bootout", &job]);
    }
    if launchctl(&["print", &job]).is_err() {
        launchctl(&["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])?;
    }
    launchctl(&["kickstart", &job])
}

/// The app's menus, macOS's own with the Mac app's keys, as items of them so they work wherever the focus is (a key
/// pressed in a screen's frame never reaches the page): View's Fold Sidebar (⌘\), File's Pair a Computer… (⌘⇧I, signed
/// in) and Join with a Pairing Code… (⌘⇧J, while this Mac's node is in no network).
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let pair = MenuItem::with_id(app, "pair", "Pair a Computer…", false, Some("CmdOrCtrl+Shift+I"))?;
    let join = MenuItem::with_id(app, "join", "Join with a Pairing Code…", false, Some("CmdOrCtrl+Shift+J"))?;
    let about = AboutMetadata { name: Some(custom_build::config::NAME.into()), ..Default::default() };
    let menu = Menu::with_items(
        app,
        &[
            &Submenu::with_items(app, custom_build::config::NAME, true, &[&P::about(app, None, Some(about))?, &P::separator(app)?, &P::services(app, None)?, &P::separator(app)?, &P::hide(app, None)?, &P::hide_others(app, None)?, &P::separator(app)?, &P::quit(app, None)?])?,
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
pub fn let_in() {}

/// Links to the network's address open only the browser.
pub fn claim(_: Option<String>) {}
