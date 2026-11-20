// Each window's procedure: what Windows says to it, as input.

use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ScreenToClient, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::System::SystemServices::MK_SHIFT;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::geometry::{DevicePixels, Point, Size};
use crate::platform::Input;
use super::keyboard;
use super::text_input::{text_input_has_client, text_input_ime, text_input_text};
use super::{Display, State, WINDOW_CLASS_NAME, caption, colour_frame, screen_name};

/// Sent as the pointer leaves the non-client area `TrackMouseEvent` was asked about (winuser.h's `WM_NCMOUSELEAVE`).
const WM_NCMOUSELEAVE: u32 = 0x02A2;

pub(super) fn register_window_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        // The program's own icon, its first, as Explorer shows it.
        let icon = LoadIconW(Some(module.into()), PCWSTR(1 as _)).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_procedure),
            hIcon: icon,
            lpszClassName: WINDOW_CLASS_NAME,
            style: CS_HREDRAW | CS_VREDRAW,
            hInstance: module.into(),
            ..Default::default()
        };
        RegisterClassW(&class);
    });
}

unsafe extern "system" fn window_procedure(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        handle_msg(hwnd, &*state, msg, wparam, lparam).map_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam), LRESULT)
    }
}

/// Keeps where an app's window is, its placement as it is when not maximized and whether it is, in its frame file.
fn keep_frame(hwnd: HWND, state: &State) {
    let Some(file) = &state.frame else { return };
    let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return;
    }
    let normal = placement.rcNormalPosition;
    let maximized = placement.showCmd == SW_SHOWMAXIMIZED.0 as u32;
    let now = crate::platform::Frame { x: normal.left, y: normal.top, width: normal.right - normal.left, height: normal.bottom - normal.top, maximized };
    if crate::platform::Frame::read(file) != Some(now) {
        now.write(file);
    }
}

fn loword(value: isize) -> i32 {
    (value & 0xffff) as i16 as i32
}

fn hiword(value: isize) -> i32 {
    ((value >> 16) & 0xffff) as i16 as i32
}

/// Where in its window a point is, in pixels from the client's top left, when it has no frame of the system's:
/// its sides and bottom keep the system's own borders, which it asks (`DefWindowProc`); its top edge and corners, and
/// the caption buttons and the rest of the title row, are its own.
fn hit(hwnd: HWND, state: &State, point: POINT, default: isize, scale: f32) -> isize {
    let (x, y) = (point.x as f32 / scale, point.y as f32 / scale);
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let width = (client.right - client.left) as f32 / scale;
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    // The window's edge, which resizes it where it isn't maximized: its top and the corners, since the system's
    // are on the sides and bottom.
    const EDGE: f32 = 5.0;
    if !zoomed && matches!(default as u32, HTLEFT | HTRIGHT | HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT) {
        return default;
    }
    if !zoomed && y < EDGE {
        return match (x < EDGE, x >= width - EDGE) {
            (true, _) => HTTOPLEFT as isize,
            (_, true) => HTTOPRIGHT as isize,
            _ => HTTOP as isize,
        };
    }
    let row = state.row.borrow();
    let height = row.as_ref().map_or(0.0, |row| row.height);
    if y < height {
        // Its caption buttons, flush with the window's end: minimize, maximize, close.
        let from_end = width - x;
        if from_end >= 0.0 && from_end < 3.0 * crate::view::CAPTION_BUTTON {
            return [HTCLOSE, HTMAXBUTTON, HTMINBUTTON][(from_end / crate::view::CAPTION_BUTTON) as usize] as isize;
        }
        if row.as_ref().is_some_and(|row| row.moves(Point { x, y })) {
            return HTCAPTION as isize;
        }
    }
    HTCLIENT as isize
}

/// What a message is as input, sent to the queue; → what the procedure answers, or `None` for Windows' own answer.
fn handle_msg(hwnd: HWND, state: &State, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
    let push = |input: Input| state.queue.borrow_mut().push((state.id, state.part, input));
    let scale = || unsafe { GetDpiForWindow(hwnd) } as f32 / USER_DEFAULT_SCREEN_DPI as f32;
    let at = |x: i32, y: i32| Point { x: x as f32 / scale(), y: y as f32 / scale() };
    match msg {
        WM_SIZE => {
            if wparam.0 as u32 != SIZE_MINIMIZED {
                push(Input::Resized(Size { width: DevicePixels(loword(lparam.0)), height: DevicePixels(hiword(lparam.0)) }));
            }
            // Maximized or back, it's kept as it is.
            if matches!(wparam.0 as u32, SIZE_MAXIMIZED | SIZE_RESTORED) {
                keep_frame(hwnd, state);
            }
            Some(0)
        }
        // Files dropped on it from Explorer, where they were dropped.
        WM_DROPFILES => {
            use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, DragQueryPoint, HDROP};
            let drop = HDROP(wparam.0 as *mut _);
            let mut point = POINT::default();
            let files = unsafe {
                let _ = DragQueryPoint(drop, &mut point);
                let count = DragQueryFileW(drop, u32::MAX, None);
                let files: Vec<std::path::PathBuf> = (0..count)
                    .map(|at| {
                        let mut name = vec![0u16; DragQueryFileW(drop, at, None) as usize + 1];
                        let length = DragQueryFileW(drop, at, Some(&mut name)) as usize;
                        std::path::PathBuf::from(String::from_utf16_lossy(&name[..length]))
                    })
                    .collect();
                DragFinish(drop);
                files
            };
            push(Input::Dropped(at(point.x, point.y), files));
            Some(0)
        }
        // Moved or resized by a person, it's kept where it was left.
        WM_EXITSIZEMOVE if !state.panel => {
            keep_frame(hwnd, state);
            None
        }
        WM_DPICHANGED => {
            // Where Windows suggests, at the new DPI, which sends WM_SIZE.
            let rect = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(hwnd, None, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            push(Input::Expose);
            Some(0)
        }
        WM_PAINT => {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            push(Input::Expose);
            Some(0)
        }
        WM_ERASEBKGND => Some(1),
        // An app's window is made no smaller than its smallest.
        WM_GETMINMAXINFO if !state.panel => {
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            let smallest = crate::platform::SMALLEST;
            let mut frame = RECT { left: 0, top: 0, right: (smallest.width.0 * scale()).round() as i32, bottom: (smallest.height.0 * scale()).round() as i32 };
            unsafe {
                let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
                let _ = AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32), GetDpiForWindow(hwnd));
            }
            // With no caption of the system's, its page's title row is what's above it.
            let bare = if state.unframed.get() { caption(hwnd) } else { 0 };
            info.ptMinTrackSize = POINT { x: frame.right - frame.left, y: frame.bottom - frame.top - bare };
            Some(0)
        }
        WM_CLOSE => {
            push(Input::Close);
            Some(0)
        }
        WM_MOUSEMOVE => {
            if !state.tracking.replace(true) {
                let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            push(Input::Moved(at(loword(lparam.0), hiword(lparam.0))));
            Some(0)
        }
        // With no frame of the system's, its page is to its top: its client area is all of it but the borders on its
        // sides and bottom, and what's at its top is told by where its title row is.
        WM_NCCALCSIZE if state.unframed.get() && wparam.0 != 0 => {
            let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            params.rgrc[0].top = top;
            // Maximized, the window is as far over the screen's top as its frame is thick.
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                params.rgrc[0].top += unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
            }
            Some(0)
        }
        WM_NCHITTEST if state.unframed.get() => {
            let default = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }.0;
            let mut point = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            Some(hit(hwnd, state, point, default, scale()))
        }
        // Its caption buttons are pressed in the non-client area, as Windows' are, which is what has Snap Layouts open
        // over the maximize one; they do what they say as they're let go.
        WM_NCLBUTTONDOWN if state.unframed.get() && matches!(wparam.0 as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) => Some(0),
        WM_NCLBUTTONUP if state.unframed.get() && matches!(wparam.0 as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) => {
            let command = match wparam.0 as u32 {
                HTMINBUTTON => SC_MINIMIZE,
                HTMAXBUTTON if unsafe { IsZoomed(hwnd) }.as_bool() => SC_RESTORE,
                HTMAXBUTTON => SC_MAXIMIZE,
                _ => SC_CLOSE,
            };
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
            }
            Some(0)
        }
        // The pointer over a caption button, which the page's window draws lit, moves in the non-client area.
        WM_NCMOUSEMOVE if state.unframed.get() => {
            if !state.tracking_caption.replace(true) {
                let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            let mut point = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            push(Input::Moved(at(point.x, point.y)));
            None
        }
        WM_NCMOUSELEAVE if state.unframed.get() => {
            state.tracking_caption.set(false);
            push(Input::Left);
            None
        }
        // Where a panel's page says a press moves it, the press is on its caption, which Windows moves it by.
        WM_NCHITTEST if state.panel => {
            let mut point = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            (state.part == 0 && state.row.borrow().as_ref().is_some_and(|row| row.moves(state.in_page(at(point.x, point.y))))).then_some(HTCAPTION as isize)
        }
        // Its parts move with it.
        WM_WINDOWPOSCHANGED if state.part == 0 && !state.parts.borrow().is_empty() => {
            if unsafe { &*(lparam.0 as *const WINDOWPOS) }.flags.0 & SWP_NOMOVE.0 == 0 {
                Display::follow(hwnd, state);
            }
            None
        }
        // A panel's part never takes the keyboard from it, clicked or not.
        WM_MOUSEACTIVATE if state.part > 0 => Some(MA_NOACTIVATE as isize),
        // A panel a person moved is told where it was left, on which monitor.
        WM_EXITSIZEMOVE if state.panel => {
            use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let mut frame = RECT::default();
            unsafe {
                let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
                let _ = GetWindowRect(hwnd, &mut frame);
            }
            let work = info.rcWork;
            push(Input::Placed(screen_name(&work), Point { x: (frame.left - work.left) as f32 / scale(), y: (frame.top - work.top) as f32 / scale() }));
            None
        }
        WM_MOUSELEAVE => {
            state.tracking.set(false);
            push(Input::Left);
            Some(0)
        }
        // Where a panel's page says a press moves it, a press on one of its parts moves the panel, by its caption.
        WM_LBUTTONDOWN if state.part > 0 && state.row.borrow().as_ref().is_some_and(|row| row.moves(state.in_page(at(loword(lparam.0), hiword(lparam.0))))) => {
            let panel = unsafe { GetWindow(hwnd, GW_OWNER) }.ok()?;
            let mut point = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut point);
                let _ = ReleaseCapture();
                SendMessageW(panel, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(((point.y as u16 as isize) << 16) | point.x as u16 as isize)));
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN => {
            unsafe { SetCapture(hwnd) };
            let button = match msg {
                WM_LBUTTONDOWN => 0,
                WM_MBUTTONDOWN => 1,
                _ => 2,
            };
            let shift = wparam.0 as u32 & MK_SHIFT.0 != 0;
            push(Input::Down(at(loword(lparam.0), hiword(lparam.0)), button, shift));
            Some(0)
        }
        WM_LBUTTONUP | WM_MBUTTONUP | WM_RBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            let button = match msg {
                WM_LBUTTONUP => 0,
                WM_MBUTTONUP => 1,
                _ => 2,
            };
            push(Input::Up(at(loword(lparam.0), hiword(lparam.0)), button));
            Some(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            // A notch is as many lines as Windows is set to scroll (3 unless changed), of 16 px; down and right
            // positive, as the DOM's are. A mouse's wheel turns by whole notches (`WHEEL_DELTA`); a precision
            // touchpad, or a wheel that turns smoothly, sends fractions of one.
            let mut lines = 3u32;
            unsafe {
                let _ = SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some((&raw mut lines).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            }
            let turned = hiword(wparam.0 as isize);
            let notches = turned as f32 / WHEEL_DELTA as f32;
            let distance = notches * lines as f32 * 16.0;
            let (delta, notched) = if msg == WM_MOUSEWHEEL { (Point { x: 0.0, y: -distance }, Point { x: 0.0, y: -notches }) } else { (Point { x: distance, y: 0.0 }, Point { x: notches, y: 0.0 }) };
            let notched = (turned % WHEEL_DELTA as i32 == 0).then_some(notched);
            let mut point = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            // A wheel message says no precision touchpad's phase: Windows' own momentum comes as more of them.
            push(Input::Wheel(at(point.x, point.y), delta, None, notched));
            Some(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if let Some(key) = keyboard::key(VIRTUAL_KEY(wparam.0 as u16)) {
                push(Input::Key(key));
            }
            // Alt's own keys (Alt-F4, Alt-Space's window menu) stay Windows'.
            if msg == WM_SYSKEYDOWN { None } else { Some(0) }
        }
        WM_CHAR => {
            let unit = wparam.0 as u16;
            if (0xD800..0xDC00).contains(&unit) {
                state.high_surrogate.set(Some(unit));
                return Some(0);
            }
            let units: Vec<u16> = state.high_surrogate.take().into_iter().chain([unit]).collect();
            let typed = String::from_utf16_lossy(&units);
            // What types a control character (Enter, Tab, Backspace, Escape, Ctrl and a letter) was its key, above. What
            // types text is the text input plugin's, where an input takes typing, as Flutter's OnText gives it.
            let plugin = state.text_input.get();
            if typed.chars().any(char::is_control) {
            } else if unsafe { text_input_has_client(plugin) } {
                unsafe { text_input_text(plugin, units.as_ptr(), units.len()) };
            } else {
                push(Input::Key(keyboard::typed(typed)));
            }
            Some(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            push(Input::Focus(msg == WM_SETFOCUS));
            Some(0)
        }
        // The input method's, as Flutter's FlutterWindow handles them (`text_input.cc`).
        // Windows' own, while the window is being made, before it has its plugin.
        WM_IME_SETCONTEXT | WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION if state.text_input.get().is_null() => None,
        // Windows' own, while the window is being made, before it has its plugin.
        WM_IME_SETCONTEXT | WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION if state.text_input.get().is_null() => None,
        WM_IME_SETCONTEXT | WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION => {
            let mut lparam = lparam;
            if unsafe { text_input_ime(state.text_input.get(), msg, wparam, &mut lparam) } {
                return Some(0);
            }
            Some(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }.0)
        }
        WM_SETCURSOR if loword(lparam.0) == HTCLIENT as i32 => {
            unsafe { SetCursor(Some(state.cursor.get())) };
            Some(1)
        }
        WM_SETTINGCHANGE => {
            // The system went light or dark.
            let area = if lparam.0 == 0 { None } else { Some(unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default()) };
            if area.as_deref() == Some("ImmersiveColorSet") && !state.panel {
                colour_frame(hwnd, state);
                push(Input::Appearance(super::super::appearance()));
            }
            None
        }
        _ => None,
    }
}
