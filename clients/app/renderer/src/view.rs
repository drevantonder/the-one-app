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
mod paint;
mod pictures;
mod selection;
pub use frames::Make as MakeFrame;
pub(crate) mod inspected;
mod transition;

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

/// What a computer's window shows above its page, in its header bar's place: Home, with the One App's ring, which
/// brings the launcher forward and keeps the window open, a muted "/", then the app's icon and name (DESIGN.md, App
/// windows), each picture a `data:` address. Its page keeps the bar's height clear, as its host tells it.
pub struct Breadcrumb {
    pub ring: String,
    pub icon: Option<String>,
    pub name: String,
    /// How far in from the window's start it begins, in the band its top inset keeps clear, past a Mac's traffic
    /// lights; full screen, where the Mac hides them, it begins at the window's edge (`Screen::full_screen`).
    pub start: f32,
    pub home: Box<dyn FnMut()>,
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

/// The system's own menu for text in a field a secondary click was on, where it was, in points of the page, and what
/// of it can be done: as every system's fields have (`Screen::take_text_menu`), whose choice comes back as its key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMenu {
    pub at: Point<f32>,
    pub cut: bool,
    pub copy: bool,
    pub paste: bool,
    pub select_all: bool,
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

/// The typeface every screen is drawn in, as the UI library's are (core/ui/src/tokens.css), by the family name its
/// file has, which a stylesheet's `@font-face` calls "Allo".
pub const FONT: &str = "Allo Variable";

/// The typeface, from the UI library's WOFF2, as TrueType, which the text system reads.
pub fn fonts() -> anyhow::Result<Vec<Vec<u8>>> {
    let ttf = wuff::decompress_woff2(include_bytes!("../../../../core/ui/src/Allo-Variable.woff2"))
        .map_err(|error| anyhow::anyhow!("Allo didn't decode: {error:?}"))?;
    Ok(vec![ttf])
}

/// How long after its shortcut summoned a panel, or after it was last pressed again, the shortcut's key is still its:
/// longer than a key's repeat delay, so one held repeats into nothing.
/// The roles of what the DOM draws as a button, which Tab reaches (core/ui's drawing-primitives/primitives.tsx `pressed`).
const BUTTON_ROLES: [&str; 9] = ["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"];

/// The page itself, as an event's node: what a key is told to, once, for what listens to every key (`keydown()`).
const PAGE: Id = 0;
/// The window's own nodes beside its page's, as Tab and the inspector reach them: its breadcrumb's Home, its notice
/// and the notice's action (`Screen::semantics`).
pub const HOME: Id = u64::MAX - 1;
pub const NOTICE: Id = u64::MAX - 2;
pub const NOTICE_ACTION: Id = u64::MAX - 3;

/// How far in from a full-screen window's edge its breadcrumb starts, in points.
const EDGE: f32 = 12.0;
const SHORTCUT_HELD: std::time::Duration = std::time::Duration::from_millis(600);

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
    /// As Colour Mode says, or as the device is (`None`).
    pub theme: Option<Appearance>,
    /// As the device is, which the window says.
    pub appearance: Appearance,
    /// The person asked for less motion, as the device says.
    pub reduced_motion: bool,
    /// The device's own text size, which a phone has, in pixels: 1rem is it, and the `phone` tokens hold.
    pub text_size: Option<f32>,
    /// Whose own tokens it draws with, as its page's styles gave them (`tokens::load_page`).
    pub page: u32,
    /// What its window says above it, on a computer.
    pub breadcrumb: Option<Breadcrumb>,
    /// Its own window buttons, at its title row's start and at its end, where it has no frame of the system's
    /// (`WindowButton`), and the one last pressed, until its window does what it says.
    pub window_buttons: (Vec<WindowButton>, Vec<WindowButton>),
    window_button: Option<WindowButton>,
    /// Its window is full screen, where a Mac hides its traffic lights, so its breadcrumb starts at its edge.
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
    /// caption buttons, and its breadcrumb; a phone's status bar, notch and home indicator. The host sets it as the
    /// window opens and as it changes, and the page is told it as an MCP App is, its host context's `safeAreaInsets`,
    /// which the UI library's `safe-top`, `safe-right`, `safe-bottom` and `safe-left` lengths are.
    pub insets: Edges<f32>,
    /// The room its page has, and whether it asked for less motion, as the page was last told them with its insets
    /// (its host context's `containerDimensions` and `reducedMotion`).
    room: Size<f32>,
    told: Option<(Size<f32>, bool, Edges<f32>)>,
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
    /// What's done once its window isn't in use, as it loses the keyboard: a page updated under it loads again then,
    /// as a desktop app or a phone's updates, never under its person.
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
    /// The pointer is a finger, as a phone's is, which pointer events say (`pointerType`).
    pub touch: bool,
    /// Its window has the rail's home line over its page, as a phone's app window has (DESIGN.md, The rail), and
    /// whether a finger's pulling it home, which takes the line with it.
    pub rail: bool,
    pub pulled: bool,
    /// The node a pointer's press went down on, which its moves and its end go to, wherever it goes.
    captured: Option<Id>,
    /// What the press drags, if it's on a node that does.
    drag: Option<drag::Drag>,
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
    pub fetch: Option<Box<dyn FnMut(&str)>>,
    /// Where each node was drawn in the last frame, for the inspector, and the frame's size and scale.
    rects: Rects,
    painted: (Size<f32>, f32),
    /// What the app asked its client that hasn't been answered, by its JSON-RPC id, for the inspector's `settle`.
    unanswered: HashMap<String, inspect::Asked>,
    /// The inspector's `settle`s waiting: until when, and what to tell when it's settled or that time comes.
    settling: Vec<(Instant, Box<dyn FnOnce(Value)>)>,
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
    /// What its page uses that this host can't draw (`tree::Mismatch`), or that it refused, which its window says in
    /// its place.
    pub mismatched: Option<String>,
    /// What the frame painted last cost, for the inspector's `profile` (`Cost`).
    cost: Cost,
    /// The call its JS was sent last that answered what it asked (`Thread::sent`), which it has run once it's started
    /// (`starting`).
    heard: u64,
}

/// What a frame cost: the mutations its JS made that it applied, since the frame before (`applied` counts them as they
/// come), and how long laying its tree out and painting it all took, as a browser's performance panel says a frame's.
#[derive(Clone, Copy, Default)]
pub(crate) struct Cost {
    pub mutations: u64,
    applied: u64,
    pub layout: Duration,
    pub paint: Duration,
}

impl Screen {
    /// The screen of an app whose JS is starting on its own thread, which it draws as that posts, with `room` logical
    /// pixels to draw in, `insets` of them its window's own (`Screen::insets`).
    pub fn new(js: Thread, asked: Option<Asked>, room: Size<f32>, insets: Edges<f32>) -> Screen {
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
            theme: None,
            appearance: Appearance::Light,
            reduced_motion: false,
            text_size: None,
            page: 0,
            breadcrumb: None,
            full_screen: false,
            text_menu: None,
            window_buttons: (Vec::new(), Vec::new()),
            window_button: None,
            panel: None,
            notice: None,
            visible: true,
            insets,
            room,
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
            going: HashSet::new(),
            coasts: HashMap::new(),
            states: States::default(),
            pointer: None,
            touch: false,
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
            fetch: None,
            rects: Rects::new(),
            painted: (Size::default(), 1.0),
            unanswered: HashMap::new(),
            settling: Vec::new(),
            content_height: None,
            over: false,
            layer: false,
            failed: None,
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
        self.pictures.pending() > 0 || self.frames.values().any(|child| child.screen.decoding())
    }

    /// Whether its app is starting: it hasn't heard back from what it asked (`waiting`), or its JS hasn't yet run
    /// the answers, so what it draws of them is on its way.
    pub fn starting(&self) -> bool {
        self.waiting() || !self.js.ran(self.heard)
    }

    /// Whether its app's JS, or a frame's, has something it was sent to run yet, or hasn't started.
    fn busy(&self) -> bool {
        self.js.busy() || self.frames.values().any(|child| child.screen.busy())
    }

    /// Why its app's module didn't run, once it's known it didn't.
    pub fn didnt_start(&self) -> Option<&str> {
        self.started.as_ref()?.as_ref().err().map(String::as_str)
    }

    /// Waits for its app's JS to run all it's been sent, and what the host answers at once of what that asked, applying
    /// what it posts: for a harness, which draws what a step left once it's done.
    pub fn idle(&mut self) {
        loop {
            self.js.wait();
            self.pictures.wait();
            for child in self.frames.values_mut() {
                child.screen.idle();
            }
            self.pump();
            if !self.busy() {
                return;
            }
        }
    }

    /// The semantic tree, as an agent and a screen reader read it, each frame's page's in its place, after the
    /// window's own: its breadcrumb's Home and its notice, while they're drawn.
    pub fn semantics(&self) -> Value {
        let mut out = Vec::new();
        if self.chrome.home.is_some() {
            out.push(json!({"id": HOME, "role": "button", "name": "Home", "description": "Home Screen", "actions": ["click"]}));
        }
        if let Some(notice) = &self.notice {
            let action = notice.action.as_ref().filter(|_| self.chrome.action.is_some());
            let children: Vec<Value> = action.map(|(label, _)| json!({"id": NOTICE_ACTION, "role": "button", "name": label, "actions": ["click"]})).into_iter().collect();
            out.push(json!({"id": NOTICE, "role": "status", "live": "polite", "name": notice.text, "children": children}));
        }
        out.extend(self.grafted(self.tree.semantics(0)));
        json!(out)
    }

    /// Presses the window's own node `id` (`HOME`, `NOTICE_ACTION`), as a click on it does. → whether it's one.
    pub(crate) fn press_chrome(&mut self, id: Id) -> bool {
        match id {
            HOME => match self.breadcrumb.as_mut() {
                Some(crumb) => (crumb.home)(),
                None => return false,
            },
            NOTICE_ACTION => match self.notice.as_mut().and_then(|notice| notice.action.as_mut()) {
                Some((_, action)) => action(),
                None => return false,
            },
            _ => return false,
        }
        true
    }

    /// Whether the app hasn't started yet, or is waiting to hear back from its client about something it asked.
    pub fn waiting(&self) -> bool {
        self.started.is_none() || !self.unanswered.is_empty()
    }

    pub fn dark(&self) -> bool {
        self.theme.unwrap_or(self.appearance) == Appearance::Dark
    }

    /// Tells the app what the person did to node `id`, and takes what it changed.
    pub fn event(&mut self, id: Id, name: &str, value: Option<String>) {
        self.tell(id, name, json!({"value": value}));
    }

    /// Tells the app event `name` on node `id`, with what it says of it (`told`), when it happened on the clock the
    /// app's own is, as every DOM event has its `timeStamp`, and takes what it changed.
    pub(super) fn tell(&mut self, id: Id, name: &str, told: Value) {
        let mut event = json!({"id": id, "name": name, "timeStamp": self.js.clock().now()});
        if let (Some(event), Value::Object(told)) = (event.as_object_mut(), told) {
            event.extend(told);
        }
        self.js.send("event", event);
    }

    /// Tells node `id` it was clicked, at `at` in the window, or in its middle by a key or an agent, with where it's
    /// drawn (`rect`), which what it opens is placed by, as a menu is under its button.
    pub(crate) fn click(&mut self, id: Id, at: Option<Point<f32>>) {
        let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        // A picker opens the device's, as a `<select>` does a phone's browser's.
        if let Some(node) = self.tree.nodes.get(&id).filter(|node| node.kind == Kind::Picker) {
            let (options, chosen) = (node.prop("options").cloned().unwrap_or_default(), node.str("value").map(str::to_owned));
            self.device.pick(id, rect, &options, chosen.as_deref());
            return;
        }
        let at = at.unwrap_or(point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
        self.tell(id, "click", json!({"clientX": at.x, "clientY": at.y, "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}));
    }

    /// The device's picker for picker node `id` chose `value`, which it's told as a `<select>`'s `input` says it.
    pub fn picked(&mut self, id: Id, value: &str) {
        self.tell(id, "input", json!({"editing": {"text": value}}));
    }

    /// Tells the app a message from its client: an answer to what it asked (`Asked`), or a notification.
    pub fn message(&mut self, message: &Value) {
        let mut message = message.clone();
        let asked = if message.get("method").is_none() { self.unanswered.remove(&message["id"].to_string()) } else { None };
        if asked.as_ref().is_some_and(|asked| asked.method == "ui/initialize") && message["result"].is_object() {
            self.place(&mut message);
        }
        self.js.send("message", message);
        if asked.is_some() {
            self.heard = self.js.sent();
        }
    }

    /// Adds to the client's answer to `ui/initialize` what only the window knows of where the page is, whether it
    /// answered at once (`Asked`) or later, as a browser's frame's client does.
    fn place(&mut self, answer: &mut Value) {
        answer["result"]["hostContext"]["containerDimensions"] = json!({"width": self.room.width, "height": self.room.height});
        answer["result"]["hostContext"]["reducedMotion"] = json!(self.reduced_motion);
        answer["result"]["hostContext"]["safeAreaInsets"] = self.safe_area();
        answer["result"]["hostContext"]["visible"] = json!(self.visible);
        self.told = Some((self.room, self.reduced_motion, self.insets));
    }

    /// Logs what the app's JS threw, as a browser's console does, and carries on.
    fn threw(&mut self, error: String) {
        eprintln!("native app error: {error}");
        self.failed = Some(error);
    }

    /// How long until the screen has something to do by itself: an inspector's `settle` coming to its deadline, a
    /// frame of what moves, or the caret's blink. The window calls `fire` then. The app's timers are its JS thread's.
    pub fn due(&self) -> Option<Duration> {
        let now = Instant::now();
        let settle = self.settling.iter().map(|(until, _)| until.saturating_duration_since(now)).min();
        let frames = self.frames.values().filter_map(|child| child.screen.due());
        settle.into_iter().chain(self.blinks()).chain(frames).min()
    }

    /// Whether its page, or a frame's in it, asked for the display's next frame (`requestAnimationFrame`).
    pub fn framing(&self) -> bool {
        self.js.framing() || self.frames.values().any(|child| child.screen.framing())
    }

    /// The display showed a frame `at`: what asked for one runs, told when, on its clock.
    pub fn frame(&mut self, at: Instant) {
        for child in self.frames.values_mut() {
            child.screen.frame(at);
        }
        if self.js.framing() {
            let time = self.js.clock().at(at);
            self.js.frame(time);
        }
    }

    /// How long until the caret blinks on or off: as GTK's does, on two thirds of each 1.2 s and off a third, until
    /// it's been still 10 s, when it stays on. None while the app's clock is paused, or no input has the keyboard.
    fn blinks(&self) -> Option<Duration> {
        let clock = self.js.clock();
        let typing = self.active && self.focus.focused.is_some_and(|id| self.editing.fields.contains_key(&id));
        let since = clock.now() - self.editing.blink;
        if !typing || clock.paused() || since >= BLINK_STOPS {
            return None;
        }
        let into = since % BLINK;
        let next = if into < BLINK * 2.0 / 3.0 { BLINK * 2.0 / 3.0 - into } else { BLINK - into };
        Some(Duration::from_secs_f64(next / 1000.0))
    }

    /// Whether the caret shows now.
    fn caret_shows(&self) -> bool {
        let since = self.js.clock().now() - self.editing.blink;
        self.active && (since >= BLINK_STOPS || since % BLINK < BLINK * 2.0 / 3.0)
    }

    /// Applies what the app's JS has posted, and tells the `settle`s whose time has come.
    pub fn fire(&mut self) {
        for child in self.frames.values_mut() {
            child.screen.fire();
        }
        self.pump();
        self.tell_settled();
    }

    /// Applies what the app's JS has posted since it was last asked, all at once, as it's one turn's or more, and
    /// answers what it asked that the host answers at once. → whether anything came.
    pub fn pump(&mut self) -> bool {
        let posted;
        (posted, self.editing.heard) = self.js.take();
        let mut came = !posted.is_empty() || self.started.is_none() || self.pictures.take();
        let chosen = std::mem::take(&mut *self.chosen_files.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        for (id, files) in chosen {
            self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "result": {"files": files_told(&files)}}));
            came = true;
        }
        let mut cant = None;
        for child in self.frames.values_mut() {
            came |= child.screen.pump();
            // A frame's page that can't be drawn here takes the page it's in to where it can, as one of its own would.
            if let Some(why) = child.screen.mismatched.clone().or_else(|| child.screen.didnt_start().map(str::to_owned)) {
                cant = Some(format!("its frame's page, {}, can't be drawn natively: {why}", child.src));
            }
        }
        if let Some(why) = cant {
            self.mismatched.get_or_insert(why);
        }
        if self.started.is_none() {
            self.started = self.js.started();
        }
        let appearance = self.theme.unwrap_or(self.appearance);
        // What's sent it here is what answers what it asked.
        let answering = self.js.sent();
        for posted in posted {
            match posted {
                Posted::Threw(error) => self.threw(error),
                // What comes after what this host can't draw, or refused, isn't drawn: the window says why in its place,
                // rather than draw a page that's no longer what it says.
                Posted::Mutation(_) if self.mismatched.is_some() => {}
                Posted::Mutation(mutation) => match self.tree.apply([mutation]) {
                    Err(error) => self.mismatched = Some(format!("{error:#}")),
                    Ok(()) => {
                        self.cost.applied += 1;
                        self.changed();
                    }
                },
                Posted::Message(message) => {
                    let Ok(message) = serde_json::from_str::<Value>(&message) else { continue };
                    // How tall its content is, as an MCP App tells its host, which a frame's page is drawn as.
                    if message["method"] == "ui/notifications/size-changed" {
                        self.content_height = message["params"]["height"].as_f64().map(|height| height as f32);
                    }
                    if message["method"] == "notifications/one/over" {
                        self.over = message["params"]["over"] == true;
                        self.layer = message["params"]["layer"] == true;
                    }
                    let Some(asked) = self.asked.as_mut() else { continue };
                    let (id, method, tool) = (message.get("id").map(Value::to_string), message["method"].as_str().map(str::to_owned), message["params"]["name"].as_str().map(str::to_owned));
                    // What it shares goes to the device's share sheet; what it copies, or shares where there's none
                    // (`done`: "copied"), on the clipboard.
                    if let (Some(id), Some(asked @ ("one/copy" | "one/share")), Some(text)) = (message.get("id"), method.as_deref(), message["params"]["text"].as_str()) {
                        let result = if asked == "one/share" && self.device.share(text) {
                            json!({"done": "shared"})
                        } else {
                            self.clipboard.write(false, text);
                            if asked == "one/share" { json!({"done": "copied"}) } else { json!({}) }
                        };
                        self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "result": result}));
                        continue;
                    }
                    // The device's own picker of files, whose choice comes back as the page's next turn (`pump`); a device
                    // without one says so.
                    if let (Some(id), Some("one/files")) = (message.get("id"), method.as_deref()) {
                        let accept: Vec<String> = message["params"]["accept"].as_array().into_iter().flatten().filter_map(|kind| kind.as_str().map(str::to_owned)).collect();
                        let (chosen, wake, asked) = (self.chosen_files.clone(), self.wake.clone(), id.clone());
                        let done = Box::new(move |files: Vec<std::path::PathBuf>| {
                            chosen.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((asked, files));
                            if let Some(wake) = wake {
                                wake();
                            }
                        });
                        if !self.device.open_files(&accept, message["params"]["multiple"] == true, done) {
                            self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "This device has no file picker"}}));
                        }
                        continue;
                    }
                    // A haptic plays where the device has them, and a device without them says so, as a client does.
                    if let (Some(id), Some("one/haptic")) = (message.get("id"), method.as_deref()) {
                        let kind = message["params"]["kind"].as_str().unwrap_or_default();
                        let answer = if self.device.haptic(kind) {
                            json!({"jsonrpc": "2.0", "id": id, "result": {}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "This device plays no haptics"}})
                        };
                        self.js.send("message", answer);
                        continue;
                    }
                    match asked(message, appearance) {
                        // What only the window knows of where the page is, which it adds to its client's answer.
                        Some(mut answer) if method.as_deref() == Some("ui/initialize") && answer["result"].is_object() => {
                            self.place(&mut answer);
                            self.js.send("message", answer);
                        }
                        Some(answer) => self.js.send("message", answer),
                        None => {
                            if let (Some(id), Some(method)) = (id, method) {
                                self.unanswered.insert(id, inspect::Asked { method, tool, since: Instant::now() });
                            }
                        }
                    }
                }
            }
        }
        if self.js.sent() > answering {
            self.heard = self.js.sent();
        }
        if came {
            // What says `autofocus` takes the keyboard as it says so, as an active menu item does.
            let wants: Vec<Id> = self.tree.autofocused().collect();
            self.focus.autofocused.retain(|id| wants.contains(id));
            if let Some(id) = wants.into_iter().filter(|id| !self.focus.autofocused.contains(id)).max() {
                self.focus.autofocused.insert(id);
                self.focus(Some(id));
            }
            self.sync_fields();
            self.sync_frames();
        }
        // The inspector's answers wait for what they did to have run.
        if !self.replies.is_empty() && !self.busy() {
            for (answer, reply) in std::mem::take(&mut self.replies) {
                reply(answer);
            }
        }
        self.tell_settled();
        came
    }

    /// The conditions its tokens' values vary by, as its window is now.
    fn conditions(&self) -> Conditions {
        let insets = [self.insets.top, self.insets.right, self.insets.bottom, self.insets.left].map(|inset| inset.round().max(0.0) as u16);
        let title_bar = [self.chrome.title_start, if self.chrome.title_end > 0 { self.chrome.title_end } else if self.chrome.title_start > 0 { 16 } else { 0 }];
        let (phone, rem) = match self.text_size {
            Some(size) => (true, (size * 100.0).round() as u16),
            None => (false, Conditions::default().rem),
        };
        Conditions { dark: self.dark(), touch: self.touch, phone, reduced_motion: self.reduced_motion, insets, title_bar, page: self.page, rem }
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

    /// Its GPU was lost, and what it drew with is made again: what it kept in the old atlas (its pictures, and the
    /// layers drawn from them) goes, and it's all drawn afresh, its frames' pages too.
    pub fn gpu_lost(&mut self) {
        self.pictures.forget();
        self.layers.clear();
        self.seen = None;
        for child in self.frames.values_mut() {
            child.screen.gpu_lost();
        }
    }

    /// Where its page asked for the system's own material, as it was last drawn, which a panel's host puts under it.
    pub fn glass(&self) -> &[crate::platform::Glass] {
        &self.chrome.glass
    }

    /// Paints the screen into `scene`, a scene of its own, all of it, as an offscreen picture of it is: `paint` keeps the
    /// scene a frame that changes nothing drew, which a new scene doesn't have.
    pub fn paint_afresh(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
        self.seen = None;
        self.paint(size, scale, text, atlas, scene);
    }

    /// Paints the screen into `scene`, `size` logical pixels at `scale` device pixels each, as a frame's passes, in
    /// order, as GPUI's and Flutter's frames run theirs: the page told its room; each frame's page painted; whether
    /// anything it shows changed, and what's styled again and what's drawn from a layer (`plan`); the tree styled, laid
    /// out and painted (`draw`); what that drew kept, for the pointer, the keyboard and the window (`keep`); and then the
    /// keyboard where a dialog came or went (`settle_focus`), and what's told of where things landed (`tell_placed`).
    pub fn paint(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
        self.tell_room(size);
        self.paint_frames(scale, text, atlas);
        let started = Instant::now();
        // A frame that changes nothing costs nothing: the scene drawn last is kept.
        let Some(plan) = self.plan(size, scale) else {
            (self.cost.mutations, self.cost.layout, self.cost.paint) = (0, Duration::ZERO, Duration::ZERO);
            return;
        };
        scene.clear();
        let drawn = self.draw(&plan, text, atlas, scene);
        let modal = drawn.modal;
        let intersections = self.keep(drawn, &plan);
        self.painted = (plan.window.size, scale);
        self.pictures.drawn();
        self.tell_settled();
        scene.finish();
        self.settle_focus(modal);
        self.tell_placed(intersections);
        self.cost.paint = started.elapsed();
        self.cost.mutations = std::mem::take(&mut self.cost.applied);
    }

    /// The page is told when its room, its motion or its insets change, as a frame's host tells an MCP App.
    fn tell_room(&mut self, size: Size<Pixels>) {
        self.room = Size { width: size.width.0, height: size.height.0 };
        if self.told.is_some_and(|told| told != (self.room, self.reduced_motion, self.insets)) {
            self.told = Some((self.room, self.reduced_motion, self.insets));
            let context = json!({
                "containerDimensions": {"width": self.room.width, "height": self.room.height},
                "reducedMotion": self.reduced_motion,
                "safeAreaInsets": self.safe_area(),
            });
            self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": context}));
        }
    }

    /// Each frame's page first, in the room its frame had, as this one looks, which this frame splices in.
    fn paint_frames(&mut self, scale: f32, text: &mut TextSystem, atlas: &Atlas) {
        for (id, child) in &mut self.frames {
            let screen = &mut child.screen;
            (screen.theme, screen.appearance, screen.reduced_motion, screen.touch) = (self.theme, self.appearance, self.reduced_motion, self.touch);
            screen.active = self.active && self.focus.focused == Some(*id);
            // A page that says it's another height has its frame laid out again.
            if child.height != screen.content_height {
                child.height = screen.content_height;
                self.layout.conditions = None;
            }
            if child.rect.size.width > 0.0 && child.rect.size.height > 0.0 {
                screen.paint(Size { width: Pixels(child.rect.size.width), height: Pixels(child.rect.size.height) }, scale, text, atlas, &mut child.scene);
                self.again |= screen.take_again();
            }
        }
    }

    /// What this frame draws: nothing, where nothing it shows has changed, which keeps the scene drawn last and costs
    /// nothing; or what's styled again (what changed, what's moving, each input, as its words are its field's, and
    /// what's changed state, with what's around it, so the rest is kept as it was laid out), and what's drawn from a
    /// layer, moved, rather than painted again.
    fn plan(&mut self, size: Size<Pixels>, scale: f32) -> Option<Plan> {
        let conditions = self.conditions();
        let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
        self.pictures.take();
        self.coast();
        self.states.focused = self.focus.focused;
        self.states.touch = self.touch;
        let now = self.js.clock().now();
        let caret = self.caret_shows();
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let mut fields: Vec<_> = self.editing.fields.iter().map(|(id, field)| (*id, field.display(), field.value(), field.placed(), field.upstream())).collect();
        fields.sort_by_key(|(id, ..)| *id);
        let seen = Seen {
            size: (window.size, scale),
            conditions,
            states: self.states.clone(),
            scrolled: self.scrolled.clone(),
            stretched: self.stretched().into_iter().collect(),
            fields,
            caret,
            hovering: (self.chrome.home.as_ref().is_some_and(hovering), self.chrome.action.as_ref().is_some_and(hovering)),
            notice: self.notice.as_ref().map(|notice| (notice.text.clone(), notice.action.as_ref().map(|(label, _)| label.clone()))),
            breadcrumb: self.breadcrumb.as_ref().map(|crumb| (crumb.name.clone(), crumb.start)),
            panel: self.panel.map(|panel| (panel.corner, panel.fill)),
            line: self.line(),
            fetched: self.fetched.values().filter(|fetched| fetched.is_some()).count(),
            decoded: self.pictures.decoded,
            frames: self.frames.iter().map(|(id, child)| (*id, child.screen.frame)).collect(),
        };
        // What changed since (`changed`), as a node held by a finger that's moved, is drawn, though nothing else did.
        if !self.animating && self.changed.is_none() && self.tree.unchanged() && self.seen.as_ref() == Some(&seen) && self.stale_lists().is_empty() {
            return None;
        }
        // What's kept of what only moves, grows or fades is drawn again where what it draws from changed: anything
        // fetched, decoded, scrolled or in another frame's page, or the window's conditions.
        if self.seen.as_ref().is_none_or(|was| (was.size, was.conditions, &was.scrolled, &was.stretched, was.fetched, was.decoded, &was.frames) != (seen.size, seen.conditions, &seen.scrolled, &seen.stretched, seen.fetched, seen.decoded, &seen.frames)) {
            self.layers.clear();
        }
        self.seen = Some(seen);
        // What's styled again: what changed, what's moving, each input (its words are its field's), and what's
        // changed state, with what's around it, so the rest is kept as it was laid out.
        let everything = self.layout.conditions != Some(conditions);
        self.layout.conditions = Some(conditions);
        let mut changed = self.tree.take_changed();
        let moving = self.transitions.moving();
        let mut below = HashSet::new();
        let states = self.states.changed(&self.styled);
        self.styled = self.states.clone();
        // A text inside a text is drawn as a run of the one it's in, which is styled again as its state changes.
        changed.extend(states.iter().filter_map(|id| self.tree.outer_text(*id)));
        // A list whose rows' heights changed holds their room again (`list`).
        let stale = self.stale_lists();
        for id in changed.iter().chain(&moving).chain(&states).copied().chain(self.tree.inputs()).chain(stale) {
            let mut at = Some(id);
            while let Some(id) = at.filter(|id| below.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What's moving, growing or fading and nothing else, with nothing in it changing, moving otherwise or holding
        // the keyboard, is drawn from its layer, moved; anything else in it that changes draws it all again.
        let whole = self.transitions.moving_whole();
        let mut touched = HashSet::new();
        let focused = self.states.focused.filter(|id| self.editing.fields.contains_key(id));
        for id in changed.iter().chain(&states).chain(moving.iter().filter(|id| !whole.contains(id))).chain(&focused).copied() {
            let mut at = Some(id);
            while let Some(id) = at.filter(|id| touched.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What's in a layer and moving as a layer of its own is drawn into it each frame, where it's moved to, as a
        // compositor draws a sublayer; anything else moving in it draws it again.
        let drawing: Vec<Id> = moving.iter().filter(|id| !whole.contains(id) || touched.contains(id)).copied().collect();
        for id in &drawing {
            let mut at = self.tree.parent(*id);
            while let Some(id) = at.filter(|id| touched.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        // What says it's about to move (`will-change`) is one as it holds still too: drawn ahead, it only moves then.
        let layered: HashSet<Id> = whole.iter().chain(&self.layout.will_change).filter(|id| !touched.contains(id)).copied().collect();
        self.layers.retain(|id, _| layered.contains(id));
        // It's drawn ahead in a frame where nothing moves, as a browser draws in idle time, not in one that's busy.
        let still = moving.is_empty();
        self.frame += 1;
        Some(Plan { conditions, window, scale, now, caret, everything, changed, moving, below, layered, still })
    }

    /// Styles, lays out and paints the tree into `scene`, as `plan` says, and the window's own breadcrumb and notice
    /// over it. → what the painter found as it drew.
    fn draw(&mut self, plan: &Plan, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) -> Drawing {
        let Plan { conditions, window, scale, now, caret, everything, ref changed, ref moving, ref below, ref layered, still } = *plan;
        let line = self.line();
        let stretched = self.stretched();
        let mut painter = Painter {
            tree: &self.tree,
            layout: &mut self.layout,
            changed,
            moving,
            below,
            everything,
            frame: self.frame,
            gone: Vec::new(),
            text,
            atlas,
            scene,
            scale,
            conditions,
            states: &self.states,
            placed: Vec::new(),
            tab_order: Vec::new(),
            scrolled: &mut self.scrolled,
            stretched,
            fields: &self.editing.fields,
            drawn: &mut self.editing.drawn,
            texts: &mut self.editing.texts,
            selected: self.editing.selected.map(|selected| (selected.id, selected.range())),
            caret,
            pictures: &mut self.pictures,
            fetched: &self.fetched,
            wanted: Vec::new(),
            rects: Rects::new(),
            time: now,
            since: self.changed.take().unwrap_or(now).min(now),
            transitions: &mut self.transitions,
            animating: false,
            intersections: Vec::new(),
            near: Bounds { origin: point(-window.size.width / 2.0, -window.size.height / 2.0), size: Size { width: window.size.width * 2.0, height: window.size.height * 2.0 } },
            window,
            modal: None,
            turn: TransformationMatrix::unit(),
            glass: self.panel.map(|_| Vec::new()),
            fill: self.panel.filter(|panel| panel.fill).map(|_| colour_token("card", conditions)),
            drag_regions: Vec::new(),
            kept_regions: Vec::new(),
            frames: &self.frames,
            framed: Vec::new(),
            layers: &mut self.layers,
            layered,
            still,
            recording: None,
            parts: Vec::new(),
            unsettled: None,
        };
        // The window, as a page's body: its root's background and text, scrolling what's taller than it.
        let (body, background) = Text::root(conditions);
        // A panel has no background of its own: what its page draws is over the system's glass, or the host's fill.
        match self.panel {
            None => painter.quad(window, window, background.unwrap_or_default(), Edges::default(), Hsla::default(), Corners::default(), false, 1.0),
            Some(panel) if panel.whole => {
                if let Some(glass) = painter.glass.as_mut() {
                    glass.push(crate::platform::Glass { bounds: window, radius: panel.corner, opacity: 1.0 });
                }
                if let Some(fill) = painter.fill {
                    painter.quad(window, window, fill, Edges::default(), Hsla::default(), Corners::all(panel.corner), false, 1.0);
                }
            }
            Some(_) => {}
        }
        let root = painter.build(0, body, true);
        // What's no longer in the tree goes, unless it moved elsewhere in it.
        for id in std::mem::take(&mut painter.gone) {
            painter.layout.forget(id, painter.frame, painter.transitions);
        }
        painter.layout.place_fixed(painter.tree);
        let available = taffy::geometry::Size { width: AvailableSpace::Definite(window.size.width), height: AvailableSpace::Definite(window.size.height) };
        let Painter { layout, text, .. } = &mut painter;
        let laying_out = Instant::now();
        let measured = layout.taffy.compute_layout_with_measure(root, available, |input, _, leaf, style| {
            // A node with nothing under it is its style's size, or its padding and border around what it measures (a
            // text's words, an input's lines), as CSS has them; `space` is the room for that content, inside them.
            taffy::compute_leaf_layout(input, style, |_, _| 0.0, |_, space| {
                let Some(Leaf { measure: Some(measure), .. }) = leaf else {
                    return taffy::geometry::Size::ZERO;
                };
                let width = match space.width {
                    AvailableSpace::Definite(width) => Some(width),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                };
                let size = match measure {
                    Measure::Text { words, runs, text: style, lines } => text.paragraph(words, &style.style, runs, width.filter(|_| style.wrap), *lines).size(),
                    Measure::Input { words, text: style, multiline } => {
                        let paragraph = text.paragraph(words, &style.style, &[], multiline.then_some(width).flatten(), None);
                        Size { width: if *multiline { paragraph.size().width } else { 0.0 }, height: paragraph.size().height }
                    }
                    Measure::Frame { height } => Size { width: width.unwrap_or(300.0), height: height.unwrap_or(150.0) },
                };
                taffy::geometry::Size { width: size.width.ceil(), height: size.height }
            })
        });
        self.cost.layout = laying_out.elapsed();
        // A list's rows laid out as other than it held room for are held for as they are in the next frame.
        if painter.layout.measure_lists(painter.tree) {
            self.again = true;
        }
        match measured {
            Err(error) => log::error!("its layout failed: {error}"),
            Ok(()) => {
                painter.laid_out();
                painter.paint(root, point(0.0, 0.0), Transform::NONE, window, 1.0, None, true)
            }
        }
        // The window's breadcrumb, over its page.
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let edge = self.full_screen.then_some(EDGE);
        // What's under the pointer, or has the keyboard by Tab, is lit.
        let keyed = |id: Id| self.states.focus_visible && self.focus.focused == Some(id);
        // Its own window buttons, as a header bar's: at the title row's start, before the breadcrumb, and at its end,
        // which the page's top bar stops short of.
        const BUTTON: f32 = 24.0;
        const EDGE_GAP: f32 = 12.0;
        let (starts, ends) = &self.window_buttons;
        let mut buttons = Vec::new();
        for (at, kind) in starts.iter().enumerate() {
            buttons.push((*kind, Bounds { origin: point(EDGE_GAP + at as f32 * (BUTTON + 8.0), (self.insets.top - BUTTON) / 2.0), size: Size { width: BUTTON, height: BUTTON } }));
        }
        for (at, kind) in ends.iter().rev().enumerate() {
            buttons.push((*kind, Bounds { origin: point(window.size.width - EDGE_GAP - BUTTON - at as f32 * (BUTTON + 8.0), (self.insets.top - BUTTON) / 2.0), size: Size { width: BUTTON, height: BUTTON } }));
        }
        for (kind, bounds) in &buttons {
            painter.window_button(*kind, *bounds, hovering(bounds) || self.chrome.pressing_button == Some(*kind));
        }
        self.chrome.buttons = buttons;
        let after_buttons = (!starts.is_empty()).then(|| EDGE_GAP + starts.len() as f32 * (BUTTON + 8.0));
        let crumb = self.breadcrumb.as_ref().map(|crumb| painter.breadcrumb(crumb, edge.or(after_buttons).unwrap_or(crumb.start), self.insets.top, self.chrome.home.as_ref().is_some_and(hovering) || keyed(HOME)));
        let home = crumb.map(|(home, _)| home);
        // The page's top bar starts after it, and ends before the buttons at the end, which it's laid out again for
        // once either moves.
        let start = crumb.map_or(0, |(_, end)| (end + 12.0).ceil() as u16);
        let end = if ends.is_empty() { 0 } else { (EDGE_GAP + ends.len() as f32 * (BUTTON + 8.0) + 4.0).ceil() as u16 };
        if (start, end) != (self.chrome.title_start, self.chrome.title_end) {
            (self.chrome.title_start, self.chrome.title_end) = (start, end);
            self.again = true;
        }
        self.chrome.action = self.notice.as_ref().and_then(|notice| painter.notice(notice, self.insets.top, self.chrome.action.as_ref().is_some_and(hovering) || keyed(NOTICE_ACTION)));
        // The rail's line, over everything: dark over a light page and light over a dark one, as iOS's home
        // indicator is, the page's own label colour.
        if let Some(line) = line {
            painter.quad(line, window, colour_token("label", conditions), Edges::default(), Hsla::default(), Corners::all(line.size.width / 2.0), false, 1.0);
        }
        self.chrome.glass = painter.glass.take().unwrap_or_default();
        self.chrome.drag_regions = std::mem::take(&mut painter.drag_regions);
        self.chrome.kept_regions = std::mem::take(&mut painter.kept_regions);
        Drawing {
            home,
            placed: painter.placed,
            tab_order: painter.tab_order,
            rects: painter.rects,
            animating: painter.animating,
            modal: painter.modal,
            wanted: painter.wanted,
            intersections: painter.intersections,
            framed: painter.framed,
        }
    }

    /// Keeps what the frame drew, for the pointer, the keyboard, the inspector and the window: where each frame's page
    /// is, the pictures to fetch, what's under the pointer and Tab's order. → where each node listening for
    /// `intersection` is near the window.
    fn keep(&mut self, drawn: Drawing, plan: &Plan) -> Vec<(Id, bool)> {
        let Drawing { home, placed, mut tab_order, rects, animating, modal, wanted, intersections, framed } = drawn;
        // Each frame's page is drawn in the room it had now from the next frame, if that's changed.
        for (id, rect) in framed {
            if let Some(child) = self.frames.get_mut(&id) {
                self.again |= child.rect.size != rect.size;
                child.rect = rect;
            }
        }
        // What it wanted to draw that's by an address is fetched by its window, once.
        for src in wanted {
            if !self.fetched.contains_key(&src) {
                self.fetched.insert(src.clone(), None);
                if let Some(fetch) = self.fetch.as_mut() {
                    fetch(&src);
                }
            }
        }
        self.chrome.home = home;
        let moving = self.transitions.finish(plan.now);
        // What's going is gone once nothing in it moves, having been drawn going.
        let still = self.transitions.moving();
        let gone: Vec<Id> = self.tree.going().filter(|id| self.going.contains(id) && !still.iter().any(|moving| self.within(*moving, *id))).collect();
        for id in gone {
            self.tree.left(id);
            self.again = true;
        }
        self.going = self.tree.going().collect();
        self.placed = placed;
        // A modal dialog holds the keyboard: Tab goes only to what's in it, and it has the keyboard as it appears.
        // What's inert, or in what is, Tab passes over.
        if let Some(modal) = modal {
            tab_order.retain(|id| self.within(*id, modal));
        }
        tab_order.retain(|id| !self.tree.inert(*id));
        // The window's own Home and notice action come first, as they're first in it, unless a modal holds the keyboard.
        if modal.is_none() {
            let chrome = [(HOME, self.chrome.home.is_some()), (NOTICE_ACTION, self.chrome.action.is_some())];
            tab_order.splice(0..0, chrome.into_iter().filter(|(_, drawn)| *drawn).map(|(id, _)| id));
        }
        self.focus.tab_order = tab_order;
        self.rects = rects;
        self.animating = animating || moving || self.coasting();
        // What's about to move and wasn't drawn ahead, as something was moving, is in a frame of its own once it stops.
        self.again |= !plan.still && self.layout.will_change.iter().any(|id| plan.layered.contains(id) && !self.layers.get(id).is_some_and(|layer| layer.recorded));
        intersections
    }

    /// A modal that's gone gives the keyboard back to what had it as it came, if the keyboard was in it or is nowhere,
    /// and that's still there; one that came takes it.
    fn settle_focus(&mut self, modal: Option<Id>) {
        while let Some(&(gone, was)) = self.focus.modals.last() {
            if self.attached(gone) && self.tree.nodes.get(&gone).is_some_and(|node| node.str("aria-modal") == Some("true")) {
                break;
            }
            self.focus.modals.pop();
            if self.focus.focused.is_none_or(|focused| !self.attached(focused) || self.within(focused, gone)) {
                self.focus(was.filter(|was| self.attached(*was)));
                self.again = true;
            }
        }
        if let Some(modal) = modal {
            // What had the keyboard as it came: what has it now, or, where what's in it took it as it came
            // (`autofocus`), what had it before that.
            if !self.focus.modals.iter().any(|(shown, _)| *shown == modal) {
                let was = [self.focus.focused, self.focus.blurred].into_iter().flatten().find(|had| !self.within(*had, modal));
                self.focus.modals.push((modal, was));
            }
            if !self.focus.focused.is_some_and(|focused| self.within(focused, modal)) {
                self.focus(Some(modal));
                self.again = true;
            }
        }
    }

    /// What's told of where things landed: what a still pointer is over now, as a browser finds again after layout;
    /// where each node listening for `layout` is; and whether each listening for `intersection` is near the window.
    fn tell_placed(&mut self, intersections: Vec<(Id, bool)>) {
        self.ask_lists(self.painted.0.height);
        // What moved under a pointer that's still is what it's over now, as a browser finds again after layout.
        if let Some(at) = self.pointer.filter(|_| !self.touch) {
            let before = self.states.hovered.clone();
            self.hover(at);
            self.again |= self.states.hovered != before;
        }
        // Where each node that listens for `layout` is, as its layout settles and whenever its size changes, and again
        // as it shows after it wasn't drawn, as a browser's ResizeObserver tells an element shown after `display: none`
        // (a menu's items, kept unseen while it's closed, measured as it opens).
        let rects = &self.rects;
        self.laid_out.retain(|id, _| rects.get(id).is_some());
        let listening: Vec<(Id, Rect)> = self
            .tree
            .laid_out()
            .filter_map(|id| Some((id, *self.rects.get(&id)?)))
            .filter(|(id, rect)| self.laid_out.get(id).is_none_or(|was| (was.width, was.height) != (rect.width, rect.height)))
            .collect();
        for (id, rect) in listening {
            self.laid_out.insert(id, rect);
            self.tell(id, "layout", json!({"rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}));
            self.again = true;
        }
        // Whether each node that listens for `intersection` is near the window, as it's first laid out and whenever
        // that changes.
        self.intersecting.retain(|id, _| self.tree.nodes.contains_key(id));
        for (id, near) in intersections {
            if self.intersecting.insert(id, near) != Some(near) {
                self.tell(id, "intersection", json!({"isIntersecting": near}));
                self.again = true;
            }
        }
    }

    /// Whether what it did as it painted (a dialog taking the keyboard) changed it, so it's painted again.
    pub fn take_again(&mut self) -> bool {
        std::mem::take(&mut self.again) || self.animating
    }

    fn placed(&self, id: Id) -> Option<&Placed> {
        self.placed.iter().rev().find(|(placed, _)| *placed == id).map(|(_, placed)| placed)
    }

    /// The topmost node that takes the pointer at `at`, and what it's in, the innermost first.
    fn under(&self, at: Point<f32>) -> Vec<Id> {
        let Some(top) = self.placed.iter().rev().find(|(_, placed)| placed.pointer && placed.shown.contains(&at)).map(|(id, _)| *id) else { return Vec::new() };
        let mut chain = vec![top];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// What a press on `chain`, from the innermost out, reaches: the first in it that takes one.
    fn pressed_in(&self, chain: &[Id]) -> Option<Id> {
        chain.iter().copied().find(|id| self.placed(*id).is_some_and(|placed| placed.pressable))
    }

    /// Gives the keyboard to what a click on `chain`, from the innermost out, gives it to, as the DOM does: the first
    /// in it that takes it, or nothing, but where what's clicked keeps it where it is (`Placed::focus`).
    pub(crate) fn click_focus(&mut self, chain: &[Id]) {
        let first = chain.iter().find_map(|id| self.placed(*id).map(|placed| (*id, placed.focus)).filter(|(_, focus)| *focus != Clicked::Passes));
        match first {
            Some((_, Clicked::Keeps)) => {}
            // A click on what takes no keyboard leaves it with what has it by its `autofocus`, as a native app's field
            // keeps it while what's around it is clicked, and a launcher's search while its card is.
            None if self.focus.focused.is_some_and(|id| self.tree.nodes.get(&id).is_some_and(|node| node.is("autofocus"))) => {}
            first => self.focus(first.map(|(id, _)| id)),
        }
    }

    /// The pointer's shape at `at`: what its style says, or a hand over what's pressed and a beam over an input.
    pub fn cursor(&self, at: Point<f32>) -> Cursor {
        if let Some(cursor) = self.frame_cursor(at) {
            return cursor;
        }
        if self.chrome.home.is_some_and(|home| home.contains(&at)) {
            return Cursor::Arrow;
        }
        if self.chrome.action.is_some_and(|action| action.contains(&at)) {
            return Cursor::Pointer;
        }
        for id in self.under(at) {
            let Some(placed) = self.placed(id) else { continue };
            if let Some(cursor) = placed.cursor {
                return cursor;
            }
            if placed.input {
                return Cursor::Text;
            }
            if placed.pressable {
                return Cursor::Pointer;
            }
        }
        Cursor::Arrow
    }

    /// A pointer event to each node in `chain` that listens for it, the innermost first, as the DOM's bubble.
    fn pointer_event(&mut self, chain: &[Id], name: &str, at: Point<f32>, button: u8, wheel: Option<(Point<f32>, Option<Phase>)>) {
        for &id in chain {
            if !self.tree.nodes.get(&id).is_some_and(|node| node.listens(name)) {
                continue;
            }
            let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
            let mut told = json!({
                "clientX": at.x, "clientY": at.y, "pointerId": 1, "pointerType": if self.touch { "touch" } else { "mouse" }, "button": button,
                // When, in milliseconds by the page's clock, as `performance.now()` says it: what a swipe's speed is
                // worked out by.
                "timeStamp": self.js.clock().now(),
                "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            });
            if let Some((delta, phase)) = wheel {
                told["deltaX"] = json!(delta.x);
                told["deltaY"] = json!(delta.y);
                if let Some(phase) = phase {
                    told["momentum"] = json!(phase.momentum);
                    if let Some(fingers) = phase.fingers {
                        told["phase"] = json!(fingers);
                    }
                }
            }
            self.tell(id, name, told);
        }
    }

    /// What the pointer at `at` is over is hovered, what it's left and what it's come into told so, as the DOM's
    /// `pointerleave` and `pointerenter`. → what it's over, the innermost first.
    fn hover(&mut self, at: Point<f32>) -> Vec<Id> {
        let chain = self.under(at);
        let mut hovered = chain.clone();
        hovered.reverse();
        if hovered == self.states.hovered {
            return chain;
        }
        let left: Vec<Id> = self.states.hovered.iter().copied().filter(|id| !hovered.contains(id)).collect();
        let entered: Vec<Id> = hovered.iter().copied().filter(|id| !self.states.hovered.contains(id)).collect();
        self.states.hovered = hovered;
        self.changed();
        for id in left.into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, None);
        }
        for id in entered {
            self.pointer_event(&[id], "pointerenter", at, 0, None);
        }
        chain
    }

    /// The pointer moved to `at`: what it's over is hovered, told as it comes and goes, and a press it holds is told
    /// it moved.
    pub fn pointer_moved(&mut self, at: Point<f32>) {
        self.frame_moved(at);
        self.pointer = Some(at);
        let chain = self.hover(at);
        let target = match self.captured {
            Some(captured) => self.chain_from(captured),
            None => chain,
        };
        self.drag_moved(at);
        self.pointer_event(&target, "pointermove", at, 0, None);
        self.select_moved(at);
        // A press held in an input selects as it's dragged.
        if let Some(id) = self.editing.selecting
            && let (Some(field), Some(drawn)) = (self.editing.fields.get_mut(&id), self.editing.drawn.get(&id))
        {
            let before = field.value();
            let (index, upstream) = drawn.hit(at);
            field.drag(index, upstream);
            self.edited(id, before);
        }
    }

    /// The pointer left the window: nothing's hovered.
    pub fn pointer_left(&mut self) {
        self.frame_left();
        let at = self.pointer.take().unwrap_or(point(0.0, 0.0));
        for id in std::mem::take(&mut self.states.hovered).into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, None);
        }
    }

    /// `id` and what it's in, the innermost first, as last painted.
    /// Whether node `id` is `outer` or in it, as the page's tree has them, whatever's laid out between.
    fn within(&self, id: Id, outer: Id) -> bool {
        let mut at = Some(id);
        while let Some(node) = at {
            if node == outer {
                return true;
            }
            at = self.tree.parent(node).filter(|parent| *parent != 0);
        }
        false
    }

    /// Whether node `id` is in the page's tree: under its root, not taken out of it, though its JS may still hold it.
    fn attached(&self, id: Id) -> bool {
        let mut at = Some(id);
        while let Some(node) = at.filter(|node| !self.tree.leaving(*node)) {
            if node == 0 {
                return true;
            }
            at = self.tree.parent(node);
        }
        false
    }

    fn chain_from(&self, id: Id) -> Vec<Id> {
        let mut chain = vec![id];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// A button went down at `at` (0 the main one, 1 the middle, 2 the secondary), with Shift held or not: what's
    /// under it is told, and with the main one what's pressed there is pressed and takes the keyboard if it can, an
    /// input putting its caret there (a double click selects a word, a triple its line, and Shift extends the
    /// selection); the middle one in an input pastes the primary selection there, as on Linux; the secondary is a
    /// `contextmenu` too.
    pub fn pointer_down(&mut self, at: Point<f32>, button: u8, shift: bool) {
        // The window's own buttons are the window's, not its page's.
        if let Some((kind, _)) = self.chrome.buttons.iter().find(|(_, bounds)| bounds.contains(&at)) {
            self.chrome.pressing_button = (button == 0).then_some(*kind);
            self.changed();
            return;
        }
        // The breadcrumb's Home is the window's, not its page's, and so is its notice's action.
        if self.chrome.home.is_some_and(|home| home.contains(&at)) {
            self.chrome.pressing_home = button == 0;
            return;
        }
        if self.chrome.action.is_some_and(|action| action.contains(&at)) {
            self.chrome.pressing_action = button == 0;
            return;
        }
        if self.frame_down(at, button, shift) {
            return;
        }
        let chain = self.under(at);
        self.captured = chain.first().copied();
        if button == 0 {
            self.drag_down(&chain, at);
        }
        self.pointer_event(&chain, "pointerdown", at, button, None);
        if button == 2 {
            // On a field, it has the keyboard and the system's menu for text, its caret where the click was unless
            // that's in what's selected, as every system's fields have.
            if let Some(input) = self.pressed_in(&chain).filter(|id| self.editing.fields.contains_key(id)) {
                self.focus(Some(input));
                let (index, _) = self.editing.drawn.get(&input).map_or((usize::MAX, false), |drawn| drawn.hit(at));
                let field = &self.editing.fields[&input];
                let (start, end) = field.range();
                if !(start..end).contains(&index) {
                    self.press_input(input, at, 1, false);
                }
                let field = &self.editing.fields[&input];
                let selected = !field.selected().is_empty();
                self.text_menu = Some(TextMenu {
                    at,
                    cut: selected && !field.secure && !field.readonly,
                    copy: selected && !field.secure,
                    paste: !field.readonly,
                    select_all: !field.value().text.is_empty(),
                });
            }
            return self.pointer_event(&chain, "contextmenu", at, button, None);
        }
        let pressed = self.pressed_in(&chain);
        let input = pressed.filter(|id| self.editing.fields.contains_key(id));
        if button == 1 {
            if let Some(input) = input.filter(|id| self.editing.fields.get(id).is_some_and(|field| !field.secure && !field.readonly)) {
                self.states.focus_visible = false;
                self.focus(Some(input));
                self.press_input(input, at, 1, false);
                if let Some(text) = self.clipboard.read(true) {
                    self.edit(input, |field| field.insert(&text));
                }
            }
            return;
        }
        self.states.pressed = pressed;
        self.changed();
        self.states.focus_visible = false;
        self.click_focus(&chain);
        // Presses close together in time and place count as a double or triple click, by GTK's defaults.
        let now = Instant::now();
        let clicks = match self.editing.last_press {
            Some((when, was, clicks)) if now - when < Duration::from_millis(400) && (was.x - at.x).abs() < 5.0 && (was.y - at.y).abs() < 5.0 => (clicks + 1).min(3),
            _ => 1,
        };
        self.editing.last_press = Some((now, at, clicks));
        // Text that's neither a field nor in something pressed is selected from here, by a pointer (a finger drags to
        // scroll); a press anywhere else lets go of what was.
        let text = (pressed.is_none() && !self.touch).then(|| chain.iter().copied().find(|id| self.editing.texts.contains_key(id))).flatten();
        self.select_down(text, at, clicks);
        if let Some(input) = input {
            self.editing.selecting = Some(input);
            self.press_input(input, at, clicks, shift);
        }
    }

    /// Input `id` pressed at `at`, `clicks` times in a row, or with Shift held.
    fn press_input(&mut self, id: Id, at: Point<f32>, clicks: u8, shift: bool) {
        let (index, upstream) = self.editing.drawn.get(&id).map_or((usize::MAX, false), |drawn| drawn.hit(at));
        self.edit(id, |field| field.press(index.min(field.display().len()), upstream, clicks, shift));
    }

    /// A button came up at `at`: what it went down on is told, and with the main one a press, if it's still over
    /// what was pressed.
    pub fn pointer_up(&mut self, at: Point<f32>, button: u8) {
        if let Some(kind) = self.chrome.pressing_button.take() {
            if self.chrome.buttons.iter().any(|(was, bounds)| *was == kind && bounds.contains(&at)) {
                self.window_button = Some(kind);
            }
            self.changed();
            return;
        }
        if std::mem::take(&mut self.chrome.pressing_action) {
            if self.chrome.action.is_some_and(|action| action.contains(&at))
                && let Some((_, action)) = self.notice.as_mut().and_then(|notice| notice.action.as_mut())
            {
                action();
            }
            return;
        }
        if std::mem::take(&mut self.chrome.pressing_home) {
            if self.chrome.home.is_some_and(|home| home.contains(&at))
                && let Some(crumb) = self.breadcrumb.as_mut()
            {
                (crumb.home)();
            }
            return;
        }
        if self.frame_up(at, button) {
            return;
        }
        let target = match self.captured.take() {
            Some(captured) => self.chain_from(captured),
            None => self.under(at),
        };
        self.drag_up(at, false);
        self.pointer_event(&target, "pointerup", at, button, None);
        self.select_up();
        // What the pointer selected goes on the primary selection, as Linux's fields put it, never a password.
        if let Some(field) = self.editing.selecting.take().and_then(|id| self.editing.fields.get(&id))
            && !field.secure
            && !field.selected().is_empty()
        {
            let selected = field.selected().to_owned();
            self.clipboard.write(true, &selected);
        }
        let Some(pressed) = self.states.pressed.take() else { return };
        let over = self.under(at).contains(&pressed);
        if over && self.placed(pressed).is_some_and(|placed| !placed.input) {
            self.click(pressed, Some(at));
        }
    }

    /// The wheel or a trackpad scrolled by `delta` pixels (down and right positive) at `at`, in `phase` where the
    /// system says: what's under it is told (`wheel`), and the innermost thing there that can still scroll that way
    /// does, telling its app if it listens. → what scrolled.
    pub fn scroll(&mut self, at: Point<f32>, delta: Point<f32>, phase: Option<Phase>) -> Option<Id> {
        // A frame's page scrolls first, and what it can't, this one does, as a browser chains them.
        if let Some(frame) = self.frame_scroll(at, delta, Some(phase)) {
            return Some(frame);
        }
        let chain = self.under(at);
        self.pointer_event(&chain, "wheel", at, 0, Some((delta, phase)));
        self.pan(at, delta)
    }

    /// Whether a finger that landed at `at` and has moved `by` (its first move past the slop) pans, as a browser decides
    /// by `touch-action`: from what it's on outwards, what can still scroll that way scrolls; a node that listens for
    /// its pointer keeps the finger, its moves its own, unless it says the device pans that way (`touchAction`); and
    /// with neither, the device pans. So a sheet's content scrolled to its top, pulled down, is the sheet's to pull,
    /// as an iPhone's and Android's sheets take it.
    pub fn pans(&self, at: Point<f32>, by: Point<f32>) -> bool {
        let along = by.x.abs() > by.y.abs();
        for id in self.under(at) {
            let Some(placed) = self.placed(id) else { continue };
            if let Some((most_y, most_x, _)) = placed.scroll {
                let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
                let (finger, now, most) = if along { (by.x, now.x, most_x) } else { (by.y, now.y, most_y) };
                if (finger < 0.0 && now < most) || (finger > 0.0 && now > 0.0) {
                    return true;
                }
            }
            // What drags that way keeps the finger, and what drags the other doesn't.
            match self.drags(id, by) {
                Some(true) => return false,
                Some(false) => continue,
                None => {}
            }
            if self.tree.nodes.get(&id).is_some_and(|node| node.listens("pointermove") || node.listens("pointerdown")) {
                return placed.touch_action == Some(if along { style::Pan::X } else { style::Pan::Y });
            }
        }
        true
    }

    /// A finger (or a screen reader) moved what's at `at` by `delta` pixels (down and right positive): the innermost
    /// thing there that can still scroll that way does, telling its app if it listens. → what scrolled.
    pub fn pan(&mut self, at: Point<f32>, delta: Point<f32>) -> Option<Id> {
        if let Some(frame) = self.frame_scroll(at, delta, None) {
            return Some(frame);
        }
        let chain = self.under(at);
        let target = chain.iter().copied().find_map(|id| {
            let (most_y, most_x, along) = self.placed(id)?.scroll?;
            let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
            // A scroll along takes the wheel's turn as along where it doesn't go down, as a row of pages does.
            let (by, now, most) = if along { (if delta.x != 0.0 { delta.x } else { delta.y }, now.x, most_x) } else { (delta.y, now.y, most_y) };
            ((by > 0.0 && now < most) || (by < 0.0 && now > 0.0)).then_some((id, by, most, along))
        })?;
        let (id, by, most, along) = target;
        let now = self.scrolled.entry(id).or_insert(point(0.0, 0.0));
        if along {
            now.x = (now.x + by).clamp(0.0, most);
        } else {
            now.y = (now.y + by).clamp(0.0, most);
        }
        let scrolled = *now;
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("scroll")) {
            self.tell(id, "scroll", json!({"scrollTop": scrolled.y, "scrollLeft": scrolled.x}));
        }
        Some(id)
    }

    /// A tap on an iPhone's status bar: everything that scrolls goes back to its top, as the DOM's page does (`toTop` in
    /// the UI library's bridge.ts).
    pub fn scroll_to_top(&mut self) {
        let scrolled: Vec<(Id, Point<f32>)> = self.scrolled.iter().filter(|(_, at)| at.y > 0.0).map(|(id, at)| (*id, point(at.x, 0.0))).collect();
        for (id, at) in scrolled {
            self.scrolled.insert(id, at);
            if self.tree.nodes.get(&id).is_some_and(|node| node.listens("scroll")) {
                self.tell(id, "scroll", json!({"scrollTop": at.y, "scrollLeft": at.x}));
            }
        }
        self.changed();
    }

    /// Gives the keyboard to `id`, or to nothing, telling the app what had it and what has it, where they listen. An
    /// input that loses it keeps what was being composed in it.
    fn focus(&mut self, id: Option<Id>) {
        if self.focus.focused == id {
            return;
        }
        self.changed();
        if let Some(input) = self.focus.focused.filter(|focused| self.editing.fields.contains_key(focused)) {
            self.edit(input, Field::blur);
        }
        self.editing.blink = self.js.clock().now();
        let blurred = std::mem::replace(&mut self.focus.focused, id);
        self.focus.blurred = blurred;
        if let Some(blurred) = blurred.filter(|blurred| self.tree.nodes.get(blurred).is_some_and(|node| node.listens("blur"))) {
            self.event(blurred, "blur", None);
        }
        if let Some(id) = id.filter(|id| self.tree.nodes.get(id).is_some_and(|node| node.listens("focus"))) {
            self.event(id, "focus", None);
        }
    }

    /// A key went down where the keyboard is, as the DOM's `KeyboardEvent` has it: Tab moves the keyboard, what
    /// listens for `keydown` hears it, Enter and Space press what has it, and an input takes what's typed. The page
    /// hears every key too, once, whatever has the keyboard, even nothing, as a browser's `document` does (`keydown()`:
    /// Escape closing what's open, ⌘K). → what had the keyboard.
    pub fn key(&mut self, key: &Key) -> Option<Id> {
        let focused = self.focus.focused;
        // The key of the shortcut that just summoned it, still held or pressed again at once, is the shortcut's, not
        // the page's: it types nothing and presses nothing (`summoned`).
        if let Some((held, since)) = self.shortcut.take() {
            if key.key == held && since.elapsed() < SHORTCUT_HELD {
                self.shortcut = Some((held, web_time::Instant::now()));
                return focused;
            }
        }
        // A frame with the keyboard: its page has the key, and Tab off its page's end goes on from the frame here.
        if let Some(None) = self.frame_key(key) {
            return focused;
        }
        // Tab is told to what has the keyboard and what it's in, as any key is, then moves the keyboard, as a
        // browser's does unless the page stops it, which a native host can't hear in time.
        if key.key == "Tab" && !key.ctrl && !key.meta && !key.alt {
            if let Some(id) = focused {
                for listener in self.chain_from(id) {
                    if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                        self.tell(listener, "keydown", key.told());
                    }
                }
            }
            self.tell(PAGE, "keydown", key.told());
            self.states.focus_visible = true;
            // It goes round, but out of a frame's page at its ends, back to the page the frame's in; into a frame's
            // page, to its first stop or its last, and past a frame whose page has none.
            let order = self.focus.tab_order.clone();
            let count = order.len();
            let mut index = focused.and_then(|focused| order.iter().position(|id| *id == focused));
            let mut next = None;
            for _ in 0..count {
                let step = match index {
                    Some(at) if key.shift => at.checked_sub(1).or((!self.nested).then(|| count - 1)),
                    Some(at) => (at + 1 < count).then_some(at + 1).or((!self.nested).then_some(0)),
                    None if key.shift => count.checked_sub(1),
                    None => (count > 0).then_some(0),
                };
                let Some(step) = step else { break };
                index = Some(step);
                if self.frames.contains_key(&order[step]) && !self.enter_frame(order[step], key.shift) {
                    continue;
                }
                next = Some(order[step]);
                break;
            }
            if next.is_none() && self.nested {
                self.left = Some(key.shift);
                self.focus(None);
                return focused;
            }
            self.focus(next);
            // What it comes to scrolls into view, as a browser's does, so Tab goes on through a long list.
            if let Some(next) = next {
                self.reveal(next);
            }
            // An input the keyboard comes to by Tab has its text selected, as a browser's and GTK's do.
            if let Some(next) = next.filter(|next| self.editing.fields.contains_key(next)) {
                self.edit(next, Field::select_all);
            }
            return focused;
        }
        // A key goes to what has the keyboard and to each node it's in that listens, the innermost first, then the page.
        if let Some(id) = focused {
            for listener in self.chain_from(id) {
                if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                    self.tell(listener, "keydown", key.told());
                }
            }
        }
        // Enter or Space on a button presses it alone, as every system's does: the page's `preventDefault` would come
        // too late to stop the press, so the page isn't told of it too (a launcher opening its chosen app behind it).
        let plain = !key.ctrl && !key.meta && !key.alt;
        let presses = plain && matches!(key.key.as_str(), "Enter" | " ") && focused.is_some_and(|id| matches!(id, HOME | NOTICE_ACTION) || (!self.editing.fields.contains_key(&id) && self.tree.nodes.get(&id).is_some_and(|node| node.listens("click"))));
        if !presses {
            self.tell(PAGE, "keydown", key.told());
        }
        // What's selected of a text that isn't a field is copied, as a browser's is.
        if self.copy_selected(key) {
            return focused;
        }
        let id = focused?;
        // Escape empties a search field with something typed in it, as a browser's `type="search"` does.
        let search = self.tree.nodes.get(&id).is_some_and(|node| node.str("type") == Some("search"));
        if search && key.key == "Escape" && self.editing.fields.get(&id).is_some_and(|field| !field.value().text.is_empty()) {
            // What's selected is replaced by nothing, as a cut is, which undo brings back.
            self.edit(id, |field| {
                field.select_all();
                field.paste("");
            });
        } else if self.editing.fields.contains_key(&id) {
            self.type_key(id, key);
        } else if presses && self.press_chrome(id) {
        } else if matches!(key.key.as_str(), "Enter" | " ") && self.tree.nodes.get(&id).is_some_and(|node| node.listens("click")) {
            self.click(id, None)
        }
        Some(id)
    }

    /// What a key does in input `id`: typing, moving, selecting, undoing and the clipboard, by the editing model
    /// (`editing::Field::key`), then its `input` if that changed it and its `action` if it was the action key.
    fn type_key(&mut self, id: Id, key: &Key) {
        let Some(field) = self.editing.fields.get_mut(&id) else { return };
        let before = field.value();
        let outcome = field.key(key, self.editing.drawn.get(&id).map(|drawn| drawn as &dyn editing::Layout));
        if let Some(copy) = outcome.copy.filter(|copy| !copy.is_empty()) {
            self.clipboard.write(false, &copy);
        }
        if outcome.paste
            && let Some(text) = self.clipboard.read(false)
        {
            field.paste(&text);
        }
        self.edited(id, before);
        if outcome.action && self.tree.nodes.get(&id).is_some_and(|node| node.listens("action")) {
            let node = &self.tree.nodes[&id];
            let action = node.str("action").unwrap_or(if node.is("multiline") { "newline" } else { "done" }).to_owned();
            self.tell(id, "action", json!({"action": action}));
        }
    }

    /// What the platform's input method says, to the input with the keyboard.
    pub fn ime(&mut self, ime: Ime) {
        if self.frame_ime(ime.clone()) {
            return;
        }
        if let Some(id) = self.focus.focused.filter(|id| self.editing.fields.contains_key(id)) {
            self.edit(id, |field| field.ime(ime));
        }
    }

    /// The picture at `src`, as its page names it, fetched by its window: as a `data:` address, or none where it wasn't
    /// found.
    pub fn fetched(&mut self, src: &str, data: Option<String>) {
        self.fetched.insert(src.to_owned(), data);
    }

    /// Its window came on screen, or went, as a panel does as it's summoned and put away: its page is told, as a
    /// browser's page is by `visibilitychange`.
    pub fn shown(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        for child in self.frames.values_mut() {
            child.screen.shown(visible);
        }
        // Shown again, it has the keyboard where it says `autofocus`, as a page opened does, from its first key on;
        // put away, nothing keeps it, so nothing left with it is pressed as it comes back.
        self.focus.autofocused.clear();
        self.focus(if visible { self.tree.autofocused().max() } else { None });
        if visible {
            self.focus.autofocused.extend(self.tree.autofocused());
        }
        self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"visible": visible}}));
    }

    /// It was summoned by a shortcut that ends with `key` (as the DOM's `KeyboardEvent.key` names it): that key, still
    /// held as it shows or pressed again right after, is the shortcut's (`key`).
    pub fn summoned(&mut self, key: Option<&str>) {
        self.shortcut = key.map(|key| (key.to_owned(), web_time::Instant::now()));
    }

    /// Files dropped on the window at `at` from the system (a Finder's, an Explorer's): what's under it that takes a
    /// drop, the innermost, is told them (`drop`).
    pub fn dropped(&mut self, at: Point<f32>, files: &[std::path::PathBuf]) {
        if let Some(id) = self.under(at).into_iter().find(|id| self.tree.nodes.get(id).is_some_and(|node| node.listens("drop"))) {
            self.tell(id, "drop", json!({"files": files_told(files)}));
        }
    }

    /// The window button last pressed, once, for its window to do.
    pub fn take_window_button(&mut self) -> Option<WindowButton> {
        self.window_button.take()
    }

    /// The system's menu for text a secondary click asked for, once.
    pub fn take_text_menu(&mut self) -> Option<TextMenu> {
        self.text_menu.take()
    }

    /// Its window got or lost the keyboard: the caret shows only while it has it, and an input losing it keeps what
    /// was being composed in it.
    pub fn activate(&mut self, active: bool) {
        self.active = active;
        self.editing.blink = self.js.clock().now();
        if let Some(id) = self.focus.focused.filter(|id| !active && self.editing.fields.contains_key(id)) {
            self.edit(id, Field::finish_composition);
        }
        // What's typed and not sent stays: what waits for it to be unused waits for the next time.
        if !active && !self.typed() {
            for unused in self.when_unused.drain(..) {
                unused();
            }
        }
    }

    /// Whether an input holds something its person typed that its app hasn't taken (`Field::typed`).
    pub fn typed(&self) -> bool {
        self.editing.fields.values().any(Field::typed) || self.frames.values().any(|child| child.screen.typed())
    }

    /// The editing state of the input with the keyboard, for a platform's keyboard bridge (`editing`).
    pub fn editing(&self) -> Option<editing::Value> {
        if let Some(child) = self.focus.focused.and_then(|id| self.frames.get(&id)) {
            return child.screen.editing();
        }
        self.focus.focused.and_then(|id| self.editing.fields.get(&id)).map(Field::value)
    }

    /// The character of the input with the keyboard nearest to `at`, in the window, in UTF-16 units of its
    /// `editing` text, as AppKit's `characterIndexForPoint:` asks.
    pub fn index_at(&self, at: Point<f32>) -> Option<usize> {
        let id = self.focus.focused?;
        if let Some(child) = self.frames.get(&id) {
            return child.screen.index_at(point(at.x - child.rect.origin.x, at.y - child.rect.origin.y));
        }
        let (field, drawn) = (self.editing.fields.get(&id)?, self.editing.drawn.get(&id)?);
        let index = drawn.index_at(at);
        // What's drawn is its editing text, what's composed in it too, but for a password's dots.
        Some(if field.secure { editing::utf16(&field.text, field.to_model(index)) } else { editing::utf16(&field.display(), index) })
    }

    /// Where the input with the keyboard has its caret, in the window, for the platform's input method to show what
    /// it composes beside: none where it takes no typing (a password takes no input method).
    pub fn caret_rect(&self) -> Option<Bounds<f32>> {
        if let Some(caret) = self.frame_caret() {
            return caret;
        }
        let id = self.focus.focused?;
        let field = self.editing.fields.get(&id).filter(|field| !field.secure && !field.readonly)?;
        let drawn = self.editing.drawn.get(&id)?;
        let (line, x) = field.placed().unwrap_or_else(|| editing::Layout::caret(drawn, field.caret(), field.upstream()));
        let height = drawn.paragraph.line_height * drawn.grown;
        Some(Bounds { origin: point(drawn.origin.x + (x - drawn.scroll.x) * drawn.grown, drawn.origin.y + line as f32 * height), size: Size { width: drawn.grown, height } })
    }

    /// Its title row, where its window has one over its page (a unified toolbar): how tall it is, and where in it
    /// something takes a press, which the rest of it doesn't, so the window moves held there, as a title bar does.
    /// What its page says moves the window (`appRegion`) is part of it too, with or without a title row.
    pub fn title_row(&self) -> Option<TitleRow> {
        let height = if self.breadcrumb.is_some() { self.insets.top } else { 0.0 };
        if height == 0.0 && self.chrome.drag_regions.is_empty() {
            return None;
        }
        // What takes a press, or a hold, in the row or in what moves the window, keeps it: a button, an input, what's
        // held to edit it. What's under them isn't in them.
        let inside = |id: Id, placed: &Placed| placed.shown.origin.y < height || self.chrome.drag_regions.iter().any(|(region, _)| self.within(id, *region));
        let takes = |id: &Id, placed: &Placed| placed.pressable || placed.input || self.tree.nodes.get(id).is_some_and(|node| node.listens("pointerdown"));
        let mut taken: Vec<Bounds<f32>> = self.placed.iter().filter(|(id, placed)| takes(id, placed) && inside(*id, placed)).map(|(_, placed)| placed.shown).collect();
        taken.extend(self.chrome.kept_regions.iter().copied());
        taken.extend(self.chrome.home);
        taken.extend(self.chrome.buttons.iter().map(|(_, bounds)| *bounds));
        taken.extend(self.chrome.action);
        Some(TitleRow { height, taken, regions: self.chrome.drag_regions.iter().map(|(_, bounds)| *bounds).collect() })
    }

    /// Does `change` to input `id`'s editing, and tells its app what changed.
    fn edit(&mut self, id: Id, change: impl FnOnce(&mut Field)) {
        let Some(field) = self.editing.fields.get_mut(&id) else { return };
        let before = field.value();
        change(field);
        self.edited(id, before);
    }

    /// Input `id`'s editing changed from `before`, or perhaps not: its caret shows again, and its app hears its
    /// editing state (`input`) if it's different, with its text as `value` too, as the DOM's event has it.
    fn edited(&mut self, id: Id, before: editing::Value) {
        self.editing.blink = self.js.clock().now();
        let Some(value) = self.editing.fields.get(&id).map(Field::value).filter(|value| *value != before) else { return };
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("input")) {
            self.tell(id, "input", json!({"editing": value, "value": value.text}));
            self.editing.edits.insert(id, self.js.sent());
        }
    }

    /// Types `text` into input `id`, a character at a time, each an `input`.
    pub(crate) fn type_text(&mut self, id: Id, text: &str) {
        let mut buffer = [0; 4];
        for typed in text.chars() {
            self.edit(id, |field| field.insert(typed.encode_utf8(&mut buffer)));
        }
    }

    /// Each input's editing as its node says, after the app's mutations: what it is (several lines, a password, read
    /// only, what Return does), and its editing state where the app set a new one.
    fn sync_fields(&mut self) {
        let tree = &self.tree;
        let input = |id: &Id| tree.nodes.get(id).is_some_and(|node| node.kind == Kind::Input);
        self.editing.fields.retain(|id, _| input(id));
        self.editing.set.retain(|id, _| input(id));
        self.editing.edits.retain(|id, _| input(id));
        self.editing.drawn.retain(|id, _| input(id));
        self.editing.texts.retain(|id, _| tree.nodes.contains_key(id));
        if self.editing.selected.is_some_and(|selected| !tree.nodes.contains_key(&selected.id)) {
            self.editing.selected = None;
        }
        for id in tree.inputs() {
            let node = &tree.nodes[&id];
            let field = self.editing.fields.entry(id).or_default();
            field.multiline = node.is("multiline");
            field.newline = field.multiline && node.str("action").is_none_or(|action| action == "newline");
            field.secure = node.str("type") == Some("password");
            field.readonly = node.is("readonly");
            let set = node.prop("editing").and_then(|editing| serde_json::from_value::<editing::Value>(editing.clone()).ok());
            if let Some(set) = set.filter(|set| self.editing.set.get(&id) != Some(set)) {
                if self.editing.edits.get(&id).is_none_or(|told| *told <= self.editing.heard) {
                    field.set(&set);
                }
                self.editing.set.insert(id, set);
            }
        }
    }
}

/// Files the device gave a page, as it's told them: each one's name, and its bytes as base64, the only way a file
/// crosses its JSON channel. One that can't be read is left out, saying why.
pub(crate) fn files_told(files: &[std::path::PathBuf]) -> Vec<Value> {
    let told = |file: &std::path::PathBuf| -> anyhow::Result<Value> {
        let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(json!({"name": name, "body": data_encoding::BASE64.encode(&std::fs::read(file)?)}))
    };
    files.iter().filter_map(|file| told(file).map_err(|error| log::warn!("{} isn't given to its page: {error:#}", file.display())).ok()).collect()
}

/// Paints a screen that says only `words`, in the middle, in the muted colour, as a page's empty state does.
pub(crate) fn paint_message(words: &str, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
    scene.clear();
    // A host's own message, in a computer's size: it has no window's conditions.
    let conditions = Conditions { dark: appearance == Appearance::Dark, ..Conditions::default() };
    let (body, background) = Text::root(conditions);
    let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
    let mask = ContentMask { bounds: window.scale_to(scale) };
    scene.insert_primitive(Quad {
        order: 0,
        border_style: Default::default(),
        bounds: window.scale_to(scale),
        content_mask: mask,
        background: Background::from(background.unwrap_or_default()),
        border_color: Hsla::default(),
        corner_radii: Corners::default(),
        border_widths: Edges::default(),
        transformation: TransformationMatrix::unit(),
    });
    let width = (window.size.width - 32.0).max(1.0);
    let paragraph = text.paragraph(words, &body.style, &[], Some(width), None);
    let top = (window.size.height - paragraph.size().height) / 2.0;
    let muted = colour_token("secondary-label", conditions);
    let _ = text.paint(&paragraph, point(px(16.0), px(top)), width, 0.5, &|_| muted, scale, mask, TransformationMatrix::unit(), atlas, scene);
    scene.finish();
}

/// What the painter keeps of each node it paints, for the pointer and the keyboard.
/// What a click on a node does with the keyboard (`Placed::focus`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clicked {
    Takes,
    Keeps,
    Passes,
}

#[derive(Clone)]
pub(crate) struct Placed {
    pub parent: Option<Id>,
    /// Where it shows, clipped, in the window.
    pub shown: Bounds<f32>,
    /// Whether it takes the pointer (`pointerEvents`): what doesn't is still in the chain of what's in it.
    pub pointer: bool,
    pub cursor: Option<Cursor>,
    pub pressable: bool,
    pub input: bool,
    /// What a click gives the keyboard, as the DOM's does: it, where it takes the keyboard (an input, what's pressed as
    /// a button is, what says its `tabindex`, a frame), or nothing, or the keyboard stays where it is, as a press of
    /// what's chosen from a list another holds it for (`option`) leaves it (core/ui's drawing-primitives/primitives.tsx
    /// `chosen`).
    pub focus: Clicked,
    /// It scrolls: how far it can, down and along, and whether along is its way.
    pub scroll: Option<(f32, f32, bool)>,
    /// Which way a finger on it pans, where it listens for its pointer (`Look::touch_action`).
    pub touch_action: Option<style::Pan>,
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
    last_press: Option<(Instant, Point<f32>, u8)>,
    blink: f64,
    /// Each selectable text as it was last drawn, what's selected of one, and whether a press is selecting
    /// (`selection`).
    texts: HashMap<Id, selection::Shown>,
    selected: Option<selection::Selected>,
    selecting_text: bool,
}

/// What the window draws of its own over its page, as it was last drawn: where its page's top bar starts in its title
/// row, after its breadcrumb (0 for a window with none); where its page asked for the system's own material, which a
/// panel's host puts under it; where its page says a press moves the window, and where in that it doesn't
/// (`appRegion`); and where its notice's action and its breadcrumb's Home are, and whether each is pressed.
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
    home: Option<Bounds<f32>>,
    pressing_home: bool,
}

/// What a frame draws, as `Screen::plan` worked it out: the conditions, the window and its scale, the time on the
/// page's clock and whether the caret shows; whether everything is styled again, or what changed, is moving or is in
/// or around either; what's drawn from its layer; and whether nothing moves, so what's about to is drawn ahead.
struct Plan {
    conditions: Conditions,
    window: Bounds<f32>,
    scale: f32,
    now: f64,
    caret: bool,
    everything: bool,
    changed: HashSet<Id>,
    moving: HashSet<Id>,
    below: HashSet<Id>,
    layered: HashSet<Id>,
    still: bool,
}

/// What the painter found as it drew a frame (`Screen::draw`): where Home was drawn, what takes the pointer and the
/// keyboard, where each node is, whether anything moves by itself, the topmost modal, the pictures it wants fetched,
/// what's near the window and where each frame's page is.
struct Drawing {
    home: Option<Bounds<f32>>,
    placed: Vec<(Id, Placed)>,
    tab_order: Vec<Id>,
    rects: Rects,
    animating: bool,
    modal: Option<Id>,
    wanted: Vec<String>,
    intersections: Vec<(Id, bool)>,
    framed: Vec<(Id, Bounds<f32>)>,
}

#[derive(PartialEq)]
struct Seen {
    size: (Size<f32>, f32),
    conditions: Conditions,
    states: States,
    scrolled: HashMap<Id, Point<f32>>,
    stretched: std::collections::BTreeMap<Id, Point<f32>>,
    fields: Vec<(Id, String, editing::Value, Option<(usize, f32)>, bool)>,
    caret: bool,
    hovering: (bool, bool),
    notice: Option<(String, Option<String>)>,
    breadcrumb: Option<(String, f32)>,
    panel: Option<(f32, bool)>,
    /// Where the rail's line shows, if it does.
    line: Option<Bounds<f32>>,
    fetched: usize,
    decoded: u64,
    /// Each frame's page's frame, as it was drawn.
    frames: std::collections::BTreeMap<Id, u64>,
}

/// What a node's state is, for its styles: where the pointer is, what's pressed and what has the keyboard.
#[derive(Default, Clone, PartialEq)]
pub(crate) struct States {
    /// What the pointer is over, from the root to the innermost.
    pub hovered: Vec<Id>,
    pub pressed: Option<Id>,
    pub focused: Option<Id>,
    /// The keyboard came there by a key, so it shows (CSS's `:focus-visible`).
    pub focus_visible: bool,
    /// The pointer is a finger, which a style's `hover` never is: that's a mouse's (`State` in the UI library), as the
    /// DOM's styles hover only where `(hover: hover)`.
    pub touch: bool,
}

impl States {
    /// What's in a different state from in `was`.
    fn changed(&self, was: &States) -> Vec<Id> {
        let mut changed: Vec<Id> = self.hovered.iter().filter(|id| !was.hovered.contains(id)).chain(was.hovered.iter().filter(|id| !self.hovered.contains(id))).copied().collect();
        if self.touch != was.touch {
            changed.extend(self.hovered.iter().copied());
        }
        if self.pressed != was.pressed {
            changed.extend(self.pressed.into_iter().chain(was.pressed));
        }
        if self.focused != was.focused || self.focus_visible != was.focus_visible {
            changed.extend(self.focused.into_iter().chain(was.focused));
        }
        changed
    }

    fn of(&self, id: Id, input: bool) -> State {
        State {
            hover: !self.touch && self.hovered.contains(&id),
            pressed: self.pressed == Some(id),
            focused: self.focused == Some(id) && (self.focus_visible || input),
            entering: false,
            exiting: false,
        }
    }
}

/// A caret's blink, in milliseconds on the app's clock: on two thirds of it and off a third, as GTK's, until it's been
/// still `BLINK_STOPS`.
const BLINK: f64 = 1200.0;
const BLINK_STOPS: f64 = 10_000.0;

/// Where an input's text was drawn: its paragraph (of its display text, or its placeholder while it's `empty`), its
/// first line's top left in the window, where each line starts along the paragraph, how much it's grown, and how far
/// it has scrolled, along one line or down several. Its `Layout` is the paragraph's, unscrolled.
pub(crate) struct Drawn {
    paragraph: Paragraph,
    empty: bool,
    origin: Point<f32>,
    offsets: Vec<f32>,
    grown: f32,
    scroll: Point<f32>,
    /// Whether it had the keyboard, where its caret was and how long its text was, to scroll to the caret only as
    /// it moves.
    seen: (bool, usize, bool, usize),
}

impl Drawn {
    /// The byte of its display text nearest to `at`, in the window.
    fn index_at(&self, at: Point<f32>) -> usize {
        self.hit(at).0
    }

    /// The byte of its display text nearest to `at`, in the window, and whether the caret there is upstream: at the
    /// end of the line `at` is on, where it wraps with no space to hang.
    fn hit(&self, at: Point<f32>) -> (usize, bool) {
        if self.empty {
            return (0, false);
        }
        let y = (at.y - self.origin.y) / self.grown;
        let line = ((y / self.paragraph.line_height).floor().max(0.0) as usize).min(self.paragraph.lines.len().saturating_sub(1));
        let x = (at.x - self.origin.x) / self.grown + self.scroll.x;
        let index = editing::Layout::index_on(self, line, x).unwrap_or(0);
        (index, self.paragraph.caret(index, false).0 != line)
    }
}

impl editing::Layout for Drawn {
    fn caret(&self, index: usize, upstream: bool) -> (usize, f32) {
        if self.empty {
            return (0, 0.0);
        }
        let (line, x) = self.paragraph.caret(index, upstream);
        (line, x + self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn index_on(&self, line: usize, x: f32) -> Option<usize> {
        if self.empty {
            return (line == 0).then_some(0);
        }
        self.paragraph.index_on(line, x - self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn line_edge(&self, line: usize, end: bool) -> usize {
        match self.paragraph.lines.get(line) {
            _ if self.empty => 0,
            Some(_) if end => self.paragraph.end(line),
            Some(drawn) => drawn.start,
            None => 0,
        }
    }

    fn bidi(&self) -> bool {
        !self.empty && self.paragraph.lines.iter().any(|line| line.rtl || line.glyphs.iter().any(|(_, _, glyph)| glyph.rtl))
    }

    fn rtl(&self, line: usize) -> bool {
        self.paragraph.lines.get(line).is_some_and(|line| line.rtl)
    }

    fn stops(&self, line: usize) -> Vec<editing::Stop> {
        let (Some(drawn), false) = (self.paragraph.lines.get(line), self.empty) else { return Vec::new() };
        let offset = self.offsets.get(line).copied().unwrap_or(0.0);
        drawn
            .glyphs
            .iter()
            .flat_map(|(_, _, glyph)| {
                let (left, right) = if glyph.rtl { (glyph.end, glyph.index) } else { (glyph.index, glyph.end) };
                let x = offset + glyph.position.x.0;
                [editing::Stop { x, index: left, right: false }, editing::Stop { x: x + glyph.advance, index: right, right: true }]
            })
            .collect()
    }
}
