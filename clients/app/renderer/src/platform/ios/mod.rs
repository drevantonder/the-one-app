//! An iPhone's native windows: each a tao window, its UIWindow over the app's, in the event loop tao runs for the app
//! (Tauri's, which owns `UIApplicationMain`, so winit can't run beside it). Its root view is a `View`, whose layer is a
//! `CAMetalLayer` wgpu draws into. The app hands each of tao's events to `event`, on the main thread, which
//! runs the jobs sent to the windows' thread (`Native::run`), with tao's event loop at hand to open windows in, and
//! gives each window what tao says of it: a finger, its size and safe area, the app going to the background and back.
//! What tao doesn't say is heard on the view itself: the system going light or dark (`traitCollectionDidChange:`), and
//! the keyboard coming and going, which its page keeps clear of as it does the safe area. Typing goes through Flutter's
//! iOS text input plugin (`text_input.mm`), which the screen speaks Flutter's text input calls to, as on every platform,
//! and VoiceOver reads its semantic tree through AccessKit's iOS adapter (`a11y`).
//!
//! A finger is the pointer, as a phone's browser has it: a tap presses, and one that moves further than
//! UIScrollView's slop pans what scrolls under it, which carries on as it's let go, slowing as a UIScrollView
//! decelerates. One that comes in from the left edge drags the window across, the app's own showing under it, and
//! closes it, as Back does on Android.

mod a11y;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use std::ffi::{CStr, CString, c_char, c_void};

use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::CGAffineTransform;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSRunLoop, NSRunLoopCommonModes, NSString};
use objc2_quartz_core::{CACurrentMediaTime, CADisplayLink, CAFrameRateRange, CAMetalLayer, kCACornerCurveContinuous};
use objc2_ui_kit::{
    UIAccessibilityIsReduceMotionEnabled, UIAccessibilitySwitchControlStatusDidChangeNotification,
    UIAccessibilityVoiceOverStatusDidChangeNotification, UIApplication, UIApplicationDidBecomeActiveNotification, UIApplicationState, UIPasteboard,
    UIKeyboardDidChangeFrameNotification, UIKeyboardDidHideNotification, UITraitCollection, UITraitEnvironment,
    UIColor, UIGestureRecognizer, UIGestureRecognizerState, UIUserInterfaceStyle, UIView, UIViewAutoresizing, UIViewController, UIWindow, UIWindowLevelNormal,
};
use tao::event::{Event, TouchPhase, WindowEvent};
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::ios::{WindowBuilderExtIOS, WindowExtIOS};

use super::{Input, Job, WindowId, WindowOptions, Windows, rail};
use crate::Appearance;
use crate::editing::{self, TextInput, TextInputClient};
use crate::geometry::{DevicePixels, Edges, Pixels, Point, Size, point};

/// Where a job is sent: the queue `event` runs, and what wakes tao's event loop to run it.
#[derive(Clone)]
pub struct Sender(mpsc::Sender<Job>);

impl Sender {
    pub fn send(&self, job: Job) -> Result<(), mpsc::SendError<Job>> {
        self.0.send(job)?;
        wake();
        Ok(())
    }
}

type Wake = Box<dyn Fn() + Send>;

static STARTED: OnceLock<Sender> = OnceLock::new();
static JOBS: Mutex<Option<mpsc::Receiver<Job>>> = Mutex::new(None);
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);
/// Whether the system is dark, as the views last said, and whether that's changed since the windows were told.
static DARK: AtomicBool = AtomicBool::new(false);
static APPEARANCE_CHANGED: AtomicBool = AtomicBool::new(false);

/// The windows' thread: the main thread, once the app has handed tao's event loop over (`attach`).
pub fn start() -> anyhow::Result<super::Native> {
    STARTED.get().cloned().map(super::Native).ok_or_else(|| anyhow::anyhow!("the app hasn't attached its native host"))
}

pub fn appearance() -> Appearance {
    if DARK.load(Ordering::Relaxed) { Appearance::Dark } else { Appearance::Light }
}

/// The text size its person chose, in Display & Brightness (Dynamic Type): its body text's, 17pt as it comes.
pub fn text_size() -> f32 {
    unsafe { device_text_size() as f32 }
}

/// Reduce Motion, in Accessibility's Motion.
pub fn reduced_motion() -> bool {
    UIAccessibilityIsReduceMotionEnabled()
}

/// The general pasteboard. An iPhone has no primary selection.
pub fn clipboard() -> Option<Box<dyn editing::Clipboard>> {
    Some(Box::new(Clipboard))
}

struct Clipboard;

impl editing::Clipboard for Clipboard {
    fn read(&mut self, primary: bool) -> Option<String> {
        if primary {
            return None;
        }
        unsafe { UIPasteboard::generalPasteboard().string() }.map(|text| text.to_string())
    }

    fn write(&mut self, primary: bool, text: &str) {
        if !primary {
            unsafe { UIPasteboard::generalPasteboard().setString(Some(&NSString::from_str(text))) };
        }
    }
}

/// The app's event loop is tao's, and `wake` has it deliver an event soon, on the main thread, which the app hands to
/// `event`: native windows can open from now on.
pub fn attach(wake: impl Fn() + Send + 'static) {
    if MainThreadMarker::new().is_some() {
        let traits = unsafe { UITraitCollection::currentTraitCollection() };
        DARK.store(dark(&traits), Ordering::Relaxed);
    }
    *WAKE.lock().expect("nothing panics holding it") = Some(Box::new(wake));
    if STARTED.get().is_some() {
        return;
    }
    let (jobs, queue) = mpsc::channel::<Job>();
    *JOBS.lock().expect("nothing panics holding it") = Some(queue);
    let _ = STARTED.set(Sender(jobs));
}

fn wake() {
    if let Some(wake) = WAKE.lock().expect("nothing panics holding it").as_ref() {
        wake();
    }
}

fn dark(traits: &UITraitCollection) -> bool {
    (unsafe { traits.userInterfaceStyle() }) == UIUserInterfaceStyle::Dark
}

type Build<'a> = dyn Fn(tao::window::WindowBuilder) -> Result<tao::window::Window, tao::error::OsError> + 'a;

thread_local! {
    /// Every window, made the first time a job comes, on the main thread.
    static WINDOWS: RefCell<Option<Windows>> = const { RefCell::new(None) };
    /// What opens a tao window, while `event` has tao's event loop at hand.
    static BUILD: Cell<Option<*const Build<'static>>> = const { Cell::new(None) };
    /// Each window's id by its tao window's, and by its view.
    static IDS: RefCell<HashMap<tao::window::WindowId, WindowId>> = RefCell::new(HashMap::new());
    static VIEWS: RefCell<HashMap<usize, WindowId>> = RefCell::new(HashMap::new());
    /// Each finger on a window, and each window's scroll carrying on after its finger let go.
    static TOUCHES: RefCell<HashMap<WindowId, Touch>> = RefCell::new(HashMap::new());
    /// Where UIKit predicts the finger down is going, as its last move came (`Predictor`).
    static PREDICTED: Cell<Option<Point<f32>>> = const { Cell::new(None) };
    /// When UIKit says the finger's latest touch happened (`UITouch.timestamp`, on `CACurrentMediaTime`'s clock), for
    /// the touch tao tells next, which says nothing of when.
    static TOUCHED: Cell<Option<f64>> = const { Cell::new(None) };
    static SLIDES: RefCell<HashMap<WindowId, Slide>> = RefCell::new(HashMap::new());
    /// What each window's text input plugin said, until the windows hear it: it can say it while they're in use.
    static SAID: RefCell<Vec<(WindowId, TextInputClient)>> = const { RefCell::new(Vec::new()) };
    /// Each window whose status bar was tapped, until the windows hear it (`device.mm`).
    static TAPPED: RefCell<Vec<WindowId>> = const { RefCell::new(Vec::new()) };
    /// What each window's picker chose, for which node, until the windows hear it (`device.mm`).
    static PICKED: RefCell<Vec<(WindowId, crate::tree::Id, String)>> = const { RefCell::new(Vec::new()) };
    /// The display link, once a frame's been wanted, and when the display showed the last frame it said.
    static LINK: RefCell<Option<Retained<CADisplayLink>>> = const { RefCell::new(None) };
    static FRAMED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The display's interval for the frame its link last said, which ProMotion changes (8.3 ms at 120 Hz).
    static REFRESH: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// One of tao's events, on the main thread: the jobs waiting run, and a native window hears what's said of it. →
/// whether it was one of theirs, which no one else need hear.
pub fn event<T>(event: &Event<'_, T>, target: &EventLoopWindowTarget<T>) -> bool {
    let build = |builder: tao::window::WindowBuilder| builder.build(target);
    let build: &Build<'_> = &build;
    // Only while this call runs, which is as long as `build` lives.
    BUILD.with(|slot| slot.set(Some(unsafe { std::mem::transmute::<*const Build<'_>, *const Build<'static>>(build) })));
    let ours = handle(event);
    BUILD.with(|slot| slot.set(None));
    ours
}

fn handle<T>(event: &Event<'_, T>) -> bool {
    let jobs: Vec<Job> = JOBS.lock().expect("nothing panics holding it").as_ref().map(|queue| queue.try_iter().collect()).unwrap_or_default();
    let window = match event {
        Event::WindowEvent { window_id, event, .. } => IDS.with(|ids| ids.borrow().get(window_id).copied()).map(|id| (id, event)),
        _ => None,
    };
    let woken = matches!(event, Event::UserEvent(_));
    if jobs.is_empty() && window.is_none() && !woken {
        return false;
    }
    with_windows(|windows| {
        for job in jobs {
            job(windows);
        }
        if let Some((id, event)) = window {
            heard(windows, id, event);
        }
    });
    window.is_some()
}

/// Window `id` heard `event` from tao.
fn heard(windows: &mut Windows, id: WindowId, event: &WindowEvent<'_>) {
    // How long ago the device says a touch happened, which how fast it went is worked out from.
    let ago = matches!(event, WindowEvent::Touch(_)).then(|| TOUCHED.take()).flatten().map(|stamp| ((CACurrentMediaTime() - stamp) * 1000.0).max(0.0));
    if matches!(event, WindowEvent::Touch(_)) && windows.check.is_some() {
        windows.record.input("touch", ago.and_then(|ago| Instant::now().checked_sub(Duration::from_secs_f64(ago / 1000.0))));
    }
    // A finger on the rail's line is the line's, which the page doesn't hear.
    if let WindowEvent::Touch(touch) = event
        && let Some(window) = windows.windows.get(&id)
    {
        let at = touch.location.to_logical::<f32>(window.parts[0].native.scale() as f64);
        let phase = match touch.phase {
            TouchPhase::Started => rail::Phase::Down,
            TouchPhase::Moved => rail::Phase::Moved,
            TouchPhase::Ended => rail::Phase::Up,
            _ => rail::Phase::Cancelled,
        };
        if windows.line(id, touch.id, point(at.x, at.y), phase) {
            return;
        }
    }
    let Some(window) = windows.windows.get_mut(&id) else { return };
    window.dirty = true;
    let native = &window.parts[0].native;
    let scale = native.scale();
    let Ok(screen) = &mut window.screen else { return };
    match event {
        WindowEvent::Touch(touch) => {
            let at = touch.location.to_logical::<f32>(scale as f64);
            let phase = match touch.phase {
                TouchPhase::Started => Phase::Down,
                TouchPhase::Moved => Phase::Moved,
                TouchPhase::Ended => Phase::Up,
                _ => Phase::Cancelled,
            };
            screen.happened(ago);
            touch_at(screen, native, id, touch.id, point(at.x, at.y), phase, crate::motion::now() - ago.unwrap_or(0.0));
            screen.happened(None);
        }
        // The app going to the background leaves its windows as they are, the keyboard with what had it, as an
        // iPhone's own apps do: only in use no longer, as Android's window losing focus is.
        WindowEvent::Suspended => screen.activate(false),
        WindowEvent::Resumed => screen.activate(true),
        _ => {}
    }
    // Its frame is kept as the app goes to the background, which it shows as it opens again, as iOS's snapshot is.
    if matches!(event, WindowEvent::Suspended) {
        windows.keep_snapshot(id);
    }
}

/// Does `f` to the windows, made now if they aren't yet, steps each window sliding from the edge, draws what changed,
/// and asks to be woken when the next timer is due, and for the display's next frame while one slides. A call that comes while the windows are in use (UIKit
/// calling back into a view as Rust changes it) is dropped.
fn with_windows<R>(f: impl FnOnce(&mut Windows) -> R) -> Option<R> {
    WINDOWS.with(|windows| {
        let mut windows = windows.try_borrow_mut().ok()?;
        if windows.is_none() {
            let started = Instant::now();
            match Windows::new(Display) {
                Ok(made) => *windows = Some(made),
                Err(error) => {
                    log::error!("native windows can't open: {error:#}");
                    return None;
                }
            }
            log::info!("native windows ready in {:?}", started.elapsed());
        }
        let windows = windows.as_mut().expect("made above");
        // In the background, where iOS launches the app for a Shortcut or it goes, Metal refuses its GPU work
        // (`Windows::background`): iOS's own word on where it is, asked each turn.
        if let Some(main) = MainThreadMarker::new() {
            windows.background(UIApplication::sharedApplication(main).applicationState() == UIApplicationState::Background);
        }
        if APPEARANCE_CHANGED.swap(false, Ordering::Relaxed) {
            for window in windows.windows.values_mut() {
                if let Ok(screen) = &mut window.screen {
                    screen.appearance = appearance();
                    screen.reduced_motion = reduced_motion();
                    screen.text_size = Some(text_size());
                }
                window.dirty = true;
            }
        }
        for (id, said) in SAID.with(|said| std::mem::take(&mut *said.borrow_mut())) {
            windows.input(id, 0, Input::TextInputClient(said));
        }
        for (id, node, value) in PICKED.with(|picked| std::mem::take(&mut *picked.borrow_mut())) {
            windows.input(id, 0, Input::Picked(node, value));
        }
        for id in TAPPED.with(|tapped| std::mem::take(&mut *tapped.borrow_mut())) {
            if let Some(window) = windows.windows.get_mut(&id)
                && let Ok(screen) = &mut window.screen
            {
                screen.scroll_to_top();
                window.dirty = true;
            }
        }
        for (id, window) in windows.windows.iter_mut() {
            let Ok(screen) = &mut window.screen else { continue };
            for request in a11y::take(*id) {
                super::a11y::act(screen, &request);
                window.dirty = true;
            }
        }
        let result = f(windows);
        // Its page keeps clear of what its safe area and the keyboard leave out, as they are now: the first tao says,
        // as the window is made, comes before it's one of these. The field the keyboard came up for shows whole above
        // it.
        for window in windows.windows.values_mut() {
            let insets = window.parts[0].native.insets();
            if let Ok(screen) = &mut window.screen
                && screen.insets != insets
            {
                screen.insets = insets;
                window.dirty = true;
                if let Some(focused) = screen.focused() {
                    screen.reveal(focused);
                }
            }
        }
        // Each window is as light or dark as its Colour Mode says, so what iOS draws over it (the status bar, the
        // keyboard, a menu, the share sheet) is too.
        for window in windows.windows.values() {
            if let Ok(screen) = &window.screen {
                let style = match screen.theme {
                    Some(Appearance::Dark) => UIUserInterfaceStyle::Dark,
                    Some(Appearance::Light) => UIUserInterfaceStyle::Light,
                    None => UIUserInterfaceStyle::Unspecified,
                };
                let ui_window = window.parts[0].native.ui_window();
                if ui_window.overrideUserInterfaceStyle() != style {
                    ui_window.setOverrideUserInterfaceStyle(style);
                }
            }
        }
        slide(windows);
        windows.turn();
        for window in windows.windows.values() {
            if let Ok(screen) = &window.screen {
                let native = &window.parts[0].native;
                native.a11y.update(screen, native.scale());
            }
        }
        // A slide takes a step each frame the display shows.
        if SLIDES.with(|slides| !slides.borrow().is_empty()) {
            Display.request_frame(true);
        }
        schedule(windows.due());
        Some(result)
    })
}

/// Which wake is the latest asked for (`schedule`): an older one does nothing.
static SCHEDULED: AtomicU64 = AtomicU64::new(0);

/// Has tao's event loop woken when `due`, or not at all: what's due fires.
fn schedule(due: Option<Duration>) {
    let scheduled = SCHEDULED.fetch_add(1, Ordering::Relaxed) + 1;
    let Some(due) = due else { return };
    let Ok(when) = DispatchTime::try_from(due.max(Duration::from_micros(100))) else { return };
    let _ = DispatchQueue::main().after(when, move || {
        if SCHEDULED.load(Ordering::Relaxed) == scheduled {
            wake();
        }
    });
}

/// The windows' host, which opens them in tao's event loop.
pub struct Display;

impl Display {
    /// A phone's window covers the one it's a sheet of already.
    pub fn attach(&self, _sheet: &NativeWindow, _owner: &NativeWindow) {}

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let main = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("a native window opens on the main thread"))?;
        let build = BUILD.with(Cell::get).ok_or_else(|| anyhow::anyhow!("a native window opens as tao's event loop runs"))?;
        // The window that had the keyboard, which has it back as this one closes.
        let behind: Option<Retained<UIWindow>> = unsafe { msg_send![&UIApplication::sharedApplication(main), keyWindow] };
        // Hidden until its page has what it first asked for (`show`).
        let builder = tao::window::WindowBuilder::new().with_title(&options.title).with_visible(false).with_root_view_class((View::class() as *const AnyClass).cast());
        let window = unsafe { (*build)(builder) }?;
        let view = window.ui_view() as usize;
        let ui_view = unsafe { &*(view as *const UIView) };
        ui_view.setMultipleTouchEnabled(false);
        // What watches its fingers for where UIKit predicts each is going (`Predictor`).
        let predictor: Retained<Predictor> = unsafe { msg_send![Predictor::alloc(main), init] };
        unsafe {
            let _: () = msg_send![&predictor, setCancelsTouchesInView: false];
            let _: () = msg_send![&predictor, setDelaysTouchesBegan: false];
            let _: () = msg_send![&predictor, setDelaysTouchesEnded: false];
            let _: () = msg_send![ui_view, addGestureRecognizer: &*predictor];
        }
        IDS.with(|ids| ids.borrow_mut().insert(window.id(), id));
        VIEWS.with(|views| views.borrow_mut().insert(view, id));
        // What it draws is a view of its own over the whole of tao's, which takes no fingers, so tao's still hears them:
        // the rail's pull grows and moves it (`set_moved`), which tao, which lays its own view out to fill the window,
        // would undo on its.
        let card: Retained<Card> = unsafe { msg_send![Card::alloc(main), initWithFrame: ui_view.bounds()] };
        card.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
        card.setUserInteractionEnabled(false);
        ui_view.addSubview(&card);
        let card = Retained::into_super(card);
        let layer = Retained::as_ptr(&card.layer()) as usize;
        // A tap on the status bar scrolls it to its top, as iOS asks the scroll view along its top (`device.mm`).
        unsafe { device_status_bar(view as *mut c_void, id as *mut c_void, status_bar_tapped) };
        let controller = window.ui_view_controller().cast::<UIViewController>();
        let text_input = Plugin(unsafe { text_input_new(controller, id as *mut c_void, said) });
        let a11y = a11y::A11y::new(view as *mut c_void, id);
        // The keyboard came, went or changed: what it covers is read from the view's keyboard guide. A screen reader
        // started or stopped: it's given the tree, or isn't. The app came back from the background: it draws again.
        let center = NSNotificationCenter::defaultCenter();
        for name in unsafe {
            [
                UIApplicationDidBecomeActiveNotification,
                UIKeyboardDidChangeFrameNotification,
                UIKeyboardDidHideNotification,
                UIAccessibilityVoiceOverStatusDidChangeNotification,
                UIAccessibilitySwitchControlStatusDidChangeNotification,
            ]
        } {
            unsafe { center.addObserver_selector_name_object(ui_view.as_ref(), sel!(changed:), Some(name), None) };
        }
        // The root window, the Home Screen, is over the app's own (its page), under every app's window, as Android's is.
        let level = unsafe { UIWindowLevelNormal } + if options.root { 1.0 } else { 2.0 };
        unsafe { &*(window.ui_window() as *const UIWindow) }.setWindowLevel(level);
        Ok(NativeWindow { window, card, layer, behind, a11y, text_input, root: options.root })
    }

    pub fn close(&self, native: &NativeWindow) {
        native.window.set_visible(false);
        let id = IDS.with(|ids| ids.borrow_mut().remove(&native.window.id()));
        VIEWS.with(|views| views.borrow_mut().remove(&(native.window.ui_view() as usize)));
        if let Some(id) = id {
            TOUCHES.with(|touches| touches.borrow_mut().remove(&id));
            SLIDES.with(|slides| slides.borrow_mut().remove(&id));
        }
        if let Some(behind) = &native.behind {
            behind.makeKeyWindow();
        }
    }

    pub fn show(&self, native: &NativeWindow) {
        native.window.set_visible(true);
        native.ui_window().makeKeyAndVisible();
    }

    pub fn activate(&self, native: &NativeWindow) {
        native.ui_window().makeKeyAndVisible();
    }

    pub fn set_cursor(&self, _: &NativeWindow, _: crate::Cursor) {}

    /// A phone's field has its own menu for text, by its own touch and hold.
    pub fn text_menu(&self, _: &NativeWindow, _: crate::view::TextMenu) {}

    /// A phone's window has none of the system's own parts to draw light or dark.
    pub fn set_appearance(&self, _: &NativeWindow, _: Option<crate::Appearance>) {}

    // A computer's launcher panel, which a phone has none of: its launcher is its root window (`WindowOptions::root`).
    pub fn summon(&self, _: &NativeWindow, _: &crate::platform::Places, _: Size<Pixels>) {}
    /// Hides the window, as a root window is while the page over it shows an app (`Windows::hide`); `show` shows it.
    pub fn put_away(&self, native: &NativeWindow, _: bool) {
        native.window.set_visible(false);
    }
    /// A part of a panel's page as a window of its own: which a phone has none of.
    pub fn open_part(&self, _: &NativeWindow, _: usize) -> anyhow::Result<NativeWindow> {
        anyhow::bail!("a panel's parts aren't windows of their own here")
    }
    pub fn arrange(&self, _: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        draw();
    }

    /// Its title row over its page: none, where the system's status bar is above the page.
    pub fn set_title_row(&self, _: &NativeWindow, _: Option<crate::view::TitleRow>) {}

    /// Draws window `native` as the rail's pull has it (`rail::Moved`): what it draws grown and moved by its
    /// transform, as opaque as it says, with round corners, over tao's view, which is see-through, dimming the page
    /// under it.
    pub fn set_moved(&self, native: &NativeWindow, moved: rail::Moved) {
        let scale = f64::from(moved.scale);
        native.card.setTransform(CGAffineTransform { a: scale, b: 0.0, c: 0.0, d: scale, tx: f64::from(moved.x), ty: f64::from(moved.y) });
        native.card.setAlpha(f64::from(moved.opacity));
        let layer = native.card.layer();
        layer.setCornerRadius(f64::from(moved.corner));
        layer.setCornerCurve(unsafe { kCACornerCurveContinuous });
        layer.setMasksToBounds(moved.corner > 0.0);
        let dim = UIColor::colorWithWhite_alpha(0.0, f64::from(moved.dim));
        native.view().setBackgroundColor(Some(&dim));
    }

    /// Tells window `native`'s text input plugin what its screen says (`editing::TextInput`).
    pub fn text_input(&self, native: &NativeWindow, calls: &[TextInput]) {
        for (method, arguments) in calls.iter().map(TextInput::call) {
            let (method, arguments) = (cstring(method), cstring(&arguments.to_string()));
            unsafe { text_input_call(native.text_input.0, method.as_ptr(), arguments.as_ptr()) };
        }
    }

    pub fn flush(&self) {}

    /// Whether it says when the display shows each frame: its display link does.
    pub fn paced(&self) -> bool {
        true
    }

    /// Runs the display link while a frame is `wanted`, as fast as the display goes (120 Hz on a ProMotion iPhone),
    /// and pauses it while none is, as a phone's own apps do: a display link that runs uses power each frame.
    pub fn request_frame(&self, wanted: bool) {
        let Some(main) = MainThreadMarker::new() else { return };
        LINK.with(|link| {
            let mut link = link.borrow_mut();
            if link.is_none() && !wanted {
                return;
            }
            let link = link.get_or_insert_with(|| {
                let ticker: Retained<Ticker> = unsafe { msg_send![Ticker::alloc(main), init] };
                let made = unsafe { CADisplayLink::displayLinkWithTarget_selector(&ticker, sel!(framed:)) };
                made.setPreferredFrameRateRange(CAFrameRateRange { minimum: 60.0, maximum: 120.0, preferred: 120.0 });
                unsafe { made.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
                made
            });
            link.setPaused(!wanted);
        });
    }

    /// How pressed the device is, as its system says (`pressure`).
    pub fn pressure(&self) -> crate::platform::Pressure {
        crate::platform::pressure::apple()
    }

    /// The display's refresh interval, as its link last said.
    pub fn refresh(&self) -> Option<Duration> {
        REFRESH.get()
    }

    /// When the display showed the frame asked for, once it has.
    pub fn frame(&self) -> Option<std::time::Instant> {
        FRAMED.take()
    }

    /// What tao says of a window comes in through `event`, not queued here.
    pub(crate) fn events(&self) -> Vec<(WindowId, usize, super::Input)> {
        Vec::new()
    }
}

/// A tao window, the view over its own that it draws in and that view's layer, the CAMetalLayer wgpu draws into, the
/// window that had the keyboard before it, and its screen reader's bridge, which reads its text input plugin's view, so
/// goes first.
pub struct NativeWindow {
    window: tao::window::Window,
    card: Retained<UIView>,
    layer: usize,
    behind: Option<Retained<UIWindow>>,
    a11y: a11y::A11y,
    text_input: Plugin,
    /// It's the root window (`WindowOptions::root`).
    root: bool,
}

/// A window as `Display::arrange` would place it, which a phone's never does.
#[derive(Clone, Copy)]
pub struct Handle;

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle
    }

    /// What the iPhone does for window `id`'s page itself, over its view: its picker, share sheet and haptics.
    pub fn device(&self, id: WindowId) -> Box<dyn crate::view::Device> {
        Box::new(Device { id, view: self.window.ui_view() as usize })
    }

    /// What wgpu draws into: the layer of the view it draws in, a CAMetalLayer as its class says (`Card`), as a Mac's
    /// window gives it.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        anyhow::ensure!(self.layer != 0, "its view has no layer to draw into");
        Ok(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(self.layer as *mut std::ffi::c_void))
    }

    fn ui_window(&self) -> &UIWindow {
        unsafe { &*(self.window.ui_window() as *const UIWindow) }
    }

    fn view(&self) -> &UIView {
        unsafe { &*(self.window.ui_view() as *const UIView) }
    }

    /// Moves what it draws `across` its window, which is see-through, so the app's own window shows where it's moved
    /// from. It's moved by its transform, which lays nothing out again and leaves the window's points, where tao says
    /// a finger is, where they are.
    fn slide(&self, across: f32) {
        self.card.setTransform(CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: across as f64, ty: 0.0 });
    }

    /// How far across it's slid (`slide`).
    fn slid(&self) -> f32 {
        self.card.transform().tx as f32
    }

    pub fn has_surface(&self) -> bool {
        true
    }

    /// A window over the page is opaque: its page draws its own background.
    pub fn transparent(&self) -> bool {
        false
    }

    pub fn size(&self) -> Size<Pixels> {
        let size = self.window.outer_size().to_logical::<f32>(self.window.scale_factor());
        Size { width: Pixels(size.width), height: Pixels(size.height) }
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        let size = self.window.outer_size();
        Size { width: DevicePixels(size.width.max(1) as i32), height: DevicePixels(size.height.max(1) as i32) }
    }

    pub fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    /// Its size is its UIWindow's, which tao says.
    pub fn resized(&self, _: Size<DevicePixels>) {}

    /// What its safe area leaves out (the status bar, the Dynamic Island, the home indicator), as tao says it: the
    /// room between its inner rectangle and its outer one, in points; and at its bottom what the keyboard covers, as
    /// its view's keyboard guide says it, where that's more.
    fn insets(&self) -> Edges<f32> {
        let scale = self.window.scale_factor();
        let (Ok(inner), Ok(outer)) = (self.window.inner_position(), self.window.outer_position()) else { return Edges::default() };
        let (inner_size, outer_size) = (self.window.inner_size(), self.window.outer_size());
        let left = (inner.x - outer.x) as f64;
        let top = (inner.y - outer.y) as f64;
        let right = outer_size.width as f64 - inner_size.width as f64 - left;
        let bottom = outer_size.height as f64 - inner_size.height as f64 - top;
        let points = |pixels: f64| (pixels / scale).max(0.0) as f32;
        let view = unsafe { &*(self.window.ui_view() as *const UIView) };
        let keyboard = (view.bounds().size.height - view.keyboardLayoutGuide().layoutFrame().origin.y).max(0.0) as f32;
        Edges { top: points(top), right: points(right), bottom: points(bottom).max(keyboard), left: points(left) }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Ticker` doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneDisplayLinkTarget"]
    struct Ticker;

    unsafe impl NSObjectProtocol for Ticker {}

    impl Ticker {
        /// The display showed a frame: when, on the clock Core Animation's media time is, as `Instant` is (mach's
        /// continuous time), and the windows' turn for it.
        #[unsafe(method(framed:))]
        fn framed(&self, link: &CADisplayLink) {
            let ago = (CACurrentMediaTime() - link.timestamp()).max(0.0);
            FRAMED.set(Some(Instant::now() - Duration::from_secs_f64(ago)));
            let interval = link.targetTimestamp() - link.timestamp();
            if interval > 0.0 {
                REFRESH.set(Some(Duration::from_secs_f64(interval)));
            }
            with_windows(|_| {});
        }
    }
);

define_class!(
    // SAFETY: UIGestureRecognizer's subclasses may override its touch methods, and `Predictor` doesn't implement
    // `Drop`. It only watches: it never recognizes, delays or cancels a touch, so tao's view hears every one as before.
    #[unsafe(super(UIGestureRecognizer, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OnePredictedTouches"]
    struct Predictor;

    unsafe impl NSObjectProtocol for Predictor {}

    impl Predictor {
        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: *mut AnyObject, _event: *mut AnyObject) {
            stamped(touches);
        }

        /// A finger moved: where UIKit predicts it's going (`predictedTouchesForTouch:`), the furthest, in its view's
        /// points, for its move tao tells next (`PREDICTED`). tao hears no `UIEvent`, which UIKit predicts by.
        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: *mut AnyObject, event: *mut AnyObject) {
            stamped(touches);
            unsafe {
                let touch: *mut AnyObject = msg_send![touches, anyObject];
                let view: *mut AnyObject = msg_send![self, view];
                let ahead: *mut AnyObject = msg_send![event, predictedTouchesForTouch: touch];
                let last: *mut AnyObject = if ahead.is_null() { std::ptr::null_mut() } else { msg_send![ahead, lastObject] };
                if !last.is_null() && !view.is_null() {
                    let at: objc2_core_foundation::CGPoint = msg_send![last, locationInView: view];
                    PREDICTED.set(Some(point(at.x as f32, at.y as f32)));
                }
            }
        }

        /// The finger's gone: it's failed, as it never recognizes, so UIKit resets it for the next.
        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: *mut AnyObject, _event: *mut AnyObject) {
            stamped(touches);
            PREDICTED.set(None);
            let _: () = unsafe { msg_send![self, setState: UIGestureRecognizerState::Failed] };
        }

        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: *mut AnyObject, _event: *mut AnyObject) {
            stamped(touches);
            PREDICTED.set(None);
            let _: () = unsafe { msg_send![self, setState: UIGestureRecognizerState::Failed] };
        }
    }
);

/// Keeps when UIKit says a finger's touch happened, for the touch tao tells next (`TOUCHED`).
fn stamped(touches: *mut AnyObject) {
    let touch: *mut AnyObject = unsafe { msg_send![touches, anyObject] };
    if !touch.is_null() {
        TOUCHED.set(Some(unsafe { msg_send![touch, timestamp] }));
    }
}

define_class!(
    // SAFETY: UIView has no subclassing requirements, and `Card` doesn't implement `Drop`.
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneNativeCard"]
    struct Card;

    unsafe impl NSObjectProtocol for Card {}

    impl Card {
        /// Its layer is a Metal layer, which the renderer draws into.
        #[unsafe(method(layerClass))]
        fn layer_class() -> &'static AnyClass {
            CAMetalLayer::class()
        }
    }
);

define_class!(
    // SAFETY: UIView has no subclassing requirements, and `View` doesn't implement `Drop`. tao makes a subclass of it,
    // which hears fingers, its layout and its scale.
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OneNativeView"]
    struct View;

    unsafe impl NSObjectProtocol for View {}

    impl View {
        /// The keyboard came, went or changed, a screen reader started or stopped, or the app became active
        /// (`Display::open`): its window's insets are read again, its tree given, and it's drawn where it's back from
        /// the background.
        #[unsafe(method(changed:))]
        fn changed(&self, _: &NSNotification) {
            wake();
        }

        /// The system went light or dark, or its text size changed: tao doesn't say it on an iPhone.
        #[unsafe(method(traitCollectionDidChange:))]
        fn trait_collection_did_change(&self, previous: Option<&UITraitCollection>) {
            let _: () = unsafe { msg_send![super(self), traitCollectionDidChange: previous] };
            DARK.store(dark(&self.traitCollection()), Ordering::Relaxed);
            APPEARANCE_CHANGED.store(true, Ordering::Relaxed);
            wake();
        }
    }
);

/// How far a finger moves before it pans rather than presses, in points: UIScrollView's.
const SLOP: f32 = 10.0;
/// How near the left edge a finger lands to drag the window, in points, as UINavigationController's back swipe.
const EDGE: f32 = 20.0;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Down,
    Moved,
    Up,
    Cancelled,
}

/// A finger: tao's id for it, where it landed, where it was last, whether it's panning, whether it landed on an input,
/// which it presses only as it lifts, or stopped a fling, which it doesn't press at all, or came in from the left
/// edge, which drags the window, and how fast it's been going, for as it lifts.
struct Touch {
    finger: u64,
    /// It came in from the left edge, which drags the window across, or with a screen of the page's over its first,
    /// goes back a screen as it's let go (`pops`).
    edge: bool,
    pops: bool,
    start: Point<f32>,
    last: Point<f32>,
    /// It's moved past the slop, so it's decided whether it pans or is what it's on's (`Screen::pans`).
    moved: bool,
    panning: bool,
    on_input: bool,
    stopped: bool,
    velocity: crate::motion::Velocity,
}

/// A window let go of from the left edge, springing back to where it was or off to the right (`target`, in points),
/// from where it is and how fast it's going (points a second), and when it last stepped.
struct Slide {
    across: f32,
    velocity: f32,
    target: f32,
    stepped: Instant,
}

/// Steps every window springing from the edge to now, with the rail's spring; one that's slid off closes.
fn slide(windows: &mut Windows) {
    let now = Instant::now();
    let mut off = Vec::new();
    SLIDES.with(|slides| {
        slides.borrow_mut().retain(|id, slide| {
            let Some(window) = windows.windows.get(id) else { return false };
            let native = &window.parts[0].native;
            let width = native.size().width.0.max(1.0);
            let dt = now.duration_since(slide.stepped).as_secs_f32();
            slide.stepped = now;
            let (at, velocity, still) = rail::spring(slide.across / width, slide.velocity / width, slide.target / width, dt);
            (slide.across, slide.velocity) = (at * width, velocity * width);
            native.slide(slide.across);
            if still && slide.target > 0.0 {
                off.push(*id);
            }
            !still
        });
    });
    for id in off {
        windows.close(id);
    }
}

/// A finger at `at`, in the window's points. One that moves further than `SLOP` pans what scrolls under it, and what
/// it pressed isn't pressed, stretching past its end as a rubber band; let go, the pan carries on as a fling, or
/// springs back (`Screen::release_scroll`). One on an input gives it the keyboard as it lifts,
/// as a phone's own fields do, so one that lands there to scroll doesn't. One that lands on a fling stops it.
/// `time` is when it happened, on `motion::now`'s clock.
fn touch_at(screen: &mut crate::Screen, native: &NativeWindow, id: WindowId, finger: u64, at: Point<f32>, phase: Phase, time: f64) {
    screen.touch = true;
    TOUCHES.with(|touches| {
        let mut touches = touches.borrow_mut();
        if phase != Phase::Down && touches.get(&id).is_none_or(|touch| touch.finger != finger) {
            return;
        }
        match phase {
            Phase::Down => {
                let stopped = screen.catch_scroll(at);
                // One that lands as the window springs from the edge takes it where it is.
                let caught = SLIDES.with(|slides| slides.borrow_mut().remove(&id)).is_some();
                // The root window, the Home Screen, has nothing to go back to, and what's open over a page (a sheet, a
                // menu) takes no swipe back, as iOS's modals don't.
                let covered = screen.covered();
                let edge = caught || (!native.root && at.x < EDGE && covered != Some(true));
                let pops = !caught && edge && covered == Some(false);
                let on_input = !edge && screen.input_at(at);
                let start = if caught { point(at.x - native.slid(), at.y) } else { at };
                let mut velocity = crate::motion::Velocity::default();
                velocity.add(at, time);
                touches.insert(id, Touch { finger, edge, pops, start, last: at, moved: false, panning: false, on_input, stopped, velocity });
                if !edge && !on_input && !stopped {
                    screen.pointer_down(at, 0, false);
                }
            }
            Phase::Moved => {
                let Some(touch) = touches.get_mut(&id) else { return };
                touch.velocity.add(at, time);
                if touch.pops {
                    return;
                }
                if touch.edge {
                    return native.slide((at.x - touch.start.x).max(0.0));
                }
                // Past the slop, it pans where what's under it scrolls that way, and is otherwise what it's on's to follow
                // (a row swiped across, a sheet pulled down), as a browser decides by `touch-action`.
                if !touch.moved && ((at.x - touch.start.x).abs() > SLOP || (at.y - touch.start.y).abs() > SLOP) {
                    touch.moved = true;
                    touch.panning = touch.on_input || touch.stopped || screen.pans(touch.start, point(at.x - touch.start.x, at.y - touch.start.y));
                    if touch.panning && !touch.on_input && !touch.stopped {
                        screen.pointer_cancel(at);
                    } else if !touch.panning {
                        screen.dragged();
                    }
                }
                // Scrolled to its top and pulled on, what it's in that drags that way (a sheet) takes the finger.
                if touch.panning && !touch.on_input && screen.hand_over(touch.start, point(touch.last.x - at.x, touch.last.y - at.y), at) {
                    touch.panning = false;
                }
                if touch.panning {
                    screen.drag_scroll(touch.start, point(touch.last.x - at.x, touch.last.y - at.y));
                } else if !touch.on_input && !touch.stopped {
                    screen.pointer_moved(at);
                    // What's dragged is drawn where UIKit predicts the finger is going, so it keeps up with it.
                    if let Some(ahead) = PREDICTED.take() {
                        screen.predicted(ahead);
                    }
                }
                touch.last = at;
            }
            Phase::Up => {
                let Some(mut touch) = touches.remove(&id) else { return };
                // How fast it was going, against the finger, as a pan scrolls.
                touch.velocity.add(at, time);
                let (now, going) = (Instant::now(), touch.velocity.get(time));
                let velocity = point(-going.x, -going.y);
                // Over a screen of the page's, it goes back a screen, the app's window staying where it is.
                if touch.pops {
                    if at.x - touch.start.x > native.size().width.0 / 3.0 || -velocity.x > 500.0 {
                        screen.back();
                    }
                    return;
                }
                if touch.edge {
                    // Let go past a third of the way across, or flicked, it goes back: through what's open over the
                    // app's page or its own screens first, as iOS's navigation does, springing back as the app goes
                    // back a screen; from its first, the window slides off and closes. Short of that, it springs back.
                    let width = native.size().width.0;
                    let across = native.slid();
                    let far = across > width / 3.0 || -velocity.x > 500.0;
                    let out = far && !screen.back();
                    let speed = if far { (-velocity.x).max(0.0) } else { 0.0 };
                    SLIDES.with(|slides| slides.borrow_mut().insert(id, Slide { across, velocity: speed, target: if out { width } else { 0.0 }, stepped: now }));
                    return;
                }
                if touch.panning {
                    return screen.release_scroll(touch.start, velocity);
                }
                if touch.stopped {
                    return;
                }
                let was = screen.focused();
                if touch.on_input {
                    screen.pointer_down(at, 0, false);
                }
                screen.pointer_up(at, 0);
                // A tap in the field that has the keyboard brings it up again, if it went, as Android's does.
                if touch.on_input && was.is_some() && screen.focused() == was {
                    Display.text_input(native, &[TextInput::Show]);
                }
            }
            Phase::Cancelled => {
                let Some(touch) = touches.remove(&id) else { return };
                if touch.pops {
                    return;
                }
                if touch.edge {
                    let across = native.slid();
                    SLIDES.with(|slides| slides.borrow_mut().insert(id, Slide { across, velocity: 0.0, target: 0.0, stepped: Instant::now() }));
                } else if touch.panning {
                    screen.release_scroll(touch.start, point(0.0, 0.0));
                } else if !touch.on_input && !touch.stopped {
                    screen.pointer_cancel(at);
                }
            }
        }
    });
    // A finger that's lifted is over nothing, as a phone's browser has it: nothing it scrolled under it is hovered.
    if matches!(phase, Phase::Up | Phase::Cancelled) {
        screen.pointer_left();
    }
}


/// What an iPhone does for a window's page itself (`device.mm`): its picker, share sheet and haptics.
struct Device {
    id: WindowId,
    view: usize,
}

unsafe extern "C" {
    fn device_pick(view: *mut c_void, x: f64, y: f64, width: f64, height: f64, options: *const c_char, chosen: *const c_char, context: *mut c_void, node: i64, picked: extern "C" fn(*mut c_void, i64, *const c_char)) -> bool;
    fn device_share(view: *mut c_void, text: *const c_char) -> bool;
    fn device_haptic(kind: *const c_char) -> bool;
    fn device_text_size() -> f64;
    fn device_status_bar(view: *mut c_void, context: *mut c_void, tapped: extern "C" fn(*mut c_void));
}

impl crate::view::Device for Device {
    fn pick(&mut self, picker: crate::tree::Id, at: crate::inspect::Rect, options: &serde_json::Value, chosen: Option<&str>) -> bool {
        let (options, chosen) = (cstring(&options.to_string()), chosen.map(cstring));
        let (x, y, width, height) = (f64::from(at.x), f64::from(at.y), f64::from(at.width), f64::from(at.height));
        let chosen = chosen.as_ref().map_or(std::ptr::null(), |chosen| chosen.as_ptr());
        unsafe { device_pick(self.view as *mut c_void, x, y, width, height, options.as_ptr(), chosen, self.id as *mut c_void, picker as i64, picked) }
    }

    fn share(&mut self, text: &str) -> bool {
        unsafe { device_share(self.view as *mut c_void, cstring(text).as_ptr()) }
    }

    fn haptic(&mut self, kind: &str) -> bool {
        unsafe { device_haptic(cstring(kind).as_ptr()) }
    }
}

/// Window `context`'s status bar was tapped, which the windows hear once they aren't in use.
extern "C" fn status_bar_tapped(context: *mut c_void) {
    TAPPED.with(|tapped| tapped.borrow_mut().push(context as WindowId));
    wake();
}

/// Window `context`'s picker chose `value` for picker node `node`, which the windows hear once they aren't in use.
extern "C" fn picked(context: *mut c_void, node: i64, value: *const c_char) {
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
    PICKED.with(|picked| picked.borrow_mut().push((context as WindowId, node as crate::tree::Id, value)));
    wake();
}

/// A window's text input plugin (`text_input.mm`): Flutter's, as a C interface.
struct Plugin(*mut c_void);

impl Drop for Plugin {
    fn drop(&mut self) {
        unsafe { text_input_free(self.0) };
    }
}

unsafe extern "C" {
    fn text_input_new(controller: *mut UIViewController, context: *mut c_void, send: extern "C" fn(*mut c_void, *const c_char, *const c_char)) -> *mut c_void;
    fn text_input_free(plugin: *mut c_void);
    fn text_input_call(plugin: *mut c_void, method: *const c_char, arguments: *const c_char);
}

/// What window `context`'s text input plugin says, as Flutter's text input channel's method calls, for its screen,
/// once the windows aren't in use.
extern "C" fn said(context: *mut c_void, method: *const c_char, arguments: *const c_char) {
    let method = unsafe { CStr::from_ptr(method) }.to_string_lossy();
    let arguments: serde_json::Value = serde_json::from_slice(unsafe { CStr::from_ptr(arguments) }.to_bytes()).unwrap_or_default();
    if let Some(said) = TextInputClient::from_call(&method, &arguments) {
        SAID.with(|queue| queue.borrow_mut().push((context as WindowId, said)));
        wake();
    }
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).expect("no NUL in it")
}
