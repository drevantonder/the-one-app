// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_linux/src/linux/x11/client.rs and x11/window.rs)
// at commit 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the strict host: one connection to the X
// server for every window, each window made, titled, classed and closed as GPUI's are, its scale from Xft.dpi, its
// keys through xkbcommon, the input method through XIM (client.rs's `enable_ime`, `reset_ime`, `update_ime_position`
// and its event loop's XIM filter), its cursor from the cursor theme, and its surface handed to the renderer by its
// raw handles. A panel (a launcher's) has no frame, stays above, out of the taskbar, and is see-through at its
// corners where a compositor runs, as GPUI's transparent windows pick a 32-bit visual. No client-side decorations,
// popups, drag and drop or XInput yet; the clipboard is arboard's (`super::clipboard`).
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU32;
use std::ptr::NonNull;
use std::rc::Rc;
use std::cell::{Cell, RefCell};

use anyhow::Context as _;
use raw_window_handle as rwh;
use x11rb::connection::Connection;
use x11rb::cursor;
use x11rb::protocol::Event;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;
use xim::x11rb::X11rbClient;
use xim::{AttributeName, Client as _, ClientCore as _, InputStyle};
use xkbcommon::xkb as xkbc;

use super::keyboard;
use super::xim_handler::{XimCallbackEvent, XimHandler};
use crate::editing::Ime;
use crate::geometry::{Bounds, DevicePixels, Pixels, Point, Size};
use crate::platform::{Input, WindowId, WindowOptions};
use crate::Cursor;

x11rb::atom_manager! {
    pub XcbAtoms: AtomsCookie {
        UTF8_STRING,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_ACTIVE_WINDOW,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _MOTIF_WM_HINTS,
    }
}

pub struct Display {
    xcb: Rc<XCBConnection>,
    screen: usize,
    atoms: XcbAtoms,
    xkb: RefCell<xkbc::State>,
    xkb_context: xkbc::Context,
    /// Dead keys' and the Compose key's sequences, and what one so far shows, as GPUI's X11 platform types them
    /// without an input method.
    compose: RefCell<Option<xkbc::compose::State>>,
    composed: RefCell<String>,
    xkb_device: i32,
    cursors: cursor::Handle,
    loaded: RefCell<HashMap<Cursor, xproto::Cursor>>,
    scale: f32,
    windows: RefCell<HashMap<xproto::Window, WindowId>>,
    /// The input method's connection, where there's an XIM server (ibus, fcitx), and what it's told.
    xim: RefCell<Option<(X11rbClient<Rc<XCBConnection>>, XimHandler)>>,
    /// Each window whose input with the keyboard takes typing, by where its caret is, in device pixels: its keys go
    /// to the input method, which shows what it composes there.
    ime: RefCell<HashMap<xproto::Window, (i16, i16)>>,
}

pub struct NativeWindow {
    x_window: xproto::Window,
    raw: RawWindow,
    size: Cell<Size<DevicePixels>>,
    scale: f32,
    cursor: Cell<Cursor>,
    /// A panel's: whether it has a 32-bit visual, see-through where it's drawn so, and the window that was active
    /// before it was summoned, to give the keyboard back to.
    transparent: bool,
    before: Cell<xproto::Window>,
}

#[derive(Debug, Clone, Copy)]
struct RawWindow {
    connection: *mut c_void,
    screen_id: usize,
    window_id: u32,
    visual_id: u32,
}

impl rwh::HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let non_zero = NonZeroU32::new(self.raw.window_id).ok_or(rwh::HandleError::Unavailable)?;
        let mut handle = rwh::XcbWindowHandle::new(non_zero);
        handle.visual_id = NonZeroU32::new(self.raw.visual_id);
        Ok(unsafe { rwh::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl rwh::HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        let non_zero = NonNull::new(self.raw.connection).ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::XcbDisplayHandle::new(Some(non_zero), self.raw.screen_id as i32);
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(handle.into()) })
    }
}

impl NativeWindow {
    pub fn device_size(&self) -> Size<DevicePixels> {
        self.size.get()
    }

    pub fn size(&self) -> Size<Pixels> {
        let size = self.size.get();
        Size { width: Pixels(size.width.0 as f32 / self.scale), height: Pixels(size.height.0 as f32 / self.scale) }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn resized(&self, size: Size<DevicePixels>) {
        self.size.set(size);
    }

    /// Whether what isn't drawn on it is see-through.
    pub fn transparent(&self) -> bool {
        self.transparent
    }
}

/// Drops what the input method is composing, as GPUI's fork of xim-rs's `reset_ic` asks.
fn reset_ic(ximc: &mut X11rbClient<Rc<XCBConnection>>, handler: &XimHandler) {
    let reset = xim::Request::ResetIc { input_method_id: handler.im_id, input_context_id: handler.ic_id };
    if let Err(error) = ximc.send_req(reset) {
        log::warn!("the input method didn't drop its composition: {error}");
    }
}

/// The connection's file, for the event loop to wake when the X server says something.
struct Fd(Rc<XCBConnection>);

impl std::os::fd::AsFd for Fd {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        // Safety: the connection's socket lives as long as the connection.
        unsafe { std::os::fd::BorrowedFd::borrow_raw(std::os::fd::AsRawFd::as_raw_fd(&*self.0)) }
    }
}

impl Display {
    pub fn connect<T: 'static>(handle: &calloop::LoopHandle<'static, T>) -> anyhow::Result<Display> {
        let (xcb, screen) = XCBConnection::connect(None).context("no X display to open a window on")?;
        let atoms = XcbAtoms::new(&xcb)?.reply()?;
        let xkb = xcb.xkb_use_extension(1, 0)?.reply()?;
        anyhow::ensure!(xkb.supported, "X server does not support XKB");
        let events = xkb::EventType::STATE_NOTIFY | xkb::EventType::MAP_NOTIFY | xkb::EventType::NEW_KEYBOARD_NOTIFY;
        let parts = xkb::MapPart::KEY_TYPES
            | xkb::MapPart::KEY_SYMS
            | xkb::MapPart::MODIFIER_MAP
            | xkb::MapPart::EXPLICIT_COMPONENTS
            | xkb::MapPart::KEY_ACTIONS
            | xkb::MapPart::KEY_BEHAVIORS
            | xkb::MapPart::VIRTUAL_MODS
            | xkb::MapPart::VIRTUAL_MOD_MAP;
        xcb.xkb_select_events(xkb::ID::USE_CORE_KBD.into(), 0u8.into(), events, parts, parts, &xkb::SelectEventsAux::new())?.check()?;
        let xkb_context = xkbc::Context::new(xkbc::CONTEXT_NO_FLAGS);
        let xkb_device = xkbc::x11::get_core_keyboard_device_id(&xcb);
        let keymap = xkbc::x11::keymap_new_from_device(&xkb_context, &xcb, xkb_device, xkbc::KEYMAP_COMPILE_NO_FLAGS);
        let xkb_state = xkbc::x11::state_new_from_device(&keymap, &xcb, xkb_device);

        let resources = x11rb::resource_manager::new_from_default(&xcb).context("Failed to create resource database")?;
        let scale = resources.get_value::<f32>("Xft.dpi", "Xft.dpi").ok().flatten().map_or(1.0, |dpi| dpi / 96.0);
        let cursors = cursor::Handle::new(&xcb, screen, &resources)?.reply()?;

        let xcb = Rc::new(xcb);
        let xim = X11rbClient::init(Rc::clone(&xcb), screen, None).ok().map(|client| (client, XimHandler::new()));
        handle
            .insert_source(calloop::generic::Generic::new(Fd(xcb.clone()), calloop::Interest::READ, calloop::Mode::Level), |_, _, _| {
                // The loop's turn reads what's queued (`events`).
                Ok(calloop::PostAction::Continue)
            })
            .map_err(|error| anyhow::anyhow!("Failed to initialize X11 event source: {error}"))?;
        Ok(Display {
            xcb,
            screen,
            atoms,
            xkb: RefCell::new(xkb_state),
            compose: RefCell::new(keyboard::compose(&xkb_context)),
            composed: RefCell::default(),
            xkb_context,
            xkb_device,
            cursors,
            loaded: RefCell::default(),
            scale,
            windows: RefCell::default(),
            xim: RefCell::new(xim),
            ime: RefCell::default(),
        })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let xcb = &self.xcb;
        let setup = &xcb.setup().roots[self.screen];
        let x_window = xcb.generate_id()?;
        let width = (options.size.width.0 * self.scale).round() as u16;
        let height = (options.size.height.0 * self.scale).round() as u16;
        // A panel is see-through at its rounded corners where a compositor draws windows over each other: a 32-bit
        // visual, with a colormap of its own, as GPUI's transparent windows have.
        let argb = options.panel.is_some().then(|| self.argb()).flatten();
        let (depth, visual) = argb.unwrap_or((setup.root_depth, setup.root_visual));
        let mut aux = xproto::CreateWindowAux::new();
        if argb.is_some() {
            let colormap = xcb.generate_id()?;
            xcb.create_colormap(xproto::ColormapAlloc::NONE, colormap, setup.root, visual)?.check()?;
            aux = aux.background_pixel(0).border_pixel(0).colormap(colormap);
        } else {
            aux = aux.background_pixel(setup.black_pixel);
        }
        let aux = aux
            .event_mask(
                xproto::EventMask::EXPOSURE
                    | xproto::EventMask::STRUCTURE_NOTIFY
                    | xproto::EventMask::FOCUS_CHANGE
                    | xproto::EventMask::KEY_PRESS
                    | xproto::EventMask::KEY_RELEASE
                    | xproto::EventMask::BUTTON_PRESS
                    | xproto::EventMask::BUTTON_RELEASE
                    | xproto::EventMask::POINTER_MOTION
                    | xproto::EventMask::LEAVE_WINDOW,
            );
        xcb.create_window(depth, x_window, setup.root, 0, 0, width, height, 0, xproto::WindowClass::INPUT_OUTPUT, visual, &aux)?
        .check()
        .context("X11 CreateWindow failed")?;
        xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms._NET_WM_PID, xproto::AtomEnum::CARDINAL, &[std::process::id()])?;
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, xproto::AtomEnum::WM_NAME, xproto::AtomEnum::STRING, options.title.as_bytes())?;
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING, options.title.as_bytes())?;
        // WM_CLASS is the instance and the class, each ending in a NUL.
        let class = format!("{0}\0{0}\0", options.app_id);
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, xproto::AtomEnum::WM_CLASS, xproto::AtomEnum::STRING, class.as_bytes())?;
        xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms.WM_PROTOCOLS, xproto::AtomEnum::ATOM, &[self.atoms.WM_DELETE_WINDOW])?;
        if options.panel.is_some() {
            // No frame (Motif's hints, which every window manager reads: flags say decorations, which are none),
            // above every other window and out of the taskbar and the pager (EWMH's initial state), and the size
            // it's given, at the place it's put (`summon`).
            xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms._MOTIF_WM_HINTS, self.atoms._MOTIF_WM_HINTS, &[2, 0, 0, 0, 0])?;
            let state = [self.atoms._NET_WM_STATE_ABOVE, self.atoms._NET_WM_STATE_SKIP_TASKBAR, self.atoms._NET_WM_STATE_SKIP_PAGER];
            xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms._NET_WM_STATE, xproto::AtomEnum::ATOM, &state)?;
            self.hint(x_window, 0, 0, width, height)?;
        }
        xcb.flush()?;
        self.windows.borrow_mut().insert(x_window, id);
        let raw = RawWindow {
            connection: xcb.get_raw_xcb_connection(),
            screen_id: self.screen,
            window_id: x_window,
            visual_id: visual,
        };
        Ok(NativeWindow {
            x_window,
            raw,
            size: Cell::new(Size { width: DevicePixels(width as i32), height: DevicePixels(height as i32) }),
            scale: self.scale,
            cursor: Cell::new(Cursor::Arrow),
            transparent: argb.is_some(),
            before: Cell::new(x11rb::NONE),
        })
    }

    /// The screen's 32-bit TrueColor visual, where a compositor runs to blend it (`_NET_WM_CM_S<screen>` is owned).
    fn argb(&self) -> Option<(u8, xproto::Visualid)> {
        let compositor = self.xcb.intern_atom(false, format!("_NET_WM_CM_S{}", self.screen).as_bytes()).ok()?.reply().ok()?.atom;
        if self.xcb.get_selection_owner(compositor).ok()?.reply().ok()?.owner == x11rb::NONE {
            return None;
        }
        let setup = &self.xcb.setup().roots[self.screen];
        setup.allowed_depths.iter().filter(|depth| depth.depth == 32).find_map(|depth| {
            depth.visuals.iter().find(|visual| visual.class == xproto::VisualClass::TRUE_COLOR).map(|visual| (32, visual.visual_id))
        })
    }

    /// Tells the window manager where a panel goes and that it's that size and no other (ICCCM's WM_NORMAL_HINTS).
    fn hint(&self, x_window: xproto::Window, x: i32, y: i32, width: u16, height: u16) -> anyhow::Result<()> {
        use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
        let mut hints = WmSizeHints::new();
        hints.position = Some((WmSizeHintsSpecification::UserSpecified, x, y));
        hints.min_size = Some((width.into(), height.into()));
        hints.max_size = Some((width.into(), height.into()));
        hints.set_normal_hints(&*self.xcb, x_window)?.check()?;
        Ok(())
    }

    /// The window that's active, as the window manager says (`_NET_ACTIVE_WINDOW`).
    fn active(&self) -> xproto::Window {
        let root = self.xcb.setup().roots[self.screen].root;
        let reply = self.xcb.get_property(false, root, self.atoms._NET_ACTIVE_WINDOW, xproto::AtomEnum::WINDOW, 0, 1).ok().and_then(|cookie| cookie.reply().ok());
        reply.and_then(|reply| reply.value32()?.next()).unwrap_or(x11rb::NONE)
    }

    /// The bounds of the screen the pointer is on, in device pixels (RandR's monitors), or the whole root window's.
    fn pointer_screen(&self) -> (i32, i32, i32, i32) {
        use x11rb::protocol::randr::ConnectionExt as _;
        let root = &self.xcb.setup().roots[self.screen];
        let whole = (0, 0, i32::from(root.width_in_pixels), i32::from(root.height_in_pixels));
        let Some(pointer) = self.xcb.query_pointer(root.root).ok().and_then(|cookie| cookie.reply().ok()) else { return whole };
        let (x, y) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
        let monitors = self.xcb.randr_get_monitors(root.root, true).ok().and_then(|cookie| cookie.reply().ok()).map(|reply| reply.monitors).unwrap_or_default();
        monitors
            .iter()
            .map(|monitor| (i32::from(monitor.x), i32::from(monitor.y), i32::from(monitor.width), i32::from(monitor.height)))
            .find(|(left, top, width, height)| x >= *left && y >= *top && x < left + width && y < top + height)
            .unwrap_or(whole)
    }

    /// Shows a panel centred near the top of the screen the pointer is on, a fifth of the way down as Spotlight sits,
    /// never off the bottom of a short screen, and gives it the keyboard, keeping which window had it.
    pub fn summon(&self, window: &NativeWindow) {
        let size = window.size.get();
        let (left, top, width, height) = self.pointer_screen();
        let x = left + (width - size.width.0) / 2;
        let y = top + (height / 5).min((height - size.height.0).max(0));
        let before = self.active();
        if before != window.x_window {
            window.before.set(before);
        }
        let _ = self.hint(window.x_window, x, y, size.width.0 as u16, size.height.0 as u16);
        let _ = self.xcb.configure_window(window.x_window, &xproto::ConfigureWindowAux::new().x(x).y(y));
        let _ = self.xcb.map_window(window.x_window);
        let _ = self.xcb.configure_window(window.x_window, &xproto::ConfigureWindowAux::new().x(x).y(y).stack_mode(xproto::StackMode::ABOVE));
        self.activate(window);
    }

    /// Puts a panel away, and with `give_back`, makes the window that had the keyboard before it active again.
    pub fn put_away(&self, window: &NativeWindow, give_back: bool) {
        let _ = self.xcb.unmap_window(window.x_window);
        let before = window.before.replace(x11rb::NONE);
        if give_back && before != x11rb::NONE {
            self.activate_x(before);
        }
        let _ = self.xcb.flush();
    }

    /// Makes a window `size`, its top left where it is.
    pub fn resize(&self, window: &NativeWindow, size: Size<Pixels>) {
        let (width, height) = ((size.width.0 * self.scale).round() as u16, (size.height.0 * self.scale).round() as u16);
        if let Ok(geometry) = self.xcb.get_geometry(window.x_window).map(|cookie| cookie.reply()) {
            let (x, y) = geometry.map(|at| (i32::from(at.x), i32::from(at.y))).unwrap_or_default();
            let _ = self.hint(window.x_window, x, y, width, height);
        }
        let _ = self.xcb.configure_window(window.x_window, &xproto::ConfigureWindowAux::new().width(u32::from(width)).height(u32::from(height)));
        let _ = self.xcb.flush();
    }

    /// Puts a window opened hidden on the screen.
    pub fn show(&self, window: &NativeWindow) {
        let _ = self.xcb.map_window(window.x_window);
        let _ = self.xcb.flush();
    }

    pub fn close(&self, window: &NativeWindow) {
        self.windows.borrow_mut().remove(&window.x_window);
        self.ime.borrow_mut().remove(&window.x_window);
        let _ = self.xcb.destroy_window(window.x_window);
        let _ = self.xcb.flush();
    }

    pub fn activate(&self, window: &NativeWindow) {
        self.activate_x(window.x_window);
    }

    /// Asks the window manager to make `x_window` active, as a pager does (source 2), which it does for whichever
    /// app's window it is.
    fn activate_x(&self, x_window: xproto::Window) {
        let root = self.xcb.setup().roots[self.screen].root;
        let data = [2, xproto::Time::CURRENT_TIME.into(), 0, 0, 0];
        let message = xproto::ClientMessageEvent::new(32, x_window, self.atoms._NET_ACTIVE_WINDOW, data);
        let _ = self.xcb.send_event(false, root, xproto::EventMask::SUBSTRUCTURE_REDIRECT | xproto::EventMask::SUBSTRUCTURE_NOTIFY, message);
        let _ = self.xcb.flush();
    }

    pub fn set_cursor(&self, window: &NativeWindow, cursor: Cursor) {
        if window.cursor.replace(cursor) == cursor {
            return;
        }
        let name = match cursor {
            Cursor::Arrow => "default",
            Cursor::Pointer => "pointer",
            Cursor::Text => "text",
        };
        let loaded = *self.loaded.borrow_mut().entry(cursor).or_insert_with(|| self.cursors.load_cursor(&*self.xcb, name).unwrap_or(x11rb::NONE));
        let _ = self.xcb.change_window_attributes(window.x_window, &xproto::ChangeWindowAttributesAux::new().cursor(loaded));
    }

    pub fn flush(&self) {
        let _ = self.xcb.flush();
    }

    /// What the X server has said since it was last asked, by the window it's about. Each event goes by the input
    /// method first, which takes what's its own, and the keys of a window whose input takes typing go to it, which
    /// sends back what it doesn't compose.
    pub fn events(&self) -> Vec<(WindowId, Input)> {
        let mut out = Vec::new();
        while let Ok(Some(event)) = self.xcb.poll_for_event() {
            let Some((mut ximc, mut handler)) = self.xim.borrow_mut().take() else {
                self.event(event, &mut out);
                continue;
            };
            let connected = handler.connected;
            // xim-rs panics on compound text it can't decode: its connection is dropped, as on an error.
            let filtered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ximc.filter_event(&event, &mut handler)))
                .unwrap_or_else(|_| Err(xim::ClientError::InvalidReply));
            let told = handler.last_callback_event.take();
            *self.xim.borrow_mut() = Some((ximc, handler));
            if let Some(told) = told {
                self.xim_told(told, &mut out);
            }
            match filtered {
                Ok(true) => {}
                Ok(false) => match event {
                    Event::KeyPress(key) | Event::KeyRelease(key) if connected && self.ime.borrow().contains_key(&key.event) => self.forward(key),
                    event => self.event(event, &mut out),
                },
                Err(error) => {
                    // The XIM server went (it crashed, or was quit): keys are typed without it from now on.
                    log::error!("the input method's XIM connection failed: {error}");
                    *self.xim.borrow_mut() = None;
                    self.event(event, &mut out);
                }
            }
        }
        out
    }

    /// What the input method said: a key it didn't take, what it committed, or what it's composing.
    fn xim_told(&self, told: XimCallbackEvent, out: &mut Vec<(WindowId, Input)>) {
        match told {
            XimCallbackEvent::XimXEvent(event) => self.event(event, out),
            XimCallbackEvent::XimCommitEvent(window, text) => {
                if let Some(id) = self.window(window) {
                    out.push((id, Input::Ime(Ime::Commit(text))));
                }
            }
            XimCallbackEvent::XimPreeditEvent(window, text, caret) => {
                if let Some(id) = self.window(window) {
                    let cursor = text.char_indices().nth(caret).map_or(text.len(), |(at, _)| at);
                    out.push((id, Input::Ime(Ime::Preedit(text, Some(cursor)))));
                }
            }
        }
    }

    /// Steps the dead keys' and Compose key's sequence with `keysym`, as GPUI's X11 key press does: while it's on
    /// its way, what it has so far shows in the input as what's being composed, and once it's done its letter is
    /// committed there; a sequence the key doesn't fit writes what it showed and starts again from the key. → whether
    /// the sequence took the key, so it's typed no other way.
    fn compose(&self, id: WindowId, keysym: xkbc::Keysym, typed: String, out: &mut Vec<(WindowId, Input)>) -> bool {
        use xkbc::compose::{FeedResult, Status};
        let mut compose = self.compose.borrow_mut();
        let Some(compose) = compose.as_mut() else { return false };
        if compose.feed(keysym) == FeedResult::Ignored {
            return false;
        }
        // A dead key shows its accent, the Compose key a dot, as GTK draws them, and any other key what it types.
        let shown = keyboard::dead_key(keysym).or_else(|| (keysym == xkbc::Keysym::Multi_key).then(|| "·".to_owned())).unwrap_or(typed);
        let mut composed = self.composed.borrow_mut();
        match compose.status() {
            Status::Composing => {
                composed.push_str(&shown);
                out.push((id, Input::Ime(Ime::Preedit(composed.clone(), None))));
                true
            }
            Status::Composed => {
                composed.clear();
                out.push((id, Input::Ime(Ime::Commit(compose.utf8().unwrap_or_default()))));
                compose.reset();
                true
            }
            Status::Cancelled => {
                compose.reset();
                if !composed.is_empty() {
                    out.push((id, Input::Ime(Ime::Commit(std::mem::take(&mut *composed)))));
                }
                drop(composed);
                // The key may start a sequence of its own, or be typed.
                compose.feed(keysym) != FeedResult::Ignored && compose.status() == Status::Composing && {
                    *self.composed.borrow_mut() = shown.clone();
                    out.push((id, Input::Ime(Ime::Preedit(shown, None))));
                    true
                }
            }
            Status::Nothing => false,
        }
    }

    /// Ends a dead key's sequence where it is, as a press or the window losing the keyboard does.
    fn end_compose(&self) {
        if let Some(compose) = self.compose.borrow_mut().as_mut() {
            compose.reset();
        }
        self.composed.borrow_mut().clear();
    }

    /// Gives the input method a key, as GPUI's `xim_handle_event` does.
    fn forward(&self, key: xproto::KeyPressEvent) {
        let mut xim = self.xim.borrow_mut();
        let Some((ximc, handler)) = xim.as_mut() else { return };
        handler.window = key.event;
        if let Err(error) = ximc.forward_event(handler.im_id, handler.ic_id, xim::ForwardEventFlag::empty(), &key) {
            log::warn!("the input method didn't take a key: {error}");
        }
    }

    /// The input with window `window`'s keyboard takes typing, its caret at `caret` (in the window's logical pixels),
    /// or none does: the input method's keys and where it shows what it composes, as GPUI's `update_ime_position`.
    /// An input that stops taking typing drops what was being composed in it, as GPUI's `reset_ime`.
    pub fn set_ime(&self, window: &NativeWindow, caret: Option<Bounds<f32>>) {
        let spot = caret.map(|caret| ((caret.origin.x * self.scale) as i16, ((caret.origin.y + caret.size.height) * self.scale) as i16));
        let was = match spot {
            Some(spot) => self.ime.borrow_mut().insert(window.x_window, spot),
            None => self.ime.borrow_mut().remove(&window.x_window),
        };
        if was == spot {
            return;
        }
        let mut xim = self.xim.borrow_mut();
        let Some((ximc, handler)) = xim.as_mut().filter(|(_, handler)| handler.connected) else { return };
        let Some((x, y)) = spot else {
            reset_ic(ximc, handler);
            return;
        };
        if handler.window != window.x_window {
            return;
        }
        let attributes = ximc
            .build_ic_attributes()
            .push(AttributeName::InputStyle, InputStyle::PREEDIT_CALLBACKS)
            .push(AttributeName::ClientWindow, handler.window)
            .push(AttributeName::FocusWindow, handler.window)
            .nested_list(AttributeName::PreeditAttributes, |b| {
                b.push(AttributeName::SpotLocation, xim::Point { x, y });
            })
            .build();
        if let Err(error) = ximc.set_ic_values(handler.im_id, handler.ic_id, attributes) {
            log::warn!("the input method wasn't told where the caret is: {error}");
        }
    }

    /// A window got the keyboard: the input method's input context is made for it, as GPUI's `enable_ime`.
    fn enable_ime(&self, x_window: xproto::Window) {
        let mut xim = self.xim.borrow_mut();
        let Some((ximc, handler)) = xim.as_mut() else { return };
        handler.window = x_window;
        let attributes = ximc
            .build_ic_attributes()
            .push(AttributeName::InputStyle, InputStyle::PREEDIT_CALLBACKS)
            .push(AttributeName::ClientWindow, x_window)
            .push(AttributeName::FocusWindow, x_window)
            .build();
        if let Err(error) = ximc.create_ic(handler.im_id, attributes) {
            log::warn!("the input method made no input context: {error}");
        }
    }

    fn window(&self, x_window: xproto::Window) -> Option<WindowId> {
        self.windows.borrow().get(&x_window).copied()
    }

    fn at(&self, x: i16, y: i16) -> Point<f32> {
        Point { x: x as f32 / self.scale, y: y as f32 / self.scale }
    }

    fn event(&self, event: Event, out: &mut Vec<(WindowId, Input)>) {
        match event {
            Event::ClientMessage(event) => {
                let data = event.data.as_data32();
                if data[0] == self.atoms.WM_DELETE_WINDOW
                    && let Some(id) = self.window(event.window)
                {
                    out.push((id, Input::Close));
                }
            }
            Event::ConfigureNotify(event) => {
                if let Some(id) = self.window(event.window) {
                    out.push((id, Input::Resized(Size { width: DevicePixels(event.width as i32), height: DevicePixels(event.height as i32) })));
                }
            }
            Event::Expose(event) => {
                if let Some(id) = self.window(event.window) {
                    out.push((id, Input::Expose));
                }
            }
            Event::MotionNotify(event) => {
                if let Some(id) = self.window(event.event) {
                    out.push((id, Input::Moved(self.at(event.event_x, event.event_y))));
                }
            }
            Event::ButtonPress(event) => {
                let Some(id) = self.window(event.event) else { return };
                // A press ends a dead key's sequence: the input writes what it showed, as GPUI's does.
                self.end_compose();
                let at = self.at(event.event_x, event.event_y);
                // Buttons 1 to 3 are the main, middle and secondary; 4 and 5 the wheel's turns, up and down, 6 and 7
                // sideways; a notch is 3 lines of 16 px.
                let notch = |x: f32, y: f32| Input::Wheel(at, Point { x, y });
                match event.detail {
                    button @ 1..=3 => out.push((id, Input::Down(at, button - 1, event.state.contains(xproto::KeyButMask::SHIFT)))),
                    4 => out.push((id, notch(0.0, -48.0))),
                    5 => out.push((id, notch(0.0, 48.0))),
                    6 => out.push((id, notch(-48.0, 0.0))),
                    7 => out.push((id, notch(48.0, 0.0))),
                    _ => {}
                }
            }
            Event::ButtonRelease(event) => {
                if (1..=3).contains(&event.detail)
                    && let Some(id) = self.window(event.event)
                {
                    out.push((id, Input::Up(self.at(event.event_x, event.event_y), event.detail - 1)));
                }
            }
            Event::FocusIn(event) => {
                if let Some(id) = self.window(event.event) {
                    self.enable_ime(event.event);
                    out.push((id, Input::Focus(true)));
                }
            }
            Event::FocusOut(event) => {
                if let Some(id) = self.window(event.event) {
                    if let Some((ximc, handler)) = self.xim.borrow_mut().as_mut() {
                        reset_ic(ximc, handler);
                    }
                    self.end_compose();
                    out.push((id, Input::Focus(false)));
                }
            }
            Event::LeaveNotify(event) => {
                if let Some(id) = self.window(event.event) {
                    out.push((id, Input::Left));
                }
            }
            Event::KeyPress(event) => {
                let Some(id) = self.window(event.event) else { return };
                let state = keyboard::state_of(&self.xkb.borrow(), event.state);
                let keysym = state.key_get_one_sym(event.detail.into());
                if !self.compose(id, keysym, state.key_get_utf8(event.detail.into()), out) {
                    out.push((id, Input::Key(keyboard::key(&state, event.detail.into()))));
                }
            }
            Event::XkbStateNotify(event) => {
                self.xkb.borrow_mut().update_mask(
                    event.base_mods.into(),
                    event.latched_mods.into(),
                    event.locked_mods.into(),
                    event.base_group as u32,
                    event.latched_group as u32,
                    event.locked_group.into(),
                );
            }
            Event::XkbNewKeyboardNotify(_) | Event::XkbMapNotify(_) => {
                let keymap = xkbc::x11::keymap_new_from_device(&self.xkb_context, &*self.xcb, self.xkb_device, xkbc::KEYMAP_COMPILE_NO_FLAGS);
                *self.xkb.borrow_mut() = xkbc::x11::state_new_from_device(&keymap, &*self.xcb, self.xkb_device);
            }
            _ => {}
        }
    }
}
