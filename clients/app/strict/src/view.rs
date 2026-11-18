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
use crate::render::scene::{PolychromeSprite, Quad, Scene};
use crate::style::{Text, colour, layout, look};
use crate::text::{Paragraph, TextSystem};
use crate::tree::{Id, Kind, Node, Tree};

pub(crate) mod inspected;

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

#[derive(Clone, Copy, PartialEq)]
enum RegionKind {
    Press,
    Input,
    Scroll { max: f32 },
}

/// Where something that takes the pointer was painted, clipped to what shows of it.
struct Region {
    id: Id,
    bounds: Bounds<f32>,
    kind: RegionKind,
}

pub struct Screen {
    tree: Tree,
    sandbox: Sandbox,
    /// As Colour Mode says, or as the device is (`None`).
    pub theme: Option<Appearance>,
    /// As the device is, which the window says.
    pub appearance: Appearance,
    asked: Option<Asked>,
    /// What has the keyboard.
    focused: Option<Id>,
    /// An input's text as it's typed, ahead of the app setting it back as its `value`.
    typed: HashMap<Id, String>,
    /// How far each scrolling node (and the root, 0) has scrolled.
    scrolled: HashMap<Id, f32>,
    pressed: Option<Id>,
    regions: Vec<Region>,
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
    /// The screen of an app whose JS has started, from what it's posted so far.
    pub fn new(sandbox: Sandbox, asked: Option<Asked>) -> Screen {
        let mut screen = Screen {
            tree: Tree::default(),
            sandbox,
            theme: None,
            appearance: Appearance::Light,
            asked,
            focused: None,
            typed: HashMap::new(),
            scrolled: HashMap::new(),
            pressed: None,
            regions: Vec::new(),
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
        timer.into_iter().chain(settle).min()
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

    /// Paints the screen into `scene`, `size` logical pixels at `scale` device pixels each.
    pub fn paint(&mut self, size: Size<Pixels>, scale: f32, text: &mut TextSystem, atlas: &BladeAtlas, scene: &mut Scene) {
        scene.clear();
        let dark = self.dark();
        let window = Bounds { origin: point(0.0, 0.0), size: Size { width: size.width.0, height: size.height.0 } };
        let mut painter = Painter {
            tree: &self.tree,
            taffy: TaffyTree::new(),
            nodes: HashMap::new(),
            text,
            atlas,
            scene,
            scale,
            dark,
            regions: Vec::new(),
            tab_order: Vec::new(),
            scrolled: &mut self.scrolled,
            focused: self.focused,
            typed: &self.typed,
            images: &mut self.images,
            rects: Rects::new(),
        };
        // The window, as a page's body: the background, scrolling what's taller than it.
        painter.quad(window, window, colour("background", dark), Edges::default(), Hsla::default(), 0.0, 1.0);
        {
            let root = painter.build(0, Text::body(dark), true);
            let available = taffy::geometry::Size { width: AvailableSpace::Definite(window.size.width), height: AvailableSpace::Definite(window.size.height) };
            let Painter { taffy, nodes, text, .. } = &mut painter;
            let measured = taffy.compute_layout_with_measure(root, available, |input, node, _, _| {
                let Some(Leaf { measure: Some(measure), .. }) = nodes.get(&node) else { return taffy::LayoutOutput::from_outer_size(taffy::geometry::Size::ZERO) };
                let width = input.known_dimensions.width.or(match input.available_space.width {
                    AvailableSpace::Definite(width) => Some(width),
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
                let size = match measure {
                    Measure::Text { words, text: style, lines } => text.paragraph(words, &style.style, width, *lines).size(),
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
            if let Err(error) = measured {
                log::error!("its layout failed: {error}");
            } else {
                painter.paint(root, point(0.0, 0.0), window, 1.0);
            }
        }
        let (regions, tab_order, rects) = (painter.regions, painter.tab_order, painter.rects);
        self.regions = regions;
        self.tab_order = tab_order;
        self.rects = rects;
        self.painted = (window.size, scale);
        scene.finish();
    }

    /// The topmost thing that takes the pointer at `at`, of a kind.
    fn region_at(&self, at: Point<f32>, press: bool) -> Option<&Region> {
        self.regions
            .iter()
            .rev()
            .find(|region| region.bounds.contains(&at) && (!press || matches!(region.kind, RegionKind::Press | RegionKind::Input)))
    }

    pub fn cursor(&self, at: Point<f32>) -> Cursor {
        match self.region_at(at, true).map(|region| region.kind) {
            Some(RegionKind::Press) => Cursor::Pointer,
            Some(RegionKind::Input) => Cursor::Text,
            _ => Cursor::Arrow,
        }
    }

    /// The primary button went down at `at`: what's pressed there is pressed, and takes the keyboard if it can.
    pub fn pointer_down(&mut self, at: Point<f32>) {
        let pressed = self.region_at(at, true).map(|region| region.id);
        self.pressed = pressed;
        self.focus(pressed);
    }

    /// The primary button came up at `at`: a press, if it's still over what went down.
    pub fn pointer_up(&mut self, at: Point<f32>) {
        let Some(pressed) = self.pressed.take() else { return };
        let over = self.region_at(at, true).filter(|region| region.id == pressed).map(|region| region.kind);
        if over == Some(RegionKind::Press) {
            self.event(pressed, "click", None);
        }
    }

    /// The wheel or a trackpad scrolled by `delta` pixels (down is positive) at `at`: the innermost thing there
    /// that can still scroll that way does.
    /// → what scrolled.
    pub fn scroll(&mut self, at: Point<f32>, delta: f32) -> Option<Id> {
        let target = self.regions.iter().rev().find_map(|region| match region.kind {
            RegionKind::Scroll { max } if region.bounds.contains(&at) => {
                let now = self.scrolled.get(&region.id).copied().unwrap_or(0.0);
                ((delta > 0.0 && now < max) || (delta < 0.0 && now > 0.0)).then_some((region.id, max))
            }
            _ => None,
        });
        let (id, max) = target?;
        let now = self.scrolled.entry(id).or_insert(0.0);
        *now = (*now + delta).clamp(0.0, max);
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
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("keydown")) {
            if let Err(error) = self.sandbox.send("event", &key.event(id)) {
                self.threw(error);
            }
            self.pump();
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
    });
    let body = Text::body(dark);
    let width = (window.size.width - 32.0).max(1.0);
    let paragraph = text.paragraph(words, &body.style, Some(width), None);
    let top = (window.size.height - paragraph.size().height) / 2.0;
    let _ = text.paint(&paragraph, point(px(16.0), px(top)), width, 0.5, colour("muted-foreground", dark), scale, mask, atlas, scene);
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

/// A tree node as laid out: which node, and the text it inherits.
struct Leaf {
    id: Id,
    text: Text,
    measure: Option<Measure>,
}

/// An image, decoded, in the atlas's byte order (BGRA).
struct Picture {
    key: u64,
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

struct Painter<'a> {
    tree: &'a Tree,
    taffy: TaffyTree<()>,
    nodes: HashMap<NodeId, Leaf>,
    text: &'a mut TextSystem,
    atlas: &'a BladeAtlas,
    scene: &'a mut Scene,
    scale: f32,
    dark: bool,
    regions: Vec<Region>,
    tab_order: Vec<Id>,
    scrolled: &'a mut HashMap<Id, f32>,
    focused: Option<Id>,
    typed: &'a HashMap<Id, String>,
    images: &'a mut HashMap<String, Option<Arc<Picture>>>,
    rects: Rects,
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

impl Painter<'_> {
    /// `id` and what's in it as Taffy's nodes. The root fills the window and scrolls, as a page's body.
    fn build(&mut self, id: Id, inherited: Text, root: bool) -> NodeId {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let mut style = layout(&node.style);
        if root {
            style.size = taffy::geometry::Size { width: taffy::style::Dimension::percent(1.0), height: taffy::style::Dimension::percent(1.0) };
            style.overflow = taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll };
            style.scrollbar_width = 0.0;
        }
        let text = inherited.of(&node.style, node.kind == Kind::Text, self.dark);
        let (measure, children) = match node.kind {
            Kind::Text => (Some(Measure::Text { words: self.tree.text(id), text, lines: node.style.line_clamp }), Vec::new()),
            Kind::RawText => (Some(Measure::Text { words: node.value.clone().unwrap_or_default(), text, lines: None }), Vec::new()),
            Kind::Input => {
                let words = self.typed.get(&id).cloned().unwrap_or_else(|| value_of(node));
                let words = if words.is_empty() { node.str("placeholder").unwrap_or_default().to_owned() } else { words };
                (Some(Measure::Input { words: if words.is_empty() { " ".into() } else { words }, text, multiline: node.is("multiline") }), Vec::new())
            }
            Kind::Image => (None, Vec::new()),
            Kind::Box => {
                let children: Vec<Id> = node.children.iter().copied().filter(|child| self.tree.nodes.contains_key(child)).collect();
                (None, children.into_iter().map(|child| self.build(child, text, false)).collect())
            }
        };
        let taffy_node = if children.is_empty() { self.taffy.new_leaf(style) } else { self.taffy.new_with_children(style, &children) }
            .expect("taffy takes any style");
        self.nodes.insert(taffy_node, Leaf { id, text, measure });
        taffy_node
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, bounds: Bounds<f32>, mask: Bounds<f32>, background: Hsla, borders: Edges<f32>, border_color: Hsla, radius: f32, opacity: f32) {
        let radius = radius.min(bounds.size.width / 2.0).min(bounds.size.height / 2.0).max(0.0);
        let scale = self.scale;
        self.scene.insert_primitive(Quad {
            order: 0,
            border_style: Default::default(),
            bounds: bounds.scale_to(scale),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            background: Background::from(background.opacity(opacity)),
            border_color: border_color.opacity(opacity),
            corner_radii: Corners::all(ScaledPixels(radius * scale)),
            border_widths: Edges {
                top: ScaledPixels(borders.top * scale),
                right: ScaledPixels(borders.right * scale),
                bottom: ScaledPixels(borders.bottom * scale),
                left: ScaledPixels(borders.left * scale),
            },
        });
    }

    /// Paints `node` with its parent's top left at `offset`, clipped to `mask`, `opacity` as opaque as it shows.
    fn paint(&mut self, taffy_node: NodeId, offset: Point<f32>, mask: Bounds<f32>, opacity: f32) {
        let Ok(laid) = self.taffy.layout(taffy_node).copied() else { return };
        let Some(leaf) = self.nodes.get(&taffy_node) else { return };
        let (id, text) = (leaf.id, leaf.text);
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let bounds = Bounds { origin: point(offset.x + laid.location.x, offset.y + laid.location.y), size: Size { width: laid.size.width, height: laid.size.height } };
        let look = look(&node.style, self.dark);
        let opacity = opacity * look.opacity;
        let content = Bounds {
            origin: point(bounds.origin.x + laid.border.left + laid.padding.left, bounds.origin.y + laid.border.top + laid.padding.top),
            size: Size { width: laid.content_box_width(), height: laid.content_box_height() },
        };
        let shown = bounds.intersect(&mask);
        let root = id == 0;
        self.rects.insert(id, Rect { x: bounds.origin.x, y: bounds.origin.y, width: bounds.size.width, height: bounds.size.height });
        if !root && (look.background.is_some() || look.borders != Edges::default()) {
            self.quad(bounds, mask, look.background.unwrap_or_default(), look.borders, look.border_color, look.radius, opacity);
        }
        let pressable = node.listens("click") && !node.is("disabled") && !node.is("aria-disabled");
        match node.kind {
            Kind::Text | Kind::RawText => {
                let words = if node.kind == Kind::Text { self.tree.text(id) } else { node.value.clone().unwrap_or_default() };
                let lines = if node.kind == Kind::Text { node.style.line_clamp } else { None };
                let paragraph = self.text.paragraph(&words, &text.style, Some(content.size.width.max(1.0)), lines);
                self.paint_text(&paragraph, content, text, opacity, mask);
            }
            Kind::Input => {
                self.tab_order.push(id);
                self.regions.push(Region { id, bounds: shown, kind: RegionKind::Input });
                let value = self.typed.get(&id).cloned().unwrap_or_else(|| value_of(node));
                let shown_value = if node.str("type") == Some("password") { "•".repeat(value.chars().count()) } else { value.clone() };
                let multiline = node.is("multiline");
                let (words, color) = if shown_value.is_empty() {
                    (node.str("placeholder").unwrap_or_default().to_owned(), colour("muted-foreground", self.dark))
                } else {
                    (shown_value.clone(), text.color)
                };
                let paragraph = self.text.paragraph(&words, &text.style, multiline.then_some(content.size.width), None);
                // A one-line input's text in the middle of its box, as it's drawn in a row centred on its items.
                let top = if multiline { content.origin.y } else { content.origin.y + (content.size.height - paragraph.size().height) / 2.0 };
                let area = Bounds { origin: point(content.origin.x, top), size: content.size };
                let clip = content.intersect(&mask);
                self.paint_text(&paragraph, area, Text { color, ..text }, opacity, clip);
                if self.focused == Some(id) {
                    let caret_at = if shown_value.is_empty() { 0 } else { shown_value.len() };
                    let caret_paragraph = if shown_value.is_empty() { None } else { Some(&paragraph) };
                    let (line, x) = caret_paragraph.map_or((0, 0.0), |paragraph| paragraph.caret(caret_at));
                    let caret = Bounds { origin: point(area.origin.x + x, area.origin.y + line as f32 * text.style.line_height), size: Size { width: 1.0, height: text.style.line_height } };
                    self.quad(caret, clip, colour("foreground", self.dark), Edges::default(), Hsla::default(), 0.0, opacity);
                }
            }
            Kind::Image => self.paint_image(node, content, look.radius, opacity, mask),
            Kind::Box => {
                if pressable {
                    self.tab_order.push(id);
                    self.regions.push(Region { id, bounds: shown, kind: RegionKind::Press });
                }
                let mut child_mask = mask;
                let mut child_offset = bounds.origin;
                if look.clips || root {
                    let padding_box = Bounds {
                        origin: point(bounds.origin.x + laid.border.left, bounds.origin.y + laid.border.top),
                        size: Size { width: bounds.size.width - laid.border.left - laid.border.right, height: bounds.size.height - laid.border.top - laid.border.bottom },
                    };
                    child_mask = padding_box.intersect(&mask);
                    if look.scrolls || root {
                        let max = (laid.scrollable_overflow_rect.bottom - padding_box.size.height).max(0.0);
                        let now = self.scrolled.entry(id).or_insert(0.0);
                        *now = now.clamp(0.0, max);
                        child_offset.y -= *now;
                        self.regions.push(Region { id, bounds: child_mask, kind: RegionKind::Scroll { max } });
                    }
                }
                let children = self.taffy.children(taffy_node).unwrap_or_default();
                for child in children {
                    self.paint(child, child_offset, child_mask, opacity);
                }
            }
        }
    }

    fn paint_text(&mut self, paragraph: &Paragraph, content: Bounds<f32>, text: Text, opacity: f32, mask: Bounds<f32>) {
        let mask = ContentMask { bounds: mask.scale_to(self.scale) };
        let origin = point(px(content.origin.x), px(content.origin.y));
        if let Err(error) = self.text.paint(paragraph, origin, content.size.width, text.align, text.color.opacity(opacity), self.scale, mask, self.atlas, self.scene) {
            log::warn!("text didn't draw: {error:#}");
        }
    }

    /// An image, scaled to fit its box keeping its shape, as GPUI's `img` is by default.
    fn paint_image(&mut self, node: &Node, content: Bounds<f32>, radius: f32, opacity: f32, mask: Bounds<f32>) {
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
        let tile = self.atlas.get_or_insert_with(&AtlasKey::Image(picture.key), &mut || {
            Ok(Some((Size { width: picture.width.into(), height: picture.height.into() }, std::borrow::Cow::Borrowed(picture.bgra.as_slice()))))
        });
        let Ok(Some(tile)) = tile else { return };
        let radius = radius.min(size.width / 2.0).min(size.height / 2.0);
        self.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false,
            opacity,
            bounds: bounds.scale_to(self.scale),
            content_mask: ContentMask { bounds: mask.scale_to(self.scale) },
            corner_radii: Corners::all(ScaledPixels(radius * self.scale)),
            tile,
        });
    }
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
