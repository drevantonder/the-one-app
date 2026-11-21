// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/window.rs and events.rs) at
// commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the native host: every
// native window on the windows' thread, made with one class, each drawn by DirectComposition (so it has no GDI surface of
// its own), per-monitor DPI aware, its keys from WM_KEYDOWN and WM_CHAR, what's typed and composed through Flutter's
// text input plugin and its IMM32 input method (`text_input.cc`, in place of events.rs's), its mouse and wheel, its cursor, and its
// frame and caption buttons Windows' own, coloured as the page's background, light or dark as the system is. A panel
// (a launcher's) is a popup with no frame, topmost and out of the taskbar, over Acrylic cut to its round corners
// (`Display::glass`). No client-side title bar, drag and drop or touch yet.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

mod panel;
mod procedure;
mod text_input;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::num::NonZeroIsize;
use std::rc::Rc;

use anyhow::Context as _;
use raw_window_handle as rwh;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, GetSystemMetricsForDpi, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContextEx};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};
use windows::UI::Composition::Compositor;
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions};

use super::keyboard;
use crate::Cursor;
use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::platform::{Input, WindowId, WindowOptions};
use panel::Glass;
use procedure::register_window_class;
use text_input::{Plugin, TextInputChannel, perform_action, text_input_new, update_editing_state};

const WINDOW_CLASS_NAME: PCWSTR = w!("One::Native");

/// What every native window's messages are turned into, by the window they're about, until `events` takes them.
type Queue = Rc<RefCell<Vec<(WindowId, usize, Input)>>>;

pub struct Display {
    queue: Queue,
    cursors: RefCell<HashMap<Cursor, HCURSOR>>,
    frames: std::sync::Arc<Frames>,
    /// The display's refresh interval, as the compositor says (`DwmGetCompositionTimingInfo`).
    refresh: std::time::Duration,
    /// Windows' visual layer, which a panel's glass and page are in (`glass`), on this thread's dispatcher queue.
    compositor: Compositor,
    _queue: windows::System::DispatcherQueueController,
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
    /// Whether it was maximized where it was left, which it shows as again.
    maximized: bool,
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
    /// A panel's, or its part's: its glass and page (`Display::glass`), and whether it's kept off the screen until its
    /// next frame is drawn (`cloak`).
    glass: std::cell::OnceCell<Glass>,
    cloaked: Cell<bool>,
    /// A panel's parts, which move with it.
    parts: RefCell<Vec<HWND>>,
    /// The Colour Mode its frame is drawn in, where its page says one (`set_appearance`).
    theme: Cell<Option<crate::Appearance>>,
    /// An app window's file of where it was left (`WindowOptions::frame`), kept as it's moved, resized or maximized.
    frame: Option<std::path::PathBuf>,
    /// Whether its page is under its title row, which has no frame of the system's above it (`unframe`): the window's
    /// caption buttons are its own, drawn in that row, and a press in it moves it.
    unframed: Cell<bool>,
    /// Whether it was put where it was left, as an outer size that has no caption of its own to take off.
    restored: Cell<bool>,
    /// Whether Windows tells it when the pointer leaves its caption buttons, until it does.
    tracking_caption: Cell<bool>,
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

    /// What wgpu draws into: a panel's composition surface (`Display::glass`), or the window, by its raw handles.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        if let Some(glass) = self.state.glass.get() {
            return Ok(wgpu::SurfaceTargetUnsafe::SurfaceHandle(glass.surface.0));
        }
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

    /// Whether what isn't drawn on it is see-through: a panel's glass shows where it isn't.
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
        // The compositor's refresh, which on Windows 8.1 and later it's asked for with no window.
        let mut timing = windows::Win32::Graphics::Dwm::DWM_TIMING_INFO { cbSize: size_of::<windows::Win32::Graphics::Dwm::DWM_TIMING_INFO>() as u32, ..Default::default() };
        let said = unsafe { windows::Win32::Graphics::Dwm::DwmGetCompositionTimingInfo(HWND::default(), &mut timing) }.is_ok() && timing.rateRefresh.uiNumerator > 0;
        let refresh = if said { std::time::Duration::from_secs_f64(f64::from(timing.rateRefresh.uiDenominator) / f64::from(timing.rateRefresh.uiNumerator)) } else { crate::platform::FRAME };
        let (waiting, thread) = (frames.clone(), unsafe { windows::Win32::System::Threading::GetCurrentThreadId() });
        std::thread::Builder::new().name("frames".into()).spawn(move || {
            // It wakes the windows' thread for each frame, so it runs as that does.
            let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Max);
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
                // Where there's no compositor clock (before Windows 11, or a locked screen's, which says so at once), the
                // display's refresh stands in, on a high-resolution timer, as Rust's `sleep` is on Windows.
                const WAIT_OBJECT_0: u32 = 0;
                if wait.is_none_or(|wait| unsafe { wait(0, std::ptr::null(), 100) } != WAIT_OBJECT_0) {
                    std::thread::sleep(refresh);
                }
                *waiting.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());
                let _ = unsafe { PostThreadMessageW(thread, WM_NULL, WPARAM(0), LPARAM(0)) };
            }
        })?;
        // Windows' visual layer runs on a dispatcher queue of the thread's, which its messages pump (`events`).
        let options = DispatcherQueueOptions { dwSize: size_of::<DispatcherQueueOptions>() as u32, threadType: DQTYPE_THREAD_CURRENT, apartmentType: DQTAT_COM_NONE };
        let queue = unsafe { CreateDispatcherQueueController(options) }.context("CreateDispatcherQueueController failed")?;
        let compositor = Compositor::new().context("Windows' compositor didn't start")?;
        Ok(Display { queue: Queue::default(), cursors: RefCell::default(), frames, refresh, compositor, _queue: queue })
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
            glass: std::cell::OnceCell::new(),
            cloaked: Cell::new(false),
            parts: RefCell::default(),
            theme: Cell::new(None),
            frame: options.frame.clone(),
            unframed: Cell::new(false),
            restored: Cell::new(false),
            tracking_caption: Cell::new(false),
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
            let _ = state.glass.set(self.glass(hwnd)?);
        } else {
            colour_frame(hwnd, &state);
        }
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
        }
        // Files can be dropped on an app's window from Explorer.
        if !panel {
            unsafe { windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true) };
        }
        // An app's window opens where it was left, as big, and maximized if it was, as Windows' own apps keep their
        // placement.
        let left = options.frame.as_deref().and_then(crate::platform::Frame::read);
        state.restored.set(left.is_some());
        if let Some(left) = left {
            let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            placement.showCmd = SW_HIDE.0 as u32;
            placement.rcNormalPosition = RECT { left: left.x, top: left.y, right: left.x + left.width, bottom: left.y + left.height };
            unsafe {
                let _ = SetWindowPlacement(hwnd, &placement);
            }
        }
        Ok(NativeWindow {
            hwnd,
            size: Cell::new(Size { width: DevicePixels(client.right - client.left), height: DevicePixels(client.bottom - client.top) }),
            state,
            panel,
            before: Cell::new(HWND::default()),
            owner: Cell::new(HWND::default()),
            maximized: left.is_some_and(|left| left.maximized),
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

    /// Puts a window opened hidden on the screen.
    pub fn show(&self, window: &NativeWindow) {
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        unsafe {
            if !window.owner.get().is_invalid() {
                let _ = EnableWindow(window.owner.get(), false);
            }
            let _ = ShowWindow(window.hwnd, if window.maximized { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL });
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

    /// Shows Windows' menu for text where a field was right-clicked (`TextMenu`), as muda shows a context menu: its
    /// choice is its key (`menu_chosen`). It goes by itself.
    pub fn text_menu(&self, window: &NativeWindow, menu: Option<crate::view::TextMenu>) {
        use muda::ContextMenu;
        let Some(menu) = menu else { return };
        let shown = super::super::text_menu_of(menu, window.state.id);
        let at = muda::dpi::LogicalPosition::new(f64::from(menu.rect.origin.x), f64::from(menu.rect.origin.y));
        // SAFETY: the window's own, on its thread.
        unsafe { shown.show_context_menu_for_hwnd(window.hwnd.0 as isize, Some(at.into())) };
    }

    /// A computer's fields have no magnifier: a pointer selects exactly.
    pub fn magnifier(&self, _: &NativeWindow, _: Option<crate::view::Magnifier>) {}

    /// Its frame's caption buttons are drawn light or dark as `theme` says, or as Windows is.
    pub fn set_appearance(&self, window: &NativeWindow, theme: Option<crate::Appearance>) {
        window.state.theme.set(theme);
        if !window.state.panel {
            colour_frame(window.hwnd, &window.state);
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

    /// Where a press moves it, as its page says (`appRegion`): a panel's, and an app's window's title row.
    pub fn set_title_row(&self, window: &NativeWindow, row: Option<crate::view::TitleRow>) {
        *window.state.row.borrow_mut() = row;
    }

    /// An app window with no frame of the system's: its page runs to its top, under its title row, which has its own
    /// caption buttons, as Edge's, Teams' and File Explorer's windows do, and a press there moves it, a double
    /// press zooms it, and Snap Layouts opens over its maximize button (`HTMAXBUTTON`).
    pub fn unframe(&self, window: &NativeWindow) {
        use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
        use windows::Win32::UI::Controls::MARGINS;
        window.state.unframed.set(true);
        unsafe {
            // A line of frame on top keeps its shadow.
            let _ = DwmExtendFrameIntoClientArea(window.hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
            // It was made with room for a caption, which is the page's now; one put where it was left already has none.
            let mut frame = RECT::default();
            let _ = GetWindowRect(window.hwnd, &mut frame);
            let caption = if window.state.restored.get() { 0 } else { caption(window.hwnd) };
            let _ = SetWindowPos(window.hwnd, None, 0, 0, frame.right - frame.left, frame.bottom - frame.top - caption, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }

    pub fn flush(&self) {}

    /// How pressed the device is, as its system says: serious while battery saver is on, as Windows has apps do less
    /// then (`SYSTEM_POWER_STATUS.SystemStatusFlag`).
    pub fn pressure(&self) -> crate::platform::Pressure {
        let mut status = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
        let saving = unsafe { windows::Win32::System::Power::GetSystemPowerStatus(&mut status) }.is_ok() && status.SystemStatusFlag == 1;
        if saving { crate::platform::Pressure::Serious } else { crate::platform::Pressure::Nominal }
    }

    pub fn refresh(&self) -> std::time::Duration {
        self.refresh
    }

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

/// Behind the caption buttons is the page's background (`--background`, #f8f8fa or #000000), and the buttons light or
/// dark, as its Colour Mode says, or as the system is.
fn colour_frame(hwnd: HWND, state: &State) {
    let dark = state.theme.get().unwrap_or_else(super::appearance) == crate::Appearance::Dark;
    let (immersive, colour) = (BOOL::from(dark), COLORREF(if dark { 0x0000_0000 } else { 0x00fa_f8f8 }));
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const immersive).cast(), size_of::<BOOL>() as u32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, (&raw const colour).cast(), size_of::<COLORREF>() as u32);
    }
}

/// A monitor's name, by where its work area is and how large, which a panel's places are kept by.
fn screen_name(work: &RECT) -> String {
    format!("{}x{}+{}+{}", work.right - work.left, work.bottom - work.top, work.left, work.top)
}

/// How tall the system's caption is at `hwnd`'s DPI, in pixels.
fn caption(hwnd: HWND) -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, GetDpiForWindow(hwnd)) }
}
