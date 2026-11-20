//! A platform with nothing behind it, for a harness, as GPUI's `TestPlatform` and Flutter's test binding are: the same
//! windows and the same loop (`Windows::turned`) as every host's, run against a stand-in for its display, whose
//! input, display ticks and surfaces come from whoever drives it (`examples/screenshot.rs`), not from a system. Its
//! windows draw offscreen, as a compositor would show them, and the picture last presented is read back from there.
//! Built with the `fake` feature, in place of the system's platform.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use super::{Input, WindowId, WindowOptions};
use crate::geometry::{DevicePixels, Pixels, Size};
use crate::view::Cursor;

pub struct Display {
    scale: f32,
    /// The display's refresh, which its ticks come at unless the driver says otherwise.
    refresh: Cell<std::time::Duration>,
    /// What's happened to its windows that the loop hasn't heard yet, in order.
    events: RefCell<VecDeque<(WindowId, usize, Input)>>,
    /// The display showed a frame, which the loop hasn't heard yet.
    tick: Cell<Option<web_time::Instant>>,
    /// Whether the loop wants the display's next frame.
    wanted: Cell<bool>,
    /// What decides which of its refreshes come late, where it's adversarial (`seed`): a xorshift's state.
    random: Cell<u64>,
    /// How long each frame takes to draw, in ms, where it's longer than a refresh, as on a device others load
    /// (`set_frame_ms`).
    drawing: Cell<f64>,
    /// The panels put on the screen since the display last showed a frame, which the frame it shows next is checked
    /// on (`Windows::tick`): a host's summon shows its window at once, with what it last presented.
    summoned: RefCell<Vec<WindowId>>,
    /// What was wrong with what it showed (`Windows::tick`), since it was last asked.
    wrong: RefCell<Vec<String>>,
    /// What each window's input with the keyboard last asked the platform's keyboard for (Flutter's
    /// `TextInputConfiguration`, as `TextInput.setClient` gives it), as a phone's keyboard is told.
    keyboard: RefCell<std::collections::HashMap<WindowId, serde_json::Value>>,
}

/// How often a refresh comes late where the display is adversarial, a frame dropped, as a busy device's are: one in
/// five, and never more than three in a row.
const LATE: u64 = 5;

pub struct NativeWindow {
    id: WindowId,
    size: Cell<Size<DevicePixels>>,
    scale: f32,
    /// How far across it's drawn, swiped back from the left edge, as an iPhone's window is (`slide`).
    slid: Cell<f32>,
}

/// A window as `Display::arrange` places it.
#[derive(Clone, Copy)]
pub struct Handle;

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle
    }

    /// It draws offscreen from the start (`super::drawn_on`).
    pub fn has_surface(&self) -> bool {
        true
    }

    pub fn device_size(&self) -> Size<DevicePixels> {
        self.size.get()
    }

    /// The window it is, as X11's names it, which nothing reads here.
    pub fn x_window(&self) -> u32 {
        self.id as u32
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

    /// Draws it `across` points to the right, as an iPhone's window is moved by its transform.
    pub fn slide(&self, across: f32) {
        self.slid.set(across);
    }

    /// How far across it's slid (`slide`).
    pub fn slid(&self) -> f32 {
        self.slid.get()
    }
}

impl Display {
    /// A display of `scale` device pixels a point, refreshing `hz` times a second.
    pub fn new(scale: f32, hz: f64) -> Display {
        Display {
            scale,
            refresh: Cell::new(std::time::Duration::from_secs_f64(1.0 / hz)),
            events: RefCell::default(),
            tick: Cell::new(None),
            wanted: Cell::new(false),
            random: Cell::new(0),
            drawing: Cell::new(0.0),
            summoned: RefCell::default(),
            wrong: RefCell::default(),
            keyboard: RefCell::default(),
        }
    }

    /// Something happened to window `id`, which the loop hears on its next turn.
    pub(crate) fn push(&self, id: WindowId, input: Input) {
        self.events.borrow_mut().push_back((id, 0, input));
    }

    /// The display showed a frame, at `at`, which the loop hears on its next turn.
    pub fn tick(&self, at: web_time::Instant) {
        self.tick.set(Some(at));
    }

    /// Its refreshes come late as `seed` decides, the same each time for the same seed; 0 for a display that never
    /// misses one.
    pub fn seed(&self, seed: u64) {
        self.random.set(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | u64::from(seed != 0));
    }

    /// A number from 0 to below `below`, as its seed decides: always 0 with none.
    pub fn random(&self, below: u64) -> u64 {
        let mut x = self.random.get();
        if x == 0 || below == 0 {
            return 0;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random.set(x);
        x % below
    }

    /// Whether the refresh due now comes late, so the frame it would show is shown a refresh later: never with no
    /// seed, and never more than three in a row.
    pub fn late(&self, in_a_row: u32) -> bool {
        in_a_row < 3 && self.random(LATE) == 1
    }

    /// Each frame takes `ms` to draw from now on, its thread busy meanwhile, as on a device others load; 0 for none.
    pub fn set_frame_ms(&self, ms: f64) {
        self.drawing.set(ms);
    }

    /// How long a frame takes to draw, in ms.
    pub fn frame_ms(&self) -> f64 {
        self.drawing.get()
    }

    /// Whether the loop asked for the display's next frame.
    pub fn wanted(&self) -> bool {
        self.wanted.get()
    }

    pub fn set_refresh(&self, hz: f64) {
        self.refresh.set(std::time::Duration::from_secs_f64(1.0 / hz));
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        let device = |points: f32| DevicePixels((points * self.scale).round() as i32);
        Ok(NativeWindow { id, size: Cell::new(Size { width: device(options.size.width.0), height: device(options.size.height.0) }), scale: self.scale, slid: Cell::new(0.0) })
    }

    /// A part of a panel's: a window of its own for each pane of the system's glass its page asks for, as on a Mac.
    pub fn open_part(&self, parent: &NativeWindow, _: usize) -> anyhow::Result<NativeWindow> {
        Ok(NativeWindow { id: parent.id, size: Cell::new(Size { width: DevicePixels(1), height: DevicePixels(1) }), scale: self.scale, slid: Cell::new(0.0) })
    }

    pub fn summon(&self, window: &NativeWindow, _: &super::Places, _: Size<Pixels>) {
        self.summoned.borrow_mut().push(window.id);
    }

    /// The panels put on the screen since it was last asked.
    pub(crate) fn summoned(&self) -> Vec<WindowId> {
        self.summoned.take()
    }

    pub(crate) fn wrong(&self, what: String) {
        self.wrong.borrow_mut().push(what);
    }

    pub(crate) fn wrongs(&self) -> Vec<String> {
        self.wrong.take()
    }

    pub fn put_away(&self, _: &NativeWindow, _: bool) {}

    pub fn arrange(&self, _: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        draw();
    }

    pub fn attach(&self, _: &NativeWindow, _: &NativeWindow) {}

    pub fn show(&self, _: &NativeWindow) {}

    pub fn close(&self, _: &NativeWindow) {}

    pub fn activate(&self, _: &NativeWindow) {}

    pub fn unframe(&self, _: &NativeWindow) {}

    pub fn minimize(&self, _: &NativeWindow) {}

    pub fn toggle_maximized(&self, _: u32) {}

    pub fn set_appearance(&self, _: &NativeWindow, _: Option<crate::Appearance>) {}

    pub fn text_menu(&self, _: &NativeWindow, _: crate::view::TextMenu) {}

    pub fn set_cursor(&self, _: &NativeWindow, _: Cursor) {}

    pub fn set_title_row(&self, _: &NativeWindow, _: Option<crate::view::TitleRow>) {}

    pub fn text_input(&self, window: &NativeWindow, calls: &[crate::editing::TextInput]) {
        for call in calls {
            match call {
                crate::editing::TextInput::SetClient(_, configuration) => _ = self.keyboard.borrow_mut().insert(window.id, configuration.clone()),
                crate::editing::TextInput::ClearClient => _ = self.keyboard.borrow_mut().remove(&window.id),
                _ => {}
            }
        }
    }

    /// What window `id`'s input with the keyboard asked the keyboard for, if one has it.
    pub(crate) fn keyboard(&self, id: WindowId) -> Option<serde_json::Value> {
        self.keyboard.borrow().get(&id).cloned()
    }

    #[allow(dead_code)]
    pub fn set_moved(&self, _: &NativeWindow, _: super::rail::Moved) {}

    pub fn flush(&self) {}

    pub fn pressure(&self) -> super::Pressure {
        super::Pressure::Nominal
    }

    pub fn refresh(&self) -> Option<std::time::Duration> {
        Some(self.refresh.get())
    }

    /// It says when it shows each frame, as its driver ticks it.
    pub fn paced(&self) -> bool {
        true
    }

    pub fn request_frame(&self, wanted: bool) {
        self.wanted.set(wanted);
    }

    pub fn frame(&self) -> Option<web_time::Instant> {
        self.tick.take()
    }

    pub(crate) fn events(&self) -> Vec<(WindowId, usize, Input)> {
        self.events.borrow_mut().drain(..).collect()
    }
}

/// The clock the fake platform tells time by: its pages', paused, which only its driver moves.
static TIME: std::sync::Mutex<Option<(web_time::Instant, crate::js::Clock)>> = std::sync::Mutex::new(None);

/// Tells time by `clock` from now on.
pub fn keep_time(clock: crate::js::Clock) {
    *TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((web_time::Instant::now() - std::time::Duration::from_secs_f64(clock.now().max(0.0) / 1000.0), clock));
}

/// Its time, in ms on the clock it tells time by.
pub fn time() -> f64 {
    TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map_or(0.0, |(_, clock)| clock.now())
}

/// Moves its time on by `ms`, the clock it tells time by with it.
pub fn advance(ms: f64) {
    if let Some((_, clock)) = &*TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        clock.pause();
        clock.advance(ms);
    }
}

/// Now, by the clock it tells time by, as an `Instant`: as long after its start as the clock says.
pub fn now() -> web_time::Instant {
    match &*TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Some((start, clock)) => *start + std::time::Duration::from_secs_f64(clock.now().max(0.0) / 1000.0),
        None => web_time::Instant::now(),
    }
}

pub fn appearance() -> crate::Appearance {
    crate::Appearance::Light
}

pub fn reduced_motion() -> bool {
    false
}

/// The harness's page keeps a clipboard of its own (`editing::Memory`).
pub fn clipboard() -> Option<Box<dyn crate::editing::Clipboard>> {
    None
}

pub fn open_files(_: &[String], _: bool, chosen: Box<dyn FnOnce(Vec<std::path::PathBuf>) + Send>) {
    chosen(Vec::new());
}

pub fn button_layout() -> (Vec<crate::WindowButton>, Vec<crate::WindowButton>) {
    (Vec::new(), Vec::new())
}
