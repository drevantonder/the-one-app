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

/// The widgets, each kind once, in the kept cache's order (as arranged on the pages): `(kind, title, app, headline)`.
pub fn widgets() -> Vec<(String, String, String, String)> {
    let cache: Value = std::fs::read(crate::widgets_dir().join("widgets.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for w in cache["widgets"].as_array().into_iter().flatten() {
        let (app, widget) = (w["app"].as_str().unwrap_or_default(), w["widget"].as_str().unwrap_or_default());
        let kind = format!("{app}.{widget}");
        if !seen.insert(kind.clone()) {
            continue;
        }
        let line = if w["error"].is_null() { headline(&w["root"]).unwrap_or_default() } else { String::new() };
        let line = line.lines().next().unwrap_or_default().trim().to_string();
        let title = if line.chars().count() > 40 { format!("{}…", line.chars().take(39).collect::<String>().trim_end()) } else { line };
        out.push((kind, w["title"].as_str().unwrap_or(widget).to_string(), app.to_string(), title));
    }
    out
}

/// A tree's most prominent text (the biggest, then the first that isn't muted), as apple/Shared/Views/WidgetTree.swift's
/// `headline`.
fn headline(tree: &Value) -> Option<String> {
    fn texts(n: &Value, out: &mut Vec<(String, i32, bool)>) {
        match n["t"].as_str() {
            Some("text") => {
                let size = match n["size"].as_str() {
                    Some("sm") => 0,
                    Some("lg") => 2,
                    Some("xl") => 3,
                    _ => 1,
                };
                out.push((n["text"].as_str().unwrap_or_default().to_string(), size, n["tone"] == "muted"));
            }
            Some("stat") => {
                let value = match &n["value"] {
                    Value::String(s) => s.clone(),
                    Value::Number(v) => v.as_f64().map(|f| if f.fract() == 0.0 { format!("{f:.0}") } else { format!("{}", (f * 100.0).round() / 100.0) }).unwrap_or_default(),
                    _ => String::new(),
                };
                let mut text = [value, n["unit"].as_str().unwrap_or_default().to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                if let Some(label) = n["label"].as_str() {
                    text = format!("{text} {label}");
                }
                out.push((text, 4, false));
            }
            Some("item") => out.push((n["title"].as_str().unwrap_or_default().to_string(), 1, false)),
            _ => n["c"].as_array().into_iter().flatten().for_each(|c| texts(c, out)),
        }
    }
    let mut all = Vec::new();
    texts(tree, &mut all);
    let all: Vec<_> = all.into_iter().filter(|t| !t.0.is_empty()).collect();
    // The biggest; of those, one that isn't muted; of those, the first.
    let best = all.iter().enumerate().max_by_key(|(i, t)| (t.1, !t.2, -(*i as i64)))?;
    Some(best.1.0.clone())
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
    for (kind, title, app, line) in wanted {
        if shown.get(&kind) == Some(&line) {
            continue;
        }
        let tray = match handle.tray_by_id(&id(&kind)) {
            Some(t) => t,
            None => match made(handle, &kind, &title, &app) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("menu bar item {kind}: {e}");
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

fn made(handle: &AppHandle, kind: &str, title: &str, app: &str) -> tauri::Result<tauri::tray::TrayIcon> {
    let label = app_label(app).unwrap_or_else(|| title.to_string());
    let open = MenuItem::with_id(handle, format!("open {app}"), format!("Open {label}"), true, None::<&str>)?;
    let hide = MenuItem::with_id(handle, format!("hide {kind}"), "Hide from Menu Bar", true, None::<&str>)?;
    let menu = Menu::with_items(handle, &[&open, &hide])?;
    let kind = kind.to_string();
    TrayIconBuilder::with_id(id(&kind))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(move |_, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
                crate::toggle_widget(&kind, rect);
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
        .build(handle)
}

/// An app's name as people read it, from the apps the window last had (kept by the page's own cache isn't readable
/// here): the widget cache has each widget's title only, so the app's own name is asked of the node once.
fn app_label(app: &str) -> Option<String> {
    APPS.lock().unwrap().get(app).cloned()
}

static APPS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// The apps' names, as the node last listed them (src/widgets.rs asks it each refresh).
pub fn named(apps: &[Value]) {
    let mut named = APPS.lock().unwrap();
    for a in apps {
        if let (Some(name), Some(title)) = (a["name"].as_str(), a["title"].as_str()) {
            named.insert(name.to_string(), title.to_string());
        }
    }
}
