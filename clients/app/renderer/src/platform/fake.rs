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
    /// How its display link ticks once it's asked for frames (`set_link`), and how many refreshes it's run since it
    /// was, while it is.
    link: Cell<Link>,
    running: Cell<Option<u32>>,
    /// The refresh it last reached, counted from the clock's start, so each is reached once, whichever side of it the
    /// clock lands (`Windows::run`).
    refreshed: Cell<Option<i64>>,
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
    /// What each window last asked its system's menu for text and its magnifier to show, as a phone's are asked, with
    /// where the system puts the menu (`placed`).
    text_menu: RefCell<std::collections::HashMap<WindowId, (crate::view::TextMenu, crate::geometry::Bounds<f32>)>>,
    magnifier: RefCell<std::collections::HashMap<WindowId, crate::view::Magnifier>>,
    /// A phone's screen, in points, where it's a phone's (`phone`): every window but a panel is the whole of it,
    /// whatever it was asked to be, as an iPhone's and an Android phone's are.
    phone: Cell<Option<Size<Pixels>>>,
    /// What a window's page asked its client that's on its way back, each with when it comes on the page's clock, as a
    /// server's answer comes a round trip after it's asked (`Windows::answering`).
    answers: RefCell<Option<(WindowId, Answers)>>,
}

/// Answers on their way to a page, each with when it comes on the page's clock.
pub(crate) type Answers = std::rc::Rc<RefCell<Vec<(f64, serde_json::Value)>>>;

/// A phone's system's menu for text: 44 points tall, as UIEditMenuInteraction's, and so wide an item, 8 from what
/// it's for.
const MENU_HEIGHT: f32 = 44.0;
const MENU_ITEM: f32 = 72.0;
const MENU_GAP: f32 = 8.0;

/// Where a phone's system puts its menu for text shown for `menu.rect` in a window `width` wide: as wide as its items,
/// centred over it and inside the window, above it where there's room in the window and below it where there isn't,
/// as Android's floating toolbar is placed in the whole of its window, which an app drawn edge to edge has under its
/// status bar too.
fn placed(menu: &crate::view::TextMenu, width: f32) -> crate::geometry::Bounds<f32> {
    let items = [menu.cut, menu.copy, menu.paste, menu.select_all, menu.select].iter().filter(|item| **item).count();
    let size = Size { width: (MENU_ITEM * items as f32).min(width), height: MENU_HEIGHT };
    let rect = menu.rect;
    let x = (rect.origin.x + rect.size.width / 2.0 - size.width / 2.0).clamp(0.0, (width - size.width).max(0.0));
    let above = rect.origin.y - MENU_GAP - size.height;
    let y = if above >= 0.0 { above } else { rect.origin.y + rect.size.height + MENU_GAP };
    crate::geometry::Bounds { origin: crate::geometry::point(x, y), size }
}

/// How often a refresh comes late where the display is adversarial, a frame dropped, as a busy device's are: one in
/// five, and never more than three in a row.
const LATE: u64 = 5;

/// How a display link ticks once it's started (`Display::set_link`): `starts` refreshes after, and with `pauses`,
/// (ticks, refreshes), not for that many refreshes once it's ticked that many. The default ticks at every refresh, from
/// the first.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Link {
    pub starts: u32,
    pub pauses: Option<(u32, u32)>,
}

pub struct NativeWindow {
    id: WindowId,
    size: Cell<Size<DevicePixels>>,
    scale: f32,
    /// How far across it's drawn, swiped back from the left edge, as an iPhone's window is (`slide`).
    slid: Cell<f32>,
    /// How it's drawn over what's under it (`set_moved`), and whether it's on the screen (`show`, `put_away`), as a
    /// compositor shows it (`Windows::screen`).
    moved: Cell<super::rail::Moved>,
    shown: Cell<bool>,
}

/// A window as `Display::arrange` places it.
#[derive(Clone, Copy)]
pub struct Handle;

impl NativeWindow {
    pub fn handle(&self) -> Handle {
        Handle
    }

    /// Whether what isn't drawn on it is see-through: never, as it draws offscreen.
    pub fn transparent(&self) -> bool {
        false
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

    /// How it's drawn over what's under it, and whether it's on the screen.
    pub(crate) fn composed(&self) -> (super::rail::Moved, bool) {
        (self.moved.get(), self.shown.get())
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
            link: Cell::new(Link::default()),
            running: Cell::new(None),
            refreshed: Cell::new(None),
            random: Cell::new(0),
            drawing: Cell::new(0.0),
            summoned: RefCell::default(),
            wrong: RefCell::default(),
            keyboard: RefCell::default(),
            text_menu: RefCell::default(),
            magnifier: RefCell::default(),
            phone: Cell::new(None),
            answers: RefCell::default(),
        }
    }

    /// It's a phone's screen, `screen` points big (`phone`).
    pub fn phone(&self, screen: Size<Pixels>) {
        self.phone.set(Some(screen));
    }

    /// Whether it's a phone's screen.
    pub fn is_phone(&self) -> bool {
        self.phone.get().is_some()
    }

    /// What window `id`'s page asked its client is answered from `answers` on their way (`Windows::answering`).
    pub(crate) fn answering(&self, id: WindowId, answers: Answers) {
        *self.answers.borrow_mut() = Some((id, answers));
    }

    /// When the next answer on its way comes, if one is.
    pub(crate) fn answer_due(&self) -> Option<f64> {
        self.answers.borrow().as_ref()?.1.borrow().iter().map(|(at, _)| *at).reduce(f64::min)
    }

    /// The answers on their way that have come by `now`, each with the window whose page asked.
    pub(crate) fn answers_due(&self, now: f64) -> Vec<(WindowId, serde_json::Value)> {
        let Some((id, answers)) = &*self.answers.borrow() else { return Vec::new() };
        let mut answers = answers.borrow_mut();
        let (due, later) = answers.drain(..).partition(|(at, _)| *at <= now);
        *answers = later;
        due.into_iter().map(|(_, answer)| (*id, answer)).collect()
    }

    /// Something happened to window `id`, which the loop hears on its next turn.
    pub(crate) fn push(&self, id: WindowId, input: Input) {
        self.push_to(id, 0, input);
    }

    /// Something happened to native window `part` of window `id` (a panel's surface of glass), where it is in that.
    pub(crate) fn push_to(&self, id: WindowId, part: usize, input: Input) {
        self.events.borrow_mut().push_back((id, part, input));
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

    /// Its display link ticks as `link` says from now on: as a Mac's, which takes two or three refreshes to start, and a
    /// ProMotion display's, whose link stops ticking a while as if what's on it were idle.
    pub fn set_link(&self, link: Link) {
        (self.link.set(link), self.running.set(None));
    }

    /// Whether its link ticks at the refresh due now: while the loop asks for frames, once it's started and unless
    /// it's paused, and with the default link at every refresh.
    pub fn ticks(&self) -> bool {
        let link = self.link.get();
        if link == Link::default() {
            return true;
        }
        let Some(run) = self.wanted.get().then(|| self.running.get().map_or(0, |run| run + 1)) else {
            return false;
        };
        self.running.set(Some(run));
        run.checked_sub(link.starts).is_some_and(|ticked| !link.pauses.is_some_and(|(after, refreshes)| (after..after + refreshes).contains(&ticked)))
    }

    pub fn set_refresh(&self, hz: f64) {
        self.refresh.set(std::time::Duration::from_secs_f64(1.0 / hz));
        self.refreshed.set(None);
    }

    /// The refresh it last reached (`refreshed`).
    pub fn refreshed(&self) -> Option<i64> {
        self.refreshed.get()
    }

    /// It reached refresh `at`, counted from the clock's start.
    pub fn reached(&self, at: i64) {
        self.refreshed.set(Some(at));
    }

    pub fn open(&self, id: WindowId, options: &WindowOptions) -> anyhow::Result<NativeWindow> {
        std::thread::sleep(costs().window);
        let device = |points: f32| DevicePixels((points * self.scale).round() as i32);
        let size = match self.phone.get() {
            Some(screen) if options.panel.is_none() => screen,
            _ => options.size,
        };
        Ok(NativeWindow { id, size: Cell::new(Size { width: device(size.width.0), height: device(size.height.0) }), scale: self.scale, slid: Cell::new(0.0), moved: Cell::new(super::rail::Moved::STILL), shown: Cell::new(false) })
    }

    /// A part of a panel's: a window of its own for each pane of the system's glass its page asks for, as on a Mac.
    pub fn open_part(&self, parent: &NativeWindow, _: usize) -> anyhow::Result<NativeWindow> {
        Ok(NativeWindow { id: parent.id, size: Cell::new(Size { width: DevicePixels(1), height: DevicePixels(1) }), scale: self.scale, slid: Cell::new(0.0), moved: Cell::new(super::rail::Moved::STILL), shown: Cell::new(true) })
    }

    pub fn summon(&self, window: &NativeWindow, _: &super::Places, _: Size<Pixels>) {
        window.shown.set(true);
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

    pub fn put_away(&self, window: &NativeWindow, _: bool) {
        window.shown.set(false);
    }

    pub fn arrange(&self, _: &[(Handle, crate::geometry::Bounds<f32>, f32)], draw: &mut dyn FnMut()) {
        draw();
    }

    pub fn attach(&self, _: &NativeWindow, _: &NativeWindow) {}

    pub fn show(&self, window: &NativeWindow) {
        window.shown.set(true);
    }

    pub fn close(&self, _: &NativeWindow) {}

    pub fn activate(&self, _: &NativeWindow) {}

    pub fn unframe(&self, _: &NativeWindow) {}

    pub fn minimize(&self, _: &NativeWindow) {}

    pub fn toggle_maximized(&self, _: u32) {}

    pub fn set_appearance(&self, _: &NativeWindow, _: Option<crate::Appearance>) {}

    pub fn text_menu(&self, window: &NativeWindow, menu: Option<crate::view::TextMenu>) {
        let mut shown = self.text_menu.borrow_mut();
        match menu {
            Some(menu) => _ = shown.insert(window.id, (menu, placed(&menu, window.size.get().width.0 as f32 / window.scale))),
            None => _ = shown.remove(&window.id),
        }
    }

    pub fn magnifier(&self, window: &NativeWindow, magnifier: Option<crate::view::Magnifier>) {
        let mut shown = self.magnifier.borrow_mut();
        match magnifier {
            Some(magnifier) => _ = shown.insert(window.id, magnifier),
            None => _ = shown.remove(&window.id),
        }
    }

    /// What window `id` last asked its system's menu for text to show, with where the system put it, and where its
    /// magnifier shows, if it does.
    pub(crate) fn text_selection(&self, id: WindowId) -> (Option<(crate::view::TextMenu, crate::geometry::Bounds<f32>)>, Option<crate::view::Magnifier>) {
        (self.text_menu.borrow().get(&id).copied(), self.magnifier.borrow().get(&id).copied())
    }

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

    pub fn set_moved(&self, window: &NativeWindow, moved: super::rail::Moved) {
        window.moved.set(moved);
    }

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

    /// Starts its link while a frame is `wanted`, and stops it once none is.
    pub fn request_frame(&self, wanted: bool) {
        self.wanted.set(wanted);
        if !wanted {
            self.running.set(None);
        }
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
/// What a device a harness stands for takes to make what the fake platform makes at once, as a tier's `measured`
/// costs say in core/ui/devices.json: its system's window and the renderer for it (`ONE_WINDOW_MS`), and the
/// pipelines a kind of window draws with (`ONE_PIPELINES_MS`). None unless it's told.
pub(crate) struct Costs {
    pub window: std::time::Duration,
    pub pipelines: std::time::Duration,
}

pub(crate) fn costs() -> Costs {
    let ms = |name: &str| std::time::Duration::from_millis(std::env::var(name).ok().and_then(|ms| ms.parse().ok()).unwrap_or(0));
    Costs { window: ms("ONE_WINDOW_MS"), pipelines: ms("ONE_PIPELINES_MS") }
}

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

/// The faces a Linux device falls back to where Allo hasn't a character, a symbol's (⇧, ⋮) and an emoji's, from the
/// harness's host (Debian's and Ubuntu's `fonts-dejavu-core` and `fonts-noto-color-emoji`), so it draws what a device
/// does and not a gap, and the monospace for code, which the system has and a harness's host loads none of; on a device it's sent to that hasn't them (`motion.js --ssh`, a Windows one), from beside it,
/// where they're sent with it, so it draws the same glyphs as here.
pub fn fallback_fonts() -> anyhow::Result<Vec<Vec<u8>>> {
    let beside = |path: &str| Some(std::env::current_exe().ok()?.parent()?.join(std::path::Path::new(path).file_name()?));
    ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"]
        .into_iter()
        .map(|path| {
            std::fs::read(path)
                .or_else(|error| beside(path).map_or(Err(error), std::fs::read))
                .map_err(|error| anyhow::anyhow!("the harness falls back to {path}, which it couldn't read (apt install fonts-dejavu-core fonts-noto-color-emoji): {error}"))
        })
        .collect()
}

/// The family of the monospace `fallback_fonts` has, which the harness's text calls its system's.
pub const MONOSPACE: &str = "DejaVu Sans Mono";

/// Whether the system is in Dark mode, which a harness says (`Windows::appearance`): Light until it does.
static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn appearance() -> crate::Appearance {
    match DARK.load(std::sync::atomic::Ordering::Relaxed) {
        true => crate::Appearance::Dark,
        false => crate::Appearance::Light,
    }
}

/// The system's appearance is now `appearance`, as a person switching between Light and Dark makes it.
pub fn appear(appearance: crate::Appearance) {
    DARK.store(appearance == crate::Appearance::Dark, std::sync::atomic::Ordering::Relaxed);
}

static REDUCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The system's Reduce Motion setting is on (`--reduced-motion`), as a person's is in their settings.
pub fn reduce(on: bool) {
    REDUCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn reduced_motion() -> bool {
    REDUCED.load(std::sync::atomic::Ordering::Relaxed)
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
