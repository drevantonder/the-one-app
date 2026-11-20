// A panel's windows, and the glass and page in Windows' visual layer that each has.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyhow::Context as _;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface as _, w};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionBrush, CompositionRoundedRectangleGeometry, CompositionStretch};
use windows::Win32::Graphics::DirectComposition::{COMPOSITIONOBJECT_READ, COMPOSITIONOBJECT_WRITE, DCompositionCreateSurfaceHandle};
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};

use crate::Cursor;
use crate::geometry::{DevicePixels, Pixels, Point, Size};
use super::{Display, Handle, NativeWindow, Plugin, State, WINDOW_CLASS_NAME, screen_name, state};

impl Display {
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
            let (width, height) = (work.right - work.left, work.bottom - work.top);
            let scale = window.scale();
            let page = ((room.width.0 * scale) as i32, (room.height.0 * scale) as i32);
            let at = window.state.placed.get().map_or((0, 0), |at| ((at.x * scale) as i32, (at.y * scale) as i32));
            let (x, y) = match places.get(&screen_name(&work)) {
                // Where it was dragged to, its search and its card kept whole in the work area.
                Some(kept) => (
                    (work.left + (kept.x * scale) as i32).clamp(work.left + at.0, work.left + at.0 + (width - page.0).max(0)),
                    (work.top + (kept.y * scale) as i32).clamp(work.top + at.1, work.top + at.1 + (height - page.1).max(0)),
                ),
                // Its search and its card, as one, in the middle of the monitor's work area, clear of the taskbar.
                None => (work.left + (width - page.0) / 2 + at.0, work.top + ((height - page.1) / 2).max(0) + at.1),
            };
            let before = GetForegroundWindow();
            if before != window.hwnd {
                window.before.set(before);
            }
            // Off the screen until its frame is drawn (`arrange`): its surfaces let go of their images as it went.
            cloak(window.hwnd, true);
            for part in window.state.parts.borrow().iter() {
                cloak(*part, true);
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
            glass: std::cell::OnceCell::new(),
            cloaked: Cell::new(false),
            parts: RefCell::default(),
            theme: Cell::new(None),
            frame: None,
            unframed: Cell::new(false),
            restored: Cell::new(false),
            tracking_caption: Cell::new(false),
        });
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        let module = unsafe { GetModuleHandleW(None) }.ok().map(Into::into);
        let hwnd = unsafe { CreateWindowExW(ex_style, WINDOW_CLASS_NAME, w!(""), WS_POPUP, 0, 0, 1, 1, Some(parent.hwnd), None, module, Some(Rc::as_ptr(&state) as *const _)) }
            .context("CreateWindowExW failed")?;
        let _ = state.glass.set(self.glass(hwnd)?);
        // Off the screen until its first frame is drawn (`arrange`).
        cloak(hwnd, true);
        parent.state.parts.borrow_mut().push(hwnd);
        Ok(NativeWindow { hwnd, size: Cell::new(Size { width: DevicePixels(1), height: DevicePixels(1) }), state, panel: true, before: Cell::new(HWND::default()), owner: Cell::new(HWND::default()), maximized: false })
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
        // On the screen once its frame is, as Chromium and WinUI show a window only after its first present.
        for (handle, ..) in placed {
            cloak(handle.hwnd, false);
        }
    }

    /// Moves panel `hwnd`'s parts with it, where it's moved to.
    pub(super) fn follow(hwnd: HWND, panel: &State) {
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
}

/// A panel's, or its part's, glass and page, in Windows' visual layer, kept while its window is.
pub(super) struct Glass {
    _target: DesktopWindowTarget,
    /// Its shape, which its glass and page are cut to (`round`).
    shape: CompositionRoundedRectangleGeometry,
    /// The composition surface wgpu draws its page into (`NativeWindow::surface`).
    pub(super) surface: windows::Win32::Foundation::HANDLE,
}

impl Drop for Glass {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.surface);
        }
    }
}

impl Display {
    /// A panel's material and page, as Windows 11's own flyouts have theirs: in Windows' visual layer, what's behind
    /// it blurred (its host backdrop brush, Acrylic's own) and tinted as Acrylic is, light or dark as the system is,
    /// with its page's frames over it, the three cut to its page's round corners (`round`). Its corners and edge are
    /// its page's, as on a Mac: Windows 11 neither rounds it nor draws its border round it. DWM's own system backdrop
    /// isn't cut by a window region: it was a square under each part, grey under a part that never takes the keyboard,
    /// Acrylic's colour for a window that isn't active.
    pub(super) fn glass(&self, hwnd: HWND) -> anyhow::Result<Glass> {
        use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
        let (backdrop, corners, border) = (BOOL::from(true), DWMWCP_DONOTROUND, DWMWA_COLOR_NONE);
        unsafe {
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, std::ptr::from_ref(&backdrop).cast(), size_of_val(&backdrop) as u32);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, std::ptr::from_ref(&corners).cast(), size_of_val(&corners) as u32);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, std::ptr::from_ref(&border).cast(), size_of_val(&border) as u32);
        }
        let compositor = &self.compositor;
        let target = unsafe { compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, false) }?;
        let whole = windows_numerics::Vector2 { X: 1.0, Y: 1.0 };
        let root = compositor.CreateContainerVisual()?;
        root.SetRelativeSizeAdjustment(whole)?;
        let shape = compositor.CreateRoundedRectangleGeometry()?;
        root.SetClip(&compositor.CreateGeometricClipWithGeometry(&shape)?)?;
        // WinUI's Acrylic for a window's backdrop, its tint over the blur: #FCFCFC at 85% light, #2C2C2C at 96% dark.
        let tint = match super::super::appearance() {
            crate::Appearance::Dark => windows::UI::Color { A: 245, R: 0x2c, G: 0x2c, B: 0x2c },
            crate::Appearance::Light => windows::UI::Color { A: 217, R: 0xfc, G: 0xfc, B: 0xfc },
        };
        let surface = unsafe { DCompositionCreateSurfaceHandle((COMPOSITIONOBJECT_READ | COMPOSITIONOBJECT_WRITE) as u32, None) }?;
        let page = compositor.CreateSurfaceBrushWithSurface(&unsafe { compositor.cast::<ICompositorInterop>()?.CreateCompositionSurfaceForHandle(surface) }?)?;
        // Its frames at their own size from its top left, never stretched to the window's as it changes size.
        page.SetStretch(CompositionStretch::None)?;
        page.SetHorizontalAlignmentRatio(0.0)?;
        page.SetVerticalAlignmentRatio(0.0)?;
        let brushes: [CompositionBrush; 3] = [compositor.CreateHostBackdropBrush()?.cast()?, compositor.CreateColorBrushWithColor(tint)?.cast()?, page.cast()?];
        for brush in brushes {
            let layer = compositor.CreateSpriteVisual()?;
            layer.SetRelativeSizeAdjustment(whole)?;
            layer.SetBrush(&brush)?;
            root.Children()?.InsertAtTop(&layer)?;
        }
        target.SetRoot(&root)?;
        Ok(Glass { _target: target, shape, surface })
    }
}

/// Cuts a panel's glass and page, `width` by `height` pixels, to round corners of `radius` pixels, its page's; again
/// only as its shape changes.
fn round(hwnd: HWND, width: i32, height: i32, radius: i32) {
    let Some(state) = state(hwnd) else { return };
    let Some(glass) = state.glass.get() else { return };
    if state.shape.replace((width, height, radius)) == (width, height, radius) {
        return;
    }
    let _ = glass.shape.SetSize(windows_numerics::Vector2 { X: width as f32, Y: height as f32 });
    let _ = glass.shape.SetCornerRadius(windows_numerics::Vector2 { X: radius as f32, Y: radius as f32 });
}

/// Keeps a panel's window `hwnd` off the screen while it's shown, or puts it back (DWM's cloak), as it was last asked.
fn cloak(hwnd: HWND, cloaked: bool) {
    use windows::Win32::Graphics::Dwm::DWMWA_CLOAK;
    let Some(state) = state(hwnd) else { return };
    if state.cloaked.replace(cloaked) == cloaked {
        return;
    }
    let cloaked = BOOL::from(cloaked);
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, std::ptr::from_ref(&cloaked).cast(), size_of::<BOOL>() as u32);
    }
}
