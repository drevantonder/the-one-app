//! The Mac's menu bar items of the person's widgets, one for each widget on their pages (each kind once), as the old Mac
//! app had them: its title the widget's headline (at most 40 characters; none, and it hides), a click showing the widget
//! under it (the panel's window, showing only that widget), a right click Open <app> and Hide from Menu Bar. Which the
//! person hid is kept (`menubar-hidden`, one kind a line); the main item's right-click menu shows them again.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::app;

/// The items there are, by kind, with the title each shows.
static SHOWN: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn hidden_file() -> std::path::PathBuf {
    app().data.join("menubar-hidden")
}

pub fn hidden() -> Vec<String> {
    std::fs::read_to_string(hidden_file()).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
}

pub fn set_hidden(kind: &str, hide: bool) {
    let mut kinds = hidden();
    kinds.retain(|k| k != kind);
    if hide {
        kinds.push(kind.to_string());
    }
    let _ = std::fs::write(hidden_file(), kinds.join("\n"));
    update();
}

/// The widgets, each kind once, in the kept cache's order (as arranged on the pages): `(kind, title, app, app's title,
/// line)`, the line its headline's first, at most 40 characters.
pub fn widgets() -> Vec<(String, String, String, String, String)> {
    let cache: Value = std::fs::read(super::widgets_dir().join("widgets.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for w in cache["widgets"].as_array().into_iter().flatten() {
        let text = |k: &str| w[k].as_str().unwrap_or_default().to_string();
        if !seen.insert(text("kind")) {
            continue;
        }
        let line = text("headline").lines().next().unwrap_or_default().trim().to_string();
        let line = if line.chars().count() > 40 { format!("{}…", line.chars().take(39).collect::<String>().trim_end()) } else { line };
        let label = w["appTitle"].as_str().map(str::to_string).unwrap_or_else(|| text("title"));
        out.push((text("kind"), text("title"), text("app"), label, line));
    }
    out
}

fn id(kind: &str) -> String {
    format!("widget.{kind}")
}

/// The items again, as the kept widgets now are: made, retitled, or gone.
pub fn update() {
    let handle = app().handle.clone();
    let _ = app().handle.run_on_main_thread(move || draw(&handle));
}

fn draw(handle: &AppHandle) {
    let hidden = hidden();
    let wanted: Vec<_> = widgets().into_iter().filter(|w| !hidden.contains(&w.0)).collect();
    let mut shown = SHOWN.lock().unwrap();
    let gone: Vec<String> = shown.keys().filter(|k| !wanted.iter().any(|w| &w.0 == *k)).cloned().collect();
    for kind in gone {
        handle.remove_tray_by_id(&id(&kind));
        shown.remove(&kind);
    }
    for (kind, title, app, label, line) in wanted {
        if shown.get(&kind) == Some(&line) {
            continue;
        }
        let tray = match handle.tray_by_id(&id(&kind)) {
            Some(t) => t,
            None => match made(handle, &kind, &label, &app) {
                Ok(t) => t,
                Err(e) => {
                    one::logs::say(&format!("menu bar item {kind}: {e}"));
                    continue;
                }
            },
        };
        let _ = tray.set_title(Some(&line));
        let _ = tray.set_tooltip(Some(&title));
        let _ = tray.set_visible(!line.is_empty());
        shown.insert(kind, line);
    }
}

fn made(handle: &AppHandle, kind: &str, label: &str, app: &str) -> tauri::Result<tauri::tray::TrayIcon> {
    let open = MenuItem::with_id(handle, format!("open {app}"), format!("Open {label}"), true, None::<&str>)?;
    let hide = MenuItem::with_id(handle, format!("hide {kind}"), "Hide from Menu Bar", true, None::<&str>)?;
    let menu = Menu::with_items(handle, &[&open, &hide])?;
    let (kind, clicked) = (kind.to_string(), kind.to_string());
    let tray = TrayIconBuilder::with_id(id(&kind))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |_, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
                toggle_widget(&clicked, rect);
            }
        })
        .on_menu_event(|_, e| {
            let id = e.id().as_ref();
            if let Some(app) = id.strip_prefix("open ") {
                crate::main(Some(format!("{}://open?app={app}", one::config::CLI)));
            } else if let Some(kind) = id.strip_prefix("hide ") {
                set_hidden(kind, true);
            }
        })
        .build(handle)?;
    // Where the person dragged it in the menu bar, kept by macOS under its name, as the old app's was.
    let name = id(&kind);
    let _ = tray.with_inner_tray_icon(move |inner| {
        if let Some(item) = inner.ns_status_item() {
            let name = objc2_foundation::NSString::from_str(&name);
            let _: () = unsafe { objc2::msg_send![&*item, setAutosaveName: &*name] };
        }
    });
    Ok(tray)
}

/// A widget's menu bar item clicked: the widget under it (the panel's page, showing only that widget, in a window of
/// its own), or gone if it shows.
fn toggle_widget(kind: &str, at: tauri::Rect) {
    let handle = &app().handle;
    let page = format!("index.html?panel&widget={}", url::form_urlencoded::byte_serialize(kind.as_bytes()).collect::<String>());
    let window = match handle.get_webview_window("widget") {
        Some(w) => {
            if !w.url().is_ok_and(|u| u.as_str().ends_with(&page)) {
                let _ = w.hide();
                let _ = w.navigate(tauri::Url::parse(&format!("https://tauri.localhost/{page}")).unwrap());
            }
            w
        }
        None => match crate::popup(handle, "widget", &page) {
            Ok(w) => w,
            Err(e) => return one::logs::say(&format!("the widget's window: {e}")),
        },
    };
    crate::toggle_under(&window, Some(at));
}

/// One of the app menu's commands, to the window's page, which does it: "sidebar" (fold it or unfold it), "join".
fn command(name: &str) {
    if let Some(w) = app().handle.get_webview_window("main") {
        let _ = w.eval(format!("window.__oneNative?.command({})", json!(name)));
    }
}
