//! A computer's windows have no title band: the page runs to the window's top, and the system's own window controls
//! sit over it, drawn and handled by the system. The page keeps the title bar's area clear of what it shows, drags
//! the window by it, and puts what goes at its start after the controls there (`titleBar` in client.js).
//!
//! - A Mac keeps its traffic lights over the page (Tauri's `TitleBarStyle::Overlay`, tao's full-size content view).
//! - Windows keeps its frame and its own minimize, maximize and close, with Snap Layouts, as Chrome and Edge do:
//!   the caption is taken off (`WM_NCCALCSIZE`), the frame reaches into the window (`DwmExtendFrameIntoClientArea`),
//!   where Desktop Window Manager draws the caption buttons and hit-tests them (`DwmDefWindowProc`), and the web
//!   view leaves them uncovered. WebView2 drags the window by the page's `app-region: drag`.
//! - Linux has a GNOME header bar (client-side decorations): GTK's own window buttons, on whichever side the desktop
//!   puts them (`gtk-decoration-layout`), over the page, which takes every other click.

/// Where the title bar is over the page, in CSS pixels: its height, where what goes at its start goes, a little
/// after any of the window's own controls there, and how much of its end the window's own controls take.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub height: f64,
    pub start: f64,
    pub end: f64,
}

/// How far what goes at the title bar's start is from the window's edge, or from the window's controls before it.
const GAP: f64 = 8.0;

/// How tall the title bar is: the page's top bar is in it, as a unified toolbar is (macOS's System Settings,
/// Raycast), 52 points, which a Mac's traffic lights and Linux's window buttons sit in the middle of.
pub const HEIGHT: f64 = 52.0;

/// Where a Mac's traffic lights are in a title bar of `HEIGHT`, as AppKit puts them in a unified toolbar: their
/// first's left edge, and how far below the window's top they start, so they're in the middle of it.
#[cfg(target_os = "macos")]
pub const LIGHTS: (f64, f64) = (20.0, (HEIGHT - 14.0) / 2.0);

impl From<Area> for serde_json::Value {
    fn from(Area { height, start, end }: Area) -> Self {
        serde_json::json!({"height": height, "start": start, "end": end})
    }
}

/// A Mac's: the traffic lights at the start of a unified toolbar's title bar, three of 14 points, 20 apart.
#[cfg(target_os = "macos")]
pub const MAC: Area = Area { height: HEIGHT, start: LIGHTS.0 + 2.0 * 20.0 + 14.0 + GAP, end: 0.0 };

#[cfg(windows)]
pub use self::windows::extend;

#[cfg(windows)]
mod windows {
    use std::cell::Cell;

    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_CAPTION_BUTTON_BOUNDS, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmDefWindowProc, DwmExtendFrameIntoClientArea,
        DwmGetWindowAttribute, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, ClientToScreen, CombineRgn, CreateRectRgn, DeleteObject, FillRect, GetStockObject, HBRUSH, HDC, RGN_DIFF, SetWindowRgn};
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetClientRect, GetWindowRect, HTCLIENT, HTTOP, IsZoomed, NCCALCSIZE_PARAMS, RegisterWindowMessageA, SM_CXPADDEDBORDER,
        SM_CYCAPTION, SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_ERASEBKGND,
        WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_SETTINGCHANGE, WM_SIZE,
    };
    use windows::core::{BOOL, s, w};

    use super::Area;

    struct State {
        changed: Box<dyn Fn(Area)>,
        area: Cell<Area>,
        /// What tao sends a window as its theme changes (`Window::set_theme`), as it has DWM draw its frame light or
        /// dark: the caption buttons' background follows it.
        theme: u32,
    }

    /// Takes the title band off `window`, keeping Windows' own caption buttons, and tells `changed` where the title
    /// bar is, now and as it changes. `window` is a tao window with a wry web view.
    pub fn extend(window: isize, changed: impl Fn(Area) + 'static) -> ::windows::core::Result<()> {
        let hwnd = HWND(window as _);
        let theme = unsafe { RegisterWindowMessageA(s!("Tao::ChangeTheme")) };
        let state = Box::into_raw(Box::new(State { changed: Box::new(changed), area: Cell::default(), theme }));
        unsafe {
            SetWindowSubclass(hwnd, Some(subclass), 1, state as usize).ok()?;
            frame(hwnd);
            colour(hwnd);
            SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)?;
            uncover(hwnd, &*state);
        }
        Ok(())
    }

    unsafe extern "system" fn subclass(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _: usize, state: usize) -> LRESULT {
        unsafe {
            // The caption buttons: drawn, hovered, pressed and hit-tested by DWM, which shows Snap Layouts over
            // maximize.
            let mut answer = LRESULT(0);
            if DwmDefWindowProc(hwnd, message, wparam, lparam, &mut answer).as_bool() {
                return answer;
            }
            let state = &*(state as *const State);
            match message {
                // The caption goes: the window's client area starts at its top. A maximized window reaches past the
                // screen's edge by its border, which is kept off.
                WM_NCCALCSIZE if wparam.0 != 0 => {
                    let rect = &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0];
                    let top = rect.top;
                    let answer = DefSubclassProc(hwnd, message, wparam, lparam);
                    rect.top = top + if IsZoomed(hwnd).as_bool() { border(hwnd) } else { 0 };
                    answer
                }
                // Its top edge still resizes it, where the web view leaves it uncovered (`uncover`).
                WM_NCHITTEST => {
                    let hit = DefSubclassProc(hwnd, message, wparam, lparam);
                    let mut window = RECT::default();
                    let y = (lparam.0 >> 16) as i16 as i32;
                    if hit.0 == HTCLIENT as isize && !IsZoomed(hwnd).as_bool() && GetWindowRect(hwnd, &mut window).is_ok() && y < window.top + border(hwnd) {
                        return LRESULT(HTTOP as isize);
                    }
                    hit
                }
                // Black is where DWM draws its frame.
                WM_ERASEBKGND => {
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    FillRect(HDC(wparam.0 as _), &client, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    LRESULT(1)
                }
                WM_NCDESTROY => {
                    let answer = DefSubclassProc(hwnd, message, wparam, lparam);
                    let _ = RemoveWindowSubclass(hwnd, Some(subclass), 1);
                    drop(Box::from_raw(state as *const State as *mut State));
                    answer
                }
                _ => {
                    let answer = DefSubclassProc(hwnd, message, wparam, lparam);
                    if message == WM_SIZE {
                        frame(hwnd);
                        uncover(hwnd, state);
                    } else if message == WM_SETTINGCHANGE || message == state.theme {
                        colour(hwnd);
                    }
                    answer
                }
            }
        }
    }

    /// The window's top border, by which it's resized: its frame and padding, at its DPI.
    unsafe fn border(hwnd: HWND) -> i32 {
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    /// The frame reaches a title bar's height into the window, where DWM draws the caption buttons.
    unsafe fn frame(hwnd: HWND) {
        unsafe {
            let height = GetSystemMetricsForDpi(SM_CYCAPTION, GetDpiForWindow(hwnd)) + border(hwnd);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cyTopHeight: height, ..Default::default() });
        }
    }

    /// Behind the caption buttons is the page's background (`--background`), light or dark as DWM draws the window.
    /// Windows 11's; Windows 10 draws its own frame colour there.
    unsafe fn colour(hwnd: HWND) {
        unsafe {
            let mut dark = BOOL(0);
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw mut dark).cast(), size_of::<BOOL>() as u32);
            let colour = COLORREF(if dark.as_bool() { 0x000a_0a0a } else { 0x00ff_ffff });
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, (&raw const colour).cast(), size_of::<COLORREF>() as u32);
        }
    }

    /// The web view covers the window but its caption buttons and, unless it's maximized, its top border, which
    /// resizes it. Tells `changed` where the title bar is, if that changed.
    unsafe fn uncover(hwnd: HWND, state: &State) {
        unsafe {
            let (mut client, mut window, mut buttons, mut origin) = (RECT::default(), RECT::default(), RECT::default(), POINT::default());
            let measured = GetClientRect(hwnd, &mut client).is_ok()
                && GetWindowRect(hwnd, &mut window).is_ok()
                && ClientToScreen(hwnd, &mut origin).as_bool()
                && DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, (&raw mut buttons).cast(), size_of::<RECT>() as u32).is_ok();
            if !measured {
                return;
            }
            // DWM says where the buttons are in the window, the web view is in its client area.
            let (x, y) = (window.left - origin.x, window.top - origin.y);
            let buttons = RECT { left: buttons.left + x, top: buttons.top + y, right: buttons.right + x, bottom: buttons.bottom + y };
            if let Ok(webview) = FindWindowExW(Some(hwnd), None, w!("WRY_WEBVIEW"), None) {
                let shown = CreateRectRgn(0, 0, client.right, client.bottom);
                let mut cuts = vec![CreateRectRgn(buttons.left, buttons.top, buttons.right, buttons.bottom)];
                if !IsZoomed(hwnd).as_bool() {
                    cuts.push(CreateRectRgn(0, 0, client.right, border(hwnd)));
                }
                for cut in cuts {
                    CombineRgn(Some(shown), Some(shown), Some(cut), RGN_DIFF);
                    let _ = DeleteObject(cut.into());
                }
                // The window owns the region from here.
                SetWindowRgn(webview, Some(shown), true);
            }
            let scale = f64::from(GetDpiForWindow(hwnd)) / 96.0;
            let area = Area { height: (f64::from(buttons.bottom) / scale).max(super::HEIGHT), start: super::GAP, end: f64::from(client.right - buttons.left) / scale };
            if area != state.area.replace(area) {
                (state.changed)(area);
            }
        }
    }
}

/// Puts a GNOME header bar over `webview`, in its window: GTK's own window buttons, as the desktop lays them out,
/// over the page, which takes every other click (an overlay that passes them through). It tells `changed` where
/// the title bar is, now and as it changes. Call before the window is shown. → The overlay the page is in, where a
/// page loaded again goes too.
#[cfg(target_os = "linux")]
pub fn header_bar(
    window: &gtk::Window,
    webview: &impl gtk::glib::IsA<gtk::Widget>,
    changed: impl Fn(Area) + 'static,
) -> gtk::Overlay {
    use std::cell::Cell;

    use gtk::prelude::*;

    let overlay = gtk::Overlay::new();
    if let Some(parent) = webview.parent().and_then(|parent| parent.downcast::<gtk::Container>().ok()) {
        parent.remove(webview);
        match parent.downcast_ref::<gtk::Box>() {
            Some(column) => column.pack_start(&overlay, true, true, 0),
            None => parent.add(&overlay),
        }
    }
    overlay.add(webview);
    let header = gtk::HeaderBar::builder().show_close_button(true).valign(gtk::Align::Start).height_request(HEIGHT as i32).build();
    let style = gtk::CssProvider::new();
    let _ = style.load_from_data(b"headerbar { background: none; border: none; box-shadow: none; }");
    header.style_context().add_provider(&style, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    overlay.add_overlay(&header);
    overlay.set_overlay_pass_through(&header, true);
    overlay.show_all();
    // The window draws its own decorations (client-side), with no title band of its own: its title bar is an empty
    // one that's never shown, even as the window shows everything in it.
    let none = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    none.set_no_show_all(true);
    window.set_titlebar(Some(&none));

    // GTK's header bar puts the window buttons on each side in a box of its own, styled `left` or `right`.
    let last = Cell::new(Area::default());
    header.connect_size_allocate(move |header, allocation| {
        let mut area = Area { height: f64::from(allocation.height()), start: GAP, end: 0.0 };
        header.forall(|child| {
            let (at, context) = (child.allocation(), child.style_context());
            if context.has_class("left") && child.is_visible() {
                area.start = f64::from(at.x() + at.width() - allocation.x()) + GAP;
            } else if context.has_class("right") && child.is_visible() {
                area.end = f64::from(allocation.x() + allocation.width() - at.x());
            }
        });
        if area != last.replace(area) {
            changed(area);
        }
    });
    overlay
}

/// Does what a double-click on a Linux window's title bar does, as the desktop says (`gtk-titlebar-double-click`):
/// maximize or restore, which is what's set until the person changes it, minimize, lower or nothing. The page's top
/// is double-clicked, which GTK's header bar passes through.
#[cfg(target_os = "linux")]
pub fn double_clicked(window: &gtk::Window) {
    use gtk::prelude::*;
    let action = gtk::Settings::default().and_then(|settings| settings.property::<Option<String>>("gtk-titlebar-double-click"));
    match action.as_deref() {
        Some("minimize") => window.iconify(),
        Some("lower") => window.window().into_iter().for_each(|window| window.lower()),
        Some("none" | "menu") => {}
        _ if window.is_maximized() => window.unmaximize(),
        _ => window.maximize(),
    }
}
