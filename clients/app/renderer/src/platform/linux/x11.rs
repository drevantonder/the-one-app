// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_linux/src/linux/x11/client.rs and x11/window.rs)
// at commit 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the native host: one connection to the X
// server for every window, each window made, titled, classed and closed as GPUI's are, its scale from Xft.dpi, its
// keys through xkbcommon, the input method through XIM (client.rs's `enable_ime`, `reset_ime`, `update_ime_position`
// and its event loop's XIM filter), its cursor from the cursor theme, and its surface handed to the renderer by its
// raw handles. A panel (a launcher's) has no frame, stays above, out of the taskbar, and is see-through at its
// corners where a compositor runs, as GPUI's transparent windows pick a 32-bit visual. No client-side decorations,
// popups, drag and drop or XInput yet; the clipboard is arboard's (`super::clipboard`).
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

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
use x11rb::protocol::present::{self, ConnectionExt as _};
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
        _NET_WM_MOVERESIZE,
        _NET_WORKAREA,
        _MOTIF_WM_HINTS,
        _NET_WM_STATE_MODAL,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _GTK_THEME_VARIANT,
        XdndAware,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndSelection,
        XdndActionCopy,
        TEXT_URI_LIST: b"text/uri-list",
        WM_CHANGE_STATE,
        _NET_WM_STATE_MAXIMIZED_VERT,
        _NET_WM_STATE_MAXIMIZED_HORZ,
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
    loaded: RefCell<HashMap<&'static str, xproto::Cursor>>,
    scale: f32,
    /// Each window's window and part (`Who`).
    windows: RefCell<HashMap<xproto::Window, Who>>,
    /// Each panel's window and its parts' (`arrange`): the panel's, and its box of the page, in points.
    parts: RefCell<HashMap<xproto::Window, (xproto::Window, Point<f32>)>>,
    /// The panels showing, whose parts are shown with them.
    shown: RefCell<std::collections::HashSet<xproto::Window>>,
    /// Where a press moves each panel (`set_title_row`), and the panels a person is dragging, since a press there and
    /// until they're summoned again, which say where they're left (`Input::Placed`).
    rows: RefCell<HashMap<xproto::Window, crate::view::TitleRow>>,
    dragged: RefCell<std::collections::HashSet<xproto::Window>>,
    /// The input method's connection, where there's an XIM server (ibus, fcitx), and what it's told.
    xim: RefCell<Option<(X11rbClient<Rc<XCBConnection>>, XimHandler)>>,
    /// Each window whose input with the keyboard takes typing, by where its caret is, in device pixels: its keys go
    /// to the input method, which shows what it composes there.
    ime: RefCell<HashMap<xproto::Window, (i16, i16)>>,
    /// The root window, whose next frame Present is asked to say it showed, where the X server has Present (all do
    /// since 2013, Xwayland and Xvfb too); whether it's been asked, and when the last frame asked for was shown.
    present: Option<xproto::Window>,
    asked: Cell<bool>,
    framed: Cell<Option<std::time::Instant>>,
    /// The desktop went light or dark since its windows were last told (`appearance_changes`).
    appearance: Rc<Cell<Option<crate::Appearance>>>,
    /// Each app window with no frame of the window manager's (`unframe`), with its size in device pixels; the edge
    /// of one the pointer is at, which resizes it (`EDGES`); a press held in a title row, where it was on the screen,
    /// which moves its window once it's dragged a few pixels, as GTK's does; and the last press there, by its time,
    /// for a double click.
    unframed: RefCell<HashMap<xproto::Window, (u16, u16)>>,
    edge: Cell<Option<(xproto::Window, usize)>>,
    title_held: Cell<Option<(xproto::Window, i16, i16)>>,
    title_pressed: Cell<Option<(xproto::Window, u32)>>,
    /// A drag from another app over a window (XDND's): its source, and where it is in the window, in points.
    dragging: Cell<Option<(xproto::Window, xproto::Window, Point<f32>)>>,
    /// Each app window's file of where it was left (`WindowOptions::frame`), kept as it's moved or resized.
    frames: RefCell<HashMap<xproto::Window, std::path::PathBuf>>,
    /// What's said of a window on another thread (GTK's menu for text), and what was since the windows were last
    /// told.
    told: Rc<RefCell<Vec<(xproto::Window, Input)>>>,
    tell: calloop::channel::Sender<(xproto::Window, Input)>,
}

/// Which window an event is for, and which of its parts: 0 the window itself, the rest a panel's parts.
type Who = (WindowId, usize);

/// An unframed window's edges and corners, from the top left round, as EWMH numbers its resizes, each with the
/// pointer's shape there, by the cursor theme's names.
const EDGES: [(u32, &str); 8] = [(0, "top_left_corner"), (1, "top_side"), (2, "top_right_corner"), (3, "right_side"), (4, "bottom_right_corner"), (5, "bottom_side"), (6, "bottom_left_corner"), (7, "left_side")];

pub struct NativeWindow {
    x_window: xproto::Window,
    raw: RawWindow,
    size: Cell<Size<DevicePixels>>,
    scale: f32,
    cursor: Cell<&'static str>,
    /// A panel's: whether it has a 32-bit visual, see-through where it's drawn so, and the window that was active
    /// before it was summoned, to give the keyboard back to.
    transparent: bool,
    before: Cell<xproto::Window>,
    panel: bool,
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

/// A window as `Display::arrange` places it: its X window and its size now.
#[derive(Clone, Copy)]
pub struct Handle {
    window: xproto::Window,
    size: Size<DevicePixels>,
}

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle { window: self.x_window, size: self.size.get() }
    }

    /// What wgpu draws into: the window, by its raw handles.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        // Safety: a window's renderer is let go before the window is.
        Ok(unsafe { wgpu::SurfaceTargetUnsafe::from_display_and_window(self, self)? })
    }

    /// An X11 window has its surface from the start.
    pub fn has_surface(&self) -> bool {
        true
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.size.get()
    }

    /// Its X11 window.
    pub fn x_window(&self) -> xproto::Window {
        self.x_window
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

/// A monitor's name, by where it is and how large, which a panel's places are kept by.
fn screen_name((left, top, width, height): (i32, i32, i32, i32)) -> String {
    format!("{width}x{height}+{left}+{top}")
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

        // Each frame the display shows is said by Present, once asked (`request_frame`).
        let root = xcb.setup().roots[screen].root;
        let present = xcb.present_query_version(1, 0)?.reply().ok().and_then(|_| {
            let id = xcb.generate_id().ok()?;
            xcb.present_select_input(id, root, present::EventMask::COMPLETE_NOTIFY).ok()?.check().ok()?;
            Some(root)
        });
        let xcb = Rc::new(xcb);
        let xim = X11rbClient::init(Rc::clone(&xcb), screen, None).ok().map(|client| (client, XimHandler::new()));
        handle
            .insert_source(calloop::generic::Generic::new(Fd(xcb.clone()), calloop::Interest::READ, calloop::Mode::Level), |_, _, _| {
                // The loop's turn reads what's queued (`events`).
                Ok(calloop::PostAction::Continue)
            })
            .map_err(|error| anyhow::anyhow!("Failed to initialize X11 event source: {error}"))?;
        // The desktop going light or dark reaches every window as it does, as its portal says.
        let (to, changes) = calloop::channel::channel();
        std::thread::spawn(move || super::appearance_changes(to));
        let appearance = Rc::new(Cell::new(None));
        let heard = appearance.clone();
        handle
            .insert_source(changes, move |event, _, _| {
                if let calloop::channel::Event::Msg(now) = event {
                    heard.set(Some(now));
                }
            })
            .map_err(|error| anyhow::anyhow!("no source for the desktop's appearance: {error}"))?;
        // What's said of a window on another thread (GTK's) is done on the loop's.
        let (tell, telling) = calloop::channel::channel();
        let told = Rc::new(RefCell::new(Vec::new()));
        let heard = told.clone();
        handle
            .insert_source(telling, move |event, _, _| {
                if let calloop::channel::Event::Msg(said) = event {
                    heard.borrow_mut().push(said);
                }
            })
            .map_err(|error| anyhow::anyhow!("no source for what other threads say: {error}"))?;
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
            parts: RefCell::default(),
            shown: RefCell::default(),
            rows: RefCell::default(),
            dragged: RefCell::default(),
            xim: RefCell::new(xim),
            ime: RefCell::default(),
            present,
            asked: Cell::new(false),
            framed: Cell::new(None),
            appearance,
            frames: RefCell::default(),
            dragging: Cell::new(None),
            unframed: RefCell::default(),
            edge: Cell::new(None),
            title_held: Cell::new(None),
            title_pressed: Cell::new(None),
            told,
            tell,
        })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        // An app's window opens where it was left, as big.
        let left = options.frame.as_deref().and_then(crate::platform::Frame::read);
        let width = left.map_or((options.size.width.0 * self.scale).round() as u16, |left| left.width.clamp(1, i32::from(u16::MAX)) as u16);
        let height = left.map_or((options.size.height.0 * self.scale).round() as u16, |left| left.height.clamp(1, i32::from(u16::MAX)) as u16);
        let window = self.create((id, 0), width, height, options.panel.is_some(), false)?;
        let x_window = window.x_window;
        let xcb = &self.xcb;
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
        } else {
            // An app's window is made no smaller than its smallest (ICCCM's WM_NORMAL_HINTS).
            let mut hints = x11rb::properties::WmSizeHints::new();
            // Where it was left is its own top left, not its frame's.
            if let Some(left) = left {
                hints.position = Some((x11rb::properties::WmSizeHintsSpecification::UserSpecified, left.x, left.y));
                hints.win_gravity = Some(xproto::Gravity::STATIC);
                xcb.configure_window(x_window, &xproto::ConfigureWindowAux::new().x(left.x).y(left.y))?;
            }
            if let Some(file) = &options.frame {
                self.frames.borrow_mut().insert(x_window, file.clone());
            }
            // Files can be dropped on it from another app (XDND, its version 5).
            xcb.change_property32(xproto::PropMode::REPLACE, x_window, self.atoms.XdndAware, xproto::AtomEnum::ATOM, &[5])?;
            let smallest = crate::platform::SMALLEST;
            hints.min_size = Some(((smallest.width.0 * self.scale).round() as i32, (smallest.height.0 * self.scale).round() as i32));
            hints.set_normal_hints(&**xcb, x_window)?.check()?;
        }
        xcb.flush()?;
        Ok(window)
    }

    /// A part of panel `parent`'s page as a window of its own, its `part`th: one the window manager leaves alone
    /// (override-redirect), as a menu's is, so it never takes the keyboard, which stays with the panel, nor is in the
    /// taskbar; it's put where its box of the page is, and moves with the panel (`arrange`).
    pub fn open_part(&self, parent: &NativeWindow, part: usize) -> anyhow::Result<NativeWindow> {
        let who = self.windows.borrow().get(&parent.x_window).copied().context("its panel is closed")?;
        let window = self.create((who.0, part), 1, 1, true, true)?;
        self.xcb.flush()?;
        Ok(window)
    }

    /// An X window `width` by `height` device pixels for `who`: a panel's see-through at its rounded corners where a
    /// compositor draws windows over each other, with a 32-bit visual and a colormap of its own, as GPUI's transparent
    /// windows have; and with `override_redirect`, one the window manager leaves alone.
    fn create(&self, who: Who, width: u16, height: u16, panel: bool, override_redirect: bool) -> anyhow::Result<NativeWindow> {
        let xcb = &self.xcb;
        let setup = &xcb.setup().roots[self.screen];
        let x_window = xcb.generate_id()?;
        let argb = panel.then(|| self.argb()).flatten();
        let (depth, visual) = argb.unwrap_or((setup.root_depth, setup.root_visual));
        let mut aux = xproto::CreateWindowAux::new().override_redirect(u32::from(override_redirect));
        if argb.is_some() {
            let colormap = xcb.generate_id()?;
            xcb.create_colormap(xproto::ColormapAlloc::NONE, colormap, setup.root, visual)?.check()?;
            aux = aux.background_pixel(0).border_pixel(0).colormap(colormap);
        } else {
            aux = aux.background_pixel(setup.black_pixel);
        }
        let aux = aux.event_mask(
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
        self.windows.borrow_mut().insert(x_window, who);
        let raw = RawWindow { connection: xcb.get_raw_xcb_connection(), screen_id: self.screen, window_id: x_window, visual_id: visual };
        Ok(NativeWindow {
            x_window,
            raw,
            size: Cell::new(Size { width: DevicePixels(width as i32), height: DevicePixels(height as i32) }),
            scale: self.scale,
            cursor: Cell::new("default"),
            transparent: argb.is_some(),
            before: Cell::new(x11rb::NONE),
            panel,
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
        let root = &self.xcb.setup().roots[self.screen];
        let Some(pointer) = self.xcb.query_pointer(root.root).ok().and_then(|cookie| cookie.reply().ok()) else {
            return (0, 0, i32::from(root.width_in_pixels), i32::from(root.height_in_pixels));
        };
        self.screen_at(i32::from(pointer.root_x), i32::from(pointer.root_y))
    }

    /// What of `monitor` the desktop's panels leave clear, as the window manager says (`_NET_WORKAREA`), or all of it.
    fn work_area(&self, monitor: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
        let root = self.xcb.setup().roots[self.screen].root;
        let area = self.xcb.get_property(false, root, self.atoms._NET_WORKAREA, xproto::AtomEnum::CARDINAL, 0, 4).ok().and_then(|cookie| cookie.reply().ok());
        let Some([x, y, w, h]) = area.and_then(|reply| reply.value32().map(|values| values.map(|value| value as i32).collect::<Vec<_>>())).and_then(|values| <[i32; 4]>::try_from(values).ok()) else {
            return monitor;
        };
        let (left, top) = (monitor.0.max(x), monitor.1.max(y));
        let (right, bottom) = ((monitor.0 + monitor.2).min(x + w), (monitor.1 + monitor.3).min(y + h));
        if right <= left || bottom <= top { monitor } else { (left, top, right - left, bottom - top) }
    }

    /// The monitor point `x`, `y` is on, as RandR says, or the whole screen: its left, top, width and height.
    fn screen_at(&self, x: i32, y: i32) -> (i32, i32, i32, i32) {
        use x11rb::protocol::randr::ConnectionExt as _;
        let root = &self.xcb.setup().roots[self.screen];
        let whole = (0, 0, i32::from(root.width_in_pixels), i32::from(root.height_in_pixels));
        let monitors = self.xcb.randr_get_monitors(root.root, true).ok().and_then(|cookie| cookie.reply().ok()).map(|reply| reply.monitors).unwrap_or_default();
        monitors
            .iter()
            .map(|monitor| (i32::from(monitor.x), i32::from(monitor.y), i32::from(monitor.width), i32::from(monitor.height)))
            .find(|(left, top, width, height)| x >= *left && y >= *top && x < left + width && y < top + height)
            .unwrap_or(whole)
    }

    /// Shows a panel's page, `room` points, centred near the top of the screen the pointer is on, a fifth of the way
    /// down as Spotlight sits, never off the bottom of a short screen, and gives it the keyboard, keeping which window
    /// had it; its parts follow it (`arrange`). Where it was dragged to on that screen before (`places`), it's there,
    /// as much of it as fits.
    pub fn summon(&self, window: &NativeWindow, places: &crate::platform::Places, room: Size<Pixels>) {
        self.dragged.borrow_mut().remove(&window.x_window);
        self.shown.borrow_mut().insert(window.x_window);
        let size = window.size.get();
        let (left, top, width, height) = self.pointer_screen();
        let (work_left, work_top, work_width, work_height) = self.work_area((left, top, width, height));
        let page = ((room.width.0 * self.scale) as i32, (room.height.0 * self.scale) as i32);
        let at = self.parts.borrow().get(&window.x_window).map_or((0, 0), |(_, at)| ((at.x * self.scale) as i32, (at.y * self.scale) as i32));
        let (x, y) = match places.get(&screen_name((left, top, width, height))) {
            // Where it was dragged to, its search and its card kept whole in what the desktop's panels leave clear.
            Some(kept) => (
                (left + (kept.x * self.scale) as i32).clamp(work_left + at.0, work_left + at.0 + (work_width - page.0).max(0)),
                (top + (kept.y * self.scale) as i32).clamp(work_top + at.1, work_top + at.1 + (work_height - page.1).max(0)),
            ),
            // Its search and its card, as one, in the middle of what the monitor has clear of the desktop's panels.
            None => (work_left + (work_width - page.0) / 2 + at.0, work_top + ((work_height - page.1) / 2).max(0) + at.1),
        };
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
        self.shown.borrow_mut().remove(&window.x_window);
        let parts: Vec<xproto::Window> = self.parts.borrow().iter().filter(|(_, (panel, _))| *panel == window.x_window).map(|(part, _)| *part).collect();
        for part in parts {
            let _ = self.xcb.unmap_window(part);
        }
        let _ = self.xcb.unmap_window(window.x_window);
        let before = window.before.replace(x11rb::NONE);
        if give_back && before != x11rb::NONE {
            self.activate_x(before);
        }
        let _ = self.xcb.flush();
    }

    /// Puts a panel's parts where its page is, each its box of the page (`placed`, the panel first), the page where it
    /// was (from where the panel is, and the box it had), its parts shown while it is, and draws them (`draw`).
    pub fn arrange(&self, placed: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        if let Some((first, ..)) = placed.first() {
            let root = self.xcb.setup().roots[self.screen].root;
            let now = self.xcb.translate_coordinates(first.window, root, 0, 0).ok().and_then(|cookie| cookie.reply().ok()).map_or((0, 0), |at| (i32::from(at.dst_x), i32::from(at.dst_y)));
            // Until it's first put, the panel is its whole page.
            let was = self.parts.borrow().get(&first.window).map_or(Point { x: 0.0, y: 0.0 }, |(_, at)| *at);
            let page = (now.0 - (was.x * self.scale).round() as i32, now.1 - (was.y * self.scale).round() as i32);
            let shown = self.shown.borrow().contains(&first.window);
            for (handle, bounds, _) in placed {
                let x = page.0 + (bounds.origin.x * self.scale).round() as i32;
                let y = page.1 + (bounds.origin.y * self.scale).round() as i32;
                let (width, height) = ((bounds.size.width * self.scale).round() as u16, (bounds.size.height * self.scale).round() as u16);
                let at = self.parts.borrow_mut().insert(handle.window, (first.window, bounds.origin)).map(|(_, at)| at);
                let moved = at != Some(bounds.origin) || (handle.size.width.0, handle.size.height.0) != (i32::from(width), i32::from(height));
                if handle.window == first.window {
                    if moved {
                        let _ = self.hint(handle.window, x, y, width, height);
                    }
                } else if shown {
                    let _ = self.xcb.map_window(handle.window);
                }
                if moved {
                    let changes = xproto::ConfigureWindowAux::new().x(x).y(y).width(u32::from(width)).height(u32::from(height));
                    let _ = self.xcb.configure_window(handle.window, &changes);
                }
            }
            let _ = self.xcb.flush();
        }
        draw();
    }

    /// Moves panel `panel`'s parts with it, where it's moved to.
    fn follow(&self, panel: xproto::Window) {
        let parts = self.parts.borrow();
        let Some((_, was)) = parts.get(&panel) else { return };
        let root = self.xcb.setup().roots[self.screen].root;
        let Some(now) = self.xcb.translate_coordinates(panel, root, 0, 0).ok().and_then(|cookie| cookie.reply().ok()) else { return };
        let page = (i32::from(now.dst_x) - (was.x * self.scale).round() as i32, i32::from(now.dst_y) - (was.y * self.scale).round() as i32);
        for (part, (_, at)) in parts.iter().filter(|(part, (of, _))| *of == panel && **part != panel) {
            let changes = xproto::ConfigureWindowAux::new().x(page.0 + (at.x * self.scale).round() as i32).y(page.1 + (at.y * self.scale).round() as i32);
            let _ = self.xcb.configure_window(*part, &changes);
        }
        let _ = self.xcb.flush();
    }

    /// Puts a window opened hidden on the screen.
    /// Makes `sheet` a dialog of `owner`'s, as GTK's modal dialogs are: transient for it, which the window manager keeps
    /// it over and centres it on, and modal to it (EWMH's dialog type and modal state), before it's first shown.
    pub fn attach(&self, sheet: &NativeWindow, owner: &NativeWindow) {
        let xcb = &self.xcb;
        let _ = xcb.change_property32(xproto::PropMode::REPLACE, sheet.x_window, xproto::AtomEnum::WM_TRANSIENT_FOR, xproto::AtomEnum::WINDOW, &[owner.x_window]);
        let _ = xcb.change_property32(xproto::PropMode::REPLACE, sheet.x_window, self.atoms._NET_WM_WINDOW_TYPE, xproto::AtomEnum::ATOM, &[self.atoms._NET_WM_WINDOW_TYPE_DIALOG]);
        let _ = xcb.change_property32(xproto::PropMode::REPLACE, sheet.x_window, self.atoms._NET_WM_STATE, xproto::AtomEnum::ATOM, &[self.atoms._NET_WM_STATE_MODAL]);
        let _ = xcb.flush();
    }

    pub fn show(&self, window: &NativeWindow) {
        let _ = self.xcb.map_window(window.x_window);
        let _ = self.xcb.flush();
    }

    pub fn close(&self, window: &NativeWindow) {
        self.unframed.borrow_mut().remove(&window.x_window);
        self.frames.borrow_mut().remove(&window.x_window);
        self.windows.borrow_mut().remove(&window.x_window);
        self.parts.borrow_mut().remove(&window.x_window);
        self.shown.borrow_mut().remove(&window.x_window);
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

    /// An app's window with no frame of the window manager's: its page's title row is its header bar, as a GNOME app's
    /// is (client-side decorations), with the desktop's window buttons (`button_layout`), moving it, and its edges
    /// resizing it.
    pub fn unframe(&self, window: &NativeWindow) {
        let _ = self.xcb.change_property32(xproto::PropMode::REPLACE, window.x_window, self.atoms._MOTIF_WM_HINTS, self.atoms._MOTIF_WM_HINTS, &[2, 0, 0, 0, 0]);
        let size = window.size.get();
        self.unframed.borrow_mut().insert(window.x_window, (size.width.0.max(0) as u16, size.height.0.max(0) as u16));
        let _ = self.xcb.flush();
    }

    /// Minimizes an app's window, as its Minimize button does (ICCCM's iconic state).
    pub fn minimize(&self, window: &NativeWindow) {
        let message = xproto::ClientMessageEvent::new(32, window.x_window, self.atoms.WM_CHANGE_STATE, [3, 0, 0, 0, 0]);
        self.to_window_manager(message);
    }

    /// Maximizes an app's window, or makes it as it was, as its Maximize button and a double click on its header bar
    /// do (EWMH's toggle of both maximized states).
    pub fn toggle_maximized(&self, x_window: xproto::Window) {
        let message = xproto::ClientMessageEvent::new(32, x_window, self.atoms._NET_WM_STATE, [2, self.atoms._NET_WM_STATE_MAXIMIZED_VERT, self.atoms._NET_WM_STATE_MAXIMIZED_HORZ, 1, 0]);
        self.to_window_manager(message);
    }

    /// Asks the window manager to move or resize `x_window` from where a press was, on the screen, in direction
    /// `direction` (EWMH's: 0 to 7 the edges and corners from the top left, 8 a move).
    fn move_or_resize(&self, x_window: xproto::Window, (x, y): (i16, i16), direction: u32) {
        let _ = self.xcb.ungrab_pointer(x11rb::CURRENT_TIME);
        let message = xproto::ClientMessageEvent::new(32, x_window, self.atoms._NET_WM_MOVERESIZE, [i32::from(x) as u32, i32::from(y) as u32, direction, 1, 1]);
        self.to_window_manager(message);
    }

    /// A message to the window manager, as EWMH sends one: to the root window.
    fn to_window_manager(&self, message: xproto::ClientMessageEvent) {
        let root = self.xcb.setup().roots[self.screen].root;
        let mask = xproto::EventMask::SUBSTRUCTURE_REDIRECT | xproto::EventMask::SUBSTRUCTURE_NOTIFY;
        let _ = self.xcb.send_event(false, root, mask, message);
        let _ = self.xcb.flush();
    }

    /// Which of an unframed window's edges, as `EDGES` has them, a point in it is at, if any.
    fn edge_at(&self, x_window: xproto::Window, x: i16, y: i16) -> Option<usize> {
        let (width, height) = *self.unframed.borrow().get(&x_window)?;
        let near = (6.0 * self.scale).round() as i32;
        let (x, y, width, height) = (i32::from(x), i32::from(y), i32::from(width), i32::from(height));
        let (left, right, top, bottom) = (x < near, x >= width - near, y < near, y >= height - near);
        Some(match (top, bottom, left, right) {
            (true, _, true, _) => 0,
            (true, _, _, true) => 2,
            (true, ..) => 1,
            (_, true, true, _) => 6,
            (_, true, _, true) => 4,
            (_, true, ..) => 5,
            (_, _, true, _) => 7,
            (_, _, _, true) => 3,
            _ => return None,
        })
    }

    /// Its window manager's frame is drawn light or dark as `theme` says, as a GTK window's (`_GTK_THEME_VARIANT`,
    /// which GNOME's and KDE's read), or with none, as the desktop is.
    pub fn set_appearance(&self, window: &NativeWindow, theme: Option<crate::Appearance>) {
        let _ = match theme {
            Some(theme) => {
                let variant = if theme == crate::Appearance::Dark { "dark" } else { "light" };
                self.xcb.change_property8(xproto::PropMode::REPLACE, window.x_window, self.atoms._GTK_THEME_VARIANT, self.atoms.UTF8_STRING, variant.as_bytes())
            }
            None => self.xcb.delete_property(window.x_window, self.atoms._GTK_THEME_VARIANT),
        };
    }

    /// Shows GTK's menu for text where a field was right-clicked (`TextMenu`), as a GNOME app's fields have, on the
    /// app's GTK thread, where there is one: its choice is its key.
    pub fn text_menu(&self, window: &NativeWindow, menu: crate::view::TextMenu) {
        use gtk::prelude::*;
        let (tell, x_window) = (self.tell.clone(), window.x_window);
        gtk::glib::MainContext::default().invoke(move || {
            if !gtk::is_initialized_main_thread() {
                return;
            }
            let shown = gtk::Menu::new();
            let items = [("Cu_t", "x", menu.cut), ("_Copy", "c", menu.copy), ("_Paste", "v", menu.paste), ("", "", true), ("Select _All", "a", menu.select_all)];
            for (title, key, enabled) in items {
                if title.is_empty() {
                    shown.append(&gtk::SeparatorMenuItem::new());
                    continue;
                }
                let item = gtk::MenuItem::with_mnemonic(title);
                item.set_sensitive(enabled);
                let tell = tell.clone();
                item.connect_activate(move |_| {
                    let key = crate::inspect::Key { key: key.to_owned(), shift: false, alt: false, ctrl: true, meta: false };
                    let _ = tell.send((x_window, Input::Key(key)));
                });
                shown.append(&item);
            }
            shown.show_all();
            #[allow(deprecated)]
            shown.popup_easy(3, gtk::current_event_time());
        });
    }

    /// An app's window is where it is on the screen, `width` by `height`, as it's moved or resized, which it keeps
    /// (`WindowOptions::frame`).
    fn placed_at(&self, x_window: xproto::Window, width: u16, height: u16) {
        let Some(file) = self.frames.borrow().get(&x_window).cloned() else { return };
        let root = self.xcb.setup().roots[self.screen].root;
        let Some(at) = self.xcb.translate_coordinates(x_window, root, 0, 0).ok().and_then(|cookie| cookie.reply().ok()) else { return };
        let now = crate::platform::Frame { x: i32::from(at.dst_x), y: i32::from(at.dst_y), width: i32::from(width), height: i32::from(height), maximized: false };
        if crate::platform::Frame::read(&file) != Some(now) {
            now.write(&file);
        }
    }

    pub fn set_cursor(&self, window: &NativeWindow, cursor: Cursor) {
        // At an unframed window's edge, it's the edge's, which resizes it.
        let name = match self.edge.get().filter(|(at, _)| *at == window.x_window) {
            Some((_, edge)) => EDGES[edge].1,
            None => match cursor {
                Cursor::Arrow => "default",
                Cursor::Pointer => "pointer",
                Cursor::Text => "text",
            },
        };
        if window.cursor.replace(name) == name {
            return;
        }
        let loaded = *self.loaded.borrow_mut().entry(name).or_insert_with(|| self.cursors.load_cursor(&*self.xcb, name).unwrap_or(x11rb::NONE));
        let _ = self.xcb.change_window_attributes(window.x_window, &xproto::ChangeWindowAttributesAux::new().cursor(loaded));
    }

    pub fn flush(&self) {
        let _ = self.xcb.flush();
    }

    /// Whether it says when it shows each frame (Present), so frames come as the display shows them.
    pub fn paced(&self) -> bool {
        self.present.is_some()
    }

    /// Asks Present to say when the display shows its next frame, while one is `wanted` and it hasn't been asked.
    pub fn request_frame(&self, wanted: bool) {
        if let Some(root) = self.present
            && wanted
            && !self.asked.replace(true)
        {
            // The next frame: its count, as `divisor` 1 and `remainder` 0 have it, past the one showing now.
            let _ = self.xcb.present_notify_msc(root, 0, 0, 1, 0);
        }
    }

    /// When the display showed the frame asked for, once it has.
    pub fn frame(&self) -> Option<std::time::Instant> {
        self.framed.take()
    }

    /// What the X server has said since it was last asked, by the window it's about. Each event goes by the input
    /// method first, which takes what's its own, and the keys of a window whose input takes typing go to it, which
    /// sends back what it doesn't compose.
    pub fn events(&self) -> Vec<(WindowId, usize, Input)> {
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
        for (x_window, input) in self.told.borrow_mut().drain(..) {
            if let Some(who) = self.window(x_window) {
                out.push((who, input));
            }
        }
        if let Some(now) = self.appearance.take() {
            out.extend(self.windows.borrow().values().filter(|who| who.1 == 0).map(|who| (*who, Input::Appearance(now))));
        }
        out.into_iter().map(|((id, part), input)| (id, part, input)).collect()
    }

    /// What the input method said: a key it didn't take, what it committed, or what it's composing.
    fn xim_told(&self, told: XimCallbackEvent, out: &mut Vec<(Who, Input)>) {
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
    fn compose(&self, id: Who, keysym: xkbc::Keysym, typed: String, out: &mut Vec<(Who, Input)>) -> bool {
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

    /// What window `window`'s text input plugin is told (`editing::TextInput`). X11 has none, nor does Flutter's
    /// GTK one fit here (it's GTK's input method context, and this has no GTK): its input method (XIM) is shown where
    /// the caret is told to be, and drops what was being composed in an input that's detached.
    pub fn text_input(&self, window: &NativeWindow, calls: &[crate::editing::TextInput]) {
        for call in calls {
            match call {
                crate::editing::TextInput::ClearClient => self.set_ime(window, None),
                crate::editing::TextInput::SetCaretRect(caret) => self.set_ime(window, Some(*caret)),
                _ => {}
            }
        }
    }

    /// X11 has no material of the system's to put under a page: its panel is filled, and what asks for the system's
    /// is blurred as any material.

    /// Its title row over its page, which moves it: a panel's where its page says (`appRegion`), and an unframed app
    /// window's, its header bar (`unframe`); a framed one has its window manager's title bar.
    pub fn set_title_row(&self, window: &NativeWindow, row: Option<crate::view::TitleRow>) {
        if !window.panel && !self.unframed.borrow().contains_key(&window.x_window) {
            return;
        }
        match row {
            Some(row) => drop(self.rows.borrow_mut().insert(window.x_window, row)),
            None => drop(self.rows.borrow_mut().remove(&window.x_window)),
        }
    }

    /// The input with window `window`'s keyboard takes typing, its caret at `caret` (in the window's logical pixels),
    /// or none does: the input method's keys and where it shows what it composes, as GPUI's `update_ime_position`.
    /// An input that stops taking typing drops what was being composed in it, as GPUI's `reset_ime`.
    fn set_ime(&self, window: &NativeWindow, caret: Option<Bounds<f32>>) {
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

    /// A panel dragged by a person: where it's been left, on which screen.
    fn placed(&self, x_window: xproto::Window) -> Option<Input> {
        let root = self.xcb.setup().roots[self.screen].root;
        let at = self.xcb.translate_coordinates(x_window, root, 0, 0).ok()?.reply().ok()?;
        let (x, y) = (i32::from(at.dst_x), i32::from(at.dst_y));
        let screen = self.screen_at(x, y);
        Some(Input::Placed(screen_name(screen), Point { x: (x - screen.0) as f32 / self.scale, y: (y - screen.1) as f32 / self.scale }))
    }

    fn window(&self, x_window: xproto::Window) -> Option<Who> {
        self.windows.borrow().get(&x_window).copied()
    }

    /// A drag from another app over an app's window, as XDND says it: it's told files may be dropped, and dropped,
    /// what was is asked of it as `text/uri-list` (`SelectionNotify`).
    fn xdnd(&self, event: &xproto::ClientMessageEvent, data: &[u32; 5]) {
        let (window, source) = (event.window, data[0]);
        if event.type_ == self.atoms.XdndPosition {
            let root = self.xcb.setup().roots[self.screen].root;
            let (x, y) = ((data[2] >> 16) as i16, (data[2] & 0xffff) as i16);
            let Some(inside) = self.xcb.translate_coordinates(root, window, x, y).ok().and_then(|cookie| cookie.reply().ok()) else { return };
            self.dragging.set(Some((window, source, self.at(inside.dst_x, inside.dst_y))));
            self.xdnd_send(source, self.atoms.XdndStatus, [window, 1, 0, 0, self.atoms.XdndActionCopy]);
        } else if event.type_ == self.atoms.XdndDrop {
            let selection = (self.atoms.XdndSelection, self.atoms.TEXT_URI_LIST);
            let _ = self.xcb.convert_selection(window, selection.0, selection.1, selection.0, data[2]);
            let _ = self.xcb.flush();
        } else if event.type_ == self.atoms.XdndLeave {
            self.dragging.set(None);
        }
    }

    /// Tells a drag's source `what` (`XdndStatus`, `XdndFinished`), as XDND has it.
    fn xdnd_send(&self, source: xproto::Window, what: xproto::Atom, data: [u32; 5]) {
        let message = xproto::ClientMessageEvent::new(32, source, what, data);
        let _ = self.xcb.send_event(false, source, xproto::EventMask::NO_EVENT, message);
        let _ = self.xcb.flush();
    }

    fn at(&self, x: i16, y: i16) -> Point<f32> {
        Point { x: x as f32 / self.scale, y: y as f32 / self.scale }
    }

    fn event(&self, event: Event, out: &mut Vec<(Who, Input)>) {
        match event {
            Event::PresentCompleteNotify(shown) if shown.kind == present::CompleteKind::NOTIFY_MSC => {
                self.asked.set(false);
                // `ust` is CLOCK_MONOTONIC's microseconds, as `Instant` is.
                let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
                unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
                let now = now.tv_sec as u64 * 1_000_000 + now.tv_nsec as u64 / 1000;
                self.framed.set(Some(std::time::Instant::now() - std::time::Duration::from_micros(now.saturating_sub(shown.ust))));
            }
            Event::ClientMessage(event) => {
                let data = event.data.as_data32();
                if data[0] == self.atoms.WM_DELETE_WINDOW
                    && let Some(id) = self.window(event.window)
                {
                    out.push((id, Input::Close));
                }
                self.xdnd(&event, &data);
            }
            // What was dropped, as its source gives it: its files' URIs, one a line.
            Event::SelectionNotify(event) if event.selection == self.atoms.XdndSelection => {
                let Some((window, source, at)) = self.dragging.take().filter(|(window, _, _)| *window == event.requestor) else { return };
                let reply = self.xcb.get_property(true, window, self.atoms.XdndSelection, xproto::AtomEnum::ANY, 0, u32::MAX).ok().and_then(|cookie| cookie.reply().ok());
                let files: Vec<std::path::PathBuf> = reply
                    .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| url::Url::parse(line.trim()).ok()?.to_file_path().ok())
                    .collect();
                self.xdnd_send(source, self.atoms.XdndFinished, [window, u32::from(!files.is_empty()), self.atoms.XdndActionCopy, 0, 0]);
                if let Some(id) = self.window(window).filter(|_| !files.is_empty()) {
                    out.push((id, Input::Dropped(at, files)));
                }
            }
            Event::ConfigureNotify(event) => {
                if let Some(id) = self.window(event.window) {
                    self.follow(event.window);
                    self.placed_at(event.window, event.width, event.height);
                    if let Some(size) = self.unframed.borrow_mut().get_mut(&event.window) {
                        *size = (event.width, event.height);
                    }
                    if self.dragged.borrow().contains(&event.window)
                        && let Some(placed) = self.placed(event.window)
                    {
                        out.push((id, placed));
                    }
                    out.push((id, Input::Resized(Size { width: DevicePixels(event.width as i32), height: DevicePixels(event.height as i32) })));
                }
            }
            Event::Expose(event) => {
                if let Some(id) = self.window(event.window) {
                    out.push((id, Input::Expose));
                }
            }
            Event::MotionNotify(event) => {
                if self.unframed.borrow().contains_key(&event.event) {
                    self.edge.set(self.edge_at(event.event, event.event_x, event.event_y).map(|edge| (event.event, edge)));
                }
                // A press held in a title row and dragged moves its window.
                if let Some((window, x, y)) = self.title_held.get()
                    && (i32::from(event.root_x) - i32::from(x)).abs().max((i32::from(event.root_y) - i32::from(y)).abs()) > 4
                {
                    self.title_held.set(None);
                    self.dragged.borrow_mut().insert(window);
                    return self.move_or_resize(window, (x, y), 8);
                }
                if let Some(id) = self.window(event.event) {
                    out.push((id, Input::Moved(self.at(event.event_x, event.event_y))));
                }
            }
            Event::ButtonPress(event) => {
                let Some(id) = self.window(event.event) else { return };
                // A press ends a dead key's sequence: the input writes what it showed, as GPUI's does.
                self.end_compose();
                let at = self.at(event.event_x, event.event_y);
                // Where a panel's page says a press moves it, the window manager moves it, as a title bar's press does;
                // a press on one of its parts, by its point in the page, moves the panel, which the part follows.
                let (panel, by) = self.parts.borrow().get(&event.event).copied().unwrap_or((event.event, Point { x: 0.0, y: 0.0 }));
                let in_page = Point { x: at.x + by.x, y: at.y + by.y };
                // An unframed window's edge resizes it, as its window manager's frame would.
                if event.detail == 1
                    && let Some(edge) = self.edge_at(event.event, event.event_x, event.event_y)
                {
                    return self.move_or_resize(event.event, (event.root_x, event.root_y), edge as u32);
                }
                if event.detail == 1 && self.rows.borrow().get(&panel).is_some_and(|row| row.moves(in_page)) {
                    // A double click on a header bar maximizes it, or makes it as it was, as GNOME's does.
                    let again = self.title_pressed.replace(Some((panel, event.time))).is_some_and(|(was, time)| was == panel && event.time.wrapping_sub(time) < 400);
                    if again && self.unframed.borrow().contains_key(&panel) {
                        self.title_pressed.set(None);
                        return self.toggle_maximized(panel);
                    }
                    self.title_held.set(Some((panel, event.root_x, event.root_y)));
                    return;
                }
                // Buttons 1 to 3 are the main, middle and secondary; 4 and 5 the wheel's turns, up and down, 6 and 7
                // sideways; a notch is 3 lines of 16 px. X11 says no trackpad's phase.
                let notch = |x: f32, y: f32| Input::Wheel(at, Point { x, y }, None);
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
                if event.detail == 1 && self.title_held.take().is_some() {
                    return;
                }
                if (1..=3).contains(&event.detail)
                    && let Some(id) = self.window(event.event)
                {
                    out.push((id, Input::Up(self.at(event.event_x, event.event_y), event.detail - 1)));
                }
            }
            // A keyboard grab, as a global shortcut's key press makes, moves no focus: as GTK and GPUI, the focus
            // events of its start and end are ignored, or a panel summoned by its shortcut would go at once.
            Event::FocusIn(event) | Event::FocusOut(event) if matches!(event.mode, xproto::NotifyMode::GRAB | xproto::NotifyMode::UNGRAB) => {}
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
