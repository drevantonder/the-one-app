// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_linux/src/linux/x11/client.rs and x11/window.rs)
// at commit 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the strict host: one connection to the X
// server for every window, each window made, titled, classed and closed as GPUI's are, its scale from Xft.dpi, its
// keys through xkbcommon, its cursor from the cursor theme, and its surface handed to the renderer by its raw handles.
// No client-side decorations, transparency, popups, clipboard, drag and drop or XInput yet.
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
use xkbcommon::xkb as xkbc;

use super::keyboard;
use crate::geometry::{DevicePixels, Pixels, Point, Size};
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
    }
}

pub struct Display {
    xcb: Rc<XCBConnection>,
    screen: usize,
    atoms: XcbAtoms,
    xkb: RefCell<xkbc::State>,
    xkb_context: xkbc::Context,
    xkb_device: i32,
    cursors: cursor::Handle,
    loaded: RefCell<HashMap<Cursor, xproto::Cursor>>,
    scale: f32,
    windows: RefCell<HashMap<xproto::Window, WindowId>>,
}

pub struct NativeWindow {
    x_window: xproto::Window,
    raw: RawWindow,
    size: Cell<Size<DevicePixels>>,
    scale: f32,
    cursor: Cell<Cursor>,
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
            xkb_context,
            xkb_device,
            cursors,
            loaded: RefCell::default(),
            scale,
            windows: RefCell::default(),
        })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let xcb = &self.xcb;
        let setup = &xcb.setup().roots[self.screen];
        let x_window = xcb.generate_id()?;
        let width = (options.size.width.0 * self.scale).round() as u16;
        let height = (options.size.height.0 * self.scale).round() as u16;
        let aux = xproto::CreateWindowAux::new()
            .background_pixel(setup.black_pixel)
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
        xcb.create_window(
            setup.root_depth,
            x_window,
            setup.root,
            0,
            0,
            width,
            height,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            setup.root_visual,
            &aux,
        )?
        .check()
        .context("X11 CreateWindow failed")?;
        xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms._NET_WM_PID, xproto::AtomEnum::CARDINAL, &[std::process::id()])?;
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, xproto::AtomEnum::WM_NAME, xproto::AtomEnum::STRING, options.title.as_bytes())?;
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING, options.title.as_bytes())?;
        // WM_CLASS is the instance and the class, each ending in a NUL.
        let class = format!("{0}\0{0}\0", options.app_id);
        xcb.change_property8(xproto::PropMode::REPLACE, x_window, xproto::AtomEnum::WM_CLASS, xproto::AtomEnum::STRING, class.as_bytes())?;
        xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms.WM_PROTOCOLS, xproto::AtomEnum::ATOM, &[self.atoms.WM_DELETE_WINDOW])?;
        xcb.map_window(x_window)?;
        xcb.flush()?;
        self.windows.borrow_mut().insert(x_window, id);
        let raw = RawWindow {
            connection: xcb.get_raw_xcb_connection(),
            screen_id: self.screen,
            window_id: x_window,
            visual_id: setup.root_visual,
        };
        Ok(NativeWindow {
            x_window,
            raw,
            size: Cell::new(Size { width: DevicePixels(width as i32), height: DevicePixels(height as i32) }),
            scale: self.scale,
            cursor: Cell::new(Cursor::Arrow),
        })
    }

    pub fn close(&self, window: &NativeWindow) {
        self.windows.borrow_mut().remove(&window.x_window);
        let _ = self.xcb.destroy_window(window.x_window);
        let _ = self.xcb.flush();
    }

    pub fn activate(&self, window: &NativeWindow) {
        let root = self.xcb.setup().roots[self.screen].root;
        let data = [1, xproto::Time::CURRENT_TIME.into(), 0, 0, 0];
        let message = xproto::ClientMessageEvent::new(32, window.x_window, self.atoms._NET_ACTIVE_WINDOW, data);
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

    /// What the X server has said since it was last asked, by the window it's about.
    pub fn events(&self) -> Vec<(WindowId, Input)> {
        let mut out = Vec::new();
        while let Ok(Some(event)) = self.xcb.poll_for_event() {
            self.event(event, &mut out);
        }
        out
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
                let at = self.at(event.event_x, event.event_y);
                // Buttons 4 and 5 are the wheel's turns, up and down, 6 and 7 sideways; a notch is 3 lines of 16 px.
                match event.detail {
                    1 => out.push((id, Input::Down(at))),
                    4 => out.push((id, Input::Wheel(at, -48.0))),
                    5 => out.push((id, Input::Wheel(at, 48.0))),
                    _ => {}
                }
            }
            Event::ButtonRelease(event) => {
                if event.detail == 1
                    && let Some(id) = self.window(event.event)
                {
                    out.push((id, Input::Up(self.at(event.event_x, event.event_y))));
                }
            }
            Event::KeyPress(event) => {
                let Some(id) = self.window(event.event) else { return };
                let key = keyboard::key(&self.xkb.borrow(), event.detail.into());
                out.push((id, Input::Key(key)));
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
