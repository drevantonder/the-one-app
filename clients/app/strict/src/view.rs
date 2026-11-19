//! An app's screen: its tree laid out by Taffy and painted as a scene with the design tokens, and what the person
//! does (a press, a key, a scroll) found on it and sent back to its JS as events. The window it's in (`platform`, or
//! the headless harness) gives it the room, the scale and the input, and draws the scene it paints.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use taffy::prelude::{AvailableSpace, NodeId, TaffyTree};

use crate::editing::{self, Clipboard, Field, Ime};
use crate::geometry::{Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels, Point, ScaledPixels, Size, point, px};
use crate::inspect::{self, Key, Rect, Rects};
use crate::js::{Posted, Thread};
use crate::render::Atlas;
use crate::render::atlas::{AtlasKey, AtlasTile};
use crate::render::scene::{Backdrop, BorderStyle, PolychromeSprite, Quad, Scene, Shadow, TransformationMatrix};
use crate::style::{self, Look, Paint, State, Text, colour};
use crate::text::{Paragraph, TextRun, TextSystem};
use crate::tokens::{Conditions, Radius};
use crate::tree::{Id, Kind, Node, Tree};

mod bridge;
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
    /// How far in from the window's start it begins, in the band its top inset keeps clear.
    pub start: f32,
    pub home: Box<dyn FnMut()>,
}

/// A window's title row over its page, its top bar's (`Screen::title_row`): how tall it is, and where in it a press is
/// its page's; anywhere else in it moves the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitleRow {
    pub height: f32,
    pub taken: Vec<Bounds<f32>>,
}

impl TitleRow {
    /// Whether a press at `at` moves the window.
    pub fn moves(&self, at: Point<f32>) -> bool {
        at.y < self.height && !self.taken.iter().any(|bounds| bounds.contains(&at))
    }
}

/// A panel's own background, which its window has no frame for (`WindowOptions::panel`): its corners' radius, and
/// whether the host fills it with a popover's colour, where the system has no glass to draw under it (Linux).
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub corner: f32,
    pub fill: bool,
}

/// What the window says over its page of where the page is, as a phone's app says it over an app (DESIGN.md, App
/// windows): its network not answering ("Reconnecting…"), or its page updated ("Updated", with Reload). A pill at its
/// top, over the page, with its one action.
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

/// The typeface, from the UI library's WOFF2, as TrueType, which the text systems read.
pub fn fonts() -> anyhow::Result<Vec<Vec<u8>>> {
    let ttf = wuff::decompress_woff2(include_bytes!("../../../../core/ui/src/Allo-Variable.woff2"))
        .map_err(|error| anyhow::anyhow!("Allo didn't decode: {error:?}"))?;
    Ok(vec![ttf])
}

/// How long after its shortcut summoned a panel, or after it was last pressed again, the shortcut's key is still its:
/// longer than a key's repeat delay, so one held repeats into nothing.
/// The roles of what the DOM draws as a button, which Tab reaches (core/ui's strict/primitives.tsx `pressed`).
const BUTTON_ROLES: [&str; 9] = ["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"];

const SHORTCUT_HELD: std::time::Duration = std::time::Duration::from_millis(600);

pub struct Screen {
    tree: Tree,
    /// Its app's JS, on a thread of its own, and whether its module has run, or why it didn't.
    js: Thread,
    started: Option<Result<(), String>>,
    /// The inspector's answers to what it did, each told once the app's JS has run what that did.
    replies: Vec<(Value, inspected::Reply)>,
    /// As Colour Mode says, or as the device is (`None`).
    pub theme: Option<Appearance>,
    /// As the device is, which the window says.
    pub appearance: Appearance,
    /// The person asked for less motion, as the device says.
    pub reduced_motion: bool,
    /// Whose own tokens it draws with, as its page's styles gave them (`tokens::load_page`).
    pub page: u32,
    /// What its window says above it, on a computer, and where its Home was drawn and whether it's pressed.
    pub breadcrumb: Option<Breadcrumb>,
    /// Where its page's top bar starts in its title row, after its breadcrumb, as it was last drawn: 0 for a window
    /// with none.
    title_start: u16,
    /// Its window is a panel, whose background the host draws, under its page's.
    pub panel: Option<Panel>,
    /// Where its page asked for the system's own material, as it was last drawn, which a panel's host puts under it.
    pub glass: Vec<crate::platform::Glass>,
    /// What the window says over its page, and where its action was drawn and whether it's pressed.
    pub notice: Option<Notice>,
    action: Option<Bounds<f32>>,
    pressing_action: bool,
    /// Whether its window is on screen, as its page is told (its host context's `visible`): a panel put away isn't.
    visible: bool,
    home: Option<Bounds<f32>>,
    pressing_home: bool,
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
    /// What has the keyboard.
    focused: Option<Id>,
    /// Each input's editing, as it's typed, ahead of the app setting it back, and the editing state its app last
    /// set, which replaces it only as it changes.
    fields: HashMap<Id, Field>,
    set: HashMap<Id, editing::Value>,
    /// Where each input's text was last drawn, for the pointer and the moves that depend on how it wraps.
    drawn: HashMap<Id, Drawn>,
    /// The input the pointer's held press selects in, and the last press, to count a double or triple click.
    selecting: Option<Id>,
    last_press: Option<(Instant, Point<f32>, u8)>,
    /// When the caret last moved, on the app's clock, from which it blinks.
    blink: f64,
    /// Its window has the keyboard, so the caret shows.
    pub active: bool,
    /// What copy and paste use: the computer's, which the window gives it, or one of its own.
    pub clipboard: Box<dyn Clipboard>,
    /// How far each scrolling node (and the root, 0) has scrolled, along and down.
    scrolled: HashMap<Id, Point<f32>>,
    /// Where the pointer is, what it's over and what it pressed, for styles' states and pointer events.
    states: States,
    pointer: Option<Point<f32>>,
    /// The pointer is a finger, as a phone's is, which pointer events say (`pointerType`).
    pub touch: bool,
    /// The node a pointer's press went down on, which its moves and its end go to, wherever it goes.
    captured: Option<Id>,
    /// What takes the pointer, as it was last painted, in the order painted, the topmost last.
    placed: Vec<(Id, Placed)>,
    /// Whether anything moved by itself in the last frame.
    animating: bool,
    again: bool,
    /// What's on its way from one value to another as a style changed.
    transitions: transition::Transitions,
    /// The tree as Taffy lays it out, kept from frame to frame (`Retained`), the states its styles were last resolved
    /// in, and the frame it's on.
    layout: Retained,
    styled: States,
    frame: u64,
    /// What the last frame showed, which a frame that would show the same doesn't paint again.
    seen: Option<Seen>,
    /// What has taken the keyboard by its `autofocus`, while that's so, and where each node listening for `layout`
    /// was last told it is.
    autofocused: std::collections::HashSet<Id>,
    /// The key of the shortcut that summoned it, and when it was last pressed (`summoned`).
    shortcut: Option<(String, std::time::Instant)>,
    laid_out: HashMap<Id, Rect>,
    /// Whether each node listening for `intersection` was last told it's near the window.
    intersecting: HashMap<Id, bool>,
    /// What takes the keyboard, in the order Tab goes, as it was last painted.
    tab_order: Vec<Id>,
    /// Each picture drawn, by its key in the atlas, and its size there, or none where it didn't decode.
    images: HashMap<u64, Option<Picture>>,
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
    /// The last error its JS threw, which is logged, as a browser's console has it, and the screen carries on.
    pub failed: Option<String>,
    /// What its page uses that this host can't draw (`tree::Mismatch`), so its window opens in a web view instead.
    pub mismatched: Option<String>,
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
            theme: None,
            appearance: Appearance::Light,
            reduced_motion: false,
            page: 0,
            breadcrumb: None,
            title_start: 0,
            panel: None,
            glass: Vec::new(),
            notice: None,
            action: None,
            pressing_action: false,
            visible: true,
            home: None,
            pressing_home: false,
            insets,
            room,
            told: None,
            changed: None,
            asked,
            focused: None,
            fields: HashMap::new(),
            set: HashMap::new(),
            drawn: HashMap::new(),
            selecting: None,
            last_press: None,
            blink: 0.0,
            active: true,
            clipboard: Box::new(editing::Memory::default()),
            scrolled: HashMap::new(),
            states: States::default(),
            pointer: None,
            touch: false,
            captured: None,
            placed: Vec::new(),
            animating: false,
            again: false,
            transitions: Default::default(),
            layout: Retained::default(),
            styled: States::default(),
            frame: 0,
            seen: None,
            autofocused: Default::default(),
            shortcut: None,
            laid_out: HashMap::new(),
            intersecting: HashMap::new(),
            tab_order: Vec::new(),
            images: HashMap::new(),
            fetched: HashMap::new(),
            fetch: None,
            rects: Rects::new(),
            painted: (Size::default(), 1.0),
            unanswered: HashMap::new(),
            settling: Vec::new(),
            failed: None,
            mismatched: None,
        };
        screen.pump();
        screen
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Calls `wake` on its app's JS thread as that posts what this hasn't applied (`pump`).
    pub fn on_post(&self, wake: impl Fn() + Send + 'static) {
        self.js.on_post(wake);
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
            self.pump();
            if !self.js.busy() {
                return;
            }
        }
    }

    /// The semantic tree, as an agent and a screen reader read it.
    pub fn semantics(&self) -> Value {
        json!(self.tree.semantics(0))
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
        let at = at.unwrap_or(point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
        self.tell(id, "click", json!({"clientX": at.x, "clientY": at.y, "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}));
    }

    /// Tells the app a message from its client: an answer to what it asked (`Asked`), or a notification.
    pub fn message(&mut self, message: &Value) {
        if message.get("method").is_none() {
            self.unanswered.remove(&message["id"].to_string());
        }
        self.js.send("message", message.clone());
    }

    /// Logs what the app's JS threw, as a browser's console does, and carries on.
    fn threw(&mut self, error: String) {
        eprintln!("strict app error: {error}");
        self.failed = Some(error);
    }

    /// How long until the screen has something to do by itself: an inspector's `settle` coming to its deadline, a
    /// frame of what moves, or the caret's blink. The window calls `fire` then. The app's timers are its JS thread's.
    pub fn due(&self) -> Option<Duration> {
        let now = Instant::now();
        let settle = self.settling.iter().map(|(until, _)| until.saturating_duration_since(now)).min();
        // What moves by itself draws again each frame.
        let frame = self.animating.then_some(Duration::from_millis(16));
        settle.into_iter().chain(frame).chain(self.blinks()).min()
    }

    /// How long until the caret blinks on or off: as GTK's does, on two thirds of each 1.2 s and off a third, until
    /// it's been still 10 s, when it stays on. None while the app's clock is paused, or no input has the keyboard.
    fn blinks(&self) -> Option<Duration> {
        let clock = self.js.clock();
        let typing = self.active && self.focused.is_some_and(|id| self.fields.contains_key(&id));
        let since = clock.now() - self.blink;
        if !typing || clock.paused() || since >= BLINK_STOPS {
            return None;
        }
        let into = since % BLINK;
        let next = if into < BLINK * 2.0 / 3.0 { BLINK * 2.0 / 3.0 - into } else { BLINK - into };
        Some(Duration::from_secs_f64(next / 1000.0))
    }

    /// Whether the caret shows now.
    fn caret_shows(&self) -> bool {
        let since = self.js.clock().now() - self.blink;
        self.active && (since >= BLINK_STOPS || since % BLINK < BLINK * 2.0 / 3.0)
    }

    /// Applies what the app's JS has posted, and tells the `settle`s whose time has come.
    pub fn fire(&mut self) {
        self.pump();
        self.tell_settled();
    }

    /// Applies what the app's JS has posted since it was last asked, all at once, as it's one turn's or more, and
    /// answers what it asked that the host answers at once. → whether anything came.
    pub fn pump(&mut self) -> bool {
        let posted = self.js.take();
        let came = !posted.is_empty() || self.started.is_none();
        if self.started.is_none() {
            self.started = self.js.started();
        }
        let appearance = self.theme.unwrap_or(self.appearance);
        for posted in posted {
            match posted {
                Posted::Threw(error) => self.threw(error),
                // What comes after what this host can't draw goes to the web view it opens in instead.
                Posted::Mutation(_) if self.mismatched.is_some() => {}
                Posted::Mutation(mutation) => match self.tree.apply([mutation]) {
                    Err(error) if error.is::<crate::tree::Mismatch>() => self.mismatched = Some(error.to_string()),
                    Err(error) => self.threw(format!("{error:#}")),
                    Ok(()) => self.changed(),
                },
                Posted::Message(message) => {
                    let (Ok(message), Some(asked)) = (serde_json::from_str::<Value>(&message), self.asked.as_mut()) else { continue };
                    let (id, method, tool) = (message.get("id").map(Value::to_string), message["method"].as_str().map(str::to_owned), message["params"]["name"].as_str().map(str::to_owned));
                    // What it copies, or shares, which a computer with no share sheet copies (`done`: "copied"), goes on
                    // the clipboard.
                    if let (Some(id), Some(asked @ ("one/copy" | "one/share")), Some(text)) = (message.get("id"), method.as_deref(), message["params"]["text"].as_str()) {
                        self.clipboard.write(false, text);
                        let result = if asked == "one/share" { json!({"done": "copied"}) } else { json!({}) };
                        self.js.send("message", json!({"jsonrpc": "2.0", "id": id, "result": result}));
                        continue;
                    }
                    match asked(message, appearance) {
                        // What only the window knows of where the page is, which it adds to its client's answer.
                        Some(mut answer) if method.as_deref() == Some("ui/initialize") && answer["result"].is_object() => {
                            answer["result"]["hostContext"]["containerDimensions"] = json!({"width": self.room.width, "height": self.room.height});
                            answer["result"]["hostContext"]["reducedMotion"] = json!(self.reduced_motion);
                            answer["result"]["hostContext"]["safeAreaInsets"] = self.safe_area();
                            answer["result"]["hostContext"]["visible"] = json!(self.visible);
                            self.told = Some((self.room, self.reduced_motion, self.insets));
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
        if came {
            // What says `autofocus` takes the keyboard as it says so, as an active menu item does.
            let wants: Vec<Id> = self.tree.autofocused().collect();
            self.autofocused.retain(|id| wants.contains(id));
            if let Some(id) = wants.into_iter().filter(|id| !self.autofocused.contains(id)).max() {
                self.autofocused.insert(id);
                self.focus(Some(id));
            }
            self.sync_fields();
        }
        // The inspector's answers wait for what they did to have run.
        if !self.replies.is_empty() && !self.js.busy() {
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
        let title_bar = [self.title_start, if self.title_start > 0 { 16 } else { 0 }];
        Conditions { dark: self.dark(), touch: self.touch, reduced_motion: self.reduced_motion, insets, title_bar, page: self.page, ..Conditions::default() }
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

    /// Paints the screen into `scene`, `size` logical pixels at `scale` device pixels each.
    pub fn paint(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
        // The page is told when its room, its motion or its insets change, as a frame's host tells an MCP App.
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
        let conditions = self.conditions();
        let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
        self.states.focused = self.focused;
        let now = self.js.clock().now();
        let caret = self.caret_shows();
        // A frame where nothing it shows has changed keeps the scene it drew, and costs nothing.
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let mut fields: Vec<_> = self.fields.iter().map(|(id, field)| (*id, field.display(), field.value(), field.placed())).collect();
        fields.sort_by_key(|(id, ..)| *id);
        let seen = Seen {
            size: (window.size, scale),
            conditions,
            states: self.states.clone(),
            scrolled: self.scrolled.clone(),
            fields,
            caret,
            hovering: (self.home.as_ref().is_some_and(hovering), self.action.as_ref().is_some_and(hovering)),
            notice: self.notice.as_ref().map(|notice| (notice.text.clone(), notice.action.as_ref().map(|(label, _)| label.clone()))),
            breadcrumb: self.breadcrumb.as_ref().map(|crumb| (crumb.name.clone(), crumb.start)),
            panel: self.panel.map(|panel| (panel.corner, panel.fill)),
            fetched: self.fetched.values().filter(|fetched| fetched.is_some()).count(),
        };
        if !self.animating && self.tree.unchanged() && self.seen.as_ref() == Some(&seen) {
            return;
        }
        self.seen = Some(seen);
        scene.clear();
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
        for id in changed.iter().chain(&moving).chain(&states).copied().chain(self.tree.inputs()) {
            let mut at = Some(id);
            while let Some(id) = at.filter(|id| below.insert(*id)) {
                at = self.tree.parent(id);
            }
        }
        self.frame += 1;
        let mut painter = Painter {
            tree: &self.tree,
            layout: &mut self.layout,
            changed: &changed,
            moving: &moving,
            below: &below,
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
            fields: &self.fields,
            drawn: &mut self.drawn,
            caret,
            images: &mut self.images,
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
            glass: self.panel.filter(|panel| !panel.fill).map(|_| Vec::new()),
        };
        // The window, as a page's body: its root's background and text, scrolling what's taller than it.
        let version2 = self.tree.styled();
        let (body, background) = if version2 { Text::root(conditions) } else { (Text::body(conditions), Some(colour("background", conditions.dark))) };
        match self.panel {
            // A panel's background is the host's: a popover's colour, or nothing over the system's glass, rounded.
            Some(panel) => {
                let fill = if panel.fill { colour("card", conditions.dark) } else { Hsla::default() };
                painter.quad(window, window, fill, Edges::default(), Hsla::default(), Corners::all(panel.corner), false, 1.0)
            }
            None => painter.quad(window, window, background.unwrap_or_default(), Edges::default(), Hsla::default(), Corners::default(), false, 1.0),
        }
        let root = painter.build(0, body, true);
        // What's no longer in the tree goes, unless it moved elsewhere in it.
        for id in std::mem::take(&mut painter.gone) {
            painter.layout.forget(id, painter.frame, painter.transitions);
        }
        painter.layout.place_fixed(painter.tree);
        let available = taffy::geometry::Size { width: AvailableSpace::Definite(window.size.width), height: AvailableSpace::Definite(window.size.height) };
        let Painter { layout, text, .. } = &mut painter;
        let measured = layout.taffy.compute_layout_with_measure(root, available, |input, _, leaf, _| {
            // What has nothing to measure is as big as its style says, or nothing.
            let Some(Leaf { measure: Some(measure), .. }) = leaf else {
                return taffy::LayoutOutput::from_outer_size(taffy::geometry::Size {
                    width: input.known_dimensions.width.unwrap_or(0.0),
                    height: input.known_dimensions.height.unwrap_or(0.0),
                });
            };
            let width = input.known_dimensions.width.or(match input.available_space.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent => Some(0.0),
                AvailableSpace::MaxContent => None,
            });
            let size = match measure {
                Measure::Text { words, runs, text: style, lines } => text.paragraph(words, &style.style, runs, width.filter(|_| style.wrap), *lines).size(),
                Measure::Input { words, text: style, multiline } => {
                    let paragraph = text.paragraph(words, &style.style, &[], multiline.then_some(width).flatten(), None);
                    Size { width: if *multiline { paragraph.size().width } else { 0.0 }, height: paragraph.size().height }
                }
            };
            taffy::LayoutOutput::from_outer_size(taffy::geometry::Size {
                width: input.known_dimensions.width.unwrap_or(size.width.ceil()),
                height: input.known_dimensions.height.unwrap_or(size.height),
            })
        });
        match measured {
            Err(error) => log::error!("its layout failed: {error}"),
            Ok(()) => painter.paint(root, point(0.0, 0.0), Transform::NONE, window, 1.0, None, true),
        }
        // The window's breadcrumb, over its page.
        let hovering = |bounds: &Bounds<f32>| self.pointer.is_some_and(|at| bounds.contains(&at));
        let crumb = self.breadcrumb.as_ref().map(|crumb| painter.breadcrumb(crumb, self.insets.top, self.home.as_ref().is_some_and(hovering)));
        let home = crumb.map(|(home, _)| home);
        // The page's top bar starts after it, which it's laid out again for once it's moved.
        let start = crumb.map_or(0, |(_, end)| (end + 12.0).ceil() as u16);
        if start != self.title_start {
            self.title_start = start;
            self.again = true;
        }
        self.action = self.notice.as_ref().and_then(|notice| painter.notice(notice, self.insets.top, self.action.as_ref().is_some_and(hovering)));
        self.glass = painter.glass.take().unwrap_or_default();
        let (placed, mut tab_order, rects, animating, modal, wanted, intersections) = (painter.placed, painter.tab_order, painter.rects, painter.animating, painter.modal, painter.wanted, painter.intersections);
        // What it wanted to draw that's by an address is fetched by its window, once.
        for src in wanted {
            if !self.fetched.contains_key(&src) {
                self.fetched.insert(src.clone(), None);
                if let Some(fetch) = self.fetch.as_mut() {
                    fetch(&src);
                }
            }
        }
        self.home = home;
        let moving = self.transitions.finish(now);
        self.placed = placed;
        // A modal dialog holds the keyboard: Tab goes only to what's in it, and it has the keyboard as it appears.
        // What's inert, or in what is, Tab passes over.
        if let Some(modal) = modal {
            tab_order.retain(|id| self.within(*id, modal));
        }
        tab_order.retain(|id| !self.tree.inert(*id));
        self.tab_order = tab_order;
        self.rects = rects;
        self.animating = animating || moving;
        self.painted = (window.size, scale);
        self.tell_settled();
        scene.finish();
        if let Some(modal) = modal
            && !self.focused.is_some_and(|focused| self.within(focused, modal))
        {
            self.focus(Some(modal));
            self.again = true;
        }
        // What moved under a pointer that's still is what it's over now, as a browser finds again after layout.
        if let Some(at) = self.pointer.filter(|_| !self.touch) {
            let before = self.states.hovered.clone();
            self.hover(at);
            self.again |= self.states.hovered != before;
        }
        // Where each node that listens for `layout` is, as its layout settles and whenever its size changes.
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

    /// The pointer's shape at `at`: what its style says, or a hand over what's pressed and a beam over an input.
    pub fn cursor(&self, at: Point<f32>) -> Cursor {
        if self.home.is_some_and(|home| home.contains(&at)) {
            return Cursor::Arrow;
        }
        if self.action.is_some_and(|action| action.contains(&at)) {
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
    fn pointer_event(&mut self, chain: &[Id], name: &str, at: Point<f32>, button: u8, delta: Point<f32>) {
        for &id in chain {
            if !self.tree.nodes.get(&id).is_some_and(|node| node.listens(name)) {
                continue;
            }
            let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
            let mut told = json!({
                "clientX": at.x, "clientY": at.y, "pointerId": 1, "pointerType": if self.touch { "touch" } else { "mouse" }, "button": button,
                "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            });
            if name == "wheel" {
                told["deltaX"] = json!(delta.x);
                told["deltaY"] = json!(delta.y);
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
            self.pointer_event(&[id], "pointerleave", at, 0, point(0.0, 0.0));
        }
        for id in entered {
            self.pointer_event(&[id], "pointerenter", at, 0, point(0.0, 0.0));
        }
        chain
    }

    /// The pointer moved to `at`: what it's over is hovered, told as it comes and goes, and a press it holds is told
    /// it moved.
    pub fn pointer_moved(&mut self, at: Point<f32>) {
        self.pointer = Some(at);
        let chain = self.hover(at);
        let target = match self.captured {
            Some(captured) => self.chain_from(captured),
            None => chain,
        };
        self.pointer_event(&target, "pointermove", at, 0, point(0.0, 0.0));
        // A press held in an input selects as it's dragged.
        if let Some(id) = self.selecting
            && let (Some(field), Some(drawn)) = (self.fields.get_mut(&id), self.drawn.get(&id))
        {
            let before = field.value();
            field.drag(drawn.index_at(at));
            self.edited(id, before);
        }
    }

    /// The pointer left the window: nothing's hovered.
    pub fn pointer_left(&mut self) {
        let at = self.pointer.take().unwrap_or(point(0.0, 0.0));
        for id in std::mem::take(&mut self.states.hovered).into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, point(0.0, 0.0));
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
        // The breadcrumb's Home is the window's, not its page's, and so is its notice's action.
        if self.home.is_some_and(|home| home.contains(&at)) {
            self.pressing_home = button == 0;
            return;
        }
        if self.action.is_some_and(|action| action.contains(&at)) {
            self.pressing_action = button == 0;
            return;
        }
        let chain = self.under(at);
        self.captured = chain.first().copied();
        self.pointer_event(&chain, "pointerdown", at, button, point(0.0, 0.0));
        if button == 2 {
            return self.pointer_event(&chain, "contextmenu", at, button, point(0.0, 0.0));
        }
        let pressed = self.pressed_in(&chain);
        let input = pressed.filter(|id| self.fields.contains_key(id));
        if button == 1 {
            if let Some(input) = input.filter(|id| self.fields.get(id).is_some_and(|field| !field.secure && !field.readonly)) {
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
        self.focus(pressed);
        if let Some(input) = input {
            // Presses close together in time and place count as a double or triple click, by GTK's defaults.
            let now = Instant::now();
            let clicks = match self.last_press {
                Some((when, was, clicks)) if now - when < Duration::from_millis(400) && (was.x - at.x).abs() < 5.0 && (was.y - at.y).abs() < 5.0 => (clicks + 1).min(3),
                _ => 1,
            };
            self.last_press = Some((now, at, clicks));
            self.selecting = Some(input);
            self.press_input(input, at, clicks, shift);
        }
    }

    /// Input `id` pressed at `at`, `clicks` times in a row, or with Shift held.
    fn press_input(&mut self, id: Id, at: Point<f32>, clicks: u8, shift: bool) {
        let index = self.drawn.get(&id).map_or(usize::MAX, |drawn| drawn.index_at(at));
        self.edit(id, |field| field.press(index.min(field.display().len()), clicks, shift));
    }

    /// A button came up at `at`: what it went down on is told, and with the main one a press, if it's still over
    /// what was pressed.
    pub fn pointer_up(&mut self, at: Point<f32>, button: u8) {
        if std::mem::take(&mut self.pressing_action) {
            if self.action.is_some_and(|action| action.contains(&at))
                && let Some((_, action)) = self.notice.as_mut().and_then(|notice| notice.action.as_mut())
            {
                action();
            }
            return;
        }
        if std::mem::take(&mut self.pressing_home) {
            if self.home.is_some_and(|home| home.contains(&at))
                && let Some(crumb) = self.breadcrumb.as_mut()
            {
                (crumb.home)();
            }
            return;
        }
        let target = match self.captured.take() {
            Some(captured) => self.chain_from(captured),
            None => self.under(at),
        };
        self.pointer_event(&target, "pointerup", at, button, point(0.0, 0.0));
        // What the pointer selected goes on the primary selection, as Linux's fields put it, never a password.
        if let Some(field) = self.selecting.take().and_then(|id| self.fields.get(&id))
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

    /// The wheel or a trackpad scrolled by `delta` pixels (down and right positive) at `at`: what's under it is told
    /// (`wheel`), and the innermost thing there that can still scroll that way does, telling its app if it listens.
    /// → what scrolled.
    pub fn scroll(&mut self, at: Point<f32>, delta: Point<f32>) -> Option<Id> {
        let chain = self.under(at);
        self.pointer_event(&chain, "wheel", at, 0, delta);
        self.pan(at, delta)
    }

    /// A finger (or a screen reader) moved what's at `at` by `delta` pixels (down and right positive): the innermost
    /// thing there that can still scroll that way does, telling its app if it listens. → what scrolled.
    pub fn pan(&mut self, at: Point<f32>, delta: Point<f32>) -> Option<Id> {
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

    /// Gives the keyboard to `id`, or to nothing, telling the app what had it and what has it, where they listen. An
    /// input that loses it keeps what was being composed in it.
    fn focus(&mut self, id: Option<Id>) {
        if self.focused == id {
            return;
        }
        self.changed();
        if let Some(input) = self.focused.filter(|focused| self.fields.contains_key(focused)) {
            self.edit(input, Field::blur);
        }
        self.blink = self.js.clock().now();
        let blurred = std::mem::replace(&mut self.focused, id);
        if let Some(blurred) = blurred.filter(|blurred| self.tree.nodes.get(blurred).is_some_and(|node| node.listens("blur"))) {
            self.event(blurred, "blur", None);
        }
        if let Some(id) = id.filter(|id| self.tree.nodes.get(id).is_some_and(|node| node.listens("focus"))) {
            self.event(id, "focus", None);
        }
    }

    /// A key went down where the keyboard is, as the DOM's `KeyboardEvent` has it: Tab moves the keyboard, what
    /// listens for `keydown` hears it, Enter and Space press what has it, and an input takes what's typed. → what had
    /// the keyboard.
    pub fn key(&mut self, key: &Key) -> Option<Id> {
        let focused = self.focused;
        // The key of the shortcut that just summoned it, still held or pressed again at once, is the shortcut's, not
        // the page's: it types nothing and presses nothing (`summoned`).
        if let Some((held, since)) = self.shortcut.take() {
            if key.key == held && since.elapsed() < SHORTCUT_HELD {
                self.shortcut = Some((held, std::time::Instant::now()));
                return focused;
            }
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
            self.states.focus_visible = true;
            let order = &self.tab_order;
            let next = match focused.and_then(|focused| order.iter().position(|id| *id == focused)) {
                Some(at) if key.shift => order.get((at + order.len() - 1) % order.len()).copied(),
                Some(at) => order.get((at + 1) % order.len()).copied(),
                None if key.shift => order.last().copied(),
                None => order.first().copied(),
            };
            self.focus(next);
            // What it comes to scrolls into view, as a browser's does, so Tab goes on through a long list.
            if let Some(next) = next {
                self.reveal(next);
            }
            // An input the keyboard comes to by Tab has its text selected, as a browser's and GTK's do.
            if let Some(next) = next.filter(|next| self.fields.contains_key(next)) {
                self.edit(next, Field::select_all);
            }
            return focused;
        }
        let id = focused?;
        // A key goes to what has the keyboard and to each node it's in that listens, the innermost first.
        for listener in self.chain_from(id) {
            if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                self.tell(listener, "keydown", key.told());
            }
        }
        // Escape empties a search field with something typed in it, as a browser's `type="search"` does.
        let search = self.tree.nodes.get(&id).is_some_and(|node| node.str("type") == Some("search"));
        if search && key.key == "Escape" && self.fields.get(&id).is_some_and(|field| !field.value().text.is_empty()) {
            // What's selected is replaced by nothing, as a cut is, which undo brings back.
            self.edit(id, |field| {
                field.select_all();
                field.paste("");
            });
        } else if self.fields.contains_key(&id) {
            self.type_key(id, key);
        } else if matches!(key.key.as_str(), "Enter" | " ") && self.tree.nodes.get(&id).is_some_and(|node| node.listens("click")) {
            self.click(id, None)
        }
        Some(id)
    }

    /// What a key does in input `id`: typing, moving, selecting, undoing and the clipboard, by the editing model
    /// (`editing::Field::key`), then its `input` if that changed it and its `action` if it was the action key.
    fn type_key(&mut self, id: Id, key: &Key) {
        let Some(field) = self.fields.get_mut(&id) else { return };
        let before = field.value();
        let outcome = field.key(key, self.drawn.get(&id).map(|drawn| drawn as &dyn editing::Layout));
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
        if let Some(id) = self.focused.filter(|id| self.fields.contains_key(id)) {
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
        // Shown again, it has the keyboard where it says `autofocus`, as a page opened does, from its first key on;
        // put away, nothing keeps it, so nothing left with it is pressed as it comes back.
        self.autofocused.clear();
        self.focus(if visible { self.tree.autofocused().max() } else { None });
        if visible {
            self.autofocused.extend(self.tree.autofocused());
        }
        self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"visible": visible}}));
    }

    /// It was summoned by a shortcut that ends with `key` (as the DOM's `KeyboardEvent.key` names it): that key, still
    /// held as it shows or pressed again right after, is the shortcut's (`key`).
    pub fn summoned(&mut self, key: Option<&str>) {
        self.shortcut = key.map(|key| (key.to_owned(), std::time::Instant::now()));
    }

    /// Its window got or lost the keyboard: the caret shows only while it has it, and an input losing it keeps what
    /// was being composed in it.
    pub fn activate(&mut self, active: bool) {
        self.active = active;
        self.blink = self.js.clock().now();
        if let Some(id) = self.focused.filter(|id| !active && self.fields.contains_key(id)) {
            self.edit(id, Field::finish_composition);
        }
    }

    /// Whether an input holds something its person typed that its app hasn't taken (`Field::typed`).
    pub fn typed(&self) -> bool {
        self.fields.values().any(Field::typed)
    }

    /// The editing state of the input with the keyboard, for a platform's keyboard bridge (`editing`).
    pub fn editing(&self) -> Option<editing::Value> {
        self.focused.and_then(|id| self.fields.get(&id)).map(Field::value)
    }

    /// The character of the input with the keyboard nearest to `at`, in the window, in UTF-16 units of its
    /// `editing` text, as AppKit's `characterIndexForPoint:` asks.
    pub fn index_at(&self, at: Point<f32>) -> Option<usize> {
        let id = self.focused?;
        let (field, drawn) = (self.fields.get(&id)?, self.drawn.get(&id)?);
        let index = drawn.index_at(at);
        // What's drawn is its editing text, what's composed in it too, but for a password's dots.
        Some(if field.secure { editing::utf16(&field.text, field.to_model(index)) } else { editing::utf16(&field.display(), index) })
    }

    /// Where the input with the keyboard has its caret, in the window, for the platform's input method to show what
    /// it composes beside: none where it takes no typing (a password takes no input method).
    pub fn caret_rect(&self) -> Option<Bounds<f32>> {
        let id = self.focused?;
        let field = self.fields.get(&id).filter(|field| !field.secure && !field.readonly)?;
        let drawn = self.drawn.get(&id)?;
        let (line, x) = field.placed().unwrap_or_else(|| editing::Layout::caret(drawn, field.caret()));
        let height = drawn.paragraph.line_height * drawn.grown;
        Some(Bounds { origin: point(drawn.origin.x + (x - drawn.scroll.x) * drawn.grown, drawn.origin.y + line as f32 * height), size: Size { width: drawn.grown, height } })
    }

    /// Its title row, where its window has one over its page (a unified toolbar): how tall it is, and where in it
    /// something takes a press, which the rest of it doesn't, so the window moves held there, as a title bar does.
    pub fn title_row(&self) -> Option<TitleRow> {
        self.breadcrumb.as_ref()?;
        let height = self.insets.top;
        let row = |bounds: &Bounds<f32>| bounds.origin.y < height;
        let mut taken: Vec<Bounds<f32>> =
            self.placed.iter().filter(|(_, placed)| (placed.pressable || placed.input) && row(&placed.shown)).map(|(_, placed)| placed.shown).collect();
        taken.extend(self.home);
        taken.extend(self.action);
        Some(TitleRow { height, taken })
    }

    /// Does `change` to input `id`'s editing, and tells its app what changed.
    fn edit(&mut self, id: Id, change: impl FnOnce(&mut Field)) {
        let Some(field) = self.fields.get_mut(&id) else { return };
        let before = field.value();
        change(field);
        self.edited(id, before);
    }

    /// Input `id`'s editing changed from `before`, or perhaps not: its caret shows again, and its app hears its
    /// editing state (`input`) if it's different, with its text as `value` too, which version 1 said alone.
    fn edited(&mut self, id: Id, before: editing::Value) {
        self.blink = self.js.clock().now();
        let Some(value) = self.fields.get(&id).map(Field::value).filter(|value| *value != before) else { return };
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("input")) {
            self.tell(id, "input", json!({"editing": value, "value": value.text}));
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
        self.fields.retain(|id, _| input(id));
        self.set.retain(|id, _| input(id));
        self.drawn.retain(|id, _| input(id));
        for id in tree.inputs() {
            let node = &tree.nodes[&id];
            let field = self.fields.entry(id).or_default();
            field.multiline = node.is("multiline");
            field.newline = field.multiline && node.str("action").is_none_or(|action| action == "newline");
            field.secure = node.str("type") == Some("password");
            field.readonly = node.is("readonly");
            // Version 1 said only its text, as `value`, the caret at its end.
            let set = match node.prop("editing") {
                Some(editing) => serde_json::from_value::<editing::Value>(editing.clone()).ok(),
                None => node.prop("value").map(|_| {
                    let text = value_of(node);
                    let end = editing::utf16(&text, text.len());
                    editing::Value { text, selection: editing::Selection { base: end, extent: end }, composing: None }
                }),
            };
            if let Some(set) = set.filter(|set| self.set.get(&id) != Some(set)) {
                field.set(&set);
                self.set.insert(id, set);
            }
        }
    }
}

/// Paints a screen that says only `words`, in the middle, in the muted colour, as a page's empty state does.
pub(crate) fn paint_message(words: &str, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &Atlas, scene: &mut Scene) {
    scene.clear();
    let dark = appearance == Appearance::Dark;
    let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
    let mask = ContentMask { bounds: window.scale_to(scale) };
    scene.insert_primitive(Quad {
        order: 0,
        border_style: Default::default(),
        bounds: window.scale_to(scale),
        content_mask: mask,
        background: Background::from(colour("background", dark)),
        border_color: Hsla::default(),
        corner_radii: Corners::default(),
        border_widths: Edges::default(),
        transformation: TransformationMatrix::unit(),
    });
    // A host's own message, in a computer's size: it has no window's conditions.
    let body = Text::body(Conditions { dark, ..Conditions::default() });
    let width = (window.size.width - 32.0).max(1.0);
    let paragraph = text.paragraph(words, &body.style, &[], Some(width), None);
    let top = (window.size.height - paragraph.size().height) / 2.0;
    let muted = colour("muted-foreground", dark);
    let _ = text.paint(&paragraph, point(px(16.0), px(top)), width, 0.5, &|_| muted, scale, mask, TransformationMatrix::unit(), atlas, scene);
    scene.finish();
}

fn value_of(node: &Node) -> String {
    match node.prop("value") {
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

/// What a leaf measures for layout.
#[derive(PartialEq)]
enum Measure {
    /// A text's words, and its runs where texts in it set some of them another way.
    Text { words: String, runs: Vec<TextRun>, text: Text, lines: Option<usize> },
    Input { words: String, text: Text, multiline: bool },
}

/// A tree node as laid out: which node, the text it inherits, how it looks, and what it measures; and a text's words
/// as they were last wrapped, to the width they were wrapped to (`None`, as wide as they are), kept until it's styled
/// again.
struct Leaf {
    id: Id,
    text: Text,
    look: Look,
    line_clamp: Option<usize>,
    positioned: bool,
    /// Placed in the window, not in its parent (`position: fixed`).
    fixed: bool,
    measure: Option<Measure>,
    paragraph: Option<(Option<f32>, Arc<Paragraph>)>,
    /// Where each text and raw text inside a text is in its words, to be pressed, hovered, reached and read there.
    spans: Vec<Span>,
}

/// A text or raw text inside a text, by where its words are in the text's (bytes `start..end`), with its pointer's
/// shape and whether it takes the pointer, as its style says.
#[derive(Clone)]
struct Span {
    id: Id,
    start: usize,
    end: usize,
    cursor: Option<Cursor>,
    pointer: Option<bool>,
}

/// A picture's size in device pixels, as the atlas holds it: its pixels are the atlas's alone once it has them.
#[derive(Clone, Copy)]
struct Picture {
    width: u32,
    height: u32,
}

/// A picture, decoded, in the atlas's byte order (BGRA).
struct Decoded {
    picture: Picture,
    bgra: Vec<u8>,
}

/// Where a node's layout is drawn in the window: moved by `offset` and grown by `scale`, as its translates and
/// scales and its parents' say.
#[derive(Clone, Copy)]
struct Transform {
    offset: Point<f32>,
    scale: f32,
}

impl Transform {
    const NONE: Transform = Transform { offset: Point { x: 0.0, y: 0.0 }, scale: 1.0 };

    fn point(&self, at: Point<f32>) -> Point<f32> {
        point(self.offset.x + at.x * self.scale, self.offset.y + at.y * self.scale)
    }

    fn bounds(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        Bounds { origin: self.point(bounds.origin), size: Size { width: bounds.size.width * self.scale, height: bounds.size.height * self.scale } }
    }

    /// This, then a node's own move and growth around its middle, `centre`.
    fn then(&self, translate: Point<f32>, scale: f32, centre: Point<f32>) -> Transform {
        // p ↦ centre + translate + (p − centre)·scale, then this.
        let moved = point(centre.x + translate.x - centre.x * scale, centre.y + translate.y - centre.y * scale);
        Transform { offset: self.point(moved), scale: self.scale * scale }
    }
}

/// What the painter keeps of each node it paints, for the pointer and the keyboard.
pub(crate) struct Placed {
    pub parent: Option<Id>,
    /// Where it shows, clipped, in the window.
    pub shown: Bounds<f32>,
    /// Whether it takes the pointer (`pointerEvents`): what doesn't is still in the chain of what's in it.
    pub pointer: bool,
    pub cursor: Option<Cursor>,
    pub pressable: bool,
    pub input: bool,
    /// It scrolls: how far it can, down and along, and whether along is its way.
    pub scroll: Option<(f32, f32, bool)>,
}

/// The tree as Taffy lays it out, kept from frame to frame, as GPUI's and Flutter's are: a frame styles again only what
/// changed (a mutation, a state, what it inherits, a transition), tells Taffy only what's different, and Taffy lays out
/// again only what that touches, from its cache; a change of style that doesn't move anything lays out nothing again.
struct Retained {
    taffy: TaffyTree<Leaf>,
    kept: HashMap<Id, Kept>,
    /// What's placed in the window (`position: fixed`), laid out as the root's children after its own.
    fixed: HashSet<Id>,
    /// The conditions it was styled in: when they change, everything's styled again.
    conditions: Option<Conditions>,
}

/// A node as it was last built: its Taffy node, what its style was resolved from, its children in the tree, those of
/// them laid out in it (all but what's fixed), and the frame it was last seen in.
struct Kept {
    node: NodeId,
    inherited: Text,
    state: State,
    children: Vec<Id>,
    inside: Vec<NodeId>,
    frame: u64,
}

impl Default for Retained {
    fn default() -> Retained {
        Retained { taffy: TaffyTree::new(), kept: HashMap::new(), fixed: HashSet::new(), conditions: None }
    }
}

impl Retained {
    fn leaf(&self, node: NodeId) -> &Leaf {
        self.taffy.get_node_context(node).expect("every node has its leaf")
    }

    /// `id` was taken out of what it was in: unless it was built elsewhere in `frame`, it and what's in it go.
    fn forget(&mut self, id: Id, frame: u64, transitions: &mut transition::Transitions) {
        if self.kept.get(&id).is_none_or(|kept| kept.frame == frame) {
            return;
        }
        let kept = self.kept.remove(&id).expect("it's kept");
        let _ = self.taffy.remove(kept.node);
        self.fixed.remove(&id);
        transitions.forget(id);
        for child in kept.children {
            self.forget(child, frame, transitions);
        }
    }

    /// Lays out what's fixed in the window, as the root's children after its own, each after what's in it and in the
    /// tree's order, as they're found.
    fn place_fixed(&mut self, tree: &Tree) {
        let Some(root) = self.kept.get(&0) else { return };
        let mut children = root.inside.clone();
        if !self.fixed.is_empty() {
            // Where each is in the tree, as the index of each node on the way to it.
            let path = |id: Id| {
                let mut path = Vec::new();
                let mut at = id;
                while let Some(parent) = tree.parent(at) {
                    path.push(tree.nodes[&parent].children.iter().position(|child| *child == at).unwrap_or(0));
                    at = parent;
                }
                path.reverse();
                path
            };
            let mut fixed: Vec<(Vec<usize>, NodeId)> = self.fixed.iter().filter_map(|id| Some((path(*id), self.kept.get(id)?.node))).collect();
            // What's in a node comes before it.
            fixed.sort_by(|(a, _), (b, _)| match a.iter().zip(b).find(|(a, b)| a != b) {
                Some((a, b)) => a.cmp(b),
                None => b.len().cmp(&a.len()),
            });
            children.extend(fixed.into_iter().map(|(_, node)| node));
        }
        if self.taffy.children(root.node).is_ok_and(|now| now != children) {
            self.taffy.set_children(root.node, &children).expect("its children are Taffy's");
        }
    }
}

struct Painter<'a> {
    tree: &'a Tree,
    layout: &'a mut Retained,
    /// What's styled again this frame (`Screen::paint`): what changed, what's moving, what's in or around either
    /// (`below`), or all of it, where the conditions changed; the frame; and what was taken out of what it was in.
    changed: &'a HashSet<Id>,
    moving: &'a HashSet<Id>,
    below: &'a HashSet<Id>,
    everything: bool,
    frame: u64,
    gone: Vec<Id>,
    text: &'a mut TextSystem,
    atlas: &'a Atlas,
    scene: &'a mut Scene,
    scale: f32,
    conditions: Conditions,
    states: &'a States,
    placed: Vec<(Id, Placed)>,
    tab_order: Vec<Id>,
    scrolled: &'a mut HashMap<Id, Point<f32>>,
    fields: &'a HashMap<Id, Field>,
    drawn: &'a mut HashMap<Id, Drawn>,
    /// The caret shows in this frame, as it blinks.
    caret: bool,
    images: &'a mut HashMap<u64, Option<Picture>>,
    /// Pictures by address, fetched, and those this frame wanted that weren't yet.
    fetched: &'a HashMap<String, Option<String>>,
    wanted: Vec<String>,
    rects: Rects,
    /// The time on the screen's clock, in milliseconds, for what moves by itself (`animationName`) and what's on its
    /// way somewhere (`transitions`), and whether anything did.
    time: f64,
    /// When what changed in this frame changed, which what it sets moving sets off from.
    since: f64,
    transitions: &'a mut transition::Transitions,
    animating: bool,
    /// Whether each node listening for `intersection` is near the window: within half the window's size of what shows
    /// of it, as an `IntersectionObserver` with a `rootMargin` and `scrollMargin` of 50% says; and where what's painted
    /// now is near the window, what clips it cut to it and what scrolls it grown as the window is.
    intersections: Vec<(Id, bool)>,
    near: Bounds<f32>,
    /// The window, where what's fixed is placed.
    window: Bounds<f32>,
    /// The topmost `aria-modal` dialog drawn, which holds the keyboard.
    modal: Option<Id>,
    /// How what's painted now is turned, in the window: the `rotate`s of what it's in.
    turn: TransformationMatrix,
    /// Where the system's own material goes under the page, as a panel's glass on a Mac, if the host has one: what has
    /// a material that's the system's is drawn on it rather than blurring what the page drew (`Glass`).
    glass: Option<Vec<crate::platform::Glass>>,
}

/// Everything outside the tree that a frame shows: its size and scale, the conditions, each node's state, how far
/// each scroll is scrolled, each input's text, caret and selection and whether the caret's on, what the window says
/// over its page and where the pointer is on it, and how many pictures have come.
#[derive(PartialEq)]
struct Seen {
    size: (Size<f32>, f32),
    conditions: Conditions,
    states: States,
    scrolled: HashMap<Id, Point<f32>>,
    fields: Vec<(Id, String, editing::Value, Option<(usize, f32)>)>,
    caret: bool,
    hovering: (bool, bool),
    notice: Option<(String, Option<String>)>,
    breadcrumb: Option<(String, f32)>,
    panel: Option<(f32, bool)>,
    fetched: usize,
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
}

impl States {
    /// What's in a different state from in `was`.
    fn changed(&self, was: &States) -> Vec<Id> {
        let mut changed: Vec<Id> = self.hovered.iter().filter(|id| !was.hovered.contains(id)).chain(was.hovered.iter().filter(|id| !self.hovered.contains(id))).copied().collect();
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
            hover: self.hovered.contains(&id),
            pressed: self.pressed == Some(id),
            focused: self.focused == Some(id) && (self.focus_visible || input),
            entering: false,
        }
    }
}

trait Scale {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels>;
}
impl Scale for Bounds<f32> {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: Point { x: ScaledPixels(self.origin.x * factor), y: ScaledPixels(self.origin.y * factor) },
            size: Size { width: ScaledPixels(self.size.width * factor), height: ScaledPixels(self.size.height * factor) },
        }
    }
}

/// A corner's radius in pixels, for a box of `size`, no more than half its shorter side, as CSS draws it.
fn corner(radius: Radius, size: Size<f32>) -> f32 {
    let px = match radius {
        Radius::Px(px) => px,
        Radius::Percent(percent) => size.width.min(size.height) * percent / 100.0,
    };
    px.min(size.width / 2.0).min(size.height / 2.0).max(0.0)
}

impl Painter<'_> {
    /// `id` and what's in it as Taffy's nodes, styled again only where it or what it inherits changed, and gone into
    /// only where something in it did. The root fills the window and scrolls, as a page's body.
    fn build(&mut self, id: Id, inherited: Text, root: bool) -> NodeId {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let state = self.states.of(id, node.kind == Kind::Input);
        let restyle = match self.layout.kept.get_mut(&id) {
            None => true,
            Some(kept) => {
                kept.frame = self.frame;
                let restyle = self.everything
                    || kept.inherited != inherited
                    || kept.state != state
                    || node.kind == Kind::Input
                    || self.changed.contains(&id)
                    || self.moving.contains(&id);
                if !restyle && !self.below.contains(&id) {
                    return kept.node;
                }
                restyle
            }
        };
        let text = match restyle {
            true => self.restyle(id, inherited, state, root),
            false => self.layout.leaf(self.layout.kept[&id].node).text,
        };
        if matches!(node.kind, Kind::Box | Kind::Scroll) {
            let children: Vec<Id> = node.children.iter().copied().filter(|child| tree.nodes.contains_key(child)).collect();
            let built: Vec<NodeId> = children.iter().map(|child| self.build(*child, text, false)).collect();
            // What's fixed is laid out in the window, as the root's (`Retained::place_fixed`).
            let inside: Vec<NodeId> = built.into_iter().filter(|child| !self.layout.leaf(*child).fixed).collect();
            let kept = self.layout.kept.get_mut(&id).expect("it's built");
            if kept.children != children {
                let now: HashSet<&Id> = children.iter().collect();
                self.gone.extend(kept.children.iter().filter(|child| !now.contains(child)));
                kept.children = children;
            }
            if kept.inside != inside {
                if !root {
                    self.layout.taffy.set_children(kept.node, &inside).expect("its children are Taffy's");
                }
                kept.inside = inside;
            }
        }
        self.layout.kept[&id].node
    }

    /// Styles `id` again, in `state`, inheriting `inherited`, telling Taffy only what's different. → the text it
    /// passes on.
    fn restyle(&mut self, id: Id, inherited: Text, state: State, root: bool) -> Text {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let mut computed = style::of(node, state, self.conditions);
        // What's just been added starts from its `entering` state, to move from.
        let entering = (computed.transition.is_some() && self.transitions.entering(id)).then(|| style::of(node, State { entering: true, ..state }, self.conditions));
        self.transitions.apply(id, &mut computed, entering.as_ref(), self.time, self.since, self.conditions.reduced_motion);
        let mut layout = computed.layout.clone();
        let mut look = computed.look.clone();
        // A version 1 page's `Text` with no type style is body text; in a version 2 page it inherits what it's in, as
        // CSS does, whatever style object it has.
        let text = match &node.styles {
            Some(_) => inherited.with(&computed),
            None => inherited.of(&node.style, node.kind == Kind::Text && !tree.styled(), self.conditions),
        };
        let level = node.prop("aria-level").and_then(|level| level.as_u64().or_else(|| level.as_str()?.parse().ok())).unwrap_or(2);
        let text = match node.kind == Kind::Text && node.styles.is_none() && node.str("role") == Some("heading") && level <= 3 {
            true => text.heading(node.style.font.is_some(), self.conditions),
            false => text,
        };
        if root {
            // The window, which places what's fixed, and what's absolute with nothing positioned around it.
            layout.position = taffy::style::Position::Relative;
            layout.size = taffy::geometry::Size { width: taffy::style::Dimension::percent(1.0), height: taffy::style::Dimension::percent(1.0) };
            layout.overflow = taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll };
            look.scrolls = true;
        }
        if node.kind == Kind::Scroll {
            let along = matches!(layout.flex_direction, taffy::style::FlexDirection::Row | taffy::style::FlexDirection::RowReverse);
            layout.overflow = if along {
                taffy::geometry::Point { x: taffy::style::Overflow::Scroll, y: taffy::style::Overflow::Hidden }
            } else {
                taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll }
            };
            look.clips = true;
            look.scrolls = true;
        }
        layout.scrollbar_width = 0.0;
        let lines = computed.line_clamp.or(node.style.line_clamp);
        let mut spans = Vec::new();
        let measure = match node.kind {
            Kind::Text => {
                let (mut words, mut runs) = (String::new(), Vec::new());
                self.runs(id, text, None, &mut words, &mut runs, &mut spans);
                // One run is the text's own style, which needs none.
                if runs.len() == 1 && runs[0].style == text.style && runs[0].color == text.color && runs[0].background_color.is_none() && runs[0].underline == text.underline {
                    runs.clear();
                }
                Some(Measure::Text { words, runs, text, lines })
            }
            Kind::RawText => Some(Measure::Text { words: node.value.clone().unwrap_or_default(), runs: Vec::new(), text, lines: None }),
            Kind::Input => {
                let words = self.fields.get(&id).map(Field::display).unwrap_or_default();
                let words = if words.is_empty() { node.str("placeholder").unwrap_or_default().to_owned() } else { words };
                Some(Measure::Input { words: if words.is_empty() { " ".into() } else { words }, text, multiline: node.is("multiline") })
            }
            Kind::Image | Kind::Path | Kind::Picker | Kind::Box | Kind::Scroll => None,
        };
        let positioned = layout.position == taffy::style::Position::Absolute;
        let leaf = Leaf { id, text, look, line_clamp: lines, positioned, fixed: computed.fixed, measure, paragraph: None, spans };
        let taffy = &mut self.layout.taffy;
        match self.layout.kept.get_mut(&id) {
            Some(kept) => {
                if taffy.style(kept.node).is_ok_and(|was| *was != layout) {
                    taffy.set_style(kept.node, layout).expect("taffy takes any style");
                }
                let was = taffy.get_node_context_mut(kept.node).expect("every node has its leaf");
                let measured = was.measure != leaf.measure;
                *was = leaf;
                if measured {
                    taffy.mark_dirty(kept.node).expect("it's Taffy's");
                }
                (kept.inherited, kept.state) = (inherited, state);
            }
            None => {
                let node = taffy.new_leaf_with_context(layout, leaf).expect("taffy takes any style");
                let kept = Kept { node, inherited, state, children: Vec::new(), inside: Vec::new(), frame: self.frame };
                self.layout.kept.insert(id, kept);
            }
        }
        if computed.fixed {
            self.layout.fixed.insert(id);
        } else {
            self.layout.fixed.remove(&id);
        }
        text
    }

    /// What's in text `id`, set as each text in it says, as CSS inherits text properties into an inline: its words, its
    /// runs, and its texts' and raw texts' spans (outer before inner), with `behind` what's behind its words.
    fn runs(&self, id: Id, text: Text, behind: Option<Hsla>, words: &mut String, runs: &mut Vec<TextRun>, spans: &mut Vec<Span>) {
        let tree = self.tree;
        let Some(node) = tree.nodes.get(&id) else { return };
        for child in &node.children {
            let Some(inner) = tree.nodes.get(child).filter(|inner| !inner.hidden()) else { continue };
            let start = words.len();
            match inner.kind {
                Kind::RawText => {
                    words.push_str(inner.value.as_deref().unwrap_or_default());
                    let run = TextRun { len: words.len() - start, style: text.style, color: text.color, background_color: behind, underline: text.underline };
                    match runs.last_mut() {
                        Some(last) if TextRun { len: last.len, ..run } == *last => last.len += run.len,
                        _ if run.len > 0 => runs.push(run),
                        _ => {}
                    }
                    spans.push(Span { id: *child, start, end: words.len(), cursor: None, pointer: None });
                }
                Kind::Text => {
                    let computed = style::of(inner, self.states.of(*child, false), self.conditions);
                    let set = match &inner.styles {
                        Some(_) => text.with(&computed),
                        None => text.of(&inner.style, false, self.conditions),
                    };
                    let at = spans.len();
                    spans.push(Span { id: *child, start, end: start, cursor: computed.look.cursor, pointer: computed.look.pointer_events });
                    self.runs(*child, set, computed.look.background.or(behind), words, runs, spans);
                    spans[at].end = words.len();
                }
                _ => self.runs(*child, text, behind, words, runs, spans),
            }
        }
    }

    /// A quad, its bounds and mask in the window.
    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, bounds: Bounds<f32>, mask: Bounds<f32>, background: Hsla, borders: Edges<f32>, border_color: Hsla, radii: Corners<f32>, dashed: bool, opacity: f32) {
        let scale = self.scale;
        self.scene.insert_primitive(Quad {
            order: 0,
            border_style: if dashed { BorderStyle::Dashed } else { BorderStyle::Solid },
            bounds: bounds.scale_to(scale),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            background: Background::from(background.opacity(opacity)),
            border_color: border_color.opacity(opacity),
            corner_radii: Corners {
                top_left: ScaledPixels(radii.top_left * scale),
                top_right: ScaledPixels(radii.top_right * scale),
                bottom_right: ScaledPixels(radii.bottom_right * scale),
                bottom_left: ScaledPixels(radii.bottom_left * scale),
            },
            border_widths: Edges {
                top: ScaledPixels(borders.top * scale),
                right: ScaledPixels(borders.right * scale),
                bottom: ScaledPixels(borders.bottom * scale),
                left: ScaledPixels(borders.left * scale),
            },
            transformation: self.turn.scaled(scale),
        });
    }

    /// A box's shadow, as CSS's `box-shadow` draws it: its box moved by its offset and grown by its spread, blurred,
    /// under the box; or an inset one inside it, where the box moved and shrunk by its spread doesn't cover it.
    fn shadow(&mut self, shadow: &crate::tokens::Shadow, bounds: Bounds<f32>, radii: Corners<f32>, grown: f32, mask: Bounds<f32>, opacity: f32) {
        let spread = if shadow.inset { -shadow.spread } else { shadow.spread } * grown;
        let shadow_bounds = Bounds {
            origin: point(bounds.origin.x + shadow.x * grown - spread, bounds.origin.y + shadow.y * grown - spread),
            size: Size { width: (bounds.size.width + 2.0 * spread).max(0.0), height: (bounds.size.height + 2.0 * spread).max(0.0) },
        };
        let scale = self.scale;
        let corners = |radii: Corners<f32>, by: f32| Corners {
            top_left: ScaledPixels((radii.top_left + by).max(0.0) * scale),
            top_right: ScaledPixels((radii.top_right + by).max(0.0) * scale),
            bottom_right: ScaledPixels((radii.bottom_right + by).max(0.0) * scale),
            bottom_left: ScaledPixels((radii.bottom_left + by).max(0.0) * scale),
        };
        self.scene.insert_primitive(Shadow {
            order: 0,
            blur_radius: ScaledPixels(shadow.blur * grown * scale / 2.0),
            bounds: shadow_bounds.scale_to(scale),
            corner_radii: corners(radii, spread),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            color: Hsla::from(shadow.colour).opacity(opacity),
            transformation: self.turn.scaled(scale),
            inset: u32::from(shadow.inset),
            pad: 0,
            outer: bounds.scale_to(scale),
            outer_radii: corners(radii, 0.0),
        });
    }

    /// `bounds` grown by half the window's size on each side, as near it as an `IntersectionObserver`'s margin of 50%.
    fn grown(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        let (x, y) = (self.window.size.width / 2.0, self.window.size.height / 2.0);
        Bounds { origin: point(bounds.origin.x - x, bounds.origin.y - y), size: Size { width: bounds.size.width + 2.0 * x, height: bounds.size.height + 2.0 * y } }
    }

    /// The box `bounds` covers in the window once it's turned as what's painted now is: itself, where nothing's turned.
    fn turned(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        if self.turn == TransformationMatrix::unit() {
            return bounds;
        }
        let (left, top, right, bottom) = (bounds.origin.x, bounds.origin.y, bounds.origin.x + bounds.size.width, bounds.origin.y + bounds.size.height);
        let corners = [[left, top], [right, top], [left, bottom], [right, bottom]].map(|corner| self.turn.apply(corner));
        let (mut low, mut high) = (corners[0], corners[0]);
        for [x, y] in corners {
            low = [low[0].min(x), low[1].min(y)];
            high = [high[0].max(x), high[1].max(y)];
        }
        Bounds { origin: point(low[0], low[1]), size: Size { width: high[0] - low[0], height: high[1] - low[1] } }
    }

    /// Paints `node` with its parent's top left at `offset` in its layout, drawn through `transform` and turned as
    /// what it's in is, clipped to `mask` (in the window), `opacity` as opaque as it shows, taking the pointer if
    /// `pointer`.
    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let turn = self.turn;
        self.paint_node(taffy_node, offset, transform, mask, opacity, parent, pointer);
        self.turn = turn;
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_node(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let Ok(laid) = self.layout.taffy.layout(taffy_node).copied() else { return };
        // What isn't displayed isn't drawn, and nothing in it is.
        if self.layout.taffy.style(taffy_node).is_ok_and(|style| style.display == taffy::style::Display::None) {
            return;
        }
        let Some(leaf) = self.layout.taffy.get_node_context(taffy_node) else { return };
        let (id, text, look, lines) = (leaf.id, leaf.text, leaf.look.clone(), leaf.line_clamp);
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let laid_out = Bounds { origin: point(offset.x + laid.location.x, offset.y + laid.location.y), size: Size { width: laid.size.width, height: laid.size.height } };
        // Its own move and growth, around its middle, which moves nothing around it.
        let centre = point(laid_out.origin.x + laid_out.size.width / 2.0, laid_out.origin.y + laid_out.size.height / 2.0);
        let translate = point(look.translate.0.of(laid_out.size.width), look.translate.1.of(laid_out.size.height));
        let transform = if translate == point(0.0, 0.0) && look.scale == 1.0 { transform } else { transform.then(translate, look.scale, centre) };
        let bounds = transform.bounds(laid_out);
        let mut opacity = opacity * look.opacity;
        let mut rotate = look.rotate;
        if let Some((name, duration)) = &look.animation {
            self.animating = true;
            let duration = f64::from(duration.max(1.0));
            let turn = ((self.time % duration) / duration) as f32;
            match name.as_str() {
                "spin" => rotate += turn * 360.0,
                _ => opacity *= 1.0 - 0.25 * (1.0 - (turn * std::f32::consts::TAU).cos()),
            }
        }
        // Turned around its middle, with what's in it.
        if rotate.rem_euclid(360.0) != 0.0 {
            let middle = transform.point(centre);
            self.turn = TransformationMatrix::rotation(rotate, [middle.x, middle.y]).then(&self.turn);
        }
        let content = transform.bounds(Bounds {
            origin: point(laid_out.origin.x + laid.border.left + laid.padding.left, laid_out.origin.y + laid.border.top + laid.padding.top),
            size: Size { width: laid.content_box_width(), height: laid.content_box_height() },
        });
        let shown = self.turned(bounds).intersect(&mask);
        let root = id == 0;
        // What's inert takes no pointer, nor does what's in it.
        let pointer = !node.is("inert") && look.pointer_events.unwrap_or(pointer);
        if node.str("aria-modal") == Some("true") {
            self.modal = Some(id);
        }
        let drawn = self.turned(bounds);
        self.rects.insert(id, Rect { x: drawn.origin.x, y: drawn.origin.y, width: drawn.size.width, height: drawn.size.height });
        if node.listens("intersection") {
            self.intersections.push((id, drawn.intersects(&self.near)));
        }
        let radii = Corners {
            top_left: corner(look.radii.top_left, bounds.size),
            top_right: corner(look.radii.top_right, bounds.size),
            bottom_right: corner(look.radii.bottom_right, bounds.size),
            bottom_left: corner(look.radii.bottom_left, bounds.size),
        };
        let scaled = |edges: Edges<f32>| Edges { top: edges.top * transform.scale, right: edges.right * transform.scale, bottom: edges.bottom * transform.scale, left: edges.left * transform.scale };
        // Shadows under it, the first nearest, then what shows through it, its fill and border, then shadows inside.
        if !root {
            for shadow in look.shadows.iter().rev().filter(|shadow| !shadow.inset) {
                self.shadow(shadow, bounds, radii, transform.scale, mask, opacity);
            }
            if let (Some((_, _, true)), Some(glass)) = (look.material, self.glass.as_mut()) {
                if opacity > 0.0 {
                    glass.push(crate::platform::Glass { bounds: bounds.intersect(&mask), radius: radii.top_left, opacity });
                }
            } else if let Some((blur, saturate, _)) = look.material {
                self.scene.insert_primitive(Backdrop {
                    order: 0,
                    pad: 0,
                    blur: ScaledPixels(blur * transform.scale * self.scale),
                    saturation: saturate,
                    bounds: bounds.scale_to(self.scale),
                    content_mask: ContentMask { bounds: mask.scale_to(self.scale) },
                    corner_radii: Corners {
                        top_left: ScaledPixels(radii.top_left * self.scale),
                        top_right: ScaledPixels(radii.top_right * self.scale),
                        bottom_right: ScaledPixels(radii.bottom_right * self.scale),
                        bottom_left: ScaledPixels(radii.bottom_left * self.scale),
                    },
                });
            }
            if look.background.is_some() || look.borders != Edges::default() {
                self.quad(bounds, mask, look.background.unwrap_or_default(), scaled(look.borders), look.border_color, radii, look.dashed, opacity);
            }
            for shadow in look.shadows.iter().rev().filter(|shadow| shadow.inset) {
                self.shadow(shadow, bounds, radii, transform.scale, mask, opacity);
            }
        }
        let pressable = (node.listens("click") || node.kind == Kind::Input) && !node.is("disabled") && !node.is("aria-disabled");
        // Tab reaches what the DOM's does: an input, what's pressed as a button is (a role the DOM draws as a
        // `<button>`, core/ui's strict/primitives.tsx `pressed`), and what says its `tabindex`.
        let tabindex = node.prop("tabindex").and_then(|index| index.as_i64().or_else(|| index.as_str()?.parse().ok()));
        // A text is never a `<button>` on the DOM, whatever its role: it's reached only by its `tabindex`.
        let button = node.kind != Kind::Text && node.listens("click") && node.str("role").is_some_and(|role| BUTTON_ROLES.contains(&role));
        if pressable && tabindex.is_none_or(|index| index >= 0) && (node.kind == Kind::Input || button || tabindex.is_some()) {
            self.tab_order.push(id);
        }
        let mut placed = Placed { parent, shown, pointer, cursor: look.cursor, pressable, input: node.kind == Kind::Input, scroll: None };
        match node.kind {
            Kind::Text | Kind::RawText => {
                let width = laid.content_box_width().max(1.0);
                let wrap = text.wrap.then_some(width);
                let kept = self.layout.leaf(taffy_node).paragraph.as_ref().filter(|(wrapped, _)| *wrapped == wrap).map(|(_, paragraph)| paragraph.clone());
                let (runs, spans) = match &self.layout.leaf(taffy_node).measure {
                    Some(Measure::Text { runs, .. }) => (runs.clone(), self.layout.leaf(taffy_node).spans.clone()),
                    _ => (Vec::new(), Vec::new()),
                };
                let paragraph = kept.unwrap_or_else(|| {
                    let words = match &self.layout.leaf(taffy_node).measure {
                        Some(Measure::Text { words, .. }) => words.clone(),
                        _ => String::new(),
                    };
                    let paragraph = Arc::new(self.text.paragraph(&words, &text.style, &runs, wrap, lines));
                    if let Some(leaf) = self.layout.taffy.get_node_context_mut(taffy_node) {
                        leaf.paragraph = Some((wrap, paragraph.clone()));
                    }
                    paragraph
                });
                // Words that don't show aren't drawn: their lines start at its content's left and run on to the right.
                let size = paragraph.size();
                let covers = Bounds { origin: content.origin, size: Size { width: size.width.max(width) * transform.scale, height: size.height * transform.scale } };
                if self.turned(covers).intersects(&mask) {
                    self.paint_text(&paragraph, &runs, content, width, transform.scale, text, opacity, mask);
                }
                if !spans.is_empty() {
                    self.placed.push((id, placed));
                    self.place_spans(id, &spans, &paragraph, content, width, transform.scale, text.align, mask, pointer);
                    return;
                }
            }
            Kind::Input => {
                let padding = transform.bounds(Bounds {
                    origin: point(laid_out.origin.x + laid.border.left, laid_out.origin.y + laid.border.top),
                    size: Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom },
                });
                // Several lines scroll by the wheel and a trackpad too, as a `<textarea>` does.
                placed.scroll = self.paint_input(id, node, content, padding, laid.content_box_width(), transform.scale, text, opacity, mask).map(|most| (most, 0.0, false));
            }
            Kind::Image => self.paint_image(node, content, radii, opacity, mask),
            Kind::Path => self.paint_path(node, content, &look, text.color, opacity, mask),
            Kind::Picker => {}
            Kind::Box | Kind::Scroll => {
                let mut child_mask = mask;
                let mut child_offset = laid_out.origin;
                let around = self.near;
                let mut near = self.window;
                if look.clips || look.scrolls {
                    let padding_box = transform.bounds(Bounds {
                        origin: point(laid_out.origin.x + laid.border.left, laid_out.origin.y + laid.border.top),
                        size: Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom },
                    });
                    child_mask = self.turned(padding_box).intersect(&mask);
                    near = self.turned(padding_box);
                    if look.scrolls {
                        near = self.grown(near);
                        let inner = Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom };
                        let most = point((laid.scrollable_overflow_rect.right - inner.width).max(0.0), (laid.scrollable_overflow_rect.bottom - inner.height).max(0.0));
                        let now = self.scrolled.entry(id).or_insert(point(0.0, 0.0));
                        *now = point(now.x.clamp(0.0, most.x), now.y.clamp(0.0, most.y));
                        child_offset = point(child_offset.x - now.x, child_offset.y - now.y);
                        let along = node.kind == Kind::Scroll && most.x > 0.0 && most.y == 0.0;
                        placed.scroll = Some((most.y, most.x, along));
                        placed.shown = child_mask;
                    }
                }
                self.placed.push((id, placed));
                // Its children in order, those with a higher `zIndex` later, and positioned ones after the rest of theirs.
                let mut children: Vec<(usize, NodeId, i32, bool)> = self
                    .layout
                    .taffy
                    .children(taffy_node)
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(at, child)| {
                        let leaf = self.layout.leaf(child);
                        (at, child, leaf.look.z_index, leaf.positioned)
                    })
                    .collect();
                children.sort_by_key(|(at, _, z, positioned)| (*z, *positioned, *at));
                // What scrolls is near as far as it'd be scrolled into view, however tight what it's in clips: its box
                // and what it's in, each grown by the margin. What only clips cuts what's near to it.
                let inside = match (look.scrolls, look.clips) {
                    (true, _) => self.grown(around).intersect(&near),
                    (false, true) => around.intersect(&near),
                    (false, false) => around,
                };
                for (_, child, _, _) in children {
                    if self.layout.leaf(child).fixed {
                        let window = self.window;
                        self.near = self.grown(window);
                        self.paint(child, point(0.0, 0.0), Transform::NONE, window, 1.0, Some(id), pointer);
                    } else {
                        self.near = inside;
                        self.paint(child, child_offset, transform, child_mask, opacity, Some(id), pointer);
                    }
                }
                self.near = around;
                return;
            }
        }
        self.placed.push((id, placed));
    }

    /// An input: its text, or its placeholder while it's empty, clipped to its box; with the keyboard, what's selected
    /// behind it, what an input method composes underlined, and the caret, as it blinks; scrolled to keep the caret
    /// in view as it moves. Where it's drawn is kept for the pointer (`Drawn`). → how far several lines can scroll.
    #[allow(clippy::too_many_arguments)]
    fn paint_input(&mut self, id: Id, node: &Node, content: Bounds<f32>, padding: Bounds<f32>, width: f32, grown: f32, text: Text, opacity: f32, mask: Bounds<f32>) -> Option<f32> {
        let field = self.fields.get(&id);
        let shown = field.map(Field::display).unwrap_or_default();
        let multiline = node.is("multiline");
        let focused = self.states.focused == Some(id) && field.is_some();
        let empty = shown.is_empty();
        let placeholder = colour_token("tertiary-label", self.conditions).unwrap_or_else(|| colour("muted-foreground", self.conditions.dark));
        let (words, color) = if empty { (node.str("placeholder").unwrap_or_default().to_owned(), placeholder) } else { (shown.clone(), text.color) };
        let paragraph = self.text.paragraph(&words, &text.style, &[], multiline.then_some(width), None);
        let line_height = text.style.line_height;
        // A one-line input's text in the middle of its box, as it's drawn in a row centred on its items.
        let top = if multiline { content.origin.y } else { content.origin.y + (content.size.height - paragraph.size().height * grown) / 2.0 };
        // Its text starts where its first strong character's direction says, as an input with `dir="auto"`: at the
        // right where it's Hebrew or Arabic.
        let rtl = !empty && paragraph.lines.first().is_some_and(|line| line.rtl);
        let text = if rtl { Text { align: 1.0 - text.align, ..text } } else { text };
        let aligned = |line: &crate::text::Line| (width - line.width).max(0.0) * text.align;
        let offsets: Vec<f32> = paragraph.lines.iter().map(aligned).collect();
        let offset = |line: usize| offsets.get(line).copied().unwrap_or(0.0);
        // Where the caret is drawn: where an arrow across text running both ways left it, or before its byte.
        let (caret_line, caret_x) = match field.and_then(Field::placed) {
            Some(placed) if !empty => placed,
            _ => {
                let (line, x) = paragraph.caret(field.map_or(0, Field::caret));
                (line, x + offset(line))
            }
        };
        // One line scrolls along as the caret goes past its ends, as an `<input>` does, and back to its start once
        // it hasn't the keyboard; several scroll down and up to keep the caret's line in its box as it moves, as a
        // `<textarea>`, and by the wheel (`Screen::scroll`, which keeps how far in `scrolled`) without its moving.
        let mut scroll = self.drawn.get(&id).map_or(point(0.0, 0.0), |drawn| drawn.scroll);
        let seen = (focused, field.map_or(0, Field::caret), shown.len());
        let moved = self.drawn.get(&id).is_none_or(|drawn| drawn.seen != seen);
        let mut most = None;
        if !multiline {
            scroll.x = if focused && !empty { scroll.x.max(caret_x - width + 1.0).min(caret_x) } else { 0.0 };
            scroll.x = scroll.x.clamp(0.0, (paragraph.size().width + 1.0 - width).max(0.0));
        } else {
            let height = content.size.height / grown;
            let top = caret_line as f32 * line_height;
            scroll.y = self.scrolled.get(&id).map_or(0.0, |scrolled| scrolled.y);
            if focused && moved {
                scroll.y = scroll.y.max(top + line_height - height).min(top);
            }
            let can = (paragraph.size().height - height).max(0.0);
            scroll.y = scroll.y.clamp(0.0, can);
            self.scrolled.insert(id, point(0.0, scroll.y));
            most = Some(can);
        }
        let origin = point(content.origin.x, top - scroll.y * grown);
        // Several lines scrolled run on into its padding, as a `<textarea>`'s do; one line stops at its content.
        let clip = if multiline { padding } else { content }.intersect(&mask);
        // A point along line `line`, `x` from where its paragraph starts, in the window.
        let at = |line: usize, x: f32| point(origin.x + (x - scroll.x) * grown, origin.y + line as f32 * line_height * grown);
        if let (true, Some(field), false) = (focused, field, empty) {
            let (start, end) = field.selection();
            if start < end {
                let tint = colour_token("blue", self.conditions).unwrap_or(text.color).opacity(0.3);
                for (line, x0, x1) in paragraph.spans(start, end) {
                    let bounds = Bounds { origin: at(line, x0 + offset(line)), size: Size { width: (x1 - x0) * grown, height: line_height * grown } };
                    self.quad(bounds, clip, tint, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                }
            }
        }
        let area = Bounds { origin: point(origin.x - scroll.x * grown, origin.y), size: content.size };
        self.paint_text(&paragraph, &[], area, width, grown, Text { color, ..text }, opacity, clip);
        if let (true, Some(field)) = (focused, field) {
            // What an input method is composing is underlined, as every platform's own fields draw it.
            if let Some((start, end)) = field.composed() {
                for (line, x0, x1) in paragraph.spans(start, end) {
                    let below = at(line, x0 + offset(line));
                    let bounds = Bounds { origin: point(below.x, below.y + (line_height - 2.0) * grown), size: Size { width: (x1 - x0) * grown, height: grown } };
                    self.quad(bounds, clip, text.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                }
            }
            if self.caret && !field.readonly {
                let place = if empty { point(origin.x + width * text.align * grown, origin.y) } else { at(caret_line, caret_x) };
                let bounds = Bounds { origin: place, size: Size { width: grown, height: line_height * grown } };
                self.quad(bounds, clip, text.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
        self.drawn.insert(id, Drawn { paragraph, words, empty, origin, offsets, grown, scroll, seen });
        most
    }

    /// A paragraph's words, set in `text`, or as `runs` say where it's given them: what's behind each run first, then
    /// its glyphs, then each underline, along each line at its baseline's foot.
    #[allow(clippy::too_many_arguments)]
    fn paint_text(&mut self, paragraph: &Paragraph, runs: &[TextRun], content: Bounds<f32>, width: f32, grown: f32, text: Text, opacity: f32, mask: Bounds<f32>) {
        let own = [TextRun { len: usize::MAX, style: text.style, color: text.color, background_color: None, underline: text.underline }];
        let runs = if runs.is_empty() { &own[..] } else { runs };
        let mut at = 0usize;
        let ranges: Vec<(usize, usize, &TextRun)> = runs
            .iter()
            .map(|run| {
                let start = at;
                at = at.saturating_add(run.len);
                (start, at, run)
            })
            .collect();
        let line_height = paragraph.line_height;
        let left = |line: usize| content.origin.x + paragraph.lines.get(line).map_or(0.0, |drawn| (width - drawn.width).max(0.0) * text.align) * grown;
        for (start, end, run) in &ranges {
            let Some(behind) = run.background_color else { continue };
            for (line, x0, x1) in paragraph.inked(*start, *end) {
                // As CSS draws an inline's background: over its content area, its font's ascent and descent.
                let (ascent, descent) = paragraph.lines.get(line).map_or((line_height, 0.0), |drawn| (drawn.ascent, drawn.descent));
                let top = content.origin.y + (line as f32 * line_height + (line_height - ascent - descent) / 2.0) * grown;
                let bounds = Bounds { origin: point(left(line) + x0 * grown, top), size: Size { width: (x1 - x0) * grown, height: (ascent + descent) * grown } };
                self.quad(bounds, mask, behind, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
        let scaled_mask = ContentMask { bounds: mask.scale_to(self.scale) };
        // Glyphs grown with their box are rasterized at that size, so they stay sharp.
        let origin = point(px(content.origin.x / grown), px(content.origin.y / grown));
        let color = |index: usize| ranges.iter().find(|(start, end, _)| (*start..*end).contains(&index)).map_or(text.color, |(_, _, run)| run.color).opacity(opacity);
        if let Err(error) = self.text.paint(paragraph, origin, width, text.align, &color, self.scale * grown, scaled_mask, self.turn.scaled(self.scale), self.atlas, self.scene) {
            log::warn!("text didn't draw: {error:#}");
        }
        for (start, end, run) in &ranges {
            if !run.underline {
                continue;
            }
            for (line, x0, x1) in paragraph.inked(*start, *end) {
                let y = content.origin.y + (line as f32 + 1.0) * line_height * grown - line_height * grown * 0.18;
                let bounds = Bounds { origin: point(left(line) + x0 * grown, y), size: Size { width: (x1 - x0) * grown, height: grown.max(1.0) } };
                self.quad(bounds, mask, run.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
    }

    /// The texts and raw texts inside text `id` where their words are drawn: each text placed for the pointer over each
    /// stretch of its words, after what it's in, so a press, hover and cursor find the innermost; each reached by Tab
    /// where the DOM's would be, a span with its `tabindex`; and each where an agent or a screen reader finds it.
    #[allow(clippy::too_many_arguments)]
    fn place_spans(&mut self, id: Id, spans: &[Span], paragraph: &Paragraph, content: Bounds<f32>, width: f32, grown: f32, align: f32, mask: Bounds<f32>, pointer: bool) {
        let tree = self.tree;
        let line_height = paragraph.line_height;
        let mut pointers: HashMap<Id, bool> = HashMap::from([(id, pointer)]);
        for span in spans {
            let Some(node) = tree.nodes.get(&span.id) else { continue };
            let parent = tree.parent(span.id).unwrap_or(id);
            let pointer = pointers.get(&parent).copied().unwrap_or(pointer) && span.pointer.unwrap_or(true) && !node.is("inert");
            pointers.insert(span.id, pointer);
            let mut drawn: Option<Bounds<f32>> = None;
            let rects: Vec<Bounds<f32>> = paragraph
                .inked(span.start, span.end)
                .into_iter()
                .map(|(line, x0, x1)| {
                    let left = content.origin.x + paragraph.lines.get(line).map_or(0.0, |drawn| (width - drawn.width).max(0.0) * align) * grown;
                    self.turned(Bounds { origin: point(left + x0 * grown, content.origin.y + line as f32 * line_height * grown), size: Size { width: (x1 - x0) * grown, height: line_height * grown } })
                })
                .collect();
            for rect in &rects {
                drawn = Some(drawn.map_or(*rect, |all| all.union(rect)));
            }
            if let Some(all) = drawn {
                self.rects.insert(span.id, Rect { x: all.origin.x, y: all.origin.y, width: all.size.width, height: all.size.height });
            }
            if node.kind != Kind::Text {
                continue;
            }
            let pressable = node.listens("click") && !node.is("disabled") && !node.is("aria-disabled");
            let tabindex = node.prop("tabindex").and_then(|index| index.as_i64().or_else(|| index.as_str()?.parse().ok()));
            if pressable && tabindex.is_some_and(|index| index >= 0) {
                self.tab_order.push(span.id);
            }
            for rect in rects {
                let placed = Placed { parent: Some(parent), shown: rect.intersect(&mask), pointer, cursor: span.cursor, pressable, input: false, scroll: None };
                self.placed.push((span.id, placed));
            }
        }
    }

    /// A window's breadcrumb over the top of its page (`Breadcrumb`), in a computer's own size, as shadcn's: Home light,
    /// highlighted under the pointer (`hovered`); a muted /; then the app's icon and its name in semibold. → where
    /// Home is, and where the breadcrumb ends.
    fn breadcrumb(&mut self, crumb: &Breadcrumb, bar: f32, hovered: bool) -> (Bounds<f32>, f32) {
        let conditions = self.conditions;
        let window = self.window;
        let colour = |id: &str, otherwise: &str| colour_token(id, conditions).unwrap_or_else(|| colour(otherwise, conditions.dark));
        let footnote = crate::tokens::table().type_style("type-footnote", conditions);
        let label = crate::tokens::table().type_style("type-label", conditions);
        let style = |weight: Option<f32>| crate::text::TextStyle {
            size: footnote.map_or(13.0, |font| font.size),
            weight: weight.or(footnote.map(|font| font.weight)).unwrap_or(450.0).round() as u16,
            tracking: footnote.map_or(0.0, |font| font.tracking),
            line_height: footnote.map_or(16.0, |font| font.size.round()),
            tabular: false,
        };
        let (home_style, app_style) = (style(None), style(label.map(|font| font.weight)));
        let middle = bar / 2.0;
        let (icon, gap, pad) = (16.0, 6.0, 6.0);
        let mut x = crumb.start;
        // Home, with the ring.
        let home_words = self.text.paragraph("Home", &home_style, &[], None, None);
        let home = Bounds { origin: point(x, middle - 11.0), size: Size { width: pad + icon + gap + home_words.size().width.ceil() + pad, height: 22.0 } };
        if hovered {
            let radius = crate::tokens::table().radius("radius-md", conditions).map_or(8.0, |radius| corner(radius, home.size));
            self.quad(home, window, colour("muted", "muted"), Edges::default(), Hsla::default(), Corners::all(radius), false, 1.0);
        }
        self.picture(&crumb.ring, Bounds { origin: point(x + pad, middle - icon / 2.0), size: Size { width: icon, height: icon } }, 0.0);
        let home_colour = if hovered { colour("foreground", "foreground") } else { colour("muted-foreground", "muted-foreground") };
        let words_at = |paragraph: &Paragraph, x: f32| Bounds { origin: point(x, middle - paragraph.size().height / 2.0), size: paragraph.size() };
        let text = Text { style: home_style, color: home_colour, align: 0.0, wrap: false, underline: false };
        self.paint_text(&home_words, &[], words_at(&home_words, x + pad + icon + gap), home_words.size().width.ceil(), 1.0, text, 1.0, window);
        x = home.origin.x + home.size.width + 4.0;
        // A muted /.
        let slash = self.text.paragraph("/", &home_style, &[], None, None);
        let muted = colour("muted-foreground", "muted-foreground");
        self.paint_text(&slash, &[], words_at(&slash, x), slash.size().width.ceil(), 1.0, Text { color: muted, ..text }, 0.6, window);
        x += slash.size().width.ceil() + 4.0 + pad;
        // The app.
        if let Some(src) = &crumb.icon {
            self.picture(src, Bounds { origin: point(x, middle - icon / 2.0), size: Size { width: icon, height: icon } }, icon * 0.225);
            x += icon + gap;
        }
        let name = self.text.paragraph(&crumb.name, &app_style, &[], None, Some(1));
        self.paint_text(&name, &[], words_at(&name, x), name.size().width.ceil(), 1.0, Text { style: app_style, color: colour("foreground", "foreground"), ..text }, 1.0, window);
        (home, x + name.size().width.ceil())
    }

    /// The window's notice over the top of its page (`Notice`), `top` points down: a pill of the foreground's colour
    /// with its words in the background's, as a toast, centred, and its action after them, highlighted under the
    /// pointer. → where its action is.
    fn notice(&mut self, notice: &Notice, top: f32, hovered: bool) -> Option<Bounds<f32>> {
        let conditions = self.conditions;
        let window = self.window;
        let colour = |id: &str| colour_token(id, conditions).unwrap_or_else(|| colour(id, conditions.dark));
        let footnote = crate::tokens::table().type_style("type-footnote", conditions);
        let label = crate::tokens::table().type_style("type-label", conditions);
        let style = |weight: Option<f32>| crate::text::TextStyle {
            size: footnote.map_or(13.0, |font| font.size),
            weight: weight.or(footnote.map(|font| font.weight)).unwrap_or(450.0).round() as u16,
            tracking: footnote.map_or(0.0, |font| font.tracking),
            line_height: footnote.map_or(16.0, |font| font.size.round()),
            tabular: false,
        };
        let (words_style, action_style) = (style(None), style(label.map(|font| font.weight)));
        let words = self.text.paragraph(&notice.text, &words_style, &[], None, Some(1));
        let action = notice.action.as_ref().map(|(name, _)| self.text.paragraph(name, &action_style, &[], None, Some(1)));
        let (pad, gap, height) = (12.0, 4.0, 30.0);
        let action_width = action.as_ref().map_or(0.0, |action| gap + pad + action.size().width.ceil() + pad - 4.0);
        let width = pad + words.size().width.ceil() + action_width + if action.is_some() { 4.0 } else { pad };
        let pill = Bounds { origin: point((window.size.width - width) / 2.0, top + 8.0), size: Size { width, height } };
        self.quad(pill, window, colour("foreground"), Edges::default(), Hsla::default(), Corners::all(height / 2.0), false, 1.0);
        let middle = pill.origin.y + height / 2.0;
        let words_at = |paragraph: &Paragraph, x: f32| Bounds { origin: point(x, middle - paragraph.size().height / 2.0), size: paragraph.size() };
        let text = Text { style: words_style, color: colour("background"), align: 0.0, wrap: false, underline: false };
        self.paint_text(&words, &[], words_at(&words, pill.origin.x + pad), words.size().width.ceil(), 1.0, text, 1.0, window);
        let action = action?;
        let x = pill.origin.x + pad + words.size().width.ceil() + gap;
        let bounds = Bounds { origin: point(x, pill.origin.y + 4.0), size: Size { width: pad + action.size().width.ceil() + pad, height: height - 8.0 } };
        if hovered {
            let mut highlight = colour("background");
            highlight.a *= 0.2;
            self.quad(bounds, window, highlight, Edges::default(), Hsla::default(), Corners::all(bounds.size.height / 2.0), false, 1.0);
        }
        self.paint_text(&action, &[], words_at(&action, x + pad), action.size().width.ceil(), 1.0, Text { style: action_style, ..text }, 1.0, window);
        Some(bounds)
    }

    /// A picture by its `data:` address at `bounds`, its corners rounded `radius`.
    fn picture(&mut self, src: &str, bounds: Bounds<f32>, radius: f32) {
        let device = Size { width: (bounds.size.width * self.scale).round() as u32, height: (bounds.size.height * self.scale).round() as u32 };
        if let Some((_, tile)) = self.image(&format!("{}x{}{src}", device.width, device.height), || decode(src, device)) {
            let window = self.window;
            self.sprite(tile, bounds, Corners::all(radius), 1.0, window);
        }
    }

    /// The picture `key` names and its tile in the atlas, decoded by `decode` the first time it's drawn and put in the
    /// atlas, which alone keeps its pixels: they're decoded again only where it hasn't them.
    fn image(&mut self, key: &str, decode: impl Fn() -> Option<Decoded>) -> Option<(Picture, AtlasTile)> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        let id = hasher.finish();
        let mut decoded = None;
        let picture = match self.images.get(&id) {
            Some(picture) => (*picture)?,
            None => {
                decoded = decode();
                let picture = decoded.as_ref().map(|decoded| decoded.picture);
                self.images.insert(id, picture);
                picture?
            }
        };
        let tile = self.atlas.get_or_insert_with(&AtlasKey::Image(id), &mut || {
            Ok(decoded.take().or_else(&decode).map(|decoded| {
                (Size { width: decoded.picture.width.into(), height: decoded.picture.height.into() }, std::borrow::Cow::Owned(decoded.bgra))
            }))
        });
        Some((picture, tile.ok()??))
    }

    /// A picture's sprite, `width`×`height` device pixels, at `bounds` in the window.
    #[allow(clippy::too_many_arguments)]
    fn sprite(&mut self, tile: AtlasTile, bounds: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let scale = self.scale;
        self.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false,
            opacity,
            bounds: bounds.scale_to(scale),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            corner_radii: Corners {
                top_left: ScaledPixels(radii.top_left * scale),
                top_right: ScaledPixels(radii.top_right * scale),
                bottom_right: ScaledPixels(radii.bottom_right * scale),
                bottom_left: ScaledPixels(radii.bottom_left * scale),
            },
            tile,
            transformation: self.turn.scaled(scale),
        });
    }

    /// An image, scaled to fit its box keeping its shape, as GPUI's `img` is by default.
    fn paint_image(&mut self, node: &Node, content: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let Some(src) = node.str("src") else { return };
        // One by an address is drawn once its window has fetched it, as a `data:` address.
        let src = match self.fetched.get(src) {
            _ if src.starts_with("data:") => src,
            Some(Some(data)) => data.as_str(),
            Some(None) => return,
            None => return self.wanted.push(src.to_owned()),
        };
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        if device.width == 0 || device.height == 0 {
            return;
        }
        let Some((picture, tile)) = self.image(&format!("{}x{}{src}", device.width, device.height), || decode(src, device)) else { return };
        let fit = (content.size.width / picture.width as f32).min(content.size.height / picture.height as f32);
        let size = Size { width: picture.width as f32 * fit, height: picture.height as f32 * fit };
        let bounds = Bounds {
            origin: point(content.origin.x + (content.size.width - size.width) / 2.0, content.origin.y + (content.size.height - size.height) / 2.0),
            size,
        };
        let radii = Corners { top_left: radii.top_left.min(size.width / 2.0), top_right: radii.top_right.min(size.width / 2.0), bottom_right: radii.bottom_right.min(size.width / 2.0), bottom_left: radii.bottom_left.min(size.width / 2.0) };
        self.sprite(tile, bounds, radii, opacity, mask);
    }

    /// A path, SVG's: its `d` in its `viewBox`, stretched to its box, filled and stroked as its style says, drawn by
    /// resvg at the window's scale.
    fn paint_path(&mut self, node: &Node, content: Bounds<f32>, look: &Look, current: Hsla, opacity: f32, mask: Bounds<f32>) {
        let Some(d) = node.str("d") else { return };
        let view_box = node.str("viewBox").unwrap_or("0 0 24 24");
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        if device.width == 0 || device.height == 0 {
            return;
        }
        let paint = |paint: Paint| match paint {
            Paint::Colour(colour) => Some(colour),
            Paint::Current => Some(current),
            Paint::None => None,
        };
        let css = |colour: Option<Hsla>| colour.map_or_else(|| "none".to_owned(), |colour| {
            let rgba = crate::geometry::Rgba::from(colour);
            format!("rgba({},{},{},{})", (rgba.r * 255.0).round(), (rgba.g * 255.0).round(), (rgba.b * 255.0).round(), rgba.a)
        });
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{view_box}" preserveAspectRatio="none"><path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{width}" vector-effect="non-scaling-stroke" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
            w = device.width,
            h = device.height,
            d = d.replace('"', ""),
            fill = css(paint(look.fill)),
            stroke = css(paint(look.stroke)),
            width = look.stroke_width * self.scale,
        );
        let Some((_, tile)) = self.image(&format!("path{svg}"), || decode(&format!("data:image/svg+xml;base64,{}", data_encoding::BASE64.encode(svg.as_bytes())), device)) else { return };
        self.sprite(tile, content, Corners::default(), opacity, mask);
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
    words: String,
    empty: bool,
    origin: Point<f32>,
    offsets: Vec<f32>,
    grown: f32,
    scroll: Point<f32>,
    /// Whether it had the keyboard, where its caret was and how long its text was, to scroll to the caret only as
    /// it moves.
    seen: (bool, usize, usize),
}

impl Drawn {
    /// The byte of its display text nearest to `at`, in the window.
    fn index_at(&self, at: Point<f32>) -> usize {
        if self.empty {
            return 0;
        }
        let y = (at.y - self.origin.y) / self.grown;
        let line = ((y / self.paragraph.line_height).floor().max(0.0) as usize).min(self.paragraph.lines.len().saturating_sub(1));
        let x = (at.x - self.origin.x) / self.grown + self.scroll.x;
        editing::Layout::index_on(self, line, x).unwrap_or(0)
    }
}

impl editing::Layout for Drawn {
    fn caret(&self, index: usize) -> (usize, f32) {
        if self.empty {
            return (0, 0.0);
        }
        let (line, x) = self.paragraph.caret(index);
        (line, x + self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn index_on(&self, line: usize, x: f32) -> Option<usize> {
        if self.empty {
            return (line == 0).then_some(0);
        }
        self.paragraph.index_on(line, x - self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn line_edge(&self, index: usize, end: bool) -> usize {
        if self.empty {
            return 0;
        }
        let at = self.paragraph.line_of(index);
        let Some(line) = self.paragraph.lines.get(at) else { return index };
        if !end {
            return line.start;
        }
        // A line that wraps ends before the space it hangs, so its end is on it.
        let wraps = self.paragraph.lines.get(at + 1).is_some_and(|next| next.start == line.end);
        match self.words[..line.end].chars().next_back() {
            Some(last) if wraps && last.is_whitespace() => line.end - last.len_utf8(),
            _ => line.end,
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

/// A colour token of the version 2 table, where there is one.
fn colour_token(id: &str, conditions: Conditions) -> Option<Hsla> {
    crate::tokens::table().colour(id, conditions).map(Hsla::from)
}

/// A `data:` address's image, decoded for a box of `device` pixels: an SVG drawn at that size, and any other at the
/// smallest of its size halved again and again that still covers what of the box it fills, as a mipmap's levels are, so its pixels
/// beyond what the box shows aren't kept, and a box that grows and shrinks (a window resized) decodes it at a few
/// sizes, not one for each.
fn decode(src: &str, device: Size<u32>) -> Option<Decoded> {
    let (head, body) = src.strip_prefix("data:")?.split_once(',')?;
    let bytes = if head.ends_with(";base64") {
        data_encoding::BASE64.decode(body.as_bytes()).ok()?
    } else {
        url::form_urlencoded::parse(format!("x={body}").as_bytes()).next().map(|(_, value)| value.into_owned().into_bytes())?
    };
    let (width, height, mut rgba) = if head.starts_with("image/svg+xml") {
        let tree = resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(device.width, device.height)?;
        let size = tree.size();
        let fit = (device.width as f32 / size.width()).min(device.height as f32 / size.height());
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(fit, fit), &mut pixmap.as_mut());
        // tiny-skia's pixels are premultiplied; the atlas takes them straight.
        let mut data = pixmap.take();
        for pixel in data.chunks_exact_mut(4) {
            let alpha = pixel[3] as f32 / 255.0;
            if alpha > 0.0 {
                for channel in &mut pixel[..3] {
                    *channel = (*channel as f32 / alpha).round().min(255.0) as u8;
                }
            }
        }
        (device.width, device.height, data)
    } else {
        let mut image = image::load_from_memory(&bytes).ok()?;
        let (mut width, mut height) = (image.width(), image.height());
        // What it's drawn at, fitted in its box keeping its shape (`paint_image`).
        let fit = (device.width as f32 / width as f32).min(device.height as f32 / height as f32);
        let shown = (width as f32 * fit, height as f32 * fit);
        while (width / 2) as f32 >= shown.0 && (height / 2) as f32 >= shown.1 {
            (width, height) = (width / 2, height / 2);
        }
        if width < image.width() {
            image = image.thumbnail_exact(width, height);
        }
        let image = image.into_rgba8();
        (image.width(), image.height(), image.into_raw())
    };
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(Decoded { picture: Picture { width, height }, bgra: rgba })
}
