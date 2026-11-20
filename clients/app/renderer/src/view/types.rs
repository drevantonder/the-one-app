//! What a screen's host sees of it and gives it: its appearance, its title row, its window's buttons, the pointer's shape, and what a phone does for its page (`Device`).

use super::*;

/// Light or dark: as the device is, or as Colour Mode says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

/// What the app's JS posts its client (`host.post`), for the host to answer, given how the device looks: → the
/// answer, where the host has it at once. One it answers later, or a notification, goes to `Screen::message`.
pub type Asked = Box<dyn FnMut(Value, Appearance) -> Option<Value>>;

/// What a computer's window shows above its page, in its title row: the app's icon, a `data:` address, and its name
/// (DESIGN.md, App windows), which nothing presses. Its page keeps the row's height clear, as its host tells it.
pub struct Title {
    pub icon: Option<String>,
    pub name: String,
    /// How far in from the window's start it begins, in the band its top inset keeps clear, past a Mac's traffic
    /// lights; full screen, where the Mac hides them, it begins at the window's edge (`Screen::full_screen`).
    pub start: f32,
}

/// Where a press moves a window rather than reaching its page (`Screen::title_row`): its title row over its page, its
/// top bar's, `height` tall, and what its page says moves it (`appRegion: "drag"`, `regions`), but where in them a
/// press is its page's (`taken`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleRow {
    pub height: f32,
    pub taken: Vec<Bounds<f32>>,
    pub regions: Vec<Bounds<f32>>,
}

impl TitleRow {
    /// Whether a press at `at` moves the window.
    pub fn moves(&self, at: Point<f32>) -> bool {
        (at.y < self.height || self.regions.iter().any(|region| region.contains(&at))) && !self.taken.iter().any(|bounds| bounds.contains(&at))
    }
}

/// Where a trackpad's scroll is, where the system says (a Mac's; the DOM's wheel events don't): `fingers`, their
/// phase, by AppKit's names (`began`, `changed`, `ended`, `cancelled`, `mayBegin`), while they're on it, and
/// `momentum`, whether it's the scrolling the system goes on with after they've lifted. A page is told them as the
/// wheel event's `phase` and `momentum`, the name wheel-gestures reads it by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    pub fingers: Option<&'static str>,
    pub momentum: bool,
}

impl Phase {
    /// The fingers' phases, by AppKit's names.
    pub const FINGERS: [&'static str; 5] = ["began", "changed", "ended", "cancelled", "mayBegin"];
}

/// How a page's batches are held back (`Screen::late`): `hold` says how many of the window's turns each waits, and
/// `held` is each waiting, with the calls it had run by then.
pub struct Late {
    pub hold: Box<dyn FnMut() -> u32>,
    pub(super) held: std::collections::VecDeque<(u32, Vec<crate::js::Posted>, u64)>,
}

impl Late {
    pub fn new(hold: Box<dyn FnMut() -> u32>) -> Late {
        Late { hold, held: Default::default() }
    }

    /// Whether a batch is waiting.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }
}

/// A panel's own background, which its window has no frame for (`WindowOptions::panel`): its corners' radius, and
/// whether the host fills where its page asks for the system's glass with a popover's colour, where the system has
/// none to draw under it (Linux); and with `whole`, the whole panel is on the glass, for a page that floats nothing over
/// the desktop of its own (the client's own pages in the launcher's place).
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub corner: f32,
    pub fill: bool,
    pub whole: bool,
}

/// What the window says over its page of where the page is, as a phone's app says it over an app (DESIGN.md, App
/// windows): its network not answering ("Reconnecting…"), or its page updated ("Updated", with Reload). A pill at its
/// top, over the page, with its one action.
/// One of a window's own buttons, which a window with no frame of the system's draws in its title row (a GNOME header
/// bar's), on the side and in the order its desktop sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Close,
    Minimize,
    Maximize,
}

/// How wide one of Windows' caption buttons is, in points, which its window draws in its title row, flush with the
/// window's edge (`WindowButton`) and as tall as the row.
pub const CAPTION_BUTTON: f32 = 46.0;

/// The system's own menu for text in a field, and what of it can be done, as every system's fields have: on a computer
/// where a secondary click was (`Screen::take_text_menu`), whose choice comes back as its key; on a phone over or
/// under what a finger selected (`Screen::text_menu_shown`), which the system's own field does itself. `rect` is
/// what it's shown beside, in points of the page: a click's point, or a selection's rectangle. `select` selects the
/// word at the caret, as iOS's menu's Select.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMenu {
    pub rect: Bounds<f32>,
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub select_all: bool,
    pub select: bool,
}

pub struct Notice {
    pub text: String,
    pub action: Option<(String, Box<dyn FnMut()>)>,
}

/// The pointer's shape over what's under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Cursor {
    #[default]
    Arrow,
    Pointer,
    Text,
}

/// What a phone does for its page itself, which its window doesn't draw (DESIGN.md: "its pickers and its share
/// sheet"), and its haptics: each says whether the device did it. A computer does none of them (`()`).
pub trait Device {
    /// Shows the device's picker for picker node `picker`, drawn at `at`, of its `options` (each its `value`, `label`
    /// and whether it's `disabled`), `chosen` the one chosen; what's chosen comes back as `Screen::picked`.
    fn pick(&mut self, picker: Id, at: Rect, options: &Value, chosen: Option<&str>) -> bool;
    /// Shows the device's share sheet for `text`.
    fn share(&mut self, text: &str) -> bool;
    /// Plays a haptic, by iOS's name (`haptic` in the UI library).
    fn haptic(&mut self, kind: &str) -> bool;
    /// Shows the device's own picker of files to open, of `accept`'s kinds (`.png`, `image/*`; any where it says
    /// none), one or `multiple`, and tells `chosen` the files chosen (none where it was cancelled), on a thread of its
    /// own (`pickFiles` in the UI library). → whether it has one.
    fn open_files(&mut self, _accept: &[String], _multiple: bool, _chosen: Box<dyn FnOnce(Vec<std::path::PathBuf>) + Send>) -> bool {
        false
    }
}

impl Device for () {
    fn pick(&mut self, _: Id, _: Rect, _: &Value, _: Option<&str>) -> bool {
        false
    }
    fn share(&mut self, _: &str) -> bool {
        false
    }
    fn haptic(&mut self, _: &str) -> bool {
        false
    }
}
