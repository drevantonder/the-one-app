//! An app's screen: its tree laid out by Taffy and painted as a scene with the design tokens, and what the person
//! does (a press, a key, a scroll) found on it and sent back to its JS as events. The window it's in (`platform`, or
//! the headless harness) gives it the room, the scale and the input, and draws the scene it paints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use taffy::prelude::{AvailableSpace, NodeId, TaffyTree};

use crate::geometry::{Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels, Point, ScaledPixels, Size, point, px};
use crate::inspect::{self, Key, Rect, Rects};
use crate::js::{Posted, Sandbox};
use crate::render::atlas::{AtlasKey, BladeAtlas};
use crate::render::scene::{Backdrop, BorderStyle, PolychromeSprite, Quad, Scene, Shadow, TransformationMatrix};
use crate::style::{self, Look, Paint, State, Text, colour};
use crate::text::{Paragraph, TextSystem};
use crate::tokens::{Conditions, Radius};
use crate::tree::{Id, Kind, Node, Tree};

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

pub struct Screen {
    tree: Tree,
    sandbox: Sandbox,
    /// As Colour Mode says, or as the device is (`None`).
    pub theme: Option<Appearance>,
    /// As the device is, which the window says.
    pub appearance: Appearance,
    /// The person asked for less motion, as the device says.
    pub reduced_motion: bool,
    /// The room its page has, and whether it asked for less motion, as the page was last told them (its host
    /// context's `containerDimensions` and `reducedMotion`, as a frame's host tells an MCP App).
    room: Size<f32>,
    told: Option<(Size<f32>, bool)>,
    asked: Option<Asked>,
    /// What has the keyboard.
    focused: Option<Id>,
    /// An input's text as it's typed, ahead of the app setting it back as its `value`.
    typed: HashMap<Id, String>,
    /// How far each scrolling node (and the root, 0) has scrolled, along and down.
    scrolled: HashMap<Id, Point<f32>>,
    /// Where the pointer is, what it's over and what it pressed, for styles' states and pointer events.
    states: States,
    pointer: Option<Point<f32>>,
    /// The node a pointer's press went down on, which its moves and its end go to, wherever it goes.
    captured: Option<Id>,
    /// What takes the pointer, as it was last painted, in the order painted, the topmost last.
    placed: Vec<(Id, Placed)>,
    /// Whether anything moved by itself in the last frame.
    animating: bool,
    again: bool,
    /// What's on its way from one value to another as a style changed.
    transitions: transition::Transitions,
    /// What has taken the keyboard by its `autofocus`, while that's so, and where each node listening for `layout`
    /// was last told it is.
    autofocused: std::collections::HashSet<Id>,
    laid_out: HashMap<Id, Rect>,
    /// What takes the keyboard, in the order Tab goes, as it was last painted.
    tab_order: Vec<Id>,
    images: HashMap<String, Option<Arc<Picture>>>,
    /// Where each node was drawn in the last frame, for the inspector, and the frame's size and scale.
    rects: Rects,
    painted: (Size<f32>, f32),
    /// What the app asked its client that hasn't been answered, by its JSON-RPC id, for the inspector's `settle`.
    unanswered: HashMap<String, inspect::Asked>,
    /// The inspector's `settle`s waiting: until when, and what to tell when it's settled or that time comes.
    settling: Vec<(Instant, Box<dyn FnOnce(Value)>)>,
    /// The last error its JS threw, which is logged, as a browser's console has it, and the screen carries on.
    pub failed: Option<String>,
    /// It sent a version of the strict protocol this host doesn't speak, so its window opens in a web view instead.
    pub mismatched: bool,
}

impl Screen {
    /// The screen of an app whose JS has started, from what it's posted so far, with `room` logical pixels to draw in.
    pub fn new(sandbox: Sandbox, asked: Option<Asked>, room: Size<f32>) -> Screen {
        let mut screen = Screen {
            tree: Tree::default(),
            sandbox,
            theme: None,
            appearance: Appearance::Light,
            reduced_motion: false,
            room,
            told: None,
            asked,
            focused: None,
            typed: HashMap::new(),
            scrolled: HashMap::new(),
            states: States::default(),
            pointer: None,
            captured: None,
            placed: Vec::new(),
            animating: false,
            again: false,
            transitions: Default::default(),
            autofocused: Default::default(),
            laid_out: HashMap::new(),
            tab_order: Vec::new(),
            images: HashMap::new(),
            rects: Rects::new(),
            painted: (Size::default(), 1.0),
            unanswered: HashMap::new(),
            settling: Vec::new(),
            failed: None,
            mismatched: false,
        };
        screen.pump();
        screen
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// The semantic tree, as an agent and a screen reader read it.
    pub fn semantics(&self) -> Value {
        json!(self.tree.semantics(0))
    }

    pub fn dark(&self) -> bool {
        self.theme.unwrap_or(self.appearance) == Appearance::Dark
    }

    /// Tells the app what the person did to node `id`, and takes what it changed.
    pub fn event(&mut self, id: Id, name: &str, value: Option<String>) {
        let event = json!({"v": crate::VERSION, "op": "event", "id": id, "name": name, "value": value});
        if let Err(error) = self.sandbox.send("event", &event) {
            self.threw(error);
        }
        self.pump();
    }

    /// Tells the app a message from its client: an answer to what it asked (`Asked`), or a notification.
    pub fn message(&mut self, message: &Value) {
        if message.get("method").is_none() {
            self.unanswered.remove(&message["id"].to_string());
        }
        if let Err(error) = self.sandbox.send("message", message) {
            self.threw(error);
        }
        self.pump();
    }

    /// Logs what the app's JS threw, as a browser's console does, and carries on.
    fn threw(&mut self, error: anyhow::Error) {
        eprintln!("strict app error: {error:#}");
        self.failed = Some(format!("{error:#}"));
    }

    /// How long until the screen has something to do by itself: the app's next timer, unless its clock is paused,
    /// or an inspector's `settle` coming to its deadline. The window calls `fire` then.
    pub fn due(&self) -> Option<Duration> {
        let timer = self.sandbox.due().filter(|_| !self.sandbox.clock().paused()).map(|due| Duration::from_secs_f64(due.max(0.0) / 1000.0));
        let now = Instant::now();
        let settle = self.settling.iter().map(|(until, _)| until.saturating_duration_since(now)).min();
        // What moves by itself draws again each frame.
        let frame = self.animating.then_some(Duration::from_millis(16));
        timer.into_iter().chain(settle).chain(frame).min()
    }

    /// Runs the app's timers that are due, and tells the `settle`s whose time has come.
    pub fn fire(&mut self) {
        if self.sandbox.due().is_some_and(|due| due <= 0.0) {
            if let Err(error) = self.sandbox.fire() {
                self.threw(error);
            }
            self.pump();
        }
        self.tell_settled();
    }

    /// Applies what the app has posted, and answers what it asked that the host answers at once.
    fn pump(&mut self) {
        loop {
            let mut answers = Vec::new();
            let appearance = self.theme.unwrap_or(self.appearance);
            for posted in self.sandbox.take() {
                match posted {
                    Posted::Update(update) => {
                        let applied = serde_json::from_str(&update).map_err(anyhow::Error::from).and_then(|update| self.tree.apply(update));
                        match applied {
                            Err(error) if error.is::<crate::tree::Mismatch>() => self.mismatched = true,
                            Err(error) => self.threw(error),
                            Ok(()) => {}
                        }
                    }
                    Posted::Message(message) => {
                        let (Ok(message), Some(asked)) = (serde_json::from_str::<Value>(&message), self.asked.as_mut()) else { continue };
                        let (id, method, tool) = (message.get("id").map(Value::to_string), message["method"].as_str().map(str::to_owned), message["params"]["name"].as_str().map(str::to_owned));
                        match asked(message, appearance) {
                            // What only the window knows of where the page is, which it adds to its client's answer.
                            Some(mut answer) if method.as_deref() == Some("ui/initialize") && answer["result"].is_object() => {
                                answer["result"]["hostContext"]["containerDimensions"] = json!({"width": self.room.width, "height": self.room.height});
                                answer["result"]["hostContext"]["reducedMotion"] = json!(self.reduced_motion);
                                self.told = Some((self.room, self.reduced_motion));
                                answers.push(answer);
                            }
                            Some(answer) => answers.push(answer),
                            None => {
                                if let (Some(id), Some(method)) = (id, method) {
                                    self.unanswered.insert(id, inspect::Asked { method, tool, since: Instant::now() });
                                }
                            }
                        }
                    }
                }
            }
            if answers.is_empty() {
                // What says `autofocus` takes the keyboard as it says so, as an active menu item does.
                let wants: Vec<Id> = self.tree.nodes.iter().filter(|(_, node)| node.is("autofocus")).map(|(id, _)| *id).collect();
                self.autofocused.retain(|id| wants.contains(id));
                if let Some(id) = wants.into_iter().filter(|id| !self.autofocused.contains(id)).max() {
                    self.autofocused.insert(id);
                    self.focus(Some(id));
                    continue;
                }
                // A timer due now runs now, the clock paused or not.
                if self.sandbox.due().is_some_and(|due| due <= 0.0) {
                    if let Err(error) = self.sandbox.fire() {
                        self.threw(error);
                    }
                    continue;
                }
                self.tell_settled();
                return;
            }
            for answer in answers {
                if let Err(error) = self.sandbox.send("message", &answer) {
                    self.threw(error);
                }
            }
        }
    }

    /// The conditions its tokens' values vary by, as its window is now.
    fn conditions(&self) -> Conditions {
        Conditions { dark: self.dark(), reduced_motion: self.reduced_motion, ..Conditions::default() }
    }

    /// Paints the screen into `scene`, `size` logical pixels at `scale` device pixels each.
    pub fn paint(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &BladeAtlas, scene: &mut Scene) {
        scene.clear();
        // The page is told when its room or its motion changes, as a frame's host tells an MCP App.
        self.room = Size { width: size.width.0, height: size.height.0 };
        if self.told.is_some_and(|told| told != (self.room, self.reduced_motion)) {
            self.told = Some((self.room, self.reduced_motion));
            let context = json!({"containerDimensions": {"width": self.room.width, "height": self.room.height}, "reducedMotion": self.reduced_motion});
            self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": context}));
        }
        let conditions = self.conditions();
        let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
        self.states.focused = self.focused;
        let now = self.sandbox.clock().now();
        let mut painter = Painter {
            tree: &self.tree,
            taffy: TaffyTree::new(),
            nodes: HashMap::new(),
            text,
            atlas,
            scene,
            scale,
            conditions,
            states: &self.states,
            placed: Vec::new(),
            tab_order: Vec::new(),
            scrolled: &mut self.scrolled,
            typed: &self.typed,
            images: &mut self.images,
            rects: Rects::new(),
            time: now,
            transitions: &mut self.transitions,
            animating: false,
            fixed: Vec::new(),
            window,
            modal: None,
            turn: TransformationMatrix::unit(),
        };
        // The window, as a page's body: its root's background and text, scrolling what's taller than it.
        let version2 = self.tree.nodes.values().any(|node| node.styles.is_some());
        let (body, background) = if version2 { Text::root(conditions) } else { (Text::body(conditions.dark), Some(colour("background", conditions.dark))) };
        painter.quad(window, window, background.unwrap_or_default(), Edges::default(), Hsla::default(), Corners::default(), false, 1.0);
        let root = painter.build(0, body, true);
        for fixed in std::mem::take(&mut painter.fixed) {
            let _ = painter.taffy.add_child(root, fixed);
        }
        let available = taffy::geometry::Size { width: AvailableSpace::Definite(window.size.width), height: AvailableSpace::Definite(window.size.height) };
        let Painter { taffy, nodes, text, .. } = &mut painter;
        let measured = taffy.compute_layout_with_measure(root, available, |input, node, _, _| {
            // What has nothing to measure is as big as its style says, or nothing.
            let Some(Leaf { measure: Some(measure), .. }) = nodes.get(&node) else {
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
                Measure::Text { words, text: style, lines } => text.paragraph(words, &style.style, width.filter(|_| style.wrap), *lines).size(),
                Measure::Input { words, text: style, multiline } => {
                    let paragraph = text.paragraph(words, &style.style, multiline.then_some(width).flatten(), None);
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
        let (placed, mut tab_order, rects, animating, modal) = (painter.placed, painter.tab_order, painter.rects, painter.animating, painter.modal);
        let moving = self.transitions.finish(now);
        self.placed = placed;
        // A modal dialog holds the keyboard: Tab goes only to what's in it, and it has the keyboard as it appears.
        if let Some(modal) = modal {
            tab_order.retain(|id| self.chain_from(*id).contains(&modal));
        }
        self.tab_order = tab_order;
        self.rects = rects;
        self.animating = animating || moving;
        self.painted = (window.size, scale);
        self.tell_settled();
        scene.finish();
        if let Some(modal) = modal
            && !self.focused.is_some_and(|focused| self.chain_from(focused).contains(&modal))
        {
            self.focus(Some(modal));
            self.again = true;
        }
        // Where each node that listens for `layout` is, as its layout settles and whenever its size changes.
        let listening: Vec<(Id, Rect)> = self
            .tree
            .nodes
            .iter()
            .filter(|(_, node)| node.listens("layout"))
            .filter_map(|(id, _)| Some((*id, *self.rects.get(id)?)))
            .filter(|(id, rect)| self.laid_out.get(id).is_none_or(|was| (was.width, was.height) != (rect.width, rect.height)))
            .collect();
        for (id, rect) in listening {
            self.laid_out.insert(id, rect);
            let event = json!({"v": crate::VERSION, "op": "event", "id": id, "name": "layout", "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}});
            if let Err(error) = self.sandbox.send("event", &event) {
                self.threw(error);
            }
            self.pump();
            self.again = true;
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
        let Some(top) = self.placed.iter().rev().find(|(_, placed)| placed.shown.contains(&at)).map(|(id, _)| *id) else { return Vec::new() };
        let mut chain = vec![top];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// The pointer's shape at `at`: what its style says, or a hand over what's pressed and a beam over an input.
    pub fn cursor(&self, at: Point<f32>) -> Cursor {
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
        let time = self.sandbox.clock().now();
        for id in chain {
            if !self.tree.nodes.get(id).is_some_and(|node| node.listens(name)) {
                continue;
            }
            let rect = self.rects.get(id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
            let mut event = json!({
                "v": crate::VERSION, "op": "event", "id": id, "name": name, "timeStamp": time,
                "clientX": at.x, "clientY": at.y, "pointerId": 1, "pointerType": "mouse", "button": button,
                "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            });
            if name == "wheel" {
                event["deltaX"] = json!(delta.x);
                event["deltaY"] = json!(delta.y);
            }
            if let Err(error) = self.sandbox.send("event", &event) {
                self.threw(error);
            }
            self.pump();
        }
    }

    /// The pointer moved to `at`: what it's over is hovered, told as it comes and goes, and a press it holds is told
    /// it moved.
    pub fn pointer_moved(&mut self, at: Point<f32>) {
        self.pointer = Some(at);
        let chain = self.under(at);
        let mut hovered = chain.clone();
        hovered.reverse();
        let left: Vec<Id> = self.states.hovered.iter().copied().filter(|id| !hovered.contains(id)).collect();
        let entered: Vec<Id> = hovered.iter().copied().filter(|id| !self.states.hovered.contains(id)).collect();
        self.states.hovered = hovered;
        for id in left.into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, point(0.0, 0.0));
        }
        for id in entered {
            self.pointer_event(&[id], "pointerenter", at, 0, point(0.0, 0.0));
        }
        let target = match self.captured {
            Some(captured) => self.chain_from(captured),
            None => chain,
        };
        self.pointer_event(&target, "pointermove", at, 0, point(0.0, 0.0));
    }

    /// The pointer left the window: nothing's hovered.
    pub fn pointer_left(&mut self) {
        let at = self.pointer.take().unwrap_or(point(0.0, 0.0));
        for id in std::mem::take(&mut self.states.hovered).into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, point(0.0, 0.0));
        }
    }

    /// `id` and what it's in, the innermost first, as last painted.
    fn chain_from(&self, id: Id) -> Vec<Id> {
        let mut chain = vec![id];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// A button went down at `at` (0 the main one, 2 the secondary): what's under it is told, and with the main one
    /// what's pressed there is pressed and takes the keyboard if it can; the secondary is a `contextmenu` too.
    pub fn pointer_down(&mut self, at: Point<f32>, button: u8) {
        let chain = self.under(at);
        self.captured = chain.first().copied();
        self.pointer_event(&chain, "pointerdown", at, button, point(0.0, 0.0));
        if button == 2 {
            return self.pointer_event(&chain, "contextmenu", at, button, point(0.0, 0.0));
        }
        let pressed = chain.iter().copied().find(|id| self.placed(*id).is_some_and(|placed| placed.pressable));
        self.states.pressed = pressed;
        self.states.focus_visible = false;
        self.focus(pressed);
    }

    /// A button came up at `at`: what it went down on is told, and with the main one a press, if it's still over
    /// what was pressed.
    pub fn pointer_up(&mut self, at: Point<f32>, button: u8) {
        let target = match self.captured.take() {
            Some(captured) => self.chain_from(captured),
            None => self.under(at),
        };
        self.pointer_event(&target, "pointerup", at, button, point(0.0, 0.0));
        let Some(pressed) = self.states.pressed.take() else { return };
        let over = self.under(at).contains(&pressed);
        if over && self.placed(pressed).is_some_and(|placed| !placed.input) {
            let rect = self.rects.get(&pressed).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
            let event = json!({"v": crate::VERSION, "op": "event", "id": pressed, "name": "click", "clientX": at.x, "clientY": at.y,
                "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}});
            if let Err(error) = self.sandbox.send("event", &event) {
                self.threw(error);
            }
            self.pump();
        }
    }

    /// The wheel or a trackpad scrolled by `delta` pixels (down and right positive) at `at`: what's under it is told
    /// (`wheel`), and the innermost thing there that can still scroll that way does, telling its app if it listens.
    /// → what scrolled.
    pub fn scroll(&mut self, at: Point<f32>, delta: Point<f32>) -> Option<Id> {
        let chain = self.under(at);
        self.pointer_event(&chain, "wheel", at, 0, delta);
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
            let event = json!({"v": crate::VERSION, "op": "event", "id": id, "name": "scroll", "scrollTop": scrolled.y, "scrollLeft": scrolled.x});
            if let Err(error) = self.sandbox.send("event", &event) {
                self.threw(error);
            }
            self.pump();
        }
        Some(id)
    }

    /// Gives the keyboard to `id`, or to nothing, telling the app what had it and what has it, where they listen.
    fn focus(&mut self, id: Option<Id>) {
        if self.focused == id {
            return;
        }
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
        if key.key == "Tab" && !key.ctrl && !key.meta && !key.alt {
            self.states.focus_visible = true;
            let order = &self.tab_order;
            let next = match focused.and_then(|focused| order.iter().position(|id| *id == focused)) {
                Some(at) if key.shift => order.get((at + order.len() - 1) % order.len()).copied(),
                Some(at) => order.get((at + 1) % order.len()).copied(),
                None if key.shift => order.last().copied(),
                None => order.first().copied(),
            };
            self.focus(next);
            return focused;
        }
        let id = focused?;
        // A key goes to what has the keyboard and to each node it's in that listens, the innermost first.
        for listener in self.chain_from(id) {
            if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                if let Err(error) = self.sandbox.send("event", &key.event(listener)) {
                    self.threw(error);
                }
                self.pump();
            }
        }
        let input = self.tree.nodes.get(&id).is_some_and(|node| node.kind == Kind::Input);
        match key.key.as_str() {
            "Enter" if input => drop(self.edit(id, Edit::Enter)),
            "Backspace" if input => drop(self.edit(id, Edit::Backspace)),
            "Enter" | " " if !input => {
                if self.tree.nodes.get(&id).is_some_and(|node| node.listens("click")) {
                    self.event(id, "click", None)
                }
            }
            typed if input && typed.chars().count() == 1 && !key.ctrl && !key.meta => drop(self.edit(id, Edit::Type(typed))),
            _ => {}
        }
        Some(id)
    }

    /// What a key does to input `id`'s text, as it's typed, ahead of the app setting it back as its `value`, and
    /// tells the app as an `input` event. → whether it took the key.
    fn edit(&mut self, id: Id, key: Edit) -> bool {
        let Some(node) = self.tree.nodes.get(&id) else { return false };
        let mut value = self.typed.get(&id).cloned().unwrap_or_else(|| value_of(node));
        match key {
            Edit::Enter if !node.is("multiline") => return false,
            Edit::Enter => value.push('\n'),
            Edit::Backspace => drop(value.pop()),
            Edit::Type(typed) => value.push_str(typed),
        }
        self.typed.insert(id, value.clone());
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("input")) {
            self.event(id, "input", Some(value));
        }
        true
    }
}

/// A key, as it changes an input's text.
enum Edit<'a> {
    Enter,
    Backspace,
    Type(&'a str),
}

/// Paints a screen that says only `words`, in the middle, in the muted colour, as a page's empty state does.
pub(crate) fn paint_message(words: &str, appearance: Appearance, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &BladeAtlas, scene: &mut Scene) {
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
    let body = Text::body(dark);
    let width = (window.size.width - 32.0).max(1.0);
    let paragraph = text.paragraph(words, &body.style, Some(width), None);
    let top = (window.size.height - paragraph.size().height) / 2.0;
    let _ = text.paint(&paragraph, point(px(16.0), px(top)), width, 0.5, colour("muted-foreground", dark), scale, mask, TransformationMatrix::unit(), atlas, scene);
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
enum Measure {
    Text { words: String, text: Text, lines: Option<usize> },
    Input { words: String, text: Text, multiline: bool },
}

/// A tree node as laid out: which node, the text it inherits, how it looks, and what it measures.
struct Leaf {
    id: Id,
    text: Text,
    look: Look,
    line_clamp: Option<usize>,
    positioned: bool,
    /// Placed in the window, not in its parent (`position: fixed`).
    fixed: bool,
    measure: Option<Measure>,
}

/// An image, decoded, in the atlas's byte order (BGRA).
struct Picture {
    key: u64,
    width: u32,
    height: u32,
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
    pub cursor: Option<Cursor>,
    pub pressable: bool,
    pub input: bool,
    /// It scrolls: how far it can, down and along, and whether along is its way.
    pub scroll: Option<(f32, f32, bool)>,
}

struct Painter<'a> {
    tree: &'a Tree,
    taffy: TaffyTree<()>,
    nodes: HashMap<NodeId, Leaf>,
    text: &'a mut TextSystem,
    atlas: &'a BladeAtlas,
    scene: &'a mut Scene,
    scale: f32,
    conditions: Conditions,
    states: &'a States,
    placed: Vec<(Id, Placed)>,
    tab_order: Vec<Id>,
    scrolled: &'a mut HashMap<Id, Point<f32>>,
    typed: &'a HashMap<Id, String>,
    images: &'a mut HashMap<String, Option<Arc<Picture>>>,
    rects: Rects,
    /// The time on the screen's clock, in milliseconds, for what moves by itself (`animationName`) and what's on its
    /// way somewhere (`transitions`), and whether anything did.
    time: f64,
    transitions: &'a mut transition::Transitions,
    animating: bool,
    /// What's placed in the window, laid out as the root's children, and the window.
    fixed: Vec<NodeId>,
    window: Bounds<f32>,
    /// The topmost `aria-modal` dialog drawn, which holds the keyboard.
    modal: Option<Id>,
    /// How what's painted now is turned, in the window: the `rotate`s of what it's in.
    turn: TransformationMatrix,
}

/// What a node's state is, for its styles: where the pointer is, what's pressed and what has the keyboard.
#[derive(Default)]
pub(crate) struct States {
    /// What the pointer is over, from the root to the innermost.
    pub hovered: Vec<Id>,
    pub pressed: Option<Id>,
    pub focused: Option<Id>,
    /// The keyboard came there by a key, so it shows (CSS's `:focus-visible`).
    pub focus_visible: bool,
}

impl States {
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
    /// `id` and what's in it as Taffy's nodes. The root fills the window and scrolls, as a page's body.
    fn build(&mut self, id: Id, inherited: Text, root: bool) -> NodeId {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let state = self.states.of(id, node.kind == Kind::Input);
        let mut computed = style::of(node, state, self.conditions);
        // What's just been added starts from its `entering` state, to move from.
        let entering = (computed.transition.is_some() && self.transitions.entering(id)).then(|| style::of(node, State { entering: true, ..state }, self.conditions));
        self.transitions.apply(id, &mut computed, entering.as_ref(), self.time, self.conditions.reduced_motion);
        let mut layout = computed.layout.clone();
        let mut look = computed.look.clone();
        let text = match &node.styles {
            Some(_) => inherited.with(&computed),
            None => inherited.of(&node.style, node.kind == Kind::Text, self.conditions.dark),
        };
        if root {
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
        let (measure, children) = match node.kind {
            Kind::Text => (Some(Measure::Text { words: tree.text(id), text, lines }), Vec::new()),
            Kind::RawText => (Some(Measure::Text { words: node.value.clone().unwrap_or_default(), text, lines: None }), Vec::new()),
            Kind::Input => {
                let words = self.typed.get(&id).cloned().unwrap_or_else(|| value_of(node));
                let words = if words.is_empty() { node.str("placeholder").unwrap_or_default().to_owned() } else { words };
                (Some(Measure::Input { words: if words.is_empty() { " ".into() } else { words }, text, multiline: node.is("multiline") }), Vec::new())
            }
            Kind::Image | Kind::Path | Kind::Picker => (None, Vec::new()),
            Kind::Box | Kind::Scroll => {
                let children: Vec<Id> = node.children.iter().copied().filter(|child| tree.nodes.contains_key(child)).collect();
                let built: Vec<NodeId> = children.into_iter().map(|child| self.build(child, text, false)).collect();
                // What's fixed is laid out in the window, as the root's (`Screen::paint`).
                let (fixed, inside): (Vec<NodeId>, Vec<NodeId>) = built.into_iter().partition(|child| self.nodes[child].fixed);
                self.fixed.extend(fixed);
                (None, inside)
            }
        };
        let positioned = layout.position == taffy::style::Position::Absolute;
        let taffy_node = if children.is_empty() { self.taffy.new_leaf(layout) } else { self.taffy.new_with_children(layout, &children) }
            .expect("taffy takes any style");
        self.nodes.insert(taffy_node, Leaf { id, text, look, line_clamp: lines, positioned, fixed: computed.fixed, measure });
        taffy_node
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
        let Ok(laid) = self.taffy.layout(taffy_node).copied() else { return };
        // What isn't displayed isn't drawn, and nothing in it is.
        if self.taffy.style(taffy_node).is_ok_and(|style| style.display == taffy::style::Display::None) {
            return;
        }
        let Some(leaf) = self.nodes.get(&taffy_node) else { return };
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
        let pointer = pointer && look.pointer_events;
        if node.str("aria-modal") == Some("true") {
            self.modal = Some(id);
        }
        let drawn = self.turned(bounds);
        self.rects.insert(id, Rect { x: drawn.origin.x, y: drawn.origin.y, width: drawn.size.width, height: drawn.size.height });
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
            if let Some((blur, saturate)) = look.material {
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
        if pressable && (node.kind == Kind::Input || node.str("tabindex") != Some("-1") && node.prop("tabindex").and_then(Value::as_i64) != Some(-1)) {
            self.tab_order.push(id);
        }
        let mut placed = Placed { parent, shown, cursor: look.cursor, pressable, input: node.kind == Kind::Input, scroll: None };
        match node.kind {
            Kind::Text | Kind::RawText => {
                let words = if node.kind == Kind::Text { tree.text(id) } else { node.value.clone().unwrap_or_default() };
                let width = laid.content_box_width().max(1.0);
                let paragraph = self.text.paragraph(&words, &text.style, text.wrap.then_some(width), lines);
                self.paint_text(&paragraph, content, width, transform.scale, text, opacity, mask);
            }
            Kind::Input => {
                let value = self.typed.get(&id).cloned().unwrap_or_else(|| value_of(node));
                let shown_value = if node.str("type") == Some("password") { "•".repeat(value.chars().count()) } else { value.clone() };
                let multiline = node.is("multiline");
                let width = laid.content_box_width();
                let placeholder = colour_token("tertiary-label", self.conditions).unwrap_or_else(|| colour("muted-foreground", self.conditions.dark));
                let (words, color) = if shown_value.is_empty() { (node.str("placeholder").unwrap_or_default().to_owned(), placeholder) } else { (shown_value.clone(), text.color) };
                let paragraph = self.text.paragraph(&words, &text.style, multiline.then_some(width), None);
                // A one-line input's text in the middle of its box, as it's drawn in a row centred on its items.
                let top = if multiline { content.origin.y } else { content.origin.y + (content.size.height - paragraph.size().height * transform.scale) / 2.0 };
                let area = Bounds { origin: point(content.origin.x, top), size: content.size };
                let clip = content.intersect(&mask);
                self.paint_text(&paragraph, area, width, transform.scale, Text { color, ..text }, opacity, clip);
                if self.states.focused == Some(id) {
                    let (line, x) = if shown_value.is_empty() { (0, 0.0) } else { paragraph.caret(shown_value.len()) };
                    let height = text.style.line_height * transform.scale;
                    let caret = Bounds { origin: point(area.origin.x + x * transform.scale, area.origin.y + line as f32 * height), size: Size { width: transform.scale.max(1.0), height } };
                    let tint = colour_token("accent", self.conditions).unwrap_or(text.color);
                    self.quad(caret, clip, tint, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                }
            }
            Kind::Image => self.paint_image(node, content, radii, opacity, mask),
            Kind::Path => self.paint_path(node, content, &look, text.color, opacity, mask),
            Kind::Picker => {}
            Kind::Box | Kind::Scroll => {
                let mut child_mask = mask;
                let mut child_offset = laid_out.origin;
                if look.clips || look.scrolls {
                    let padding_box = transform.bounds(Bounds {
                        origin: point(laid_out.origin.x + laid.border.left, laid_out.origin.y + laid.border.top),
                        size: Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom },
                    });
                    child_mask = self.turned(padding_box).intersect(&mask);
                    if look.scrolls {
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
                if pointer {
                    self.placed.push((id, placed));
                }
                // Its children in order, those with a higher `zIndex` later, and positioned ones after the rest of theirs.
                let mut children: Vec<(usize, NodeId, i32, bool)> = self
                    .taffy
                    .children(taffy_node)
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(at, child)| {
                        let leaf = &self.nodes[&child];
                        (at, child, leaf.look.z_index, leaf.positioned)
                    })
                    .collect();
                children.sort_by_key(|(at, _, z, positioned)| (*z, *positioned, *at));
                for (_, child, _, _) in children {
                    if self.nodes[&child].fixed {
                        let window = self.window;
                        self.paint(child, point(0.0, 0.0), Transform::NONE, window, 1.0, Some(id), true);
                    } else {
                        self.paint(child, child_offset, transform, child_mask, opacity, Some(id), pointer);
                    }
                }
                return;
            }
        }
        if pointer {
            self.placed.push((id, placed));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_text(&mut self, paragraph: &Paragraph, content: Bounds<f32>, width: f32, grown: f32, text: Text, opacity: f32, mask: Bounds<f32>) {
        let mask = ContentMask { bounds: mask.scale_to(self.scale) };
        // Glyphs grown with their box are rasterized at that size, so they stay sharp.
        let origin = point(px(content.origin.x / grown), px(content.origin.y / grown));
        if let Err(error) = self.text.paint(paragraph, origin, width, text.align, text.color.opacity(opacity), self.scale * grown, mask, self.turn.scaled(self.scale), self.atlas, self.scene) {
            log::warn!("text didn't draw: {error:#}");
        }
        if text.underline {
            for (at, line) in paragraph.lines.iter().enumerate() {
                let y = content.origin.y + (at as f32 + 1.0) * paragraph.line_height * grown - paragraph.line_height * grown * 0.18;
                let left = content.origin.x + (width - line.width).max(0.0) * text.align * grown;
                let bounds = Bounds { origin: point(left, y), size: Size { width: line.width * grown, height: grown.max(1.0) } };
                self.quad(bounds, Bounds { origin: point(mask.bounds.origin.x.0 / self.scale, mask.bounds.origin.y.0 / self.scale), size: Size { width: mask.bounds.size.width.0 / self.scale, height: mask.bounds.size.height.0 / self.scale } }, text.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
    }

    /// A picture's sprite, `width`×`height` device pixels, at `bounds` in the window.
    #[allow(clippy::too_many_arguments)]
    fn sprite(&mut self, picture: &Picture, bounds: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let tile = self.atlas.get_or_insert_with(&AtlasKey::Image(picture.key), &mut || {
            Ok(Some((Size { width: picture.width.into(), height: picture.height.into() }, std::borrow::Cow::Borrowed(picture.bgra.as_slice()))))
        });
        let Ok(Some(tile)) = tile else { return };
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
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        if device.width == 0 || device.height == 0 {
            return;
        }
        let key = if src.starts_with("data:image/svg+xml") { format!("{}x{}{src}", device.width, device.height) } else { src.to_owned() };
        let picture = self.images.entry(key.clone()).or_insert_with(|| decode(src, device, &key).map(Arc::new)).clone();
        let Some(picture) = picture else { return };
        let fit = (content.size.width / picture.width as f32).min(content.size.height / picture.height as f32);
        let size = Size { width: picture.width as f32 * fit, height: picture.height as f32 * fit };
        let bounds = Bounds {
            origin: point(content.origin.x + (content.size.width - size.width) / 2.0, content.origin.y + (content.size.height - size.height) / 2.0),
            size,
        };
        let radii = Corners { top_left: radii.top_left.min(size.width / 2.0), top_right: radii.top_right.min(size.width / 2.0), bottom_right: radii.bottom_right.min(size.width / 2.0), bottom_left: radii.bottom_left.min(size.width / 2.0) };
        self.sprite(&picture, bounds, radii, opacity, mask);
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
        let key = format!("path{svg}");
        let picture = self.images.entry(key.clone()).or_insert_with(|| decode(&format!("data:image/svg+xml;base64,{}", data_encoding::BASE64.encode(svg.as_bytes())), device, &key).map(Arc::new)).clone();
        let Some(picture) = picture else { return };
        self.sprite(&picture, content, Corners::default(), opacity, mask);
    }
}

/// A colour token of the version 2 table, where there is one.
fn colour_token(id: &str, conditions: Conditions) -> Option<Hsla> {
    crate::tokens::table().colour(id, conditions).map(Hsla::from)
}

/// A `data:` address's image, decoded; an SVG drawn at `device` pixels.
fn decode(src: &str, device: Size<u32>, key: &str) -> Option<Picture> {
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
        let image = image::load_from_memory(&bytes).ok()?.into_rgba8();
        (image.width(), image.height(), image.into_raw())
    };
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    Some(Picture { key: hasher.finish(), width, height, bgra: rgba })
}
