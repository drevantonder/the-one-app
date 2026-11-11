//! The Mac's menu bar: the app's item and one for each widget on the user's pages (each kind once; its title the widget's
//! headline, at most 40 characters, and none, it hides), as real menus AppKit draws (mac/Sources/OneNativeMac), so their
//! look and behaviour are the system's. This says what shows, as `MenuState` there, and does what's chosen. Which
//! widgets' items the user hid is kept (`menubar-hidden`, one kind a line).

use std::ffi::{CStr, CString, c_char};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::app;

unsafe extern "C" {
    fn one_menu_start(mark: *const u8, length: isize);
    fn one_menu_update(json: *const c_char);
}

/// The items, showing what's kept, until the node's answer redraws them.
pub fn start() {
    let mark = include_bytes!(concat!(env!("OUT_DIR"), "/menubar.png"));
    // SAFETY: Swift copies the bytes before it returns.
    unsafe { one_menu_start(mark.as_ptr(), mark.len() as isize) };
    update();
}

fn hidden_file() -> std::path::PathBuf {
    app().data.join("menubar-hidden")
}

fn hidden() -> Vec<String> {
    std::fs::read_to_string(hidden_file()).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
}

fn set_hidden(kind: &str, hide: bool) {
    let mut kinds = hidden();
    kinds.retain(|k| k != kind);
    if hide {
        kinds.push(kind.to_string());
    }
    let _ = std::fs::write(hidden_file(), kinds.join("\n"));
    update();
}

/// The widgets, each kind once, in the kept cache's order (as arranged on the pages), as `MenuState.Item`.
fn items() -> Vec<Value> {
    let cache: Value = std::fs::read(super::widgets_dir().join("widgets.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let hidden = hidden();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for w in cache["widgets"].as_array().into_iter().flatten() {
        let text = |k: &str| w[k].as_str().unwrap_or_default().to_string();
        let kind = text("kind");
        if !seen.insert(kind.clone()) {
            continue;
        }
        let line = text("headline").lines().next().unwrap_or_default().trim().to_string();
        let line = if line.chars().count() > 40 { format!("{}…", line.chars().take(39).collect::<String>().trim_end()) } else { line };
        let label = w["appTitle"].as_str().map(str::to_string).unwrap_or_else(|| text("title"));
        out.push(json!({ "kind": kind, "title": text("title"), "label": label, "line": line, "hidden": hidden.contains(&kind) }));
    }
    out
}

/// The items again, as things are now. Off the main thread, since asking this Mac's node for its key can take seconds;
/// one at a time, so the last asked is the last drawn.
pub fn update() {
    static ONE: Mutex<()> = Mutex::new(());
    std::thread::spawn(|| {
        let _one = ONE.lock().unwrap();
        let stands = crate::here::stands().filter(|s| s["in"] == true);
        let state = json!({
            "name": custom_build::config::NAME,
            "signedIn": stands.is_some(),
            "here": stands.as_ref().and_then(|s| s["join"].as_str()).map(|j| j[..64].to_string()),
            "login": super::login().unwrap_or(false),
            "computerUse": super::computer_use(),
            "items": items(),
            "dir": super::widgets_dir(),
        });
        let Ok(state) = CString::new(state.to_string()) else { return };
        // SAFETY: Swift copies the string before it returns.
        unsafe { one_menu_update(state.as_ptr()) };
    });
}

/// What the user chose in a menu (`chose`, in Swift), or that one opened.
///
/// # Safety
/// `json` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_menu_chose(json: *const c_char) {
    let chose: Value = serde_json::from_str(&unsafe { CStr::from_ptr(json) }.to_string_lossy()).unwrap_or_default();
    let kind = chose["kind"].as_str().unwrap_or_default();
    match chose["do"].as_str().unwrap_or_default() {
        "open" => match chose["link"].as_str() {
            Some(link) if link.starts_with("http") => drop(crate::open(link.to_string())),
            link => crate::main(link.map(str::to_string)),
        },
        "devices" => crate::main(Some(format!("{}://settings/devices", custom_build::config::CLI))),
        "login" => {
            super::set_login(!super::login().unwrap_or(false));
            update();
        }
        "computer-use" => {
            super::set_computer_use(!super::computer_use());
            update();
        }
        "hide" => set_hidden(kind, true),
        "show" => set_hidden(kind, false),
        // What a widget's button or toggle does; its tool's run draws the widgets again.
        "run" => drop(tauri::async_runtime::spawn(async move {
            if let Some(error) = crate::headless::run_here("call", &chose).await["error"].as_str() {
                one::logs::say(&format!("a menu bar widget's button: {error}"));
            }
        })),
        // Its devices and widgets as they are now, drawn in as they come.
        "opened" => crate::changed(),
        "quit" => app().handle.exit(0),
        _ => {}
    }
}
