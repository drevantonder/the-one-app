// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/mac/window.rs, events.rs and
// platform/blade/apple_compat.rs) at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and
// cut down for the strict host, written on objc2's AppKit bindings in place of the deprecated `cocoa` crate: a window
// with no title band and its traffic lights kept, its view's layer a CAMetalLayer Blade draws into (GPUI's
// `macos-blade`), its size and Retina scale as AppKit says them, the mouse and the wheel, and keys through the view's
// input context (NSTextInputClient), so a dead key or an input method composes at the caret and types what it
// composed, and the window having the keyboard. A panel (a launcher's) is GPUI's pop-up: a non-activating NSPanel
// that can take the keyboard, with no title band or traffic lights, over every Space and full-screen app, and under
// its view the system's glass (NSGlassEffectView on macOS 26, NSVisualEffectView before), rounded. AppKit is the main thread's: what's made or changed is changed there,
// and what AppKit says is queued for the strict thread, which the queue wakes. No tabs, full screen callbacks, drag
// and drop, or window restoration.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use dispatch2::{DispatchQueue, MainThreadBound, run_on_main};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBackingStoreType, NSCursor, NSEvent, NSEventModifierFlags, NSPanel, NSResponder, NSTextInputClient, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSViewLayerContentsRedrawPolicy, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowTabbingMode,
    NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSNotification, NSObject, NSPoint, NSRange, NSRangePointer, NSRect, NSSize, NSString,
    NSUInteger,
};
use objc2_quartz_core::{CAAutoresizingMask, CALayer, CAMetalLayer, kCAGravityTopLeft};
use raw_window_handle as rwh;

use super::events;
use crate::Cursor;
use crate::editing::{self, Ime};
use crate::inspect::Key;
use crate::geometry::{Bounds, DevicePixels, Pixels, Point, Size};
use crate::platform::{Input, WindowId, WindowOptions};

/// What AppKit said, queued for the strict thread, which `ping` wakes.
#[derive(Clone)]
struct Queue {
    inputs: Arc<Mutex<Vec<(WindowId, Input)>>>,
    ping: calloop::ping::Ping,
}

/// What a window's view and the strict thread share.
struct Shared {
    id: WindowId,
    queue: Queue,
    /// Its view's size in device pixels, and its scale, as AppKit last said.
    size: Mutex<(Size<DevicePixels>, f32)>,
    /// The key that went down, while the input context decides what it does (GPUI's `keystroke_for_do_command`):
    /// what it types (`insertText:`), a command (`doCommandBySelector:`), or nothing yet (a dead key).
    pending: Mutex<Option<crate::inspect::Key>>,
    /// Where the caret of the input that takes typing is, in the view's logical pixels from its top left, for the
    /// input method to show what it composes beside; none where nothing takes typing.
    caret: Mutex<Option<Bounds<f32>>>,
}

impl Shared {
    fn send(&self, input: Input) {
        self.queue.inputs.lock().expect("nothing panics holding it").push((self.id, input));
        self.queue.ping.ping();
    }
}

pub struct Display {
    queue: Queue,
}

pub struct NativeWindow {
    window: MainThreadBound<Retained<NSWindow>>,
    /// A panel's: see-through where it isn't drawn, over its glass.
    panel: bool,
    /// A window holds its delegate weakly.
    _delegate: MainThreadBound<Retained<Delegate>>,
    /// Its view, for Blade's surface, which lives as long as the window: an address, as AppKit's objects are the
    /// main thread's.
    view: usize,
    shared: Arc<Shared>,
    cursor: Cell<Cursor>,
}

impl rwh::HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let view = NonNull::new(self.view as *mut c_void).ok_or(rwh::HandleError::Unavailable)?;
        Ok(unsafe { rwh::WindowHandle::borrow_raw(rwh::AppKitWindowHandle::new(view).into()) })
    }
}

impl rwh::HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(rwh::AppKitDisplayHandle::new().into()) })
    }
}

impl NativeWindow {
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

impl Display {
    pub fn connect<T: 'static>(handle: &calloop::LoopHandle<'static, T>) -> anyhow::Result<Display> {
        let (ping, source) = calloop::ping::make_ping()?;
        // The loop's turn reads what's queued (`events`).
        handle.insert_source(source, |_, _, _| {}).map_err(|error| anyhow::anyhow!("no queue for AppKit's events: {error}"))?;
        Ok(Display { queue: Queue { inputs: Arc::default(), ping } })
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let queue = self.queue.clone();
        run_on_main(|main| {
            let shared = Arc::new(Shared { id, queue, size: Mutex::new((Size::default(), 1.0)), pending: Mutex::default(), caret: Mutex::default() });
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
            window.setAcceptsMouseMovedEvents(true);

            let view = View::new(main, rect, shared.clone());
            match options.panel {
                Some(corner) => unsafe { panel(&window, &view, rect, corner.into()) },
                None => window.setContentView(Some(&view)),
            }
            window.makeFirstResponder(Some(&view));
            let delegate = Delegate::new(main, shared.clone());
            window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.measure(&window);

            window.center();
            Ok(NativeWindow {
                view: Retained::as_ptr(&view) as usize,
                panel: options.panel.is_some(),
                window: MainThreadBound::new(window, main),
                _delegate: MainThreadBound::new(delegate, main),
                shared,
                cursor: Cell::new(Cursor::Arrow),
            })
        })
    }

    /// Puts a window opened hidden on the screen, in front.
    pub fn show(&self, window: &NativeWindow) {
        self.activate(window);
    }

    pub fn close(&self, window: &NativeWindow) {
        window.window.get_on_main(|window| {
            window.setDelegate(None);
            window.close();
        });
    }

    /// Shows a panel centred near the top of the screen the pointer is on, a fifth of the way down as Spotlight sits,
    /// with the keyboard. It never activates the One App, so the app in front stays in front, and has the keyboard
    /// back as it goes.
    pub fn summon(&self, window: &NativeWindow) {
        window.window.get_on_main(|window| unsafe {
            let mouse: NSPoint = msg_send![objc2::class!(NSEvent), mouseLocation];
            let screens: Retained<NSArray<AnyObject>> = msg_send![objc2::class!(NSScreen), screens];
            let frame_of = |screen: &AnyObject| -> NSRect { msg_send![screen, frame] };
            let on = screens.iter().map(|screen| frame_of(&screen)).find(|frame| {
                mouse.x >= frame.origin.x && mouse.y >= frame.origin.y && mouse.x < frame.origin.x + frame.size.width && mouse.y < frame.origin.y + frame.size.height
            });
            if let Some(screen) = on.or_else(|| screens.firstObject().map(|screen| frame_of(&screen))) {
                let size = window.frame().size;
                let x = screen.origin.x + (screen.size.width - size.width) / 2.0;
                let top = screen.origin.y + screen.size.height - (screen.size.height * 0.2).min((screen.size.height - size.height).max(0.0));
                window.setFrameTopLeftPoint(NSPoint::new(x, top));
            }
            window.makeKeyAndOrderFront(None);
        });
    }

    /// Puts a panel away. The One App never came in front for it, so the app that had the keyboard has it again.
    pub fn put_away(&self, window: &NativeWindow, _give_back: bool) {
        window.window.get_on_main(|window| window.orderOut(None));
    }

    /// Makes a window `size`, its top where it is.
    pub fn resize(&self, window: &NativeWindow, size: Size<Pixels>) {
        window.window.get_on_main(|window| {
            let frame = window.frame();
            let height = f64::from(size.height.0);
            let to = NSRect::new(NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height - height), NSSize::new(size.width.0.into(), height));
            window.setFrame_display(to, true);
        });
    }

    pub fn activate(&self, window: &NativeWindow) {
        window.window.get_on_main(|window| {
            window.makeKeyAndOrderFront(None);
            NSApplication::sharedApplication(window.mtm()).activate();
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

    /// The input with window `window`'s keyboard takes typing, its caret at `caret`, or none does. An input that
    /// stops taking typing drops what was being composed in it, as GPUI's `reset_ime`.
    pub fn set_ime(&self, window: &NativeWindow, caret: Option<Bounds<f32>>) {
        let was = std::mem::replace(&mut *window.shared.caret.lock().expect("nothing panics holding it"), caret);
        if was.is_some() && caret.is_none() {
            window.window.get_on_main(|window| {
                if let Some(context) = window.contentView().and_then(|view| view.inputContext()) {
                    context.discardMarkedText();
                }
            });
        }
    }

    pub fn flush(&self) {}

    /// What AppKit has said since it was last asked, by the window it's about.
    pub fn events(&self) -> Vec<(WindowId, Input)> {
        std::mem::take(&mut *self.queue.inputs.lock().expect("nothing panics holding it"))
    }
}

struct ViewIvars {
    shared: Arc<Shared>,
    /// The input method is composing: what it marked is drawn at the caret and not yet typed, so every key is its
    /// first, and what it inserts replaces what it marked.
    composing: Cell<bool>,
    /// The main button went down with Control held, which a Mac takes as the secondary one, so it comes up as that.
    secondary: Cell<bool>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements, and `View` doesn't implement `Drop`.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneStrictView"]
    #[ivars = ViewIvars]
    struct View;

    impl View {
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

        // AppKit asks for the layer's contents again (GPUI's `displayLayer:`): it's drawn on the strict thread.
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
            // A key with ⌘ or Control the menu bar didn't take is the page's; the input context would type Control's.
            // While composing, every key is the input method's first.
            if (key.meta || key.ctrl) && !self.ivars().composing.get() {
                return self.key(key);
            }
            *shared.pending.lock().expect("nothing panics holding it") = Some(key);
            let handled = self.inputContext().is_some_and(|context| context.handleEvent(event));
            let unhandled = shared.pending.lock().expect("nothing panics holding it").take().filter(|_| !handled);
            if let Some(key) = unhandled {
                self.key(key);
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
            self.ivars().shared.send(Input::Wheel(self.at(event), delta));
        }
    }

    unsafe impl NSObjectProtocol for View {}

    // What's typed comes through here, to the screen's input with the keyboard (`editing`): what a key types or
    // which command it is, what an input method composes, and where.
    unsafe impl NSTextInputClient for View {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, replacement: NSRange) {
            let text = text_of(string);
            let composed = self.ivars().composing.replace(false);
            let pending = self.ivars().shared.pending.lock().expect("nothing panics holding it").take();
            let replacing = range(replacement);
            match pending {
                // A key that types what it says is that key, as the DOM's `keydown` has it.
                Some(key) if !composed && replacing.is_none() && text.chars().count() == 1 => {
                    self.key(Key { key: text, shift: key.shift, alt: key.alt, ctrl: false, meta: false })
                }
                // What an input method composed, an accent picked for the character before, a key that typed more
                // than one character: it replaces what's composed or the range given.
                _ => self.tell(move |screen| {
                    if let Some(replacing) = replacing {
                        screen.ime(Ime::Select(replacing));
                    }
                    screen.ime(Ime::Commit(text));
                }),
            }
        }

        // What it stands for is the key that asked, as the user may have bound the shortcut to something else.
        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: Sel) {
            let pending = self.ivars().shared.pending.lock().expect("nothing panics holding it").take();
            if let Some(key) = pending.or_else(|| events::command(selector.name().to_str().unwrap_or_default())) {
                self.key(key);
            }
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, replacement: NSRange) {
            // Composing: the key went into what's marked.
            self.ivars().shared.pending.lock().expect("nothing panics holding it").take();
            let text = text_of(string);
            self.ivars().composing.set(!text.is_empty());
            let (cursor, replacing) = (range(selected).map(|selected| editing::byte(&text, selected.start)), range(replacement));
            self.tell(move |screen| {
                if let Some(replacing) = replacing {
                    screen.ime(Ime::Select(replacing));
                }
                screen.ime(Ime::Preedit(text, cursor));
            });
        }

        // The input method is done composing: what it composed is typed.
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().composing.set(false);
            self.tell(|screen| screen.ime(Ime::Finish));
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            self.ask(|screen| screen.editing())
                .flatten()
                .map_or(NSRange::new(NSNotFound as NSUInteger, 0), |value| {
                    let (from, to) = (value.selection.base.min(value.selection.extent), value.selection.base.max(value.selection.extent));
                    NSRange::new(from, to - from)
                })
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            self.ask(|screen| screen.editing().and_then(|value| value.composing))
                .flatten()
                .map_or(NSRange::new(NSNotFound as NSUInteger, 0), |composing| NSRange::new(composing.start, composing.end - composing.start))
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.ask(|screen| screen.editing().is_some_and(|value| value.composing.is_some())).unwrap_or(false)
        }

        // The input's text in `range`, in UTF-16 units, as much of it as there is, which `actual` says.
        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring_for_proposed_range(&self, wanted: NSRange, actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
            self.substring(wanted, actual)
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        // Where the input method shows what it composes: at the caret, on the screen, from the bottom up.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range(&self, _range: NSRange, _actual: NSRangePointer) -> NSRect {
            let (Some(caret), Some(window)) = (self.ask(|screen| screen.caret_rect()).flatten(), self.window()) else { return NSRect::ZERO };
            let height = self.frame().size.height;
            let rect = NSRect::new(
                NSPoint::new(caret.origin.x.into(), height - f64::from(caret.origin.y + caret.size.height)),
                NSSize::new(caret.size.width.into(), caret.size.height.into()),
            );
            window.convertRectToScreen(self.convertRect_toView(rect, None))
        }

        // The character nearest a point on the screen, in UTF-16 units of the input's text.
        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, point: NSPoint) -> NSUInteger {
            let Some(window) = self.window() else { return NSNotFound as NSUInteger };
            let local = self.convertPoint_fromView(window.convertPointFromScreen(point), None);
            let at = Point { x: local.x as f32, y: (self.frame().size.height - local.y) as f32 };
            self.ask(move |screen| screen.index_at(at)).flatten().unwrap_or(NSNotFound as NSUInteger)
        }
    }
);

impl View {
    /// Asks this window's screen on the strict thread, and waits a little for its answer: the strict thread may be
    /// waiting on this one (opening or closing a window), so an input method gets nothing rather than both waiting.
    fn ask<R: Send + 'static>(&self, f: impl FnOnce(&mut crate::Screen) -> R + Send + 'static) -> Option<R> {
        let (id, (to, from)) = (self.ivars().shared.id, std::sync::mpsc::channel());
        crate::platform::Strict::get().ok()?.run(move |windows| drop(to.send(windows.with(id, f))));
        from.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
    }

    /// Tells this window's screen something on the strict thread, in order with what it's asked.
    fn tell(&self, f: impl FnOnce(&mut crate::Screen) + Send + 'static) {
        let id = self.ivars().shared.id;
        if let Ok(strict) = crate::platform::Strict::get() {
            strict.run(move |windows| {
                let _ = windows.with(id, f);
            });
        }
    }

    fn key(&self, key: Key) {
        self.tell(move |screen| {
            let _ = screen.key(&key);
        });
    }

    /// The input's text in `wanted`, in UTF-16 units, as much of it as there is, which `actual` says.
    fn substring(&self, wanted: NSRange, actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
        let (value, wanted) = (self.ask(|screen| screen.editing()).flatten()?, range(wanted)?);
        let units: Vec<u16> = value.text.encode_utf16().collect();
        let (start, end) = (wanted.start.min(units.len()), wanted.end.min(units.len()));
        if !actual.is_null() {
            unsafe { actual.write(NSRange::new(start, end - start)) };
        }
        Some(NSAttributedString::from_nsstring(&NSString::from_str(&String::from_utf16_lossy(&units[start..end]))))
    }

    /// ⌘ and `key`, with Shift if `shift`, as a menu's command is its key.
    fn command(&self, key: &str, shift: bool) {
        self.key(Key { key: key.to_owned(), shift, alt: false, ctrl: false, meta: true });
    }

    fn new(main: MainThreadMarker, frame: NSRect, shared: Arc<Shared>) -> Retained<View> {
        let this = main.alloc::<View>().set_ivars(ViewIvars { shared, composing: Cell::new(false), secondary: Cell::new(false) });
        let view: Retained<View> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // Layer-backed from the start, its layer the CAMetalLayer Blade draws into (`makeBackingLayer`).
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

    /// Its size and scale now, to draw at, which tells the strict thread it changed.
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
    #[name = "OneStrictPanel"]
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
        let () = msg_send![window, setMovable: false];
        let () = msg_send![window, setHidesOnDeactivate: false];
        window.setOpaque(false);
        let clear: *mut AnyObject = msg_send![objc2::class!(NSColor), clearColor];
        let () = msg_send![window, setBackgroundColor: clear];
        window.setHasShadow(true);
        let glass: *mut AnyObject = match objc2::runtime::AnyClass::get(c"NSGlassEffectView") {
            Some(class) => {
                let glass: *mut AnyObject = msg_send![class, alloc];
                let glass: *mut AnyObject = msg_send![glass, initWithFrame: rect];
                let () = msg_send![glass, setCornerRadius: corner];
                glass
            }
            None => {
                let glass: *mut AnyObject = msg_send![objc2::class!(NSVisualEffectView), alloc];
                let glass: *mut AnyObject = msg_send![glass, initWithFrame: rect];
                // HUD window material, blended behind the window, always active.
                let () = msg_send![glass, setMaterial: 13isize];
                let () = msg_send![glass, setBlendingMode: 0isize];
                let () = msg_send![glass, setState: 1isize];
                let () = msg_send![glass, setWantsLayer: true];
                let layer: *mut AnyObject = msg_send![glass, layer];
                let () = msg_send![layer, setCornerRadius: corner];
                let () = msg_send![layer, setMasksToBounds: true];
                glass
            }
        };
        // Width and height follow the window's.
        let () = msg_send![glass, setAutoresizingMask: 18usize];
        window.setContentView(Some(&*(glass as *mut NSView)));
        view.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable);
        let nothing: *mut AnyObject = std::ptr::null_mut();
        // NSWindowAbove: over the glass.
        let () = msg_send![glass, addSubview: view, positioned: 1isize, relativeTo: nothing];
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(corner);
            layer.setMasksToBounds(true);
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneStrictWindowDelegate"]
    #[ivars = Arc<Shared>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSWindowDelegate for Delegate {
        // Its close button, or the Window menu's Close: the strict thread closes it (`Display::close`).
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            self.ivars().send(Input::Close);
            false
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            self.ivars().send(Input::Focus(true));
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.ivars().send(Input::Focus(false));
        }
    }
);

/// A range AppKit gives, in UTF-16 units, unless it's none (`NSNotFound`).
fn range(range: NSRange) -> Option<std::ops::Range<usize>> {
    (range.location != NSNotFound as NSUInteger).then(|| range.location..range.location + range.length)
}

/// What an NSTextInputClient is given, a string or an attributed one, as text.
fn text_of(string: &AnyObject) -> String {
    match string.downcast_ref::<NSAttributedString>() {
        Some(attributed) => attributed.string().to_string(),
        None => string.downcast_ref::<NSString>().map(|string| string.to_string()).unwrap_or_default(),
    }
}

impl Delegate {
    fn new(main: MainThreadMarker, shared: Arc<Shared>) -> Retained<Delegate> {
        let this = main.alloc::<Delegate>().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}
