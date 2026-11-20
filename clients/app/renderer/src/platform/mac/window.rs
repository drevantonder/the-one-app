// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/mac/window.rs, events.rs and
// platform/blade/apple_compat.rs) at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and
// cut down for the native host, written on objc2's AppKit bindings in place of the deprecated `cocoa` crate: a window
// with no title band and its traffic lights kept, its view's layer a CAMetalLayer wgpu draws into (GPUI's
// `macos-blade`), its size and Retina scale as AppKit says them, the mouse and the wheel, keys, and the window having
// the keyboard. What's typed goes through Flutter's text input plugin (`text_input.mm`), in place of GPUI's
// NSTextInputClient. A panel (a launcher's) is GPUI's pop-up: a non-activating NSPanel
// that can take the keyboard, with no title band or traffic lights, over every Space and full-screen app, and under
// its view the system's glass (NSGlassEffectView on macOS 26, NSVisualEffectView before), rounded. AppKit is the main thread's: what's made or changed is changed there,
// and what AppKit says is queued for the native thread, which the queue wakes. No tabs, full screen callbacks, drag
// and drop, or window restoration.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, Mutex};

use dispatch2::{DispatchQueue, MainThreadBound, run_on_main};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBackingStoreType, NSCursor, NSEvent, NSEventModifierFlags, NSEventPhase, NSPanel, NSResponder, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSViewLayerContentsRedrawPolicy, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowTabbingMode,
    NSWindowTitleVisibility,
};
use objc2_foundation::{NSArray, NSNotification, NSObject, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CAAutoresizingMask, CALayer, CAMetalLayer, CATransaction, kCACornerCurveContinuous, kCAGravityTopLeft};

use super::events;
use crate::Cursor;
use crate::editing::{TextInput, TextInputClient};
use crate::inspect::Key;
use crate::geometry::{DevicePixels, Pixels, Point, Size};
use crate::platform::{Input, WindowId, WindowOptions};
use crate::view::Phase;

/// What AppKit said, queued for the native thread, which `ping` wakes.
#[derive(Clone)]
struct Queue {
    inputs: Arc<Mutex<Vec<(WindowId, usize, Input)>>>,
    ping: calloop::ping::Ping,
}

/// What a window's view and the native thread share.
struct Shared {
    id: WindowId,
    /// Which part of its window's page it is: 0 the window itself, the rest its child windows.
    part: usize,
    /// Its box of its page, in points, as it was last put (`Display::arrange`): where its page is, from where it is.
    placed: Mutex<Option<crate::geometry::Bounds<f32>>>,
    queue: Queue,
    /// Its view's size in device pixels, and its scale, as AppKit last said.
    size: Mutex<(Size<DevicePixels>, f32)>,
    /// Its title row over its page, as its page was last drawn (`Screen::title_row`): held where nothing in it takes a
    /// press, it moves the window, as AppKit's title bar does.
    title_row: Arc<Mutex<Option<crate::view::TitleRow>>>,
    /// A panel being dragged by a person, since a press in its title row and until it's summoned again: where it
    /// moves to is told (`Input::Placed`).
    dragged: Arc<std::sync::atomic::AtomicBool>,
    /// A panel's: whether it's showing (`summon`), and whether its parts are its child windows while it does.
    shown: std::sync::atomic::AtomicBool,
    attached: std::sync::atomic::AtomicBool,
    /// Whether its layer presents with Core Animation's transaction, as it does the frame its window changes size.
    synced: std::sync::atomic::AtomicBool,
}

/// A screen's name, by its frame, which a panel's places are kept by: the same screen in the same arrangement.
fn screen_name(screen: NSRect) -> String {
    format!("{}x{}+{}+{}", screen.size.width, screen.size.height, screen.origin.x, screen.origin.y)
}

impl Shared {
    fn send(&self, input: Input) {
        self.queue.inputs.lock().expect("nothing panics holding it").push((self.id, self.part, input));
        self.queue.ping.ping();
    }
}

pub struct Display {
    queue: Queue,
    /// The display link, while there's a display, and when the display last showed a frame, which its callback says
    /// on a thread of its own, waking the native thread.
    link: Option<DisplayLink>,
    frames: Arc<Frames>,
    /// How many app windows are open, as against panels: while there's one, the One App is a regular app, with its
    /// Dock icon and Cmd-Tab entry, and with none an accessory, as Raycast is with only its launcher.
    apps: std::sync::atomic::AtomicUsize,
}

pub struct NativeWindow {
    window: MainThreadBound<Retained<NSWindow>>,
    /// A panel's, or one of its parts': see-through where it isn't drawn, over its glass.
    panel: bool,
    /// A window holds its delegate weakly; a panel's part has none.
    _delegate: MainThreadBound<Option<Retained<Delegate>>>,
    /// Its view, which its text input plugin is called on, and its view's layer, the CAMetalLayer wgpu draws into,
    /// which live as long as the window: addresses, as AppKit's objects are the main thread's.
    view: usize,
    layer: usize,
    /// Its window, as an address, for `Handle`.
    address: usize,
    shared: Arc<Shared>,
    cursor: Cell<Cursor>,
    /// The window it's a sheet of (`attach`), which it's shown on as AppKit's sheets are.
    owner: std::cell::RefCell<Option<MainThreadBound<Retained<NSWindow>>>>,
}

/// A window as `Display::arrange` places it, on the main thread: its window and view, as addresses, and what it shares
/// with its view.
#[derive(Clone)]
pub struct Handle {
    window: usize,
    view: usize,
    shared: Arc<Shared>,
}

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle { window: self.address, view: self.view, shared: self.shared.clone() }
    }

    /// What wgpu draws into: its view's layer, taken on the main thread as the window was made, where wgpu would
    /// take it from the view on the native thread, which AppKit doesn't allow.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        anyhow::ensure!(self.layer != 0, "its view has no layer to draw into");
        Ok(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(self.layer as *mut c_void))
    }

    /// A computer's window has its surface from the start.
    pub fn has_surface(&self) -> bool {
        true
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.shared.size.lock().expect("nothing panics holding it").0
    }

    pub fn size(&self) -> Size<Pixels> {
        let (size, scale) = *self.shared.size.lock().expect("nothing panics holding it");
        Size { width: Pixels(size.width.0 as f32 / scale), height: Pixels(size.height.0 as f32 / scale) }
    }

    pub fn scale(&self) -> f32 {
        self.shared.size.lock().expect("nothing panics holding it").1
    }

    /// Its size, which its view keeps as AppKit changes it.
    pub fn resized(&self, _size: Size<DevicePixels>) {}

    /// Whether what isn't drawn on it is see-through: a panel's glass shows where it isn't.
    pub fn transparent(&self) -> bool {
        self.panel
    }
}

/// When the display last showed a frame, and what wakes the native thread to it.
struct Frames {
    shown: Mutex<Option<std::time::Instant>>,
    /// The display's refresh interval, as its link last said.
    refresh: Mutex<Option<std::time::Duration>>,
    ping: calloop::ping::Ping,
}

/// A Core Video display link, which says each frame the active displays show. AppKit's own (`NSView.displayLink`,
/// macOS 14) is a view's, on the main thread; this one calls back on a thread of its own, as the native thread wants.
struct DisplayLink(std::ptr::NonNull<objc2_core_video::CVDisplayLink>);

#[allow(deprecated)]
impl DisplayLink {
    fn new(frames: &Arc<Frames>) -> Option<DisplayLink> {
        use objc2_core_video::{CVDisplayLink, CVOptionFlags, CVReturn, CVTimeStamp, kCVReturnSuccess};
        unsafe extern "C-unwind" fn shown(
            _: std::ptr::NonNull<CVDisplayLink>,
            _: std::ptr::NonNull<CVTimeStamp>,
            output: std::ptr::NonNull<CVTimeStamp>,
            _: CVOptionFlags,
            _: std::ptr::NonNull<CVOptionFlags>,
            frames: *mut c_void,
        ) -> CVReturn {
            let frames = unsafe { &*frames.cast::<Frames>() };
            *frames.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::time::Instant::now());
            let output = unsafe { output.as_ref() };
            if output.videoRefreshPeriod > 0 && output.videoTimeScale > 0 {
                let refresh = std::time::Duration::from_secs_f64(output.videoRefreshPeriod as f64 / f64::from(output.videoTimeScale));
                *frames.refresh.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(refresh);
            }
            frames.ping.ping();
            kCVReturnSuccess
        }
        let mut link = std::ptr::null_mut();
        if unsafe { CVDisplayLink::create_with_active_cg_displays(std::ptr::NonNull::from(&mut link)) } != kCVReturnSuccess {
            return None;
        }
        let link = std::ptr::NonNull::new(link)?;
        // The frames it tells of are the display's for as long as the program runs, as the native thread is.
        let told = Arc::into_raw(frames.clone()).cast_mut().cast::<c_void>();
        unsafe { link.as_ref().set_output_callback(Some(shown), told) };
        Some(DisplayLink(link))
    }

    fn set_running(&self, running: bool) {
        let link = unsafe { self.0.as_ref() };
        if running != link.is_running() {
            let _ = if running { link.start() } else { link.stop() };
        }
    }
}

impl Display {
    pub fn connect<T: 'static>(handle: &calloop::LoopHandle<'static, T>) -> anyhow::Result<Display> {
        let (ping, source) = calloop::ping::make_ping()?;
        // The loop's turn reads what's queued (`events`).
        handle.insert_source(source, |_, _, _| {}).map_err(|error| anyhow::anyhow!("no queue for AppKit's events: {error}"))?;
        let frames = Arc::new(Frames { shown: Mutex::new(None), refresh: Mutex::new(None), ping: ping.clone() });
        let link = DisplayLink::new(&frames);
        Ok(Display { queue: Queue { inputs: Arc::default(), ping }, apps: Default::default(), link, frames })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let queue = self.queue.clone();
        let first = options.panel.is_none() && self.apps.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
        run_on_main(|main| {
            // An app's window gives the One App its Dock icon and Cmd-Tab entry, as a regular app's.
            if first {
                NSApplication::sharedApplication(main).setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
            }
            let shared = Arc::new(Shared { id, part: 0, placed: Mutex::default(), queue, size: Mutex::new((Size::default(), 1.0)), title_row: Arc::default(), dragged: Arc::default(), shown: Default::default(), attached: Default::default(), synced: Default::default() });
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(options.size.width.0.into(), options.size.height.0.into()));
            let window = match options.panel {
                // GPUI's pop-up: titled, so it can take the keyboard, with nothing of its title band shown.
                Some(_) => {
                    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::FullSizeContentView | NSWindowStyleMask::NonactivatingPanel;
                    let panel: Retained<Panel> = unsafe {
                        msg_send![main.alloc::<Panel>(), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
                    };
                    Retained::into_super(Retained::into_super(panel))
                }
                None => {
                    let style = NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable
                        | NSWindowStyleMask::FullSizeContentView;
                    unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(main.alloc(), rect, style, NSBackingStoreType::Buffered, false) }
                }
            };
            unsafe { window.setReleasedWhenClosed(false) };
            // No title band: the app runs to the window's top, under the traffic lights, its title kept for Mission
            // Control, the Window menu and Cmd-Tab.
            window.setTitle(&NSString::from_str(&options.title));
            window.setTitlebarAppearsTransparent(true);
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            window.setTabbingMode(NSWindowTabbingMode::Disallowed);
            if options.panel.is_none() {
                unsafe { place_lights(&window) };
                window.setContentMinSize(NSSize::new(crate::platform::SMALLEST.width.0.into(), crate::platform::SMALLEST.height.0.into()));
            }
            window.setAcceptsMouseMovedEvents(true);

            let view = View::new(main, rect, shared.clone());
            // An app's window takes files dropped from the Finder.
            if options.panel.is_none() {
                unsafe {
                    let types: *mut AnyObject = msg_send![objc2::class!(NSArray), arrayWithObject: objc2_app_kit::NSPasteboardTypeFileURL];
                    let () = msg_send![&*view, registerForDraggedTypes: types];
                }
            }
            match options.panel {
                Some(corner) => unsafe { panel(&window, &view, rect, corner.into()) },
                None => window.setContentView(Some(&view)),
            }
            window.makeFirstResponder(Some(&view));
            let delegate = Delegate::new(main, shared.clone());
            window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.measure(&window);

            window.center();
            // An app's window opens where it was last, as AppKit keeps a named window's frame in the user's defaults,
            // so one opened again as the app starts again on a new build is where it was (`updater`).
            if options.panel.is_none() {
                let name = NSString::from_str(&format!("one {}", options.tag));
                window.setFrameUsingName(&name);
                window.setFrameAutosaveName(&name);
            }
            let layer = view.layer().map_or(0, |layer| Retained::as_ptr(&layer) as usize);
            Ok(NativeWindow {
                view: Retained::as_ptr(&view) as usize,
                layer,
                address: Retained::as_ptr(&window) as usize,
                panel: options.panel.is_some(),
                window: MainThreadBound::new(window, main),
                _delegate: MainThreadBound::new(Some(delegate), main),
                shared,
                cursor: Cell::new(Cursor::Arrow),
                owner: std::cell::RefCell::default(),
            })
        })
    }

    /// Makes `sheet` a sheet of `owner`'s, shown on it as AppKit's sheets are (`beginSheet`) once it's first shown.
    pub fn attach(&self, sheet: &NativeWindow, owner: &NativeWindow) {
        let owner = &owner.window;
        let owner = run_on_main(|main| MainThreadBound::new(owner.get(main).clone(), main));
        *sheet.owner.borrow_mut() = Some(owner);
    }

    /// Puts a window opened hidden on the screen, in front: a sheet on its window.
    pub fn show(&self, window: &NativeWindow) {
        let sheet = &window.window;
        match &*window.owner.borrow() {
            Some(owner) => run_on_main(|main| owner.get(main).beginSheet_completionHandler(sheet.get(main), None)),
            None => self.activate(window),
        }
    }

    pub fn close(&self, window: &NativeWindow) {
        let last = !window.panel && self.apps.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 1;
        if let Some(owner) = window.owner.borrow_mut().take() {
            let sheet = &window.window;
            run_on_main(|main| owner.get(main).endSheet(sheet.get(main)));
        }
        window.window.get_on_main(|window| {
            window.setDelegate(None);
            window.close();
            // The last app window gone, the One App is an accessory again: its launcher has no Dock icon.
            if last {
                NSApplication::sharedApplication(window.mtm()).setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
            }
        });
    }

    /// Shows a panel's page, `room` points, centred near the top of the screen the pointer is on, a fifth of the way
    /// down as Spotlight sits, with the keyboard; its parts follow it (`arrange`). It never activates the One App, so
    /// the app in front stays in front, and has the keyboard back as it goes.
    /// Where it was dragged to on that screen before (`places`), it's there, as Raycast's is, as much of it as fits.
    pub fn summon(&self, window: &NativeWindow, places: &crate::platform::Places, room: Size<Pixels>) {
        window.shared.dragged.store(false, std::sync::atomic::Ordering::Relaxed);
        window.shared.shown.store(true, std::sync::atomic::Ordering::Relaxed);
        let placed = *window.shared.placed.lock().expect("nothing panics holding it");
        window.window.get_on_main(|window| unsafe {
            let mouse: NSPoint = msg_send![objc2::class!(NSEvent), mouseLocation];
            let screens: Retained<NSArray<AnyObject>> = msg_send![objc2::class!(NSScreen), screens];
            let frame_of = |screen: &AnyObject| -> NSRect { msg_send![screen, frame] };
            // The screen the pointer is on, and what of it the menu bar and the Dock leave clear.
            let on = screens.iter().find(|screen| {
                let frame = frame_of(screen);
                mouse.x >= frame.origin.x && mouse.y >= frame.origin.y && mouse.x < frame.origin.x + frame.size.width && mouse.y < frame.origin.y + frame.size.height
            });
            if let Some(on) = on.or_else(|| screens.firstObject()) {
                let (screen, visible): (NSRect, NSRect) = (frame_of(&on), msg_send![&*on, visibleFrame]);
                let (width, height) = (f64::from(room.width.0), f64::from(room.height.0));
                let at = placed.map_or((0.0, 0.0), |placed| (f64::from(placed.origin.x), f64::from(placed.origin.y)));
                let (x, top) = match places.get(&screen_name(screen)) {
                    // Where it was dragged to, its search and its card kept whole in what the menu bar and the Dock
                    // leave clear.
                    Some(kept) => (
                        (screen.origin.x + f64::from(kept.x)).clamp(visible.origin.x + at.0, visible.origin.x + at.0 + (visible.size.width - width).max(0.0)),
                        (screen.origin.y + screen.size.height - f64::from(kept.y))
                            .clamp(visible.origin.y + height.min(visible.size.height) - at.1, visible.origin.y + visible.size.height - at.1),
                    ),
                    // Its search and its card, as one, in the middle of what the menu bar and the Dock leave clear.
                    None => (
                        visible.origin.x + (visible.size.width - width) / 2.0 + at.0,
                        visible.origin.y + visible.size.height - ((visible.size.height - height) / 2.0).max(0.0) - at.1,
                    ),
                };
                window.setFrameTopLeftPoint(NSPoint::new(x, top));
            }
            window.makeKeyAndOrderFront(None);
        });
    }

    /// Puts a panel away, its parts with it, which are its children again as it shows (`arrange`). The One App never
    /// came in front for it, so the app that had the keyboard has it again.
    pub fn put_away(&self, window: &NativeWindow, _give_back: bool) {
        window.shared.shown.store(false, std::sync::atomic::Ordering::Relaxed);
        window.shared.attached.store(false, std::sync::atomic::Ordering::Relaxed);
        window.window.get_on_main(|window| {
            if let Some(children) = window.childWindows() {
                for child in children.iter() {
                    window.removeChildWindow(&child);
                    child.orderOut(None);
                }
            }
            window.orderOut(None);
        });
    }

    /// A part of panel `parent`'s page as a window of its own, its `part`th: a child of the panel, so it moves, hides
    /// and stacks with it, with the system's glass under it, rounded, and its own shadow. It never takes the keyboard,
    /// which stays with the panel.
    pub fn open_part(&self, parent: &NativeWindow, part: usize) -> anyhow::Result<NativeWindow> {
        let (id, queue) = (parent.shared.id, self.queue.clone());
        let (title_row, dragged) = (parent.shared.title_row.clone(), parent.shared.dragged.clone());
        run_on_main(move |main| {
            let shared = Arc::new(Shared { id, part, placed: Mutex::default(), queue, size: Mutex::new((Size::default(), 1.0)), title_row, dragged, shown: Default::default(), attached: Default::default(), synced: Default::default() });
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
            // With no title band it can't become the key window, as NSPanel's borderless kind can't.
            let style = NSWindowStyleMask::NonactivatingPanel;
            let window: Retained<NSPanel> =
                unsafe { msg_send![main.alloc::<NSPanel>(), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false] };
            let window: Retained<NSWindow> = Retained::into_super(window);
            unsafe { window.setReleasedWhenClosed(false) };
            window.setAcceptsMouseMovedEvents(true);
            let view = View::new(main, rect, shared.clone());
            unsafe { panel(&window, &view, rect, 0.0) };
            view.measure(&window);
            let layer = view.layer().map_or(0, |layer| Retained::as_ptr(&layer) as usize);
            Ok(NativeWindow {
                view: Retained::as_ptr(&view) as usize,
                layer,
                address: Retained::as_ptr(&window) as usize,
                panel: true,
                window: MainThreadBound::new(window, main),
                _delegate: MainThreadBound::new(None, main),
                shared,
                cursor: Cell::new(Cursor::Arrow),
                owner: std::cell::RefCell::default(),
            })
        })
    }

    /// Puts a panel's parts where its page is, each its box of the page (`placed`, the panel first) with its glass
    /// rounded so, the page where it was (from where the panel is, and the box it had), and draws them (`draw`) as
    /// they're put: on the main thread, in one Core Animation transaction, with a layer whose window changes size
    /// presenting in it, so its frame and what's drawn in it come on screen together. Where nothing moves, and the
    /// parts of a panel showing are its children, they're drawn here, without waiting on the main thread.
    pub fn arrange(&self, placed: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        let still = placed.iter().all(|(handle, bounds, _)| {
            *handle.shared.placed.lock().expect("nothing panics holding it") == Some(*bounds) && !handle.shared.synced.load(std::sync::atomic::Ordering::Relaxed)
        });
        let attached = placed.first().is_none_or(|(first, ..)| {
            placed.len() == 1 || !first.shared.shown.load(std::sync::atomic::Ordering::Relaxed) || first.shared.attached.load(std::sync::atomic::Ordering::Relaxed)
        });
        if still && attached {
            return draw();
        }
        struct Draw<'a>(*mut (dyn FnMut() + 'a));
        // SAFETY: the main thread runs it while this thread waits for it, so it's used by one thread at a time.
        unsafe impl Send for Draw<'_> {}
        let draw = Draw(draw as *mut (dyn FnMut() + '_));
        let placed: Vec<(Handle, crate::geometry::Bounds<f32>, f64)> = placed.iter().map(|(handle, bounds, radius)| (handle.clone(), *bounds, f64::from(*radius))).collect();
        run_on_main(move |_| unsafe {
            let draw = draw;
            let Some((first, ..)) = placed.first() else { return (*draw.0)() };
            CATransaction::begin();
            let parent = &*(first.window as *const NSWindow);
            // Until it's first put, the panel is its whole page.
            let was = first.shared.placed.lock().expect("nothing panics holding it").map_or(Point { x: 0.0, y: 0.0 }, |placed| placed.origin);
            let frame = parent.frame();
            let (left, top) = (frame.origin.x - f64::from(was.x), frame.origin.y + frame.size.height + f64::from(was.y));
            let visible = parent.isVisible();
            for (at, (handle, bounds, radius)) in placed.iter().enumerate() {
                let window = &*(handle.window as *const NSWindow);
                let (x, y, w, h) = (f64::from(bounds.origin.x), f64::from(bounds.origin.y), f64::from(bounds.size.width), f64::from(bounds.size.height));
                let to = NSRect::new(NSPoint::new(left + x, top - y - h), NSSize::new(w, h));
                let now = window.frame();
                if (now.origin.x, now.origin.y, now.size.width, now.size.height) != (to.origin.x, to.origin.y, to.size.width, to.size.height) {
                    window.setFrame_display(to, false);
                    window.invalidateShadow();
                }
                let view = &*(handle.view as *const View);
                if let Some(layer) = view.layer() {
                    layer.setCornerRadius(*radius);
                    layer.setCornerCurve(kCACornerCurveContinuous);
                    // A window that changes size this frame shows its new frame of drawing with the transaction, so the
                    // two change together; any other's comes as it's drawn, without the main thread waiting on its GPU.
                    let resized = (now.size.width, now.size.height) != (to.size.width, to.size.height);
                    if let Ok(layer) = layer.downcast::<CAMetalLayer>() {
                        layer.setPresentsWithTransaction(resized);
                    }
                    handle.shared.synced.store(resized, std::sync::atomic::Ordering::Relaxed);
                }
                if let Some(glass) = window.contentView().and_then(|content| content.subviews().firstObject()) {
                    round(&glass, *radius);
                }
                if at > 0 && visible && window.parentWindow().is_none() {
                    // NSWindowAbove.
                    parent.addChildWindow_ordered(window, objc2_app_kit::NSWindowOrderingMode::Above);
                }
            }
            for (handle, bounds, _) in &placed {
                *handle.shared.placed.lock().expect("nothing panics holding it") = Some(*bounds);
            }
            first.shared.attached.store(visible, std::sync::atomic::Ordering::Relaxed);
            (*draw.0)();
            CATransaction::commit();
        });
    }

    /// Brings an app's window in front, the One App with it, as its person opened it from the launcher, whose panel
    /// never activates the One App: since macOS 14 `activate` waits for the app in front to yield, which it never
    /// does here, so the window would open behind it.
    pub fn activate(&self, window: &NativeWindow) {
        window.window.get_on_main(|window| {
            window.makeKeyAndOrderFront(None);
            #[allow(deprecated)]
            NSApplication::sharedApplication(window.mtm()).activateIgnoringOtherApps(true);
        });
    }

    /// Shows a Mac's menu for text where a field was Control-clicked or right-clicked (`TextMenu`), as muda shows a
    /// context menu, on the main thread: its choice is its key (`menu_chosen`).
    pub fn text_menu(&self, window: &NativeWindow, menu: crate::view::TextMenu) {
        use muda::ContextMenu;
        let (view, id) = (window.view, window.shared.id);
        DispatchQueue::main().exec_async(move || {
            let shown = super::super::text_menu_of(menu, id);
            let at = muda::dpi::LogicalPosition::new(f64::from(menu.at.x), f64::from(menu.at.y));
            // SAFETY: the window's view, which lives as long as the window, on the main thread.
            unsafe { shown.show_context_menu_for_nsview(view as *const c_void, Some(at.into())) };
        });
    }

    /// Its title bar and glass are drawn light or dark as `theme` says (Aqua, Dark Aqua), or as the Mac is.
    pub fn set_appearance(&self, window: &NativeWindow, theme: Option<crate::Appearance>) {
        window.window.get_on_main(move |window| {
            let name = theme.map(|theme| unsafe { if theme == crate::Appearance::Dark { objc2_app_kit::NSAppearanceNameDarkAqua } else { objc2_app_kit::NSAppearanceNameAqua } });
            let appearance = name.and_then(|name| objc2_app_kit::NSAppearance::appearanceNamed(name));
            window.setAppearance(appearance.as_deref());
        });
    }

    pub fn set_cursor(&self, window: &NativeWindow, cursor: Cursor) {
        if window.cursor.replace(cursor) == cursor {
            return;
        }
        DispatchQueue::main().exec_async(move || {
            let cursor = match cursor {
                Cursor::Arrow => NSCursor::arrowCursor(),
                Cursor::Pointer => NSCursor::pointingHandCursor(),
                Cursor::Text => NSCursor::IBeamCursor(),
            };
            cursor.set();
        });
    }

    /// Tells window `window`'s text input plugin what the screen says (`editing::TextInput`), on the main thread.
    pub fn text_input(&self, window: &NativeWindow, calls: &[TextInput]) {
        let calls: Vec<_> = calls.iter().map(|call| call.call()).map(|(method, arguments)| (cstring(method), cstring(&arguments.to_string()))).collect();
        let view = window.view;
        window.window.get_on_main(move |_| {
            // SAFETY: the window's content view, which lives as long as the window, which this holds.
            let view = unsafe { &*(view as *const View) };
            for (method, arguments) in calls {
                unsafe { text_input_call(view.ivars().text_input.get(), method.as_ptr(), arguments.as_ptr()) };
            }
        });
    }

    /// Its title row, as its page was last drawn: what of it moves the window (`View`'s `mouseDown:`).
    pub fn set_title_row(&self, window: &NativeWindow, row: Option<crate::view::TitleRow>) {
        *window.shared.title_row.lock().expect("nothing panics holding it") = row;
    }

    pub fn flush(&self) {}

    /// Whether it says when the display shows each frame: where there's a display link.
    pub fn paced(&self) -> bool {
        self.link.is_some()
    }

    /// Runs the display link while a frame is `wanted`, and stops it once none is.
    pub fn request_frame(&self, wanted: bool) {
        if let Some(link) = &self.link {
            link.set_running(wanted);
        }
    }

    /// How pressed the device is, as its system says (`pressure`).
    pub fn pressure(&self) -> crate::platform::Pressure {
        crate::platform::pressure::apple()
    }

    /// The display's refresh interval, as its link says.
    pub fn refresh(&self) -> Option<std::time::Duration> {
        *self.frames.refresh.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// When the display last showed a frame, once it has since this was last asked.
    pub fn frame(&self) -> Option<std::time::Instant> {
        self.frames.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    /// What AppKit has said since it was last asked, by the window and part it's about.
    pub fn events(&self) -> Vec<(WindowId, usize, Input)> {
        std::mem::take(&mut *self.queue.inputs.lock().expect("nothing panics holding it"))
    }
}

struct ViewIvars {
    /// Its text input plugin, dropped before what it tells.
    text_input: Plugin,
    shared: Arc<Shared>,
    /// The main button went down with Control held, which a Mac takes as the secondary one, so it comes up as that.
    secondary: Cell<bool>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements, and `View` doesn't implement `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneNativeView"]
    #[ivars = ViewIvars]
    struct View;

    impl View {
        // Files dragged from the Finder over an app's window may be dropped on it, as copies (NSDragOperationCopy).
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _sender: &AnyObject) -> usize {
            1
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, _sender: &AnyObject) -> usize {
            1
        }

        // Dropped, its files' URLs are the page's, where they were dropped.
        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &AnyObject) -> bool {
            let (location, pasteboard): (NSPoint, *mut AnyObject) = unsafe { (msg_send![sender, draggingLocation], msg_send![sender, draggingPasteboard]) };
            let point = self.convertPoint_fromView(location, None);
            let at = Point { x: point.x as f32, y: (self.frame().size.height - point.y) as f32 };
            let mut files = Vec::new();
            unsafe {
                let classes: *mut AnyObject = msg_send![objc2::class!(NSArray), arrayWithObject: objc2::class!(NSURL)];
                let urls: *mut AnyObject = msg_send![pasteboard, readObjectsForClasses: classes, options: std::ptr::null::<AnyObject>()];
                let count: usize = if urls.is_null() { 0 } else { msg_send![urls, count] };
                for at in 0..count {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: at];
                    let (file, path): (bool, *mut AnyObject) = (msg_send![url, isFileURL], msg_send![url, path]);
                    let path: *const c_char = if file && !path.is_null() { msg_send![path, UTF8String] } else { std::ptr::null() };
                    if !path.is_null() {
                        files.push(std::path::PathBuf::from(CStr::from_ptr(path).to_string_lossy().into_owned()));
                    }
                }
            }
            self.ivars().shared.send(Input::Dropped(at, files));
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            let layer = CAMetalLayer::new();
            layer.setAutoresizingMask(CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable);
            layer.setContentsGravity(unsafe { kCAGravityTopLeft });
            layer.setNeedsDisplayOnBoundsChange(true);
            Retained::into_super(layer)
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        // AppKit asks for the layer's contents again (GPUI's `displayLayer:`): it's drawn on the native thread.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.ivars().shared.send(Input::Expose);
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(window) = self.window() {
                self.measure(&window);
            }
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            if let Some(window) = self.window() {
                self.measure(&window);
            }
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            self.ivars().shared.send(Input::Appearance(super::of(&self.effectiveAppearance())));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let key = events::key(event);
            let shared = &self.ivars().shared;
            // ⌘W closes the window, as a Mac app's does.
            if key.meta && !key.ctrl && !key.alt && key.key.eq_ignore_ascii_case("w") {
                return shared.send(Input::Close);
            }
            // A shortcut (⌘ or Control) the menu bar didn't take, or a key that types nothing (Enter, an arrow,
            // Backspace), is the page's first, as Flutter's keyboard manager gives its framework each key before the
            // text input plugin; while its input method composes, every key is the input method's.
            let plugin = self.ivars().text_input.get();
            let composing = unsafe { text_input_composing(plugin) };
            // Control with a letter a Mac binds to editing (its StandardKeyBinding.dict: ⌃A and ⌃E the line's start
            // and end, ⌃K delete to its end, ⌃D and ⌃H delete, ⌃B ⌃F ⌃P ⌃N move) is a field's, through its
            // input context, as in every Mac text field; with no field it's the page's.
            let bound = key.ctrl && !key.meta && !key.alt && matches!(key.key.as_str(), "a" | "e" | "k" | "d" | "h" | "b" | "f" | "p" | "n");
            if bound && !composing && unsafe { text_input_key(plugin, event) } {
                return;
            }
            if !composing && (key.meta || key.ctrl || key.key.chars().count() > 1) {
                return shared.send(Input::Key(key));
            }
            if !unsafe { text_input_key(plugin, event) } {
                shared.send(Input::Key(key));
            }
        }

        // The Edit menu's, and their keys' (⌘C, ⌘X, ⌘V, ⌘A, ⌘Z, ⇧⌘Z): the page's, as those keys.
        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.command("c", false);
        }

        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {
            self.command("x", false);
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.command("v", false);
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.command("a", false);
        }

        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            self.command("z", false);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            self.command("z", true);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // On a Mac, a Control-click is a secondary click.
            let flags = event.modifierFlags();
            let secondary = flags.contains(NSEventModifierFlags::Control);
            // What's empty in its title row is its title bar: it moves the window, and double-clicked does what the
            // person set in System Settings (Desktop & Dock, "Double-click a window's title bar to").
            // Its title row is its page's: a panel's part's point, there, is further along by where the part is.
            let (at, shared) = (self.at(event), &self.ivars().shared);
            let by = shared.placed.lock().expect("nothing panics holding it").map_or(Point { x: 0.0, y: 0.0 }, |placed| placed.origin);
            let at = Point { x: at.x + by.x, y: at.y + by.y };
            let moves = shared.title_row.lock().expect("nothing panics holding it").as_ref().is_some_and(|row| row.moves(at));
            // A panel's part is dragged by its panel, which it follows.
            if moves && !secondary && let Some(window) = self.window().map(|window| window.parentWindow().unwrap_or(window)) {
                // A panel has no zoom; it's only dragged.
                let panel = window.isKindOfClass(objc2::class!(NSPanel));
                if event.clickCount() == 2 && !panel {
                    double_clicked(&window)
                } else {
                    self.ivars().shared.dragged.store(panel, std::sync::atomic::Ordering::Relaxed);
                    window.performWindowDragWithEvent(event)
                }
                return;
            }
            self.ivars().secondary.set(secondary);
            self.ivars().shared.send(Input::Down(self.at(event), if secondary { 2 } else { 0 }, flags.contains(NSEventModifierFlags::Shift)));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let button = if self.ivars().secondary.take() { 2 } else { 0 };
            self.ivars().shared.send(Input::Up(self.at(event), button));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Down(self.at(event), 2, event.modifierFlags().contains(NSEventModifierFlags::Shift)));
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Up(self.at(event), 2));
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            if event.buttonNumber() == 2 {
                self.ivars().shared.send(Input::Down(self.at(event), 1, event.modifierFlags().contains(NSEventModifierFlags::Shift)));
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            if event.buttonNumber() == 2 {
                self.ivars().shared.send(Input::Up(self.at(event), 1));
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Moved(self.at(event)));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Moved(self.at(event)));
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Moved(self.at(event)));
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.ivars().shared.send(Input::Moved(self.at(event)));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().shared.send(Input::Left);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // A trackpad says pixels; a wheel says lines, which WebKit scrolls 40 px each. AppKit's deltas move the
            // content, so the DOM's, which move the view, are the other way.
            let per = if event.hasPreciseScrollingDeltas() { 1.0 } else { 40.0 };
            let delta = Point { x: -(event.scrollingDeltaX() as f32) * per, y: -(event.scrollingDeltaY() as f32) * per };
            // A trackpad's fingers' phase, and the momentum AppKit goes on with once they've lifted; a wheel has neither.
            let (fingers, momentum) = (event.phase(), event.momentumPhase());
            let phase = (fingers != NSEventPhase::None || momentum != NSEventPhase::None).then(|| Phase {
                fingers: [NSEventPhase::Began, NSEventPhase::Changed, NSEventPhase::Ended, NSEventPhase::Cancelled, NSEventPhase::MayBegin]
                    .iter()
                    .position(|one| *one == fingers)
                    .map(|at| Phase::FINGERS[at]),
                momentum: momentum != NSEventPhase::None,
            });
            self.ivars().shared.send(Input::Wheel(self.at(event), delta, phase));
        }
    }

    unsafe impl NSObjectProtocol for View {}
);

impl View {
    /// ⌘ and `key`, with Shift if `shift`, as a menu's command is its key.
    fn command(&self, key: &str, shift: bool) {
        self.ivars().shared.send(Input::Key(Key { key: key.to_owned(), shift, alt: false, ctrl: false, meta: true }));
    }

    fn new(main: MainThreadMarker, frame: NSRect, shared: Arc<Shared>) -> Retained<View> {
        let this = main.alloc::<View>().set_ivars(ViewIvars { text_input: Plugin(Cell::new(std::ptr::null_mut())), shared, secondary: Cell::new(false) });
        let view: Retained<View> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let shared = Arc::as_ptr(&view.ivars().shared).cast_mut().cast();
        view.ivars().text_input.0.set(unsafe { text_input_new(Retained::as_ptr(&view).cast_mut().cast(), shared, said) });
        // Layer-backed from the start, its layer the CAMetalLayer wgpu draws into (`makeBackingLayer`).
        view.setWantsLayer(true);
        view.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::DuringViewResize);
        // The pointer, over it, whether or not its window has the keyboard, and as it leaves.
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(main.alloc(), NSRect::ZERO, options, Some(&view), None) };
        view.addTrackingArea(&area);
        view
    }

    /// Its size and scale now, to draw at, which tells the native thread it changed.
    fn measure(&self, window: &NSWindow) {
        let scale = window.backingScaleFactor();
        if let Some(layer) = self.layer() {
            layer.setContentsScale(scale);
        }
        let frame = self.frame();
        let size = Size { width: DevicePixels((frame.size.width * scale).round() as i32), height: DevicePixels((frame.size.height * scale).round() as i32) };
        let shared = &self.ivars().shared;
        *shared.size.lock().expect("nothing panics holding it") = (size, scale as f32);
        shared.send(Input::Resized(size));
    }

    /// Where `event` is in the view, from its top left: AppKit's y is from the bottom.
    fn at(&self, event: &NSEvent) -> Point<f32> {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        Point { x: point.x as f32, y: (self.frame().size.height - point.y) as f32 }
    }
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements, and `Panel` doesn't implement `Drop`.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneNativePanel"]
    struct Panel;

    impl Panel {
        // A panel with no frame takes the keyboard only if it says it can, as GPUI's `GPUIPanel` does.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

/// Where its traffic lights are in its title row, 52 points tall as its page's top bar is (a unified toolbar), as
/// AppKit puts them in one: the first's left edge, and how far below the window's top they start, in the middle of it.
const LIGHTS: (f64, f64) = (20.0, (52.0 - 14.0) / 2.0);

/// Puts `window`'s traffic lights where a unified toolbar has them (`LIGHTS`): their title bar container as tall as
/// that, and each light along it, as far apart as AppKit put them.
unsafe fn place_lights(window: &NSWindow) {
    unsafe {
        let buttons: [*mut AnyObject; 3] = [0usize, 1, 2].map(|button| msg_send![window, standardWindowButton: button]);
        if buttons.iter().any(|button| button.is_null()) {
            return;
        }
        let first: NSRect = msg_send![buttons[0], frame];
        let second: NSRect = msg_send![buttons[1], frame];
        let apart = second.origin.x - first.origin.x;
        let lights: *mut AnyObject = msg_send![buttons[0], superview];
        let container: *mut AnyObject = if lights.is_null() { lights } else { msg_send![lights, superview] };
        if !container.is_null() {
            let mut frame: NSRect = msg_send![container, frame];
            frame.size.height = first.size.height + LIGHTS.1;
            frame.origin.y = window.frame().size.height - frame.size.height;
            let () = msg_send![container, setFrame: frame];
        }
        for (at, button) in buttons.into_iter().enumerate() {
            let mut origin: NSRect = msg_send![button, frame];
            origin.origin.x = LIGHTS.0 + at as f64 * apart;
            let () = msg_send![button, setFrameOrigin: origin.origin];
        }
    }
}

/// Does what a double-click on a window's title bar does, as its person set it in System Settings (Desktop & Dock):
/// zoom, which is what's set until they change it, minimize, or nothing.
fn double_clicked(window: &NSWindow) {
    unsafe {
        let defaults: *mut AnyObject = msg_send![objc2::class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::from_str("AppleActionOnDoubleClick");
        let set: *mut NSString = msg_send![defaults, stringForKey: &*key];
        match set.as_ref().map(|set| set.to_string()).as_deref() {
            Some("None") => {}
            Some("Minimize") => window.miniaturize(None),
            _ => window.zoom(None),
        }
    }
}

/// Makes `window` a panel, as a launcher's is: over other apps' windows on every Space, full-screen apps' too, out of
/// Cmd-` and Mission Control, see-through, with a shadow, and its title band and traffic lights hidden; its content
/// the system's glass, rounded `corner` points, with `view` over it.
unsafe fn panel(window: &NSWindow, view: &View, rect: NSRect, corner: f64) {
    unsafe {
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        for button in 0..3usize {
            let button: *mut AnyObject = msg_send![window, standardWindowButton: button];
            if !button.is_null() {
                let () = msg_send![button, setHidden: true];
            }
        }
        // NSWindowCollectionBehavior: canJoinAllSpaces, stationary, ignoresCycle, fullScreenAuxiliary.
        let () = msg_send![window, setCollectionBehavior: (1usize << 0) | (1 << 4) | (1 << 6) | (1 << 8)];
        // NSStatusWindowLevel, as Spotlight's.
        let () = msg_send![window, setLevel: 25isize];
        // It moves only by what its page says moves it (`appRegion`), as Raycast's is dragged by its own.
        let () = msg_send![window, setMovable: true];
        let () = msg_send![window, setHidesOnDeactivate: false];
        window.setOpaque(false);
        let clear: *mut AnyObject = msg_send![objc2::class!(NSColor), clearColor];
        let () = msg_send![window, setBackgroundColor: clear];
        window.setHasShadow(true);
        // A view that holds the panel's glass and the page over it, both following the window's size.
        let sizable = objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
        let main = window.mtm();
        let content = NSView::initWithFrame(main.alloc(), rect);
        window.setContentView(Some(&content));
        let whole = glass(rect, corner);
        whole.setAutoresizingMask(sizable);
        content.addSubview(&whole);
        view.setAutoresizingMask(sizable);
        content.addSubview(view);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(corner);
            layer.setCornerCurve(kCACornerCurveContinuous);
            layer.setMasksToBounds(true);
        }
    }
}

/// The system's glass, `rect`, rounded `corner` points: Liquid Glass (NSGlassEffectView) on macOS 26, and before it the
/// HUD window's material, blended behind the window and always active.
unsafe fn glass(rect: NSRect, corner: f64) -> Retained<NSView> {
    unsafe {
        let glass: *mut AnyObject = match objc2::runtime::AnyClass::get(c"NSGlassEffectView") {
            Some(class) => {
                let glass: *mut AnyObject = msg_send![class, alloc];
                msg_send![glass, initWithFrame: rect]
            }
            None => {
                let glass: *mut AnyObject = msg_send![objc2::class!(NSVisualEffectView), alloc];
                let glass: *mut AnyObject = msg_send![glass, initWithFrame: rect];
                let () = msg_send![glass, setMaterial: 13isize];
                let () = msg_send![glass, setBlendingMode: 0isize];
                let () = msg_send![glass, setState: 1isize];
                let () = msg_send![glass, setWantsLayer: true];
                glass
            }
        };
        let glass = Retained::from_raw(glass as *mut NSView).expect("AppKit made it");
        round(&glass, corner);
        glass
    }
}

/// Rounds the system's glass `corner` points.
unsafe fn round(glass: &NSView, corner: f64) {
    unsafe {
        let responds: bool = msg_send![glass, respondsToSelector: objc2::sel!(setCornerRadius:)];
        if responds {
            let () = msg_send![glass, setCornerRadius: corner];
        } else if let Some(layer) = glass.layer() {
            layer.setCornerRadius(corner);
            layer.setCornerCurve(kCACornerCurveContinuous);
            layer.setMasksToBounds(true);
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneNativeWindowDelegate"]
    #[ivars = Arc<Shared>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSWindowDelegate for Delegate {
        // Its close button, or the Window menu's Close: the native thread closes it (`Display::close`).
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            self.ivars().send(Input::Close);
            false
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            self.ivars().send(Input::Focus(true));
        }

        // A panel dragged by a person is told where it was left on its screen.
        #[unsafe(method(windowDidMove:))]
        fn window_did_move(&self, notification: &NSNotification) {
            if !self.ivars().dragged.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let window: *mut AnyObject = unsafe { msg_send![notification, object] };
            let screen: *mut AnyObject = if window.is_null() { std::ptr::null_mut() } else { unsafe { msg_send![window, screen] } };
            if screen.is_null() {
                return;
            }
            let (frame, on): (NSRect, NSRect) = unsafe { (msg_send![window, frame], msg_send![screen, frame]) };
            let at = Point { x: (frame.origin.x - on.origin.x) as f32, y: (on.origin.y + on.size.height - (frame.origin.y + frame.size.height)) as f32 };
            self.ivars().send(Input::Placed(screen_name(on), at));
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.ivars().send(Input::Focus(false));
        }

        // Full screen, a Mac hides the traffic lights, so the breadcrumb starts at the window's edge.
        #[unsafe(method(windowWillEnterFullScreen:))]
        fn window_will_enter_full_screen(&self, _notification: &NSNotification) {
            self.ivars().send(Input::FullScreen(true));
        }

        #[unsafe(method(windowWillExitFullScreen:))]
        fn window_will_exit_full_screen(&self, _notification: &NSNotification) {
            self.ivars().send(Input::FullScreen(false));
        }

        // AppKit lays the title bar out again as the window changes size, and out of full screen.
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &NSNotification) {
            if let Some(window) = notification.object().and_then(|object| object.downcast::<NSWindow>().ok()) {
                unsafe { place_lights(&window) };
            }
        }
    }
);

/// A native view's text input plugin (`text_input.mm`): Flutter's, as a C interface.
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

unsafe extern "C" {
    fn text_input_new(view: *mut c_void, context: *mut c_void, send: extern "C" fn(*mut c_void, *const c_char, *const c_char)) -> *mut c_void;
    fn text_input_free(plugin: *mut c_void);
    fn text_input_call(plugin: *mut c_void, method: *const c_char, arguments: *const c_char);
    fn text_input_key(plugin: *mut c_void, event: &NSEvent) -> bool;
    fn text_input_composing(plugin: *mut c_void) -> bool;
}

/// What a view's text input plugin says, as Flutter's text input channel's method calls, for its window's screen.
/// The commands its input method gives (`doCommandBySelector:`) are the keys they stand for, as the page hears keys.
extern "C" fn said(shared: *mut c_void, method: *const c_char, arguments: *const c_char) {
    // SAFETY: the view's, which outlives its plugin (`ViewIvars`).
    let shared = unsafe { &*shared.cast::<Shared>() };
    let method = unsafe { CStr::from_ptr(method) }.to_string_lossy();
    let arguments: serde_json::Value = serde_json::from_slice(unsafe { CStr::from_ptr(arguments) }.to_bytes()).unwrap_or_default();
    if method == "TextInputClient.performSelectors" {
        let selectors = arguments[1].as_array().into_iter().flatten().filter_map(|selector| selector.as_str());
        for key in selectors.filter_map(events::command) {
            shared.send(Input::Key(key));
        }
    } else if let Some(said) = TextInputClient::from_call(&method, &arguments) {
        shared.send(Input::TextInputClient(said));
    }
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).expect("no NUL in it")
}

impl Delegate {
    fn new(main: MainThreadMarker, shared: Arc<Shared>) -> Retained<Delegate> {
        let this = main.alloc::<Delegate>().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}
