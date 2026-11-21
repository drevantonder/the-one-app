//! The tree an app's mutations build (`Mutations` in core/ui/src/drawing-primitives/renderer.ts), as React Native's mounting
//! layer applies Fabric's, which the window draws and which is its semantic tree: each node's role, name, value,
//! states and actions.

use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// A node, as the app names it. `0` is the root, which the window makes.
pub type Id = u64;

/// JS → host: one mutation of the tree, as the app's JS called it (`js::Sandbox`).
pub enum Mutation {
    Create { id: Id, kind: Kind, value: Option<String> },
    Set { id: Id, name: String, value: Value },
    Insert { parent: Id, id: Id, before: Option<Id> },
    Remove { parent: Id, id: Id },
    Drop { id: Id },
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Box,
    /// A box whose content scrolls along its `flexDirection`.
    Scroll,
    /// A column of `count` rows of which only those near the window are in it, each by its `index`, as UIKit's
    /// collection view and Android's RecyclerView hold only those: it's as tall as all of them, those it hasn't drawn
    /// as tall as the drawn ones are on average (`estimate` until it has drawn one), and asks for the rows it needs by
    /// `range` (`view::list`).
    List,
    Text,
    Image,
    /// A vector shape, SVG's `<path>`: its `d` in its `viewBox`.
    Path,
    /// The device's own chooser, see-through over its box, which a computer draws nothing of: the UI library draws
    /// one only where a finger points.
    Picker,
    Input,
    RawText,
    /// A screen's live video over WebRTC, drawn in its box as big as fits: on the web the browser connects and plays it
    /// (`platform::web`, `clients/web/renderer/video.js`), and natively its own connection does (`video`).
    Video,
    /// Another app's page (`src`), for `subject`, drawn in its box by a sandbox of its own, as an iframe is (`Screen`'s
    /// frames): its own JS, calling as its own app.
    Frame,
}

pub struct Node {
    pub kind: Kind,
    /// A `raw-text`'s text.
    pub value: Option<String>,
    /// Every prop but `style`, as it was set.
    pub props: Map<String, Value>,
    /// Its styles, each by id or an object of its own, merged in order (`style::resolve`).
    pub styles: Option<Vec<Value>>,
    pub children: Vec<Id>,
    /// What it's in, so it's taken out of only that as it moves.
    parent: Option<Id>,
}

impl Node {
    fn new(kind: Kind, value: Option<String>) -> Node {
        Node { kind, value, props: Map::new(), styles: None, children: Vec::new(), parent: None }
    }

    pub fn prop(&self, name: &str) -> Option<&Value> {
        self.props.get(name).filter(|value| !value.is_null())
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.prop(name).and_then(Value::as_str)
    }

    /// Whether a state is so: `true`, or ARIA's `"true"`.
    pub fn is(&self, name: &str) -> bool {
        matches!(self.prop(name), Some(Value::Bool(true))) || self.str(name) == Some("true")
    }

    /// Whether it isn't displayed (`display: none`), by its styles, so neither it nor what's in it is drawn or read.
    pub fn hidden(&self) -> bool {
        self.styles.as_ref().is_some_and(|styles| crate::style::resolve(styles, crate::style::State::default()).get("display").and_then(Value::as_str) == Some("none"))
    }

    /// Whether it listens for `event`, by its prop as JSX names it (`keydown` is `onKeyDown`, as the DOM's).
    pub fn listens(&self, event: &str) -> bool {
        let prop = match event {
            "click" => "onClick",
            "input" => "onInput",
            "action" => "onAction",
            "focus" => "onFocus",
            "blur" => "onBlur",
            "keydown" => "onKeyDown",
            "pointerenter" => "onPointerEnter",
            "pointerleave" => "onPointerLeave",
            "pointerdown" => "onPointerDown",
            "pointermove" => "onPointerMove",
            "pointerup" => "onPointerUp",
            "pointercancel" => "onPointerCancel",
            "contextmenu" => "onContextMenu",
            "wheel" => "onWheel",
            "scroll" => "onScroll",
            "layout" => "onLayout",
            "intersection" => "onIntersection",
            "range" => "onRange",
            "drop" => "onDrop",
            "dragstart" => "onDragStart",
            "drag" => "onDrag",
            "dragend" => "onDragEnd",
            _ => return false,
        };
        self.is(prop)
    }
}


pub struct Tree {
    pub nodes: HashMap<Id, Node>,
    /// What changed since the window last took it (`take_changed`): each node whose own props, style, text or
    /// children did, and each `text` whose words did, so only that's laid out and painted again.
    changed: HashSet<Id>,
    /// The inputs, and what says `autofocus` or listens for `layout`, found without going through every node.
    inputs: HashSet<Id>,
    autofocus: HashSet<Id>,
    layout: HashSet<Id>,
    /// What's been taken out but is drawn going, as its `exiting` style moves it out (Motion's `AnimatePresence`, CSS's
    /// exit transitions), until the screen says it's gone (`left`): inert, read by nobody, as if gone. What in it
    /// the page has dropped meanwhile goes with it.
    leaving: HashSet<Id>,
    dropped: HashSet<Id>,
    /// What moves from where it was laid out to where it's laid out now as that changes (`layout`, `layoutId`).
    moves_layout: HashSet<Id>,
}

impl Default for Tree {
    fn default() -> Self {
        Tree {
            nodes: HashMap::from([(0, Node::new(Kind::Box, None))]),
            changed: HashSet::from([0]),
            inputs: HashSet::new(),
            autofocus: HashSet::new(),
            layout: HashSet::new(),
            leaving: HashSet::new(),
            dropped: HashSet::new(),
            moves_layout: HashSet::new(),
        }
    }
}

/// The roles a node is named by what's in it where it isn't named otherwise, as WAI-ARIA's "name from content" has
/// them (and the DOM's accessibility tree): a menu's items, a list's options and tabs as a button is.
const NAMED_BY_CONTENT: [&str; 13] = ["button", "radio", "checkbox", "switch", "link", "menuitem", "menuitemcheckbox", "menuitemradio", "option", "tab", "treeitem", "cell", "tooltip"];

impl Tree {
    pub fn root(&self) -> &Node {
        &self.nodes[&0]
    }

    /// What `id` is in.
    pub fn parent(&self, id: Id) -> Option<Id> {
        self.nodes.get(&id)?.parent
    }

    /// The outermost text `id` is inside, where it's inside one, which draws it as a run of its own.
    pub fn outer_text(&self, id: Id) -> Option<Id> {
        let mut outer = None;
        let mut at = self.parent(id);
        while let Some(parent) = at.filter(|parent| self.nodes.get(parent).is_some_and(|node| node.kind == Kind::Text)) {
            outer = Some(parent);
            at = self.parent(parent);
        }
        outer
    }

    /// Whether a text in text `id` (not displayed: not) says something of its own to a screen reader: a role, a
    /// name, or that it's pressed, as an inline link does.
    fn speaks(&self, id: Id) -> bool {
        self.nodes.get(&id).is_some_and(|node| {
            node.children.iter().any(|child| {
                self.nodes.get(child).is_some_and(|inner| {
                    inner.kind == Kind::Text
                        && !inner.hidden()
                        && (inner.str("role").is_some() || inner.str("aria-label").is_some() || inner.listens("click") || self.speaks(*child))
                })
            })
        })
    }

    /// Whether `id` is inert, or in what is (`inert`, as the DOM's).
    pub fn inert(&self, id: Id) -> bool {
        let mut at = Some(id);
        while let Some((id, node)) = at.and_then(|id| Some((id, self.nodes.get(&id)?))) {
            if node.is("inert") || self.leaving.contains(&id) {
                return true;
            }
            at = node.parent;
        }
        false
    }

    /// Whether `id` was taken out and is drawn going (`leaving`), itself.
    pub fn leaving(&self, id: Id) -> bool {
        self.leaving.contains(&id)
    }

    /// Whether `id` is going, or in what is.
    pub fn leaving_in(&self, id: Id) -> bool {
        let mut at = Some(id);
        while let Some(id) = at.filter(|_| !self.leaving.is_empty()) {
            if self.leaving.contains(&id) {
                return true;
            }
            at = self.parent(id);
        }
        false
    }

    /// What's going.
    pub fn going(&self) -> impl Iterator<Item = Id> + '_ {
        self.leaving.iter().copied()
    }

    /// `id` has gone: it's taken out, and what in it the page has dropped is gone too.
    pub fn left(&mut self, id: Id) {
        if !self.leaving.remove(&id) {
            return;
        }
        self.detach(id);
        let mut at = vec![id];
        while let Some(one) = at.pop() {
            at.extend(self.nodes.get(&one).map(|node| node.children.clone()).unwrap_or_default());
            if self.dropped.remove(&one) {
                self.forget(one);
            }
        }
    }

    /// Whether anything in `id`, or it, looks a way of its own as it goes.
    fn exits(&self, id: Id) -> bool {
        self.nodes.get(&id).is_some_and(|node| node.styles.as_ref().is_some_and(|styles| crate::style::exits(styles)) || node.children.iter().any(|child| self.exits(*child)))
    }

    /// What moves from where it was laid out as that changes, as Motion's `layout`, and with what it shares its place
    /// with (`layoutId`), as Motion's and SwiftUI's `matchedGeometryEffect`.
    pub fn moves_layout(&self) -> impl Iterator<Item = (Id, Option<&str>)> + '_ {
        self.moves_layout.iter().filter_map(|id| {
            let node = self.nodes.get(id)?;
            let shared = node.str("layoutId");
            (node.is("layout") || shared.is_some()).then_some((*id, shared))
        })
    }

    fn forget(&mut self, id: Id) {
        self.moves_layout.remove(&id);
        self.detach(id);
        self.nodes.remove(&id);
        self.inputs.remove(&id);
        self.autofocus.remove(&id);
        self.layout.remove(&id);
    }

    /// Whether nothing's changed since `take_changed` was last asked.
    pub fn unchanged(&self) -> bool {
        self.changed.is_empty()
    }

    /// What's changed since this was last asked, and forgets it.
    pub fn take_changed(&mut self) -> HashSet<Id> {
        std::mem::take(&mut self.changed)
    }

    pub fn inputs(&self) -> impl Iterator<Item = Id> + '_ {
        self.inputs.iter().copied()
    }

    /// What says `autofocus`.
    pub fn autofocused(&self) -> impl Iterator<Item = Id> + '_ {
        self.autofocus.iter().copied().filter(|id| self.nodes.get(id).is_some_and(|node| node.is("autofocus")))
    }

    /// What listens for `layout`.
    pub fn laid_out(&self) -> impl Iterator<Item = Id> + '_ {
        self.layout.iter().copied().filter(|id| self.nodes.get(id).is_some_and(|node| node.listens("layout")))
    }

    /// `id` changed, and so did the words of each `text` it's in.
    fn change(&mut self, id: Id) {
        let mut at = Some(id);
        while let Some(changed) = at {
            self.changed.insert(changed);
            at = self.nodes.get(&changed).and_then(|node| node.parent).filter(|parent| self.nodes.get(parent).is_some_and(|node| node.kind == Kind::Text));
        }
    }

    /// Applies the app's mutations, in order.
    pub fn apply(&mut self, mutations: impl IntoIterator<Item = Mutation>) -> anyhow::Result<()> {
        for mutation in mutations {
            match mutation {
                Mutation::Create { id, kind, value } => {
                    self.nodes.insert(id, Node::new(kind, value));
                    if kind == Kind::Input {
                        self.inputs.insert(id);
                    }
                    self.changed.insert(id);
                }
                Mutation::Set { id, name, value } => {
                    let node = self.nodes.get_mut(&id).with_context(|| format!("there's no node {id}"))?;
                    match name.as_str() {
                        "autofocus" => drop(self.autofocus.insert(id)),
                        "onLayout" => drop(self.layout.insert(id)),
                        "layout" | "layoutId" => drop(self.moves_layout.insert(id)),
                        _ => {}
                    }
                    match name.as_str() {
                        "style" => match value {
                            Value::Array(styles) => node.styles = Some(styles),
                            Value::Null => node.styles = None,
                            value => anyhow::bail!("node {id}'s style is {value}, not a list of styles: a page from before styles()"),
                        },
                        "value" if node.kind == Kind::RawText => node.value = value.as_str().map(str::to_owned),
                        _ => drop(node.props.insert(name, value)),
                    }
                    self.change(id);
                }
                Mutation::Insert { parent, id, before } => {
                    anyhow::ensure!(self.nodes.contains_key(&id), "there's no node {id}");
                    // Put back as it goes, it comes back.
                    if self.leaving.remove(&id) {
                        self.change(id);
                    }
                    self.detach(id);
                    let into = self.nodes.get_mut(&parent).with_context(|| format!("there's no node {parent}"))?;
                    let at = before.and_then(|before| into.children.iter().position(|child| *child == before));
                    into.children.insert(at.unwrap_or(into.children.len()), id);
                    self.nodes.get_mut(&id).expect("it's there").parent = Some(parent);
                    self.change(parent);
                }
                // What looks a way of its own as it goes is drawn going first.
                Mutation::Remove { id, .. } if self.exits(id) => {
                    self.leaving.insert(id);
                    self.change(id);
                }
                Mutation::Remove { id, .. } => self.detach(id),
                Mutation::Drop { id } if self.leaving_in(id) => drop(self.dropped.insert(id)),
                Mutation::Drop { id } => self.forget(id),
            }
        }
        Ok(())
    }

    /// Takes `id` out of what it's in, as a node inserted again moves.
    fn detach(&mut self, id: Id) {
        let Some(parent) = self.nodes.get_mut(&id).and_then(|node| node.parent.take()) else { return };
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.retain(|child| *child != id);
            self.change(parent);
        }
    }

    /// What a `text` says: its runs of text, and those of the texts in it.
    pub fn text(&self, id: Id) -> String {
        let Some(node) = self.nodes.get(&id) else { return String::new() };
        match node.kind {
            Kind::RawText => node.value.clone().unwrap_or_default(),
            _ => node.children.iter().map(|child| self.text(*child)).collect(),
        }
    }

    /// The semantic tree from `id`, as a screen reader and an agent read it: each node's role, name, value, states
    /// and actions. A plain group with nothing to say is left out, its children in its place, as AccessKit's
    /// generic containers are, and so is what's `aria-hidden`. While a modal (`aria-modal`) is shown, it's all that's
    /// read: what's behind it is inert, as the DOM's is.
    pub fn semantics(&self, id: Id) -> Vec<Value> {
        let mut path = Vec::new();
        let held = self.modal(id, &mut path).then_some(path);
        self.read(id, held.as_deref(), false)
    }

    /// The semantic tree from `id` with what a screen reader doesn't read in it too (`aria-hidden`, inert, not
    /// displayed, behind a modal), as Playwright's `includeHidden` finds it: what's drawn, whether or not it's read.
    pub fn semantics_including_hidden(&self, id: Id) -> Vec<Value> {
        self.read(id, None, true)
    }

    /// Whether there's a modal shown in `id`, the last one there putting the way to it, from `id`, in `path`.
    fn modal(&self, id: Id, path: &mut Vec<Id>) -> bool {
        let Some(node) = self.nodes.get(&id) else { return false };
        if node.hidden() || node.str("aria-hidden") == Some("true") || node.is("inert") || self.leaving.contains(&id) {
            return false;
        }
        path.push(id);
        if node.children.iter().rev().any(|child| self.modal(*child, path)) || node.str("aria-modal") == Some("true") {
            return true;
        }
        path.pop();
        false
    }

    /// The semantic tree from `id`, where `held` is the way to the modal shown, which only what's on it is read on,
    /// and with what's `hidden` from a screen reader where it says.
    fn read(&self, id: Id, held: Option<&[Id]>, hidden: bool) -> Vec<Value> {
        let Some(node) = self.nodes.get(&id) else { return Vec::new() };
        if self.leaving.contains(&id) || !hidden && (node.hidden() || node.str("aria-hidden") == Some("true") || node.is("inert")) {
            return Vec::new();
        }
        let held = match held {
            Some([at, rest @ ..]) if *at == id => (!rest.is_empty()).then_some(rest),
            Some(_) => return Vec::new(),
            None => None,
        };
        // A text that's only words, with a text in it that says something of its own (an inline link), is its words
        // and that text side by side, as the DOM's spans are read.
        if node.kind == Kind::Text && node.str("role").is_none() && node.str("aria-label").is_none() && !node.listens("click") && self.speaks(id) {
            return node.children.iter().flat_map(|child| self.read(*child, held, hidden)).collect();
        }
        // What's in a frame is its own page's, which the screen puts in its place (`Screen::semantics`).
        if node.kind == Kind::Frame {
            return vec![json!({"frame": id})];
        }
        let text = matches!(node.kind, Kind::Text | Kind::RawText);
        let mut children: Vec<Value> =
            if text { Vec::new() } else { node.children.iter().flat_map(|child| self.read(*child, held, hidden)).collect() };
        // A `list`'s rows say where each is in all of them, which a screen reader reads ("3 of 10,000"), as ARIA's
        // `aria-posinset` and `aria-setsize` have it: it holds only those near the window.
        if node.kind == Kind::List {
            let count = node.prop("count").cloned().unwrap_or(json!(node.children.len()));
            children = node
                .children
                .iter()
                .flat_map(|child| {
                    let index = self.nodes.get(child).and_then(|row| row.prop("index")).and_then(Value::as_u64);
                    let mut read = self.read(*child, held, hidden);
                    for one in read.iter_mut().filter_map(Value::as_object_mut) {
                        if let Some(index) = index {
                            one.insert("posinset".into(), json!(index + 1));
                            one.insert("setsize".into(), count.clone());
                        }
                    }
                    read
                })
                .collect();
        }
        let role = node.str("role").map(str::to_owned).or_else(|| match node.kind {
            Kind::Text | Kind::RawText => Some("text".into()),
            Kind::Image if node.str("alt").is_some_and(|alt| !alt.is_empty()) => Some("img".into()),
            Kind::Video => Some("img".into()),
            Kind::Input if node.str("type") == Some("search") => Some("searchbox".into()),
            Kind::Input => Some("textbox".into()),
            _ => None,
        });
        let name = node
            .str("aria-label")
            .or_else(|| node.str("alt"))
            // A video's, what it shows (renderer.ts's `label`), as an image's `alt`.
            .or_else(|| node.str("label"))
            .map(str::to_owned)
            .or_else(|| text.then(|| self.text(id)))
            .or_else(|| node.str("role").is_some_and(|role| NAMED_BY_CONTENT.contains(&role)).then(|| self.label(id, hidden)));
        let actions: Vec<&str> =
            ["click", "input", "focus", "blur"].into_iter().filter(|event| node.listens(event)).collect();
        // A live region, whose words a screen reader reads as they change: as it says, or as its role is one (ARIA's
        // `status` is polite, its `alert` assertive).
        let live = node.str("aria-live").map(str::to_owned).or_else(|| match role.as_deref() {
            Some("status" | "log") => Some("polite".into()),
            Some("alert") => Some("assertive".into()),
            _ => None,
        });
        if role.is_none() && actions.is_empty() && node.str("aria-label").is_none() && live.is_none() {
            return children;
        }
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        // What holds a list's rows says how many there are, as AccessKit has it on the container.
        if let Some(size) = children.iter().filter(|child| child.get("posinset").is_some()).find_map(|child| child.get("setsize")) {
            out.insert("setsize".into(), size.clone());
        }
        if let Some(role) = role {
            out.insert("role".into(), json!(role));
        }
        if let Some(live) = live.filter(|live| live != "off") {
            out.insert("live".into(), json!(live));
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            out.insert("name".into(), json!(name));
        }
        // An input's value is its text, as its app last set it, but a password's, which is never read out.
        let secret = node.str("type") == Some("password");
        if node.kind == Kind::Input
            && !secret
            && let Some(text) = node.prop("editing").and_then(|editing| editing.get("text"))
        {
            out.insert("value".into(), text.clone());
        }
        for (prop, said) in [
            ("aria-description", "description"),
            ("aria-level", "level"),
            ("value", "value"),
            ("aria-valuenow", "value"),
            ("aria-valuemin", "min"),
            ("aria-valuemax", "max"),
            ("placeholder", "placeholder"),
        ] {
            if let Some(value) = node.prop(prop).filter(|_| !(secret && prop == "value")) {
                out.insert(said.into(), value.clone());
            }
        }
        let mut states: Map<String, Value> = ["selected", "checked", "expanded", "busy", "current", "invalid", "disabled"]
            .into_iter()
            .filter_map(|state| {
                let value = node.prop(&format!("aria-{state}"))?;
                Some((state.to_owned(), match value.as_str() {
                    Some("true") => json!(true),
                    Some("false") => json!(false),
                    _ => value.clone(),
                }))
            })
            .collect();
        for state in ["disabled", "required"] {
            if node.is(state) {
                states.insert(state.into(), json!(true));
            }
        }
        if !states.is_empty() {
            out.insert("states".into(), Value::Object(states));
        }
        if !actions.is_empty() {
            out.insert("actions".into(), json!(actions));
        }
        if !children.is_empty() {
            out.insert("children".into(), Value::Array(children));
        }
        vec![Value::Object(out)]
    }

    /// What's written in `id`, its texts' words one after another, as ARIA names something from what's in it: what
    /// isn't read (`aria-hidden`, or not displayed) says nothing, but where it's read anyway (`hidden`), and what's named
    /// (`aria-label`) says its name. A button's name where it has no `aria-label`.
    fn label(&self, id: Id, hidden: bool) -> String {
        let Some(node) = self.nodes.get(&id) else { return String::new() };
        if !hidden && (node.hidden() || node.str("aria-hidden") == Some("true") || node.is("inert")) {
            return String::new();
        }
        if let Some(name) = node.str("aria-label") {
            return name.to_owned();
        }
        match node.kind {
            Kind::Text | Kind::RawText => self.text(id),
            _ => node.children.iter().map(|child| self.label(*child, hidden)).filter(|words| !words.is_empty()).collect::<Vec<_>>().join(" "),
        }
    }
}
