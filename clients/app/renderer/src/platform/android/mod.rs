//! Android's native windows: each a view (`NativeView`, a SurfaceView, `clients/app/renderer/android`) over the app's web
//! view, in its activity's content, which the app keeps clear of the system's bars and the keyboard. Every window is
//! drawn on the app's UI thread, where its views are: a job for the windows' thread (`Native::run`) wakes that thread's
//! looper, as Android's own Handler does, and what a view hears (its surface, a finger, the keyboard, autofill, the
//! screen reader) comes in through JNI on it. The keyboard bridge is Flutter's (`app.theone.renderer.editing`), and
//! speaks the renderer's editing state, which the screen's editing model keeps (`editing`); the screen reader's is
//! AccessKit's (`a11y`).
//!
//! Rust asks the view to do something (show the keyboard for a field, give it the field's state, hide it) by
//! `NativeView.command`, which it posts to the UI thread, so nothing calls back into Rust while Rust has the windows.

mod a11y;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint, jlong};
use jni::{JNIEnv, JavaVM};
use raw_window_handle as rwh;
use serde_json::{Value, json};

use super::{Input, Job, WindowId, WindowOptions, Windows};
use crate::geometry::{DevicePixels, Pixels, Point, Size, point};
use crate::tree::Id;
use crate::Appearance;
use crate::editing::{self, Composing, Selection, TextInputClient};

/// Where a job is sent: the UI thread's queue, and the file that wakes its looper.
#[derive(Clone)]
pub struct Sender {
    jobs: mpsc::Sender<Job>,
    wake: Arc<OwnedFd>,
}

impl Sender {
    pub fn send(&self, job: Job) -> Result<(), mpsc::SendError<Job>> {
        self.jobs.send(job)?;
        let one: u64 = 1;
        unsafe { libc::write(self.wake.as_raw_fd(), (&one as *const u64).cast(), 8) };
        Ok(())
    }
}

static STARTED: OnceLock<Sender> = OnceLock::new();
/// Whether the system is dark, and whether its person asked for less motion (Remove animations), as the activity
/// last said.
static DARK: AtomicBool = AtomicBool::new(false);
static REDUCED: AtomicBool = AtomicBool::new(false);

/// The windows' thread: the UI thread, once the activity has attached (`NativeHost.attach`).
pub fn start() -> anyhow::Result<super::Native> {
    STARTED.get().cloned().map(super::Native).ok_or_else(|| anyhow::anyhow!("the app's activity hasn't attached its native host"))
}

pub fn appearance() -> Appearance {
    if DARK.load(Ordering::Relaxed) { Appearance::Dark } else { Appearance::Light }
}

pub fn reduced_motion() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Android's clipboard (`ClipboardManager`, through the host), which a keyboard's Ctrl+C and Ctrl+V use; it has no
/// primary selection.
pub fn clipboard() -> Option<Box<dyn editing::Clipboard>> {
    Some(Box::new(Clipboard))
}

struct Clipboard;

impl editing::Clipboard for Clipboard {
    fn read(&mut self, primary: bool) -> Option<String> {
        if primary {
            return None;
        }
        java(|env, host| {
            let text = env.call_static_method(<&JClass>::from(host.as_obj()), "paste", "()Ljava/lang/String;", &[])?.l()?;
            if text.is_null() {
                return Ok(None);
            }
            Ok(Some(env.get_string(&JString::from(text))?.into()))
        })
        .flatten()
    }

    fn write(&mut self, primary: bool, text: &str) {
        if primary {
            return;
        }
        java(|env, host| {
            let text = env.new_string(text)?;
            env.call_static_method(<&JClass>::from(host.as_obj()), "copy", "(Ljava/lang/String;)V", &[JValue::Object(&text)]).map(drop)
        });
    }
}

thread_local! {
    /// Every window, made the first time a job comes, on the UI thread.
    static WINDOWS: RefCell<Option<Windows>> = const { RefCell::new(None) };
    /// The UI thread's Java: the VM, and the host's class (`NativeHost`), which makes and closes views.
    static JAVA: RefCell<Option<(JavaVM, GlobalRef)>> = const { RefCell::new(None) };
    /// The file the looper wakes on when the next timer of any window is due.
    static TIMER: RefCell<Option<OwnedFd>> = const { RefCell::new(None) };
    /// Whether Choreographer has been asked for the next frame, and when the display showed the last one asked for.
    static ASKED: Cell<bool> = const { Cell::new(false) };
    static FRAMED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Choreographer's frame, on the UI thread: when it was shown, on CLOCK_MONOTONIC as `Instant` is, and the windows'
/// turn for it.
unsafe extern "C" fn framed(frame_time_nanos: std::ffi::c_long, _: *mut c_void) {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    let now = now.tv_sec as i64 * 1_000_000_000 + now.tv_nsec as i64;
    let ago = std::time::Duration::from_nanos(now.saturating_sub(frame_time_nanos as i64).max(0) as u64);
    ASKED.set(false);
    FRAMED.set(Some(Instant::now() - ago));
    with_windows(|_| {});
}

/// Does `f` to the windows, made now if they aren't yet, then what that changed: timers, drawing, the keyboard. A call
/// that comes while the windows are in use (it can't, as Rust only posts to Java) is dropped.
fn with_windows<R>(f: impl FnOnce(&mut Windows) -> R) -> Option<R> {
    WINDOWS.with(|windows| {
        let mut windows = windows.try_borrow_mut().ok()?;
        if windows.is_none() {
            let started = Instant::now();
            match Windows::new(Display::default()) {
                Ok(made) => *windows = Some(made),
                Err(error) => {
                    log::error!("native windows can't open: {error:#}");
                    return None;
                }
            }
            log::info!("native windows ready in {:?}", started.elapsed());
        }
        let windows = windows.as_mut().expect("made above");
        let result = f(windows);
        windows.turn();
        for id in windows.windows.keys().copied().collect::<Vec<_>>() {
            keyboard(windows, id);
            if let Some(window) = windows.windows.get_mut(&id)
                && let (Some(a11y), Ok(screen)) = (window.parts[0].native.a11y.as_mut(), &window.screen)
            {
                a11y.update(screen);
            }
        }
        // What the keyboard scrolled.
        windows.turn();
        schedule(windows.due());
        Some(result)
    })
}

/// Has the looper wake when the next timer is due, or not at all.
fn schedule(due: Option<std::time::Duration>) {
    TIMER.with(|timer| {
        let Some(timer) = timer.borrow().as_ref().map(|timer| timer.as_raw_fd()) else { return };
        let due = due.map(|due| due.max(std::time::Duration::from_micros(100)));
        let value = libc::timespec { tv_sec: due.map_or(0, |due| due.as_secs() as _), tv_nsec: due.map_or(0, |due| due.subsec_nanos() as _) };
        let spec = libc::itimerspec { it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 }, it_value: value };
        unsafe { libc::timerfd_settime(timer, 0, &spec, std::ptr::null_mut()) };
    });
}

fn java<R>(f: impl FnOnce(&mut JNIEnv, &GlobalRef) -> jni::errors::Result<R>) -> Option<R> {
    JAVA.with(|java| {
        let java = java.borrow();
        let (vm, host) = java.as_ref()?;
        let mut env = vm.attach_current_thread_permanently().ok()?;
        let result = f(&mut env, host);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result.ok()
    })
}

/// The views' host, which asks Java to make, raise and close them.
#[derive(Default)]
pub struct Display;

impl Display {
    /// A phone's window covers the one it's a sheet of already.
    pub fn attach(&self, _sheet: &NativeWindow, _owner: &NativeWindow) {}

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let view = java(|env, host| {
            let title = env.new_string(&options.title)?;
            let class: &JClass = host.as_obj().into();
            let view = env.call_static_method(class, "open", "(JLjava/lang/String;Z)Lapp/theone/renderer/NativeView;", &[JValue::Long(id as i64), JValue::Object(&title), JValue::Bool(options.root.into())])?.l()?;
            env.new_global_ref(view)
        })
        .ok_or_else(|| anyhow::anyhow!("the activity didn't make a view"))?;
        let density = java(|env, host| env.call_static_method(<&JClass>::from(host.as_obj()), "density", "()F", &[])?.f()).unwrap_or(1.0);
        Ok(NativeWindow { view, surface: None, device: Cell::new(Size { width: DevicePixels(1), height: DevicePixels(1) }), density, a11y: None, revealed: None })
    }

    pub fn close(&self, window: &NativeWindow) {
        java(|env, host| env.call_static_method(<&JClass>::from(host.as_obj()), "close", "(Lapp/theone/renderer/NativeView;)V", &[JValue::Object(window.view.as_obj())]).map(drop));
    }

    /// Shows the view, made hidden, once its page has what it first asked for: its surface is made then.
    pub fn show(&self, window: &NativeWindow) {
        java(|env, host| env.call_static_method(<&JClass>::from(host.as_obj()), "show", "(Lapp/theone/renderer/NativeView;)V", &[JValue::Object(window.view.as_obj())]).map(drop));
    }

    pub fn activate(&self, window: &NativeWindow) {
        java(|env, host| env.call_static_method(<&JClass>::from(host.as_obj()), "activate", "(Lapp/theone/renderer/NativeView;)V", &[JValue::Object(window.view.as_obj())]).map(drop));
    }

    pub fn set_cursor(&self, _: &NativeWindow, _: crate::Cursor) {}

    // A computer's launcher panel, which a phone has none of: its launcher is its root window (`WindowOptions::root`).
    pub fn summon(&self, _: &NativeWindow, _: &crate::platform::Places, _: Size<Pixels>) {}
    /// Hides the view, as a root window is while the page over it shows an app (`Windows::hide`).
    pub fn put_away(&self, window: &NativeWindow, _: bool) {
        java(|env, host| env.call_static_method(<&JClass>::from(host.as_obj()), "hide", "(Lapp/theone/renderer/NativeView;)V", &[JValue::Object(window.view.as_obj())]).map(drop));
    }
    /// Draws window `native` as the rail's pull has it (`rail::Moved`): its view grown, moved, faded and rounded, and
    /// what's under it dimmed (`NativeHost.moved`).
    pub fn set_moved(&self, native: &NativeWindow, moved: super::rail::Moved) {
        java(|env, host| {
            let floats = [moved.scale, moved.x, moved.y, moved.opacity, moved.corner, moved.dim].map(JValue::Float);
            let [scale, x, y, opacity, corner, dim] = floats;
            env.call_static_method(<&JClass>::from(host.as_obj()), "moved", "(Lapp/theone/renderer/NativeView;FFFFFF)V", &[JValue::Object(native.view.as_obj()), scale, x, y, opacity, corner, dim]).map(drop)
        });
    }
    /// A part of a panel's page as a window of its own: which a phone has none of.
    pub fn open_part(&self, _: &NativeWindow, _: usize) -> anyhow::Result<NativeWindow> {
        anyhow::bail!("a panel's parts aren't windows of their own here")
    }
    pub fn arrange(&self, _: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        draw();
    }

    /// Its title row over its page: none here, where the system's own title bar is above the page and moves the window.
    pub fn set_title_row(&self, _: &NativeWindow, _: Option<crate::view::TitleRow>) {}

    /// Tells window `window`'s text input plugin (`TextInputPlugin`, through `NativeView.apply`) Flutter's method calls.
    pub fn text_input(&self, window: &NativeWindow, calls: &[crate::editing::TextInput]) {
        for call in calls {
            let (method, args) = call.call();
            window.apply(json!({"method": method, "args": args}));
        }
    }

    pub fn flush(&self) {}

    /// Whether it says when the display shows each frame: Choreographer does.
    pub fn paced(&self) -> bool {
        true
    }

    /// Asks Choreographer for the next frame, while one is `wanted` and it hasn't been asked. (Its 64-bit
    /// `postFrameCallback64` is Android 10's; the app runs from Android 7.)
    #[allow(deprecated)]
    pub fn request_frame(&self, wanted: bool) {
        if wanted && !ASKED.replace(true) {
            unsafe { ndk_sys::AChoreographer_postFrameCallback(ndk_sys::AChoreographer_getInstance(), Some(framed), std::ptr::null_mut()) };
        }
    }

    /// When the display showed the frame asked for, once it has.
    pub fn frame(&self) -> Option<std::time::Instant> {
        FRAMED.take()
    }

    /// What the views heard comes straight in through JNI (`input`), not queued here.
    pub(crate) fn events(&self) -> Vec<(WindowId, usize, Input)> {
        Vec::new()
    }
}

/// A view's window: its surface, once it has one, its size in device pixels, and its density.
pub struct NativeWindow {
    view: GlobalRef,
    surface: Option<ndk::native_window::NativeWindow>,
    device: Cell<Size<DevicePixels>>,
    density: f32,
    a11y: Option<a11y::A11y>,
    /// The field last scrolled into view, and the size the window was then.
    revealed: Option<(Id, Size<DevicePixels>)>,
}

/// A view as `Display::arrange` would place it, which a phone's never does.
#[derive(Clone, Copy)]
pub struct Handle;

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle
    }

    /// What wgpu draws into: its view's surface, by its raw handles.
    pub fn surface(&self) -> anyhow::Result<wgpu::SurfaceTargetUnsafe> {
        // Safety: a window's renderer is let go before the window is.
        Ok(unsafe { wgpu::SurfaceTargetUnsafe::from_display_and_window(self, self)? })
    }

    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// A view over the page is opaque: its page draws its own background.
    pub fn transparent(&self) -> bool {
        false
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.device.get()
    }

    pub fn size(&self) -> Size<Pixels> {
        let device = self.device.get();
        Size { width: Pixels(device.width.0 as f32 / self.density), height: Pixels(device.height.0 as f32 / self.density) }
    }

    pub fn scale(&self) -> f32 {
        self.density
    }

    pub fn resized(&self, size: Size<DevicePixels>) {
        self.device.set(size);
    }

    /// Asks the view to do `command` (`NativeView.apply`), once Rust has let go of the windows.
    fn apply(&self, command: Value) {
        let command = command.to_string();
        java(|env, _| {
            let command = env.new_string(command)?;
            env.call_method(&self.view, "command", "(Ljava/lang/String;)V", &[JValue::Object(&command)]).map(drop)
        });
    }
}

impl rwh::HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let surface = self.surface.as_ref().ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::AndroidNdkWindowHandle::new(surface.ptr().cast::<c_void>());
        Ok(unsafe { rwh::WindowHandle::borrow_raw(rwh::RawWindowHandle::AndroidNdk(handle)) })
    }
}

impl rwh::HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(rwh::RawDisplayHandle::Android(rwh::AndroidDisplayHandle::new())) })
    }
}

/// Tells window `id`'s text input plugin what changed since it was told (`Screen::text_input`), and shows the input
/// with the keyboard whole, as it takes the keyboard and as the keyboard coming up makes the window shorter.
fn keyboard(windows: &mut Windows, id: WindowId) {
    let Some(window) = windows.windows.get_mut(&id) else { return };
    let Ok(screen) = &mut window.screen else { return };
    let revealing = screen.focused().filter(|_| screen.editing().is_some()).map(|focused| (focused, window.parts[0].native.device_size()));
    if revealing.is_some() && revealing != window.parts[0].native.revealed {
        window.parts[0].native.revealed = revealing;
        if revealing.is_some_and(|(focused, _)| screen.reveal(focused)) {
            window.dirty = true;
        }
    }
    let calls = screen.text_input(&mut window.text_input);
    windows.display.text_input(&window.parts[0].native, &calls);
}

/// A finger, as the view's MotionEvent says it, in logical pixels: where it landed, where it was last, whether it's
/// panning, and whether it landed on an input, which it presses only as it lifts.
struct Touch {
    start: Point<f32>,
    last: Point<f32>,
    /// It's moved past the slop, so it's decided whether it pans or is what it's on's (`Screen::pans`).
    moved: bool,
    panning: bool,
    on_input: bool,
}

thread_local! {
    static TOUCHES: RefCell<HashMap<WindowId, Touch>> = RefCell::new(HashMap::new());
}

fn string(env: &mut JNIEnv, string: &JString) -> String {
    env.get_string(string).map(Into::into).unwrap_or_default()
}

// The host (`NativeHost`): its activity attaches it as it's made.

/// The activity is made: the UI thread is the windows' thread from now, and the system is light or dark, with or
/// without animations.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeHost_nativeAttach(env: JNIEnv, host: JClass, dark: jboolean, reduced: jboolean) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_filter(android_logger::FilterBuilder::new().parse("warn,renderer=info").build()).with_tag("native"));
    DARK.store(dark != 0, Ordering::Relaxed);
    REDUCED.store(reduced != 0, Ordering::Relaxed);
    if STARTED.get().is_some() {
        return;
    }
    let (Ok(vm), Ok(host)) = (env.get_java_vm(), env.new_global_ref(host)) else { return };
    JAVA.with(|java| *java.borrow_mut() = Some((vm, host)));
    let Some(looper) = ndk::looper::ThreadLooper::for_thread() else { return log::error!("the UI thread has no looper") };
    let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    let timer = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_CLOEXEC | libc::TFD_NONBLOCK) };
    if wake < 0 || timer < 0 {
        return log::error!("no eventfd or timerfd: {}", std::io::Error::last_os_error());
    }
    let (wake, timer) = unsafe { (Arc::new(OwnedFd::from_raw_fd(wake)), OwnedFd::from_raw_fd(timer)) };
    let (jobs, queue) = mpsc::channel::<Job>();
    let drain = |fd: std::os::fd::BorrowedFd<'_>| {
        let mut count = 0u64;
        unsafe { libc::read(fd.as_raw_fd(), (&mut count as *mut u64).cast(), 8) };
    };
    let added = looper.add_fd_with_callback(wake.as_fd(), ndk::looper::FdEvent::INPUT, move |fd, _| {
        drain(fd);
        let jobs: Vec<Job> = queue.try_iter().collect();
        with_windows(|windows| {
            for job in jobs {
                job(windows);
            }
        });
        true
    });
    let timed = looper.add_fd_with_callback(timer.as_fd(), ndk::looper::FdEvent::INPUT, move |fd, _| {
        drain(fd);
        with_windows(|windows| {
            for window in windows.windows.values_mut() {
                if let Ok(screen) = &mut window.screen
                    && screen.due().is_some_and(|due| due.is_zero())
                {
                    screen.fire();
                    window.dirty = true;
                }
            }
        });
        true
    });
    if let Err(error) = added.and(timed) {
        return log::error!("the looper didn't take its files: {error}");
    }
    TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
    let _ = STARTED.set(Sender { jobs, wake });
}

/// The system turned light or dark, or animations were turned on or off.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeHost_nativeAppearance(_: JNIEnv, _: JClass, dark: jboolean, reduced: jboolean) {
    DARK.store(dark != 0, Ordering::Relaxed);
    REDUCED.store(reduced != 0, Ordering::Relaxed);
    with_windows(|windows| {
        for window in windows.windows.values_mut() {
            if let Ok(screen) = &mut window.screen {
                screen.appearance = appearance();
                screen.reduced_motion = reduced_motion();
            }
            window.dirty = true;
        }
    });
}

// Each view (`NativeView`), by its window's id.

/// Its surface came or changed, `width` by `height` device pixels.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeSurface(env: JNIEnv, _: JObject, id: jlong, surface: JObject, width: jint, height: jint) {
    let surface = unsafe { ndk::native_window::NativeWindow::from_surface(env.get_raw().cast(), surface.as_raw().cast()) };
    let id = id as WindowId;
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&id) else { return };
        let size = Size { width: DevicePixels(width), height: DevicePixels(height) };
        let same = window.parts[0].native.surface.as_ref().zip(surface.as_ref()).is_some_and(|(now, new)| now.ptr() == new.ptr());
        window.parts[0].native.resized(size);
        if same {
            return windows.input(id, 0, Input::Resized(size));
        }
        window.parts[0].native.surface = surface;
        windows.input(id, 0, Input::Surface);
    });
}

/// Its surface is going: nothing may draw on it once this returns.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeSurfaceDestroyed(_: JNIEnv, _: JObject, id: jlong) {
    let id = id as WindowId;
    with_windows(|windows| {
        windows.input(id, 0, Input::SurfaceLost);
        if let Some(window) = windows.windows.get_mut(&id) {
            window.parts[0].native.surface = None;
        }
    });
}

/// The view's window got or lost the keyboard (the app went behind another, or came back): the caret shows only while
/// it has it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeActive(_: JNIEnv, _: JObject, id: jlong, active: jboolean) {
    with_windows(|windows| {
        if let Some(window) = windows.windows.get_mut(&(id as WindowId)) {
            if let Ok(screen) = &mut window.screen {
                screen.activate(active != 0);
            }
            window.dirty = true;
        }
    });
}

/// Back, with nothing of Android's own to close first: the app's screen closes, and what's under it shows.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeBack(_: JNIEnv, _: JObject, id: jlong) -> jboolean {
    with_windows(|windows| windows.back(id as WindowId)).unwrap_or(false).into()
}

/// A finger: `action` is MotionEvent's (0 down, 1 up, 2 move, 3 cancel), at `x`, `y` in device pixels. A finger
/// that moves further than `slop` pans what scrolls under it, and what it pressed isn't pressed. One on an input gives
/// it the keyboard as it lifts, as a phone's own fields do, so one that lands there to scroll doesn't.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeTouch(_: JNIEnv, _: JObject, id: jlong, action: jint, x: jfloat, y: jfloat, slop: jfloat) -> jboolean {
    let id = id as WindowId;
    let panning = with_windows(|windows| {
        let Some(window) = windows.windows.get(&id) else { return false };
        let density = window.parts[0].native.density;
        let at = point(x / density, y / density);
        // A finger on the rail's line is the line's, which the page doesn't hear, and never flings. Only the first
        // finger is followed.
        let phase = match action {
            0 => super::rail::Phase::Down,
            1 => super::rail::Phase::Up,
            2 => super::rail::Phase::Moved,
            _ => super::rail::Phase::Cancelled,
        };
        if windows.line(id, 0, at, phase) {
            return false;
        }
        let Some(window) = windows.windows.get_mut(&id) else { return false };
        let Ok(screen) = &mut window.screen else { return false };
        window.dirty = true;
        screen.touch = true;
        TOUCHES.with(|touches| {
            let mut touches = touches.borrow_mut();
            match action {
                0 => {
                    let on_input = screen.input_at(at);
                    touches.insert(id, Touch { start: at, last: at, moved: false, panning: false, on_input });
                    if !on_input {
                        screen.pointer_down(at, 0, false);
                    }
                }
                2 => {
                    let Some(touch) = touches.get_mut(&id) else { return };
                    if !touch.moved && ((at.x - touch.start.x).abs() > slop / density || (at.y - touch.start.y).abs() > slop / density) {
                        touch.moved = true;
                        touch.panning = touch.on_input || screen.pans(touch.start, point(at.x - touch.start.x, at.y - touch.start.y));
                        if touch.panning && !touch.on_input {
                            screen.pointer_cancel(at);
                        } else if !touch.panning {
                            screen.dragged();
                        }
                    }
                    if touch.panning {
                        screen.pan(touch.start, point(touch.last.x - at.x, touch.last.y - at.y));
                    } else if !touch.on_input {
                        screen.pointer_moved(at);
                    }
                    touch.last = at;
                }
                1 => {
                    let Some(touch) = touches.remove(&id) else { return };
                    if touch.panning {
                        return;
                    }
                    let was = screen.focused();
                    if touch.on_input {
                        screen.pointer_down(at, 0, false);
                    }
                    screen.pointer_up(at, 0);
                    // A tap in the field that has the keyboard brings it up again, if it went.
                    if touch.on_input && was.is_some() && screen.focused() == was {
                        windows.display.text_input(&window.parts[0].native, &[editing::TextInput::Show]);
                    }
                }
                _ => {
                    if touches.remove(&id).is_some_and(|touch| !touch.on_input && !touch.panning) {
                        screen.pointer_cancel(at);
                    }
                }
            }
        });
        TOUCHES.with(|touches| touches.borrow().get(&id).is_some_and(|touch| touch.panning))
    });
    panning.unwrap_or(false).into()
}

/// A step of a fling Java runs (`OverScroller`, in NativeView): what's under `x`, `y`, where the finger went down,
/// scrolls by `dx`, `dy` (device pixels, down and right positive). → whether it did, or its fling stops.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativePan(_: JNIEnv, _: JObject, id: jlong, x: jfloat, y: jfloat, dx: jfloat, dy: jfloat) -> jboolean {
    with_windows(|windows| {
        let window = windows.windows.get_mut(&(id as WindowId))?;
        let density = window.parts[0].native.density;
        let screen = window.screen.as_mut().ok()?;
        let moved = screen.pan(point(x / density, y / density), point(dx / density, dy / density)).is_some();
        window.dirty |= moved;
        Some(moved)
    })
    .flatten()
    .unwrap_or(false)
    .into()
}

/// A finger held: in an input, the word under it is selected, for Android's Cut, Copy and Paste menu; elsewhere, what's
/// under it is told, as a right-click (`contextmenu`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeLongPress(_: JNIEnv, _: JObject, id: jlong, x: jfloat, y: jfloat) {
    let id = id as WindowId;
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&id) else { return };
        // A finger held on the rail's line is the line's.
        if window.pull.is_some() {
            return;
        }
        let density = window.parts[0].native.density;
        let at = point(x / density, y / density);
        let Ok(screen) = &mut window.screen else { return };
        window.dirty = true;
        let touch = TOUCHES.with(|touches| touches.borrow_mut().remove(&id));
        if touch.is_some_and(|touch| !touch.on_input) {
            screen.pointer_cancel(at);
        }
        if screen.select_word(at) {
            return window.parts[0].native.apply(json!({"menu": true}));
        }
        screen.pointer_down(at, 2, false);
        screen.pointer_up(at, 2);
    });
}

/// The keyboard changed field `field` (Flutter's `updateEditingState`), in UTF-16 offsets, -1 for none.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeEditingState(
    mut env: JNIEnv,
    _: JObject,
    id: jlong,
    field: jint,
    text: JString,
    base: jint,
    extent: jint,
    composing_start: jint,
    composing_end: jint,
) {
    let text = string(&mut env, &text);
    let id = id as WindowId;
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&id) else { return };
        let Ok(screen) = &mut window.screen else { return };
        let end = text.encode_utf16().count();
        let at = |offset: jint| if offset < 0 { end } else { (offset as usize).min(end) };
        let value = editing::Value {
            selection: Selection { base: at(base), extent: at(extent) },
            composing: (composing_start >= 0 && composing_end > composing_start).then(|| Composing { start: at(composing_start), end: at(composing_end) }),
            text,
        };
        screen.text_input_client(&mut window.text_input, TextInputClient::UpdateEditingState(field as Id, value));
        window.dirty = true;
    });
}

/// The keyboard's action key (Flutter's `performEditorAction`): next, done, go, search, send.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeAction(mut env: JNIEnv, _: JObject, id: jlong, field: jint, action: JString) {
    let action = string(&mut env, &action);
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&(id as WindowId)) else { return };
        let Ok(screen) = &mut window.screen else { return };
        screen.text_input_client(&mut window.text_input, TextInputClient::PerformAction(field as Id, action));
        window.dirty = true;
    });
}

/// Every field, for Android's autofill (Flutter's `onProvideAutofillVirtualStructure`): its field and its text.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeFields<'local>(env: JNIEnv<'local>, _: JObject, id: jlong) -> JString<'local> {
    let fields = with_windows(|windows| {
        let window = windows.windows.get(&(id as WindowId))?;
        let screen = window.screen.as_ref().ok()?;
        Some(Value::Array(
            screen
                .inputs()
                .into_iter()
                .filter_map(|input| {
                    let (field, value) = (screen.field(input)?, screen.value(input)?);
                    let configuration = editing::configuration(&field, &value);
                    Some(json!({"id": input, "configuration": configuration, "label": field["label"], "rect": field["rect"], "text": value.text}))
                })
                .collect(),
        ))
    })
    .flatten()
    .unwrap_or(json!([]));
    env.new_string(fields.to_string()).unwrap_or_default()
}

/// A password manager filled field `field` (Flutter's `autofill`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeAutofill(mut env: JNIEnv, _: JObject, id: jlong, field: jint, value: JString) {
    let value = string(&mut env, &value);
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&(id as WindowId)) else { return };
        let Ok(screen) = &mut window.screen else { return };
        let end = value.encode_utf16().count();
        screen.set_editing(field as Id, &editing::Value { text: value, selection: Selection { base: end, extent: end }, composing: None });
        window.dirty = true;
    });
}

/// The view is ready for the screen reader: AccessKit's adapter becomes its accessibility delegate.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeAccessibility(mut env: JNIEnv, view: JObject, id: jlong) {
    let id = id as WindowId;
    let adapter = a11y::A11y::new(&mut env, &view, id);
    with_windows(|windows| {
        if let Some(window) = windows.windows.get_mut(&id) {
            window.parts[0].native.a11y = Some(adapter);
        }
    });
}

/// What the screen reader asked of window `id`, done on the UI thread (`NativeView.requestAccessibilityDrain`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_theone_renderer_NativeView_nativeAccessibilityDrain(_: JNIEnv, _: JObject, id: jlong) {
    let id = id as WindowId;
    with_windows(|windows| {
        let Some(window) = windows.windows.get_mut(&id) else { return };
        let Ok(screen) = &mut window.screen else { return };
        window.dirty = true;
        for request in a11y::take(id) {
            let Some(node) = a11y::target(&request) else { continue };
            match request.action {
                accesskit::Action::Click => drop(screen.press(node)),
                accesskit::Action::Focus => screen.set_focused(Some(node)),
                accesskit::Action::ShowContextMenu => {
                    if let Some(rect) = screen.rect(node) {
                        let at = point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                        screen.pointer_down(at, 2, false);
                        screen.pointer_up(at, 2);
                    }
                }
                accesskit::Action::ScrollDown | accesskit::Action::ScrollUp => {
                    if let Some(rect) = screen.rect(node) {
                        let by = rect.height * 0.8 * if request.action == accesskit::Action::ScrollDown { 1.0 } else { -1.0 };
                        screen.pan(point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0), point(0.0, by));
                    }
                }
                _ => {}
            }
        }
    });
}
