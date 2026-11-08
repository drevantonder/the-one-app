//! A computer's tray item whose click shows a small window of the web client's (Linux, Windows): the panel and where it
//! goes. A Mac's menu bar is AppKit's own (`menubar`).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::app;

/// The panel's size, as the Mac app's was wide.
const PANEL: (f64, f64) = (340.0, 560.0);

fn toggle_panel(at: Option<tauri::Rect>) {
    if let Some(p) = app().handle.get_webview_window("panel") {
        toggle_under(&p, at);
    }
}

/// A window like the panel's, shown where this computer shows its tray's flyouts, or hidden if it shows: by the item at
/// `at` (its place and size on the screen, physical) if given, else in the corner where the tray is.
fn toggle_under(p: &tauri::WebviewWindow, at: Option<tauri::Rect>) {
    if p.is_visible().unwrap_or(false) {
        let _ = p.hide();
        return;
    }
    let Some(m) = p.current_monitor().ok().flatten() else { return };
    let scale = m.scale_factor();
    // Its size as last fitted to its screen; a window never shown yet says none, so as made.
    let size = p.outer_size().ok().filter(|s| s.width > 0 && s.height > 0).unwrap_or(tauri::PhysicalSize::new((PANEL.0 * scale) as u32, (PANEL.1 * scale) as u32));
    // What shows of it is what's placed: Windows' window reaches past it, invisibly.
    let (left, top, right, bottom) = super::unseen(p);
    let seen = ((size.width as i32 - left - right).max(1) as u32, (size.height as i32 - top - bottom).max(1) as u32);
    let item = at.map(|at| (at.position.to_physical::<f64>(scale), at.size.to_physical::<f64>(scale)));
    let place = flyout(m.work_area(), (m.position().x, m.position().y, m.size().width, m.size().height), seen, item.map(|(p, s)| (p.x, p.y, s.width, s.height)), scale);
    let _ = p.set_position(tauri::PhysicalPosition::new(place.0 - left, place.1 - top));
    let _ = p.show();
    let _ = p.set_focus();
}

/// The bar the tray is on: which side of the screen it takes from the work area.
#[derive(Clone, Copy, PartialEq)]
enum Bar {
    Top,
    Bottom,
    Left,
    Right,
}

/// Where a flyout of `size` goes, all physical: inside the monitor's work area (`work`; the taskbar's or panel's room
/// left out of `screen`), a gap from the bar and the edges as the OS keeps its own (Windows 11's 12 px, a Mac's popover
/// just under the menu bar), centred on its tray `item` if it has one and that's on the bar, else in the bar's corner:
/// the end where the tray is (the right, or the bottom of a side bar). An item in Windows' overflow, above its taskbar,
/// is off the bar, so it goes in the corner, as Windows' own flyouts do.
fn flyout(work: &tauri::PhysicalRect<i32, u32>, screen: (i32, i32, u32, u32), size: (u32, u32), item: Option<(f64, f64, f64, f64)>, scale: f64) -> (i32, i32) {
    let gap = scale * super::FLYOUT_GAP;
    let (wx, wy, ww, wh) = (work.position.x as f64, work.position.y as f64, work.size.width as f64, work.size.height as f64);
    let (sx, sy, sw, sh) = (screen.0 as f64, screen.1 as f64, screen.2 as f64, screen.3 as f64);
    let (w, h) = (size.0 as f64, size.1 as f64);
    // The side the work area gives up, else the item's side of the screen, else where this OS keeps it.
    let bar = if wy > sy {
        Bar::Top
    } else if wy + wh < sy + sh {
        Bar::Bottom
    } else if wx > sx {
        Bar::Left
    } else if wx + ww < sx + sw {
        Bar::Right
    } else if let Some((_, iy, _, _)) = item {
        if iy < sy + sh / 2.0 { Bar::Top } else { Bar::Bottom }
    } else if super::TRAY_AT_BOTTOM {
        Bar::Bottom
    } else {
        Bar::Top
    };
    let on_bar = item.filter(|&(ix, iy, iw, ih)| match bar {
        Bar::Top => iy < wy,
        Bar::Bottom => iy + ih > wy + wh,
        Bar::Left => ix < wx,
        Bar::Right => ix + iw > wx + ww,
    });
    let (min_x, max_x) = (wx + gap, (wx + ww - w - gap).max(wx + gap));
    let (min_y, max_y) = (wy + gap, (wy + wh - h - gap).max(wy + gap));
    let (x, y) = match (bar, on_bar) {
        (Bar::Top, Some((ix, _, iw, _))) => (ix + iw / 2.0 - w / 2.0, min_y),
        (Bar::Bottom, Some((ix, _, iw, _))) => (ix + iw / 2.0 - w / 2.0, max_y),
        (Bar::Left, Some((_, iy, _, ih))) => (min_x, iy + ih / 2.0 - h / 2.0),
        (Bar::Right, Some((_, iy, _, ih))) => (max_x, iy + ih / 2.0 - h / 2.0),
        (Bar::Top, None) => (max_x, min_y),
        (Bar::Bottom, None) => (max_x, max_y),
        (Bar::Left, None) => (min_x, max_y),
        (Bar::Right, None) => (max_x, max_y),
    };
    (x.clamp(min_x, max_x).round() as i32, y.clamp(min_y, max_y).round() as i32)
}

/// The panel this tall, as its screen is: under its item it grows down, over one (a taskbar) up.
pub fn fit(window: &tauri::WebviewWindow, height: f64) {
    if window.label() != "panel" {
        return;
    }
    let p = window;
    let height = height.clamp(160.0, 900.0);
    let scale = p.scale_factor().unwrap_or(1.0);
    let was = p.outer_size().map(|s| s.height as f64).unwrap_or(PANEL.1 * scale);
    let _ = p.set_size(tauri::LogicalSize::new(PANEL.0, height));
    if let (Ok(at), Some(m)) = (p.outer_position(), p.current_monitor().ok().flatten())
        && (at.y as f64) > m.position().y as f64 + m.size().height as f64 / 2.0
    {
        let _ = p.set_position(tauri::PhysicalPosition::new(at.x, at.y + (was - height * scale) as i32));
    }
}

/// A window like a menu bar item's popover, of this page at `page`: hidden until its item's clicked, gone when it loses
/// focus.
fn popup(handle: &AppHandle, label: &str, page: &str) -> tauri::Result<tauri::WebviewWindow> {
    let window = WebviewWindowBuilder::new(handle, label, WebviewUrl::App(page.into()))
        .title(one::config::NAME)
        .initialization_script(include_str!("../native.js"))
        .use_https_scheme(true)
        .inner_size(PANEL.0, PANEL.1)
        // Sized by its screen (`fit`), which GTK refuses a window that isn't resizable; with no edges to drag, a user
        // can't.
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        // Windows' system shadow, as its flyouts have.
        .shadow(true)
        .build()?;
    super::appear(&window);
    super::round(&window);
    let hide = window.clone();
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::Focused(false) = e {
            let _ = hide.hide();
        }
    });
    Ok(window)
}

/// The tray item: a click shows the panel (Linux's tray items show only their menu); its menu has Open, Open at Login
/// and Quit. The panel is this page in panel mode, hidden until the item's clicked, gone when it loses focus.
pub fn start_tray(app: &tauri::App) -> tauri::Result<()> {
    let tray = tauri::tray::TrayIconBuilder::with_id("main")
        .tooltip(one::config::NAME)
        .show_menu_on_left_click(cfg!(target_os = "linux"))
        .on_tray_icon_event(|_, e| {
            if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, rect, .. } = e {
                toggle_panel(Some(rect));
            }
        })
        .on_menu_event(move |app, e| match e.id().as_ref() {
            "open" => super::show(),
            "login" => {
                super::set_login(!super::login().unwrap_or(true));
                tray_menu(app);
            }
            "quit" => app.exit(0),
            _ => {}
        });
    let tray = match app.default_window_icon() {
        Some(icon) => tray.icon(icon.clone()),
        None => tray,
    };
    tray.build(app)?;
    tray_menu(app.handle());
    popup(app.handle(), "panel", "index.html?panel")?;
    Ok(())
}

/// The tray item's menu, as things are now.
fn tray_menu(app: &AppHandle) {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    let Some(tray) = app.tray_by_id("main") else { return };
    let made = || -> tauri::Result<Menu<tauri::Wry>> {
        Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, "open", format!("Open {}", one::config::NAME), true, None::<&str>)?,
                &CheckMenuItem::with_id(app, "login", "Open at Login", true, super::login().unwrap_or(false), None::<&str>)?,
                &PredefinedMenuItem::separator(app)?,
                &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
            ],
        )
    };
    match made() {
        Ok(menu) => drop(tray.set_menu(Some(menu))),
        Err(e) => one::logs::say(&format!("the tray's menu: {e}")),
    }
}
