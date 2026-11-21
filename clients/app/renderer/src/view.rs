//! An app's screen: its tree laid out by Taffy and painted as a scene with the design tokens, and what the person
//! does (a press, a key, a scroll) found on it and sent back to its JS as events. The window it's in (`platform`, or
//! the headless harness) gives it the room, the scale and the input, and draws the scene it paints.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use serde_json::{Value, json};
use taffy::prelude::{AvailableSpace, NodeId, TaffyTree};

use crate::editing::{self, Clipboard, Field, Ime};
use crate::geometry::{Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels, Point, ScaledPixels, Size, point, px};
use crate::inspect::{self, Key, Rect, Rects};
use crate::js::{Posted, Thread};
use crate::render::atlas::{Atlas, AtlasTile};
use crate::render::scene::{Backdrop, BorderStyle, PolychromeSprite, Quad, Recording, Scene, Shadow, TransformationMatrix};
use crate::style::{self, Look, Offset, Paint, State, Text};
use crate::text::{Paragraph, TextRun, TextSystem};
use crate::tokens::{Conditions, Radius};
use crate::tree::{Id, Kind, Node, Tree};
use paint::*;
pub(crate) use paint::{Decoded, Picture};

mod bounce;
mod bridge;
mod drag;
mod finger;
pub use bridge::TextInputConnection;
mod frames;
mod list;
#[cfg(not(target_arch = "wasm32"))]
mod videos;
mod paint;
mod pictures;
#[cfg(feature = "fake")]
pub use pictures::{cut_short, decoding_takes};
mod selection;
mod text_selection;
pub use text_selection::Magnifier;
pub mod touch;
pub use frames::Make as MakeFrame;
pub(crate) mod inspected;
mod transition;
mod draw;
mod input;
mod keyboard;
mod pointer;
mod pump;
mod types;
mod window;
pub use types::*;
pub(crate) use draw::paint_message;
use draw::Seen;
pub(crate) use input::Drawn;
pub(crate) use pointer::{Clicked, Placed, States};
pub(crate) use pump::files_told;

/// The typeface every screen is drawn in, as the UI library's are (core/ui/src/tokens.css), by the family name its
/// file has, which a stylesheet's `@font-face` calls "Allo".
pub const FONT: &str = "Allo Variable";

/// The typeface, the UI library's WOFF2.
pub(crate) const ALLO: &[u8] = include_bytes!("../../../../core/ui/src/Allo-Variable.woff2");

/// The typeface as TrueType, which the text system reads.
pub fn fonts() -> anyhow::Result<Vec<Vec<u8>>> {
    let ttf = wuff::decompress_woff2(ALLO)
        .map_err(|error| anyhow::anyhow!("Allo didn't decode: {error:?}"))?;
    Ok(vec![ttf])
}

/// How long a scroll stops before what's under a still pointer is hovered again, as Blink's fake mouse move after a
/// scroll waits (`MouseEventManager`'s 100 ms).
const HOVER_AFTER_SCROLL: f64 = 100.0;

/// How long after its shortcut summoned a panel, or after it was last pressed again, the shortcut's key is still its:
/// longer than a key's repeat delay, so one held repeats into nothing.
/// The roles of what the DOM draws as a button, which Tab reaches (core/ui's drawing-primitives/primitives.tsx `pressed`).
const BUTTON_ROLES: [&str; 9] = ["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"];

/// The page itself, as an event's node: what a key is told to, once, for what listens to every key (`keydown()`).
const PAGE: Id = 0;
/// The window's own nodes beside its page's, as Tab and the inspector reach them: its notice and the notice's action
/// (`Screen::semantics`).
pub const NOTICE: Id = u64::MAX - 2;
pub const NOTICE_ACTION: Id = u64::MAX - 3;

/// How far in from a full-screen window's edge its title starts, in points.
const EDGE: f32 = 12.0;
const SHORTCUT_HELD: std::time::Duration = std::time::Duration::from_millis(600);

pub struct Screen {
    tree: Tree,
    /// Its app's JS, on a thread of its own, and whether its module has run, or why it didn't.
    js: Thread,
    started: Option<Result<(), String>>,
    /// The inspector's answers to what it did, each told once the app's JS has run what that did.
    replies: Vec<(Value, inspected::Reply)>,
    /// What wakes its window as its JS, or a frame's, posts what this hasn't applied.
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The files the device's picker chose, by what asked (`one/files`), until they're given to the page.
    chosen_files: Arc<std::sync::Mutex<Vec<(Value, Vec<std::path::PathBuf>)>>>,
    /// Each frame's page, by the frame's node, and the pages asked for that haven't come (`frames`); what makes one,
    /// its host's; and whether this is a frame's page itself, and Tab took the keyboard out of it, back or on.
    frames: HashMap<Id, frames::Child>,
    asking: HashSet<(Id, String)>,
    pub make_frame: Option<frames::Make>,
    nested: bool,
    left: Option<bool>,
    /// The frame the pointer's over, and the one a press it holds went down in.
    pointing: Option<Id>,
    captured_frame: Option<Id>,
    /// Each video's connection (`videos`), the one a press went down on, the one keys go to, and how far the wheel
    /// over one has turned that isn't yet a step.
    #[cfg(not(target_arch = "wasm32"))]
    videos: HashMap<Id, crate::video::Video>,
    #[cfg(not(target_arch = "wasm32"))]
    video_pressed: Option<Id>,
    #[cfg(not(target_arch = "wasm32"))]
    video_keys: Option<Id>,
    #[cfg(not(target_arch = "wasm32"))]
    video_turned: Point<f32>,
    /// As Colour Mode says, or as the device is (`None`).
    pub theme: Option<Appearance>,
    /// As the device is, which the window says.
    pub appearance: Appearance,
    /// The person asked for less motion, as the device says.
    pub reduced_motion: bool,
    /// How pressed the device is (`platform::Pressure`), which the page hears as `hostContext.pressure`, to do less as a
    /// web page's `PressureObserver` would. What's drawn is drawn as ever: what moves goes on moving, as a cross-fade
    /// costs a frame no less than a slide, and glass keeps its blur, which a still page behind it costs once
    /// (`Renderer`'s kept blurs), where drawn plain it showed that page sharp through its tint for the seconds
    /// pressure took to ease (20 Nov, the App Library's categories on a loaded Mac).
    pub pressure: crate::platform::Pressure,
    /// The device's own text size, which a phone has, in pixels: 1rem is it, and the `phone` tokens hold.
    pub text_size: Option<f32>,
    /// Whose own tokens it draws with, as its page's styles gave them (`tokens::load_page`).
    pub page: u32,
    /// What its window says above it, on a computer: the app's icon and name.
    pub title: Option<Title>,
    /// Its own window buttons, at its title row's start and at its end, where it has no frame of the system's
    /// (`WindowButton`), and the one last pressed, until its window does what it says.
    pub window_buttons: (Vec<WindowButton>, Vec<WindowButton>),
    window_button: Option<WindowButton>,
    /// Its window is full screen, where a Mac hides its traffic lights, so its title starts at its edge.
    pub full_screen: bool,
    /// The system's menu for text a secondary click asked for, until its window shows it.
    text_menu: Option<TextMenu>,
    /// Its window is a panel, whose background the host draws, under its page's.
    pub panel: Option<Panel>,
    /// What the window says over its page.
    pub notice: Option<Notice>,
    /// What the window draws of its own over its page, as it was last drawn (`Chrome`).
    chrome: Chrome,
    /// Whether its window is on screen, as its page is told (its host context's `visible`): a panel put away isn't.
    visible: bool,
    /// What the window keeps for itself at each of its page's edges: a computer's title bar, its traffic lights or
    /// caption buttons, and its title; a phone's status bar, notch and home indicator. The host sets it as the
    /// window opens and as it changes, and the page is told it as an MCP App is, its host context's `safeAreaInsets`,
    /// which the UI library's `safe-top`, `safe-right`, `safe-bottom` and `safe-left` lengths are.
    pub insets: Edges<f32>,
    /// The room its page has, and whether it asked for less motion, as the page was last told them with its insets
    /// (its host context's `containerDimensions` and `reducedMotion`). Its room is its window's from the start
    /// (`Windows::open`), which only the window knows: a phone's is its screen, whatever it was asked to be.
    pub(crate) room: Size<f32>,
    told: Option<(Size<f32>, bool, Edges<f32>, crate::platform::Pressure)>,
    /// When what it draws first changed since the frame drawn last, on its clock: what moves as that changed sets off
    /// then, as CSS's transitions do as a style changes, not when a frame next happens to be drawn, so a clock moved
    /// on before a frame is drawn (a paused one, stepped) finds it where it's got to.
    pub(super) changed: Option<f64>,
    asked: Option<Asked>,
    /// What has the keyboard (`Focus`), and each input's editing (`Editing`).
    focus: Focus,
    editing: Editing,
    /// Its window has the keyboard, so the caret shows.
    pub active: bool,
    /// What's done once its page isn't in use (`in_use`): a page updated under it loads again then, never under a
    /// press, a drag or what its person is typing.
    pub when_unused: Vec<Box<dyn FnOnce() + Send>>,
    /// What copy and paste use: the computer's, which the window gives it, or one of its own.
    pub clipboard: Box<dyn Clipboard>,
    /// What the device does for it: its pickers, its share sheet and its haptics.
    pub device: Box<dyn Device>,
    /// How far each scrolling node (and the root, 0) has scrolled, along and down.
    scrolled: HashMap<Id, Point<f32>>,
    /// Where the pointer is, what it's over and what it pressed, for styles' states and pointer events.
    states: States,
    pointer: Option<Point<f32>>,
    /// When, on the page's clock, what's under a still pointer is found again after what's under it scrolled: it isn't
    /// as it scrolls, as browsers find it again only once a scroll has stopped for a moment (`HOVER_AFTER_SCROLL`).
    hover_after: Option<f64>,
    /// The pointer is a finger, as a phone's is, which pointer events say (`pointerType`).
    pub touch: bool,
    /// When the device says the input being handled happened, on the page's clock (`happened`).
    happened: Option<f64>,
    /// The finger it follows, as a phone's host hands it each touch (`touch`).
    finger: Option<touch::Finger>,
    /// Its window has the rail's home line over its page, as a phone's app window has (DESIGN.md, The rail), and
    /// whether a finger's pulling it home, which takes the line with it.
    pub rail: bool,
    pub pulled: bool,
    /// The node a pointer's press went down on, which its moves and its end go to, wherever it goes.
    captured: Option<Id>,
    /// What the press drags, if it's on a node that does.
    drag: Option<drag::Drag>,
    /// A trackpad's fingers dragging what drags (`drag_wheel`): where they'd be as a finger, and once lifted, that the
    /// scrolling the system goes on with is theirs too.
    trackpad: drag::Trackpad,
    /// What was let go, and the call of the page's that tells it so, until the page has run it.
    letting_go: Option<(Id, u64)>,
    /// What was going (`Tree::leaving`) as the last frame was drawn.
    going: HashSet<Id>,
    /// What scrolls by itself or past its end, as a finger left it (`bounce`).
    coasts: HashMap<Id, bounce::Coast>,
    /// What takes the pointer, as it was last painted, in the order painted, the topmost last.
    placed: Vec<(Id, Placed)>,
    /// Whether anything moved by itself in the last frame.
    animating: bool,
    /// How many frames a second its display shows, which its page's frames come at while its clock is paused and
    /// stepped (`step`): 60 unless the inspector's `replay` says.
    hz: f64,
    again: bool,
    /// What's on its way from one value to another as a style changed.
    transitions: transition::Transitions,
    /// What's in each node that's only moving, growing or fading, as it was drawn as that started (`Layer`).
    layers: HashMap<Id, Layer>,
    /// The tree as Taffy lays it out, kept from frame to frame (`Retained`), the states its styles were last resolved
    /// in, and the frame it's on.
    layout: Retained,
    styled: States,
    frame: u64,
    /// What the last frame showed, which a frame that would show the same doesn't paint again.
    seen: Option<Seen>,
    /// The key of the shortcut that summoned it, and when it was last pressed (`summoned`).
    shortcut: Option<(String, web_time::Instant)>,
    /// Where each node listening for `layout` was last told it is.
    laid_out: HashMap<Id, Rect>,
    /// Whether each node listening for `intersection` was last told it's near the window.
    intersecting: HashMap<Id, bool>,
    /// Each picture drawn, decoded in the background.
    pictures: pictures::Pictures,
    /// Each picture its page names by an address rather than a `data:` one, as a `data:` address once its window has
    /// fetched it (`fetched`), or none while it's on its way or wasn't found; and what fetches one, the window's.
    fetched: HashMap<String, Option<String>>,
    /// Each picture asked of `fetch` that hasn't come back yet (`fetched`), which `settle` waits for.
    fetching: HashSet<String>,
    pub fetch: Option<Box<dyn FnMut(&str)>>,
    /// What a fetch brought back off the window's own turn, which it takes as its page's posts (`fetched`, as each
    /// comes): a harness's, which reads its page's files as the page asks.
    pub arrived: std::sync::Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>,
    /// Where each node was drawn in the last frame, for the inspector, and the frame's size and scale.
    rects: Rects,
    painted: (Size<f32>, f32),
    /// What the app asked its client that hasn't been answered, by its JSON-RPC id, for the inspector's `settle`.
    unanswered: HashMap<String, inspect::Asked>,
    /// The inspector's `settle`s waiting: until when, and what to tell when it's settled or that time comes.
    settling: Vec<(Instant, Box<dyn FnOnce(Value)>)>,
    /// The inspector's taps waiting for what they press (`tap_ready`).
    tapping: Vec<inspected::Tapping>,
    /// How tall its page says its content is (`ui/notifications/size-changed`), which a frame showing it is drawn as.
    content_height: Option<f32>,
    /// Something is open over its page (a sheet, a menu), as its page says (`notifications/one/over`): Back shuts that
    /// first (`back`).
    over: bool,
    /// What's open over its page is drawn over it (a sheet, a menu), not only a screen of its own, which the rail's
    /// line keeps out of the way of (`line`).
    layer: bool,
    /// The last error its JS threw, which is logged, as a browser's console has it, and the screen carries on.
    pub failed: Option<String>,
    /// Its page's batches held back a few of the window's turns each, as its host's harness says (the fake platform's
    /// seed): a page slower than its window, the window drawing and telling it what it has meanwhile.
    pub late: Option<Late>,
    /// What its page made that this host refused (a mutation it can't apply), which its window says in
    /// its place.
    pub mismatched: Option<String>,
    /// What the frame painted last cost, for the inspector's `profile` (`Cost`).
    cost: Cost,
    /// The call its JS was sent last that answered what it asked (`Thread::sent`), which it has run once it's started
    /// (`starting`).
    heard: u64,
}

/// What its frames cost since it was last asked (`Screen::cost`), as a browser's performance panel says a frame's: the
/// mutations its JS made that it applied, how long its JS thread spent running, how long laying its tree out and
/// painting it all took, each its thread's time on a CPU (`platform::on_cpu`), so a busy machine doesn't read as slow
/// work, and how many times it painted. Frames that changed nothing cost their JS all the same.
#[derive(Clone, Copy, Default)]
pub(crate) struct Cost {
    mutations: u64,
    script: Duration,
    layout: Duration,
    paint: Duration,
    painted: u32,
}

impl Screen {
    /// The screen of an app whose JS is starting on its own thread, which it draws as that posts, in the room the window
    /// it's opened in has (`Windows::open`).
    pub fn new(js: Thread, asked: Option<Asked>) -> Screen {
        let mut screen = Screen {
            tree: Tree::default(),
            js,
            started: None,
            replies: Vec::new(),
            wake: None,
            frames: HashMap::new(),
            asking: HashSet::new(),
            make_frame: None,
            nested: false,
            left: None,
            pointing: None,
            captured_frame: None,
            #[cfg(not(target_arch = "wasm32"))]
            videos: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            video_pressed: None,
            #[cfg(not(target_arch = "wasm32"))]
            video_keys: None,
            #[cfg(not(target_arch = "wasm32"))]
            video_turned: point(0.0, 0.0),
            theme: None,
            appearance: Appearance::Light,
            reduced_motion: false,
            pressure: Default::default(),
            text_size: None,
            page: 0,
            title: None,
            full_screen: false,
            text_menu: None,
            window_buttons: (Vec::new(), Vec::new()),
            window_button: None,
            panel: None,
            notice: None,
            visible: true,
            insets: Edges::default(),
            room: Size::default(),
            told: None,
            changed: None,
            asked,
            active: true,
            when_unused: Vec::new(),
            clipboard: Box::new(editing::Memory::default()),
            device: Box::new(()),
            chosen_files: Arc::default(),
            scrolled: HashMap::new(),
            drag: None,
            trackpad: drag::Trackpad::default(),
            letting_go: None,
            going: HashSet::new(),
            coasts: HashMap::new(),
            states: States::default(),
            pointer: None,
            hover_after: None,
            touch: false,
            happened: None,
            finger: None,
            rail: false,
            pulled: false,
            captured: None,
            placed: Vec::new(),
            layers: HashMap::new(),
            animating: false,
            hz: 60.0,
            again: false,
            transitions: Default::default(),
            layout: Retained::default(),
            styled: States::default(),
            frame: 0,
            seen: None,
            shortcut: None,
            laid_out: HashMap::new(),
            focus: Focus::default(),
            editing: Editing::default(),
            chrome: Chrome::default(),
            intersecting: HashMap::new(),
            pictures: Default::default(),
            fetched: HashMap::new(),
            fetching: HashSet::new(),
            fetch: None,
            arrived: Default::default(),
            rects: Rects::new(),
            painted: (Size::default(), 1.0),
            unanswered: HashMap::new(),
            settling: Vec::new(),
            tapping: Vec::new(),
            content_height: None,
            over: false,
            layer: false,
            failed: None,
            late: None,
            mismatched: None,
            cost: Cost::default(),
            heard: 0,
        };
        screen.pump();
        screen
    }

    /// A picture came from outside it, a video's frame as the web's browser played it (`platform::web::video`): drawn
    /// in the next frame, as one decoded is.
    /// What's selected of a text that isn't a field, where something is: the web mirrors it in the page, for the
    /// browser's own Copy.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn selection_text(&self) -> Option<String> {
        self.selected_text()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn picture_came(&mut self) {
        self.pictures.decoded += 1;
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The room its page has, in logical pixels (`room`).
    pub fn room(&self) -> Size<f32> {
        self.room
    }

    /// The room its page was last told it has, if it's been told one: what its page is laid out for.
    pub fn told_room(&self) -> Option<Size<f32>> {
        self.told.map(|(room, ..)| room)
    }

    /// Calls `wake` as its app's JS thread, or a frame's, posts what this hasn't applied (`pump`).
    pub fn on_post(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let calls = wake.clone();
        self.js.on_post(move || calls());
        for child in self.frames.values_mut() {
            child.screen.on_post(wake.clone());
        }
        self.pictures.wake = Some(wake.clone());
        self.wake = Some(wake);
    }

    /// Whether a picture it drew, or a frame's page did, is decoding, or is decoded and not yet drawn: a harness draws
    /// again once it's waited for them (`idle`).
    pub fn decoding(&self) -> bool {
        self.pictures.decoding() || self.frames.values().any(|child| child.screen.decoding())
    }

    /// Whether its page has drawn nothing: its tree's root holds nothing.
    pub fn drew_nothing(&self) -> bool {
        self.tree.nodes.get(&0).is_none_or(|root| root.children.is_empty())
    }

    /// Whether its app is starting: it hasn't heard back from what it asked (`waiting`), or what its JS made of the
    /// answers isn't applied yet (`editing.heard`, which comes with what it changed), so what it draws of them is on
    /// its way. Its thread having run them isn't enough: what that changed may not have reached the window yet.
    pub fn starting(&self) -> bool {
        self.waiting() || (self.editing.heard < self.heard && !self.js.ended())
    }

    /// Whether its page, and each frame's, has run all it was told (`Thread::told`), and what that changed is applied:
    /// what the person does next is then done to what the page made of what they did before, as a browser runs each
    /// input event once the page has run the one before. Its timers and frames needn't have run.
    pub fn caught_up(&self) -> bool {
        self.editing.heard >= self.js.told() && self.frames.values().all(|child| child.screen.caught_up())
    }

    /// Whether what's applied of its page, and each frame's, is laid out as it was last painted, which what the person
    /// does is hit and moved through by.
    pub fn laid_out(&self) -> bool {
        self.tree.unchanged() && self.frames.values().all(|child| child.screen.laid_out())
    }

    /// Whether its app's JS, or a frame's, has something it was sent to run yet, or hasn't started.
    fn busy(&self) -> bool {
        self.js.busy() || self.frames.values().any(|child| child.screen.busy())
    }

    /// The bytes its JS held as it was last collected, and its frames' (`Thread::memory`).
    pub fn js_memory(&self) -> usize {
        self.js.memory() + self.frames.values().map(|child| child.screen.js_memory()).sum::<usize>()
    }

    /// Why its app's module didn't run, once it's known it didn't.
    pub fn didnt_start(&self) -> Option<&str> {
        self.started.as_ref()?.as_ref().err().map(String::as_str)
    }

    /// Why its page crashed after it started (`Thread::crashed`), once it has: what it shows is no longer alive.
    pub fn crashed(&self) -> Option<String> {
        self.js.crashed()
    }

    /// Waits for its app's JS to run all it's been sent, and what the host answers at once of what that asked, applying
    /// what it posts: for a harness, which draws what a step left once it's done. → whether anything came.
    pub fn idle(&mut self) -> bool {
        let mut came = false;
        loop {
            self.js.wait();
            self.pictures.wait();
            for child in self.frames.values_mut() {
                child.screen.idle();
            }
            came |= self.pump();
            if !self.busy() {
                return came;
            }
        }
    }

    /// Whether the app hasn't started yet, or is waiting to hear back from its client about something it asked.
    pub fn waiting(&self) -> bool {
        self.started.is_none() || !self.unanswered.is_empty()
    }

    pub fn dark(&self) -> bool {
        self.theme.unwrap_or(self.appearance) == Appearance::Dark
    }

    /// Its insets as MCP Apps' host context says them (`safeAreaInsets`).
    fn safe_area(&self) -> Value {
        json!({"top": self.insets.top, "right": self.insets.right, "bottom": self.insets.bottom, "left": self.insets.left})
    }

    /// Notes that what it draws changed now, if nothing had since the frame drawn last.
    fn changed(&mut self) {
        let now = self.js.clock().now();
        self.changed.get_or_insert(now);
    }

}

/// Everything outside the tree that a frame shows: its size and scale, the conditions, each node's state, how far
/// each scroll is scrolled, each input's text, caret and selection and whether the caret's on, what the window says
/// over its page and where the pointer is on it, and how many pictures have come.
/// What has the keyboard, as a browser's focus has it: what has it now and had it before, each modal dialog shown,
/// the topmost last, with what had the keyboard as it came, which it gives the keyboard back to as it goes, as the
/// DOM's `<dialog>` does; what's taken it by its `autofocus`, while that's so; and what takes it, in the order Tab
/// goes, as it was last painted.
#[derive(Default)]
struct Focus {
    focused: Option<Id>,
    blurred: Option<Id>,
    modals: Vec<(Id, Option<Id>)>,
    autofocused: HashSet<Id>,
    /// What took the keyboard as it said `autofocus`, to be scrolled into view once it's drawn.
    reveal: Option<Id>,
    tab_order: Vec<Id>,
}

/// Each input's editing, as it's typed, ahead of the app setting it back, and the editing state its app last set,
/// which replaces it only as it changes, and only once the app has heard the input's last edit: the number of the call
/// that told it (`edits`), which its JS had run (`heard`) when it set it. One set before that answers an older edit and
/// would undo what's been typed since, as React Native's `mostRecentEventCount` keeps from doing. With where each
/// input's text was last drawn (`drawn`), for the pointer and the moves that depend on how it wraps; the input the
/// pointer's held press selects in, and the last press, to count a double or triple click; and when the caret last
/// moved, on the app's clock, from which it blinks.
#[derive(Default)]
struct Editing {
    fields: HashMap<Id, Field>,
    set: HashMap<Id, editing::Value>,
    edits: HashMap<Id, u64>,
    heard: u64,
    drawn: HashMap<Id, Drawn>,
    selecting: Option<Id>,
    /// When on the page's clock, in ms.
    last_press: Option<(f64, Point<f32>, u8)>,
    blink: f64,
    /// Each selectable text as it was last drawn, what's selected of one, and whether a press is selecting
    /// (`selection`).
    texts: HashMap<Id, selection::Shown>,
    selected: Option<selection::Selected>,
    selecting_text: bool,
    /// What a finger selected in an input, with its handles and the system's menu (`text_selection`).
    overlay: Option<text_selection::Overlay>,
}

/// What the window draws of its own over its page, as it was last drawn: where its page's top bar starts in its title
/// row, after its title (0 for a window with none); where its page asked for the system's own material, which a
/// panel's host puts under it; where its page says a press moves the window, and where in that it doesn't
/// (`appRegion`); and where its notice's action is, and whether it's pressed.
#[derive(Default)]
struct Chrome {
    title_start: u16,
    title_end: u16,
    buttons: Vec<(WindowButton, Bounds<f32>)>,
    pressing_button: Option<WindowButton>,
    glass: Vec<crate::platform::Glass>,
    drag_regions: Vec<(Id, Bounds<f32>)>,
    kept_regions: Vec<Bounds<f32>>,
    action: Option<Bounds<f32>>,
    pressing_action: bool,
}
