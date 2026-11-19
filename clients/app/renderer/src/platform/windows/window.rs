// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/window.rs and events.rs) at
// commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the native host: every
// native window on the windows' thread, made with one class, each drawn by DirectComposition (so it has no GDI surface of
// its own), per-monitor DPI aware, its keys from WM_KEYDOWN and WM_CHAR, what's typed and composed through Flutter's
// text input plugin and its IMM32 input method (`text_input.cc`, in place of events.rs's), its mouse and wheel, its cursor, and its
// frame and caption buttons Windows' own, coloured as the page's background, light or dark as the system is. A panel
// (a launcher's) is a popup with no frame, topmost and out of the taskbar, over Windows 11's Acrylic with its round
// corners, as GPUI's `WindowBackgroundAppearance::Blurred` asks DWM for a system backdrop. No client-side title bar,
// drag and drop or touch yet.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::collections::HashMap;
use std::num::NonZeroIsize;
use std::rc::Rc;
use std::sync::Once;

use anyhow::Context as _;
use raw_window_handle as rwh;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{ScreenToClient, ValidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::Ime::{HIMC, IACE_DEFAULT, ImmAssociateContextEx};
use windows::Win32::System::SystemServices::MK_SHIFT;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use super::keyboard;
use crate::Cursor;
use crate::editing::{TextInput, TextInputClient, Value};
use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::platform::{Input, WindowId, WindowOptions};

const WINDOW_CLASS_NAME: PCWSTR = w!("One::Native");

/// What every native window's messages are turned into, by the window they're about, until `events` takes them.
type Queue = Rc<RefCell<Vec<(WindowId, usize, Input)>>>;

pub struct Display {
    queue: Queue,
    cursors: RefCell<HashMap<Cursor, HCURSOR>>,
    frames: std::sync::Arc<Frames>,
}

/// The display's frames, as a thread of their own waits for each by the compositor's clock (Windows 11's), while one
/// is wanted, and wakes the windows' thread with when it came, as Chromium's vsync on Windows does.
#[derive(Default)]
struct Frames {
    wanted: std::sync::Mutex<bool>,
    told: std::sync::Condvar,
    shown: std::sync::Mutex<Option<std::time::Instant>>,
}

pub struct NativeWindow {
    hwnd: HWND,
    size: Cell<Size<DevicePixels>>,
    state: Rc<State>,
    /// A panel's: the window that was in front before it was summoned, to give the keyboard back to.
    panel: bool,
    before: Cell<HWND>,
    /// The window it's a sheet of (`attach`), which it's modal to while it's open.
    owner: Cell<HWND>,
}

/// What a window's procedure knows of it.
struct State {
    id: WindowId,
    /// Which part of its window's page it is: 0 the window itself, the rest a panel's parts.
    part: usize,
    queue: Queue,
    cursor: Cell<HCURSOR>,
    /// Whether Windows tells it when the pointer leaves (`TrackMouseEvent`), until it does.
    tracking: Cell<bool>,
    /// The first half of a character WM_CHAR sends in two.
    high_surrogate: Cell<Option<u16>>,
    /// Its text input plugin, Flutter's (`text_input.cc`).
    text_input: Plugin,
    /// A panel's, and its parts': where a press moves it, as its page says (`set_title_row`).
    row: Rc<RefCell<Option<crate::view::TitleRow>>>,
    panel: bool,
    /// A panel's, or its part's: the top left of its box of the page, in points, as it was last put (`arrange`), and
    /// its shape, its width, height and corners' radius in pixels, as it was last cut to (`round`).
    placed: Cell<Option<Point<f32>>>,
    shape: Cell<(i32, i32, i32)>,
    /// A panel's parts, which move with it.
    parts: RefCell<Vec<HWND>>,
}

impl State {
    /// Where point `at` of it is in its page.
    fn in_page(&self, at: Point<f32>) -> Point<f32> {
        let by = self.placed.get().unwrap_or(Point { x: 0.0, y: 0.0 });
        Point { x: at.x + by.x, y: at.y + by.y }
    }
}

/// What window `hwnd`'s procedure knows of it, while it's open.
fn state(hwnd: HWND) -> Option<&'static State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State).as_ref() }
}

impl rwh::HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let hwnd = NonZeroIsize::new(self.hwnd.0 as isize).ok_or(rwh::HandleError::Unavailable)?;
        let mut handle = rwh::Win32WindowHandle::new(hwnd);
        handle.hinstance = unsafe { GetModuleHandleW(None) }.ok().and_then(|module| NonZeroIsize::new(module.0 as isize));
        Ok(unsafe { rwh::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl rwh::HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(rwh::DisplayHandle::windows())
    }
}

/// A window as `Display::arrange` places it: its window and its scale.
#[derive(Clone, Copy)]
pub struct Handle {
    hwnd: HWND,
    scale: f32,
}

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle { hwnd: self.hwnd, scale: self.scale() }
    }

    /// What wgpu draws into: the window, by its raw handles.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        // Safety: a window's renderer is let go before the window is.
        Ok(unsafe { wgpu::SurfaceTargetUnsafe::from_display_and_window(self, self)? })
    }

    /// A computer's window has its surface from the start.
    pub fn has_surface(&self) -> bool {
        true
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.size.get()
    }

    pub fn size(&self) -> Size<Pixels> {
        let (size, scale) = (self.size.get(), self.scale());
        Size { width: Pixels(size.width.0 as f32 / scale), height: Pixels(size.height.0 as f32 / scale) }
    }

    /// Its monitor's scale, as Windows says it now.
    pub fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.hwnd) as f32 / USER_DEFAULT_SCREEN_DPI as f32 }
    }

    pub fn resized(&self, size: Size<DevicePixels>) {
        self.size.set(size);
    }

    /// Whether what isn't drawn on it is see-through: a panel's Acrylic shows where it isn't.
    pub fn transparent(&self) -> bool {
        self.panel
    }
}

impl Display {
    pub fn connect() -> anyhow::Result<Display> {
        // Every window this thread makes is drawn at its monitor's DPI, whatever the process's manifest says.
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        register_window_class();
        let frames = std::sync::Arc::new(Frames::default());
        let (waiting, thread) = (frames.clone(), unsafe { windows::Win32::System::Threading::GetCurrentThreadId() });
        std::thread::Builder::new().name("frames".into()).spawn(move || {
            // Windows 11's, looked up as it runs, so the program still starts on Windows 10, which hasn't it.
            type Wait = unsafe extern "system" fn(u32, *const windows::Win32::Foundation::HANDLE, u32) -> u32;
            let wait: Option<Wait> = unsafe {
                use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
                let dcomp = LoadLibraryW(w!("dcomp.dll")).ok();
                dcomp.and_then(|dcomp| GetProcAddress(dcomp, windows::core::s!("DCompositionWaitForCompositorClock"))).map(|found| std::mem::transmute::<_, Wait>(found))
            };
            loop {
                let mut wanted = waiting.wanted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while !*wanted {
                    wanted = waiting.told.wait(wanted).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                *wanted = false;
                drop(wanted);
                // Where there's no compositor clock (before Windows 11, or a locked screen's, which says so at once), a
                // 60 Hz display's frame stands in, on a high-resolution timer, as Rust's `sleep` is on Windows.
                const WAIT_OBJECT_0: u32 = 0;
                if wait.is_none_or(|wait| unsafe { wait(0, std::ptr::null(), 100) } != WAIT_OBJECT_0) {
                    std::thread::sleep(std::time::Duration::from_micros(16_667));
                }
                *waiting.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());
                let _ = unsafe { PostThreadMessageW(thread, WM_NULL, WPARAM(0), LPARAM(0)) };
            }
        })?;
        Ok(Display { queue: Queue::default(), cursors: RefCell::default(), frames })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let state = Rc::new(State {
            id,
            part: 0,
            queue: self.queue.clone(),
            cursor: Cell::new(self.cursor(Cursor::Arrow)),
            tracking: Cell::new(false),
            high_surrogate: Cell::new(None),
            text_input: Plugin(Cell::new(std::ptr::null_mut())),
            row: Rc::default(),
            panel: options.panel.is_some(),
            placed: Cell::new(None),
            shape: Cell::new((0, 0, 0)),
            parts: RefCell::default(),
        });
        let panel = options.panel.is_some();
        let style = if panel { WS_POPUP } else { WS_OVERLAPPEDWINDOW };
        // No GDI surface: DirectComposition draws the whole client area. A panel is over every other window and out
        // of the taskbar and Alt-Tab, as a tool window is.
        let ex_style = if panel { WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP } else { WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP };
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                WINDOW_CLASS_NAME,
                &HSTRING::from(options.title.as_str()),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                Some(Rc::as_ptr(&state) as *const _),
            )
        }
        .context("CreateWindowExW failed")?;
        let channel = TextInputChannel { context: Rc::as_ptr(&state).cast_mut().cast(), update_editing_state, perform_action };
        state.text_input.0.set(unsafe { text_input_new(hwnd, channel) });
        // Its input method is off until an input takes typing.
        unsafe {
            let _ = ImmAssociateContextEx(hwnd, HIMC::default(), 0);
        }
        // Its size at the DPI of the monitor it's on, with its frame around it.
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = dpi as f32 / USER_DEFAULT_SCREEN_DPI as f32;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (options.size.width.0 * scale).round() as i32,
            bottom: (options.size.height.0 * scale).round() as i32,
        };
        unsafe {
            let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi);
            let _ = SetWindowPos(hwnd, None, 0, 0, rect.right - rect.left, rect.bottom - rect.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        if panel {
            glass(hwnd);
        } else {
            colour_frame(hwnd);
        }
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        Ok(NativeWindow {
            hwnd,
            size: Cell::new(Size { width: DevicePixels(client.right - client.left), height: DevicePixels(client.bottom - client.top) }),
            state,
            panel,
            before: Cell::new(HWND::default()),
            owner: Cell::new(HWND::default()),
        })
    }

    /// Makes `sheet` a modal dialog of `owner`'s, as Win32's are: owned by it, so it's over it and out of the taskbar,
    /// centred on it, with no minimize or maximize; its owner takes no input while it's shown (`show`, `close`).
    pub fn attach(&self, sheet: &NativeWindow, owner: &NativeWindow) {
        use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, GetWindowRect, SWP_FRAMECHANGED, WS_MAXIMIZEBOX, WS_MINIMIZEBOX};
        sheet.owner.set(owner.hwnd);
        unsafe {
            SetWindowLongPtrW(sheet.hwnd, GWLP_HWNDPARENT, owner.hwnd.0 as isize);
            let style = GetWindowLongPtrW(sheet.hwnd, GWL_STYLE) & !((WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0) as isize);
            SetWindowLongPtrW(sheet.hwnd, GWL_STYLE, style);
            let ex_style = GetWindowLongPtrW(sheet.hwnd, GWL_EXSTYLE) & !(WS_EX_APPWINDOW.0 as isize);
            SetWindowLongPtrW(sheet.hwnd, GWL_EXSTYLE, ex_style);
            let (mut over, mut own) = (RECT::default(), RECT::default());
            let _ = GetWindowRect(owner.hwnd, &mut over);
            let _ = GetWindowRect(sheet.hwnd, &mut own);
            let (width, height) = (own.right - own.left, own.bottom - own.top);
            let (x, y) = (over.left + (over.right - over.left - width) / 2, over.top + (over.bottom - over.top - height) / 2);
            let _ = SetWindowPos(sheet.hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }

    /// Shows a panel's page, `room` points, centred near the top of the monitor the pointer is on, a fifth of the way
    /// down its work area as Spotlight sits, in front with the keyboard, keeping which window was in front; its parts
    /// follow it (`arrange`). Where it was dragged to on that monitor before (`places`), it's there, as much of it as
    /// fits.
    pub fn summon(&self, window: &NativeWindow, places: &crate::platform::Places, room: Size<Pixels>) {
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
        unsafe {
            let mut pointer = POINT::default();
            let _ = GetCursorPos(&mut pointer);
            let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let work = info.rcWork;
            let size = window.size.get();
            let (width, height) = (work.right - work.left, work.bottom - work.top);
            let scale = window.scale();
            let page = ((room.width.0 * scale) as i32, (room.height.0 * scale) as i32);
            let at = window.state.placed.get().map_or((0, 0), |at| ((at.x * scale) as i32, (at.y * scale) as i32));
            let (x, y) = match places.get(&screen_name(&work)) {
                Some(at) => (
                    (work.left + (at.x * scale) as i32).clamp(work.left, work.left + (width - size.width.0).max(0)),
                    (work.top + (at.y * scale) as i32).clamp(work.top, work.top + (height - size.height.0).max(0)),
                ),
                // Its search and its card, as one, in the middle of the monitor's work area, clear of the taskbar.
                None => (work.left + (width - page.0) / 2 + at.0, work.top + ((height - page.1) / 2).max(0) + at.1),
            };
            let before = GetForegroundWindow();
            if before != window.hwnd {
                window.before.set(before);
            }
            let _ = SetWindowPos(window.hwnd, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(window.hwnd);
        }
    }

    /// Puts a panel away, its parts with it, and with `give_back`, brings the window that was in front before it back.
    pub fn put_away(&self, window: &NativeWindow, give_back: bool) {
        unsafe {
            for part in window.state.parts.borrow().iter() {
                let _ = ShowWindow(*part, SW_HIDE);
            }
            let _ = ShowWindow(window.hwnd, SW_HIDE);
            let before = window.before.replace(HWND::default());
            if give_back && !before.is_invalid() {
                let _ = SetForegroundWindow(before);
            }
        }
    }

    /// A part of panel `parent`'s page as a window of its own, its `part`th: owned by the panel, so it's above it and
    /// out of the taskbar and Alt-Tab as the panel is, with Acrylic under it and round corners; it never takes the
    /// keyboard, which stays with the panel, and moves with it (`arrange`).
    pub fn open_part(&self, parent: &NativeWindow, part: usize) -> anyhow::Result<NativeWindow> {
        let state = Rc::new(State {
            id: parent.state.id,
            part,
            queue: self.queue.clone(),
            cursor: Cell::new(self.cursor(Cursor::Arrow)),
            tracking: Cell::new(false),
            high_surrogate: Cell::new(None),
            text_input: Plugin(Cell::new(std::ptr::null_mut())),
            row: parent.state.row.clone(),
            panel: true,
            placed: Cell::new(None),
            shape: Cell::new((0, 0, 0)),
            parts: RefCell::default(),
        });
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
        let hwnd = unsafe { CreateWindowExW(ex_style, WINDOW_CLASS_NAME, w!(""), WS_POPUP, 0, 0, 1, 1, Some(parent.hwnd), None, module, Some(Rc::as_ptr(&state) as *const _)) }
            .context("CreateWindowExW failed")?;
        glass(hwnd);
        parent.state.parts.borrow_mut().push(hwnd);
        Ok(NativeWindow { hwnd, size: Cell::new(Size { width: DevicePixels(1), height: DevicePixels(1) }), state, panel: true, before: Cell::new(HWND::default()), owner: Cell::new(HWND::default()) })
    }

    /// Puts a panel's parts where its page is, each its box of the page (`placed`, the panel first), the page where it
    /// was (from where the panel is, and the box it had), its parts shown while it is, and draws them (`draw`).
    pub fn arrange(&self, placed: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        if let Some((first, ..)) = placed.first() {
            let mut frame = RECT::default();
            unsafe {
                let _ = GetWindowRect(first.hwnd, &mut frame);
            }
            let scale = first.scale;
            // Until it's first put, the panel is its whole page.
            let was = state(first.hwnd).and_then(|state| state.placed.get()).unwrap_or(Point { x: 0.0, y: 0.0 });
            let page = (frame.left - (was.x * scale).round() as i32, frame.top - (was.y * scale).round() as i32);
            let shown = unsafe { IsWindowVisible(first.hwnd) }.as_bool();
            for (handle, bounds, _) in placed {
                if let Some(state) = state(handle.hwnd) {
                    state.placed.set(Some(bounds.origin));
                }
            }
            for (handle, bounds, radius) in placed {
                let (x, y) = (page.0 + (bounds.origin.x * scale).round() as i32, page.1 + (bounds.origin.y * scale).round() as i32);
                let (width, height) = ((bounds.size.width * scale).round() as i32, (bounds.size.height * scale).round() as i32);
                let mut now = RECT::default();
                unsafe {
                    let _ = GetWindowRect(handle.hwnd, &mut now);
                }
                let part = handle.hwnd != first.hwnd;
                let show = part && shown && !unsafe { IsWindowVisible(handle.hwnd) }.as_bool();
                if show || (now.left, now.top, now.right - now.left, now.bottom - now.top) != (x, y, width, height) {
                    let flags = SWP_NOZORDER | SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { SET_WINDOW_POS_FLAGS(0) };
                    unsafe {
                        let _ = SetWindowPos(handle.hwnd, None, x, y, width, height, flags);
                    }
                }
                round(handle.hwnd, width, height, (radius * scale).round() as i32);
            }
        }
        draw();
    }

    /// Moves panel `hwnd`'s parts with it, where it's moved to.
    fn follow(hwnd: HWND, panel: &State) {
        let Some(was) = panel.placed.get() else { return };
        let mut frame = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut frame);
        }
        let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / USER_DEFAULT_SCREEN_DPI as f32;
        let page = (frame.left - (was.x * scale).round() as i32, frame.top - (was.y * scale).round() as i32);
        for part in panel.parts.borrow().iter() {
            let Some(at) = state(*part).and_then(|state| state.placed.get()) else { continue };
            unsafe {
                let _ = SetWindowPos(*part, None, page.0 + (at.x * scale).round() as i32, page.1 + (at.y * scale).round() as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }

    /// Puts a window opened hidden on the screen.
    pub fn show(&self, window: &NativeWindow) {
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        unsafe {
            if !window.owner.get().is_invalid() {
                let _ = EnableWindow(window.owner.get(), false);
            }
            let _ = ShowWindow(window.hwnd, SW_SHOWNORMAL);
        }
    }

    pub fn close(&self, window: &NativeWindow) {
        if let Some(panel) = unsafe { GetWindow(window.hwnd, GW_OWNER) }.ok().and_then(state) {
            panel.parts.borrow_mut().retain(|part| *part != window.hwnd);
        }
        unsafe {
            // Its owner takes input again before it goes, so the keyboard goes back to it, as a modal dialog's does.
            if !window.owner.get().is_invalid() {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(window.owner.get(), true);
            }
            SetWindowLongPtrW(window.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(window.hwnd);
        }
    }

    pub fn activate(&self, window: &NativeWindow) {
        unsafe {
            if IsIconic(window.hwnd).as_bool() {
                let _ = ShowWindow(window.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(window.hwnd);
        }
    }

    pub fn set_cursor(&self, window: &NativeWindow, cursor: Cursor) {
        let cursor = self.cursor(cursor);
        if window.state.cursor.replace(cursor) != cursor {
            unsafe { SetCursor(Some(cursor)) };
        }
    }

    fn cursor(&self, cursor: Cursor) -> HCURSOR {
        *self.cursors.borrow_mut().entry(cursor).or_insert_with(|| {
            let name = match cursor {
                Cursor::Arrow => IDC_ARROW,
                Cursor::Pointer => IDC_HAND,
                Cursor::Text => IDC_IBEAM,
            };
            unsafe { LoadCursorW(None, name) }.unwrap_or_default()
        })
    }

    /// Where a press moves it: a panel's page says where (`appRegion`); an app's window has the system's own title bar
    /// above its page, which moves it.
    pub fn set_title_row(&self, window: &NativeWindow, row: Option<crate::view::TitleRow>) {
        if window.panel {
            *window.state.row.borrow_mut() = row;
        }
    }

    /// Tells window `window`'s text input plugin what the screen says (`editing::TextInput`), its method calls decoded.
    /// Its input method is on while an input takes typing, and off while none does.
    pub fn text_input(&self, window: &NativeWindow, calls: &[TextInput]) {
        let plugin = window.state.text_input.get();
        for call in calls {
            unsafe {
                match call {
                    TextInput::SetClient(id, configuration) => {
                        let (kind, action) = (cstring(configuration["inputType"]["name"].as_str().unwrap_or_default()), cstring(configuration["inputAction"].as_str().unwrap_or_default()));
                        text_input_set_client(plugin, *id as i32, kind.as_ptr(), action.as_ptr());
                        let _ = ImmAssociateContextEx(window.hwnd, HIMC::default(), IACE_DEFAULT);
                    }
                    TextInput::ClearClient => {
                        text_input_clear_client(plugin);
                        let _ = ImmAssociateContextEx(window.hwnd, HIMC::default(), 0);
                    }
                    TextInput::SetEditingState(value) => {
                        let state = value.flutter();
                        let at = |key: &str| state[key].as_i64().unwrap_or(-1) as i32;
                        let text = cstring(&value.text);
                        let (base, extent) = (at("selectionBase"), at("selectionExtent"));
                        text_input_set_editing_state(plugin, text.as_ptr(), base, extent, at("composingBase"), at("composingExtent"));
                    }
                    TextInput::SetMarkedTextRect(rect) => {
                        let (x, y, width, height) = (rect.origin.x.into(), rect.origin.y.into(), rect.size.width.into(), rect.size.height.into());
                        text_input_set_marked_text_rect(plugin, x, y, width, height);
                    }
                    TextInput::SetEditableSizeAndTransform { transform, .. } => {
                        let transform = transform.map(f64::from);
                        text_input_set_editable_transform(plugin, transform.as_ptr());
                    }
                    TextInput::SetCaretRect(_) | TextInput::Show | TextInput::Hide => {}
                }
            }
        }
    }

    pub fn flush(&self) {}

    /// Whether it says when the compositor shows each frame: it does.
    pub fn paced(&self) -> bool {
        true
    }

    /// Has its thread wait for the compositor's next frame, while one is `wanted`.
    pub fn request_frame(&self, wanted: bool) {
        if wanted {
            *self.frames.wanted.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            self.frames.told.notify_one();
        }
    }

    /// When the compositor showed the frame asked for, once it has.
    pub fn frame(&self) -> Option<std::time::Instant> {
        self.frames.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    /// What Windows has said since it was last asked, by the window it's about: every message waiting for this
    /// thread dispatched to its window's procedure, which turns it into input.
    pub fn events(&self) -> Vec<(WindowId, usize, Input)> {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::mem::take(&mut *self.queue.borrow_mut())
    }
}

/// A window's text input plugin (`text_input.cc`): Flutter's, as a C interface.
struct Plugin(Cell<*mut c_void>);

impl Plugin {
    fn get(&self) -> *mut c_void {
        self.0.get()
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        unsafe { text_input_free(self.0.get()) };
    }
}

/// Flutter's TextInputClient calls, as the plugin makes them (`flutter/.../windows/text_input_channel.h`).
#[repr(C)]
struct TextInputChannel {
    context: *mut c_void,
    update_editing_state: extern "C" fn(*mut c_void, i32, *const c_char, i32, i32, i32, i32),
    perform_action: extern "C" fn(*mut c_void, i32, *const c_char),
}

unsafe extern "C" {
    fn text_input_new(window: HWND, channel: TextInputChannel) -> *mut c_void;
    fn text_input_free(plugin: *mut c_void);
    fn text_input_has_client(plugin: *mut c_void) -> bool;
    fn text_input_set_client(plugin: *mut c_void, client: i32, input_type: *const c_char, input_action: *const c_char);
    fn text_input_clear_client(plugin: *mut c_void);
    fn text_input_set_editing_state(plugin: *mut c_void, text: *const c_char, base: i32, extent: i32, composing_base: i32, composing_extent: i32);
    fn text_input_set_marked_text_rect(plugin: *mut c_void, x: f64, y: f64, width: f64, height: f64);
    fn text_input_set_editable_transform(plugin: *mut c_void, transform: *const f64);
    fn text_input_text(plugin: *mut c_void, text: *const u16, length: usize);
    fn text_input_ime(plugin: *mut c_void, message: u32, wparam: WPARAM, lparam: *mut LPARAM) -> bool;
}

/// The plugin says input `client`'s whole editing state, for the window's screen.
extern "C" fn update_editing_state(state: *mut c_void, client: i32, text: *const c_char, base: i32, extent: i32, composing_base: i32, composing_extent: i32) {
    // SAFETY: the window's, which outlives its plugin (`State`).
    let state = unsafe { &*state.cast::<State>() };
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let value = Value::from_flutter(&serde_json::json!({
        "text": text, "selectionBase": base, "selectionExtent": extent, "composingBase": composing_base, "composingExtent": composing_extent,
    }));
    state.queue.borrow_mut().push((state.id, state.part, Input::TextInputClient(TextInputClient::UpdateEditingState(client as u64, value))));
}

/// The plugin says input `client`'s action key was pressed.
extern "C" fn perform_action(state: *mut c_void, client: i32, action: *const c_char) {
    let state = unsafe { &*state.cast::<State>() };
    let action = unsafe { CStr::from_ptr(action) }.to_string_lossy().into_owned();
    state.queue.borrow_mut().push((state.id, state.part, Input::TextInputClient(TextInputClient::PerformAction(client as u64, action))));
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).expect("no NUL in it")
}

fn register_window_class() {
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

/// A panel's material: Windows 11's Acrylic under what its page doesn't draw (DWM's transient window backdrop, over
/// the whole window as its frame), light or dark as the system is. Its corners and its edge are its page's, as on a
/// Mac: Windows 11 neither rounds it nor draws its border round it (each would be a second edge, its corners 8 points
/// where its page's are its own), and it's cut to its page's corners (`round`).
fn glass(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{
        DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmExtendFrameIntoClientArea,
    };
    use windows::Win32::UI::Controls::MARGINS;
    let dark = BOOL::from(super::appearance() == crate::Appearance::Dark);
    let (backdrop, corners, border) = (DWMSBT_TRANSIENTWINDOW, DWMWCP_DONOTROUND, DWMWA_COLOR_NONE);
    let whole = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &whole);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, std::ptr::from_ref(&dark).cast(), size_of::<BOOL>() as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, std::ptr::from_ref(&backdrop).cast(), size_of_val(&backdrop) as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, std::ptr::from_ref(&corners).cast(), size_of_val(&corners) as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, std::ptr::from_ref(&border).cast(), size_of_val(&border) as u32);
    }
}

/// Cuts a panel's window, `width` by `height` pixels, to round corners of `radius` pixels, its page's, so its Acrylic
/// is only under what its page draws; again only as its shape changes.
fn round(hwnd: HWND, width: i32, height: i32, radius: i32) {
    let Some(state) = state(hwnd) else { return };
    if state.shape.replace((width, height, radius)) == (width, height, radius) {
        return;
    }
    unsafe {
        // The region's right and bottom edges are outside it, so it's a pixel larger; it's the window's once set.
        let region = windows::Win32::Graphics::Gdi::CreateRoundRectRgn(0, 0, width + 1, height + 1, 2 * radius, 2 * radius);
        let _ = windows::Win32::Graphics::Gdi::SetWindowRgn(hwnd, Some(region), true);
    }
}

/// Behind the caption buttons is the page's background (`--background`), and the buttons light or dark, as the
/// system is.
fn colour_frame(hwnd: HWND) {
    let dark = super::appearance() == crate::Appearance::Dark;
    let (immersive, colour) = (BOOL::from(dark), COLORREF(if dark { 0x000a_0a0a } else { 0x00ff_ffff }));
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const immersive).cast(), size_of::<BOOL>() as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, (&raw const colour).cast(), size_of::<COLORREF>() as u32);
    }
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

/// A monitor's name, by where its work area is and how large, which a panel's places are kept by.
fn screen_name(work: &RECT) -> String {
    format!("{}x{}+{}+{}", work.right - work.left, work.bottom - work.top, work.left, work.top)
}

fn loword(value: isize) -> i32 {
    (value & 0xffff) as i16 as i32
}

fn hiword(value: isize) -> i32 {
    ((value >> 16) & 0xffff) as i16 as i32
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
            Some(0)
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
            // positive, as the DOM's are. A precision touchpad sends fractions of a notch.
            let mut lines = 3u32;
            unsafe {
                let _ = SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some((&raw mut lines).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            }
            let notches = hiword(wparam.0 as isize) as f32 / WHEEL_DELTA as f32;
            let distance = notches * lines as f32 * 16.0;
            let delta = if msg == WM_MOUSEWHEEL { Point { x: 0.0, y: -distance } } else { Point { x: distance, y: 0.0 } };
            let mut point = POINT { x: loword(lparam.0), y: hiword(lparam.0) };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut point);
            }
            // A wheel message says no precision touchpad's phase: Windows' own momentum comes as more of them.
            push(Input::Wheel(at(point.x, point.y), delta, None));
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
            if area.as_deref() == Some("ImmersiveColorSet") {
                colour_frame(hwnd);
                push(Input::Appearance(super::appearance()));
            }
            None
        }
        _ => None,
    }
}

