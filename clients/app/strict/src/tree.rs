//! The tree an app's ops build (`core/ui/src/strict/protocol.ts`), which the window draws and which is its semantic
//! tree: each node's role, name, value, states and actions.

use std::collections::HashMap;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// A node, as the app names it. `0` is the root, which the window makes.
pub type Id = u64;

/// JS → host: one batch of ops.
#[derive(Deserialize)]
pub struct Update {
    pub v: u32,
    pub ops: Vec<Op>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    Create {
        id: Id,
        #[serde(rename = "type")]
        kind: Kind,
        value: Option<String>,
    },
    Set {
        id: Id,
        name: String,
        value: Value,
    },
    Insert {
        parent: Id,
        id: Id,
        before: Option<Id>,
    },
    Remove {
        parent: Id,
        id: Id,
    },
    Drop {
        id: Id,
    },
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Box,
    Text,
    Image,
    Input,
    RawText,
}

/// A length: pixels or a word (`gap`, `auto`, `50%`, `control`).
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Length {
    Px(f32),
    Word(String),
}

/// The strict style subset, as the protocol has it.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Style {
    pub display: Option<String>,
    pub flex_direction: Option<String>,
    pub flex_wrap: Option<String>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<Length>,
    pub align_items: Option<String>,
    pub align_self: Option<String>,
    pub justify_content: Option<String>,
    pub grid_template_columns: Option<String>,
    pub grid_column: Option<String>,
    pub gap: Option<Length>,
    pub row_gap: Option<Length>,
    pub column_gap: Option<Length>,
    pub padding: Option<Length>,
    pub padding_inline: Option<Length>,
    pub padding_block: Option<Length>,
    pub padding_top: Option<Length>,
    pub padding_right: Option<Length>,
    pub padding_bottom: Option<Length>,
    pub padding_left: Option<Length>,
    pub margin: Option<Length>,
    pub margin_inline: Option<Length>,
    pub margin_block: Option<Length>,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub min_width: Option<Length>,
    pub min_height: Option<Length>,
    pub max_width: Option<Length>,
    pub max_height: Option<Length>,
    pub aspect_ratio: Option<f32>,
    pub overflow: Option<String>,
    pub opacity: Option<f32>,
    pub background_color: Option<String>,
    pub color: Option<String>,
    pub border_color: Option<String>,
    pub border_width: Option<u8>,
    pub border_top_width: Option<u8>,
    pub border_right_width: Option<u8>,
    pub border_bottom_width: Option<u8>,
    pub border_left_width: Option<u8>,
    pub border_radius: Option<Length>,
    pub font: Option<String>,
    pub text_align: Option<String>,
    pub font_variant_numeric: Option<String>,
    pub line_clamp: Option<usize>,
}

pub struct Node {
    pub kind: Kind,
    /// A `raw-text`'s text.
    pub value: Option<String>,
    /// Every prop but `style`, as it was set.
    pub props: Map<String, Value>,
    pub style: Style,
    pub children: Vec<Id>,
}

impl Node {
    fn new(kind: Kind, value: Option<String>) -> Node {
        Node { kind, value, props: Map::new(), style: Style::default(), children: Vec::new() }
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

    pub fn listens(&self, event: &str) -> bool {
        let mut on = String::from("on");
        on.push_str(&event[..1].to_uppercase());
        on.push_str(&event[1..]);
        self.is(&on)
    }
}

pub struct Tree {
    pub nodes: HashMap<Id, Node>,
}

impl Default for Tree {
    fn default() -> Self {
        Tree { nodes: HashMap::from([(0, Node::new(Kind::Box, None))]) }
    }
}

impl Tree {
    pub fn root(&self) -> &Node {
        &self.nodes[&0]
    }

    /// Applies one batch, in order. A batch of a version this host doesn't speak is refused whole.
    pub fn apply(&mut self, update: Update) -> anyhow::Result<()> {
        if update.v != crate::VERSION {
            bail!("this host speaks version {} of the strict protocol, not {}", crate::VERSION, update.v);
        }
        for op in update.ops {
            match op {
                Op::Create { id, kind, value } => {
                    self.nodes.insert(id, Node::new(kind, value));
                }
                Op::Set { id, name, value } => {
                    let node = self.nodes.get_mut(&id).with_context(|| format!("there's no node {id}"))?;
                    match name.as_str() {
                        "style" => node.style = if value.is_null() { Style::default() } else { serde_json::from_value(value)? },
                        "value" if node.kind == Kind::RawText => node.value = value.as_str().map(str::to_owned),
                        _ => drop(node.props.insert(name, value)),
                    }
                }
                Op::Insert { parent, id, before } => {
                    self.detach(id);
                    let parent = self.nodes.get_mut(&parent).with_context(|| format!("there's no node {parent}"))?;
                    let at = before.and_then(|before| parent.children.iter().position(|child| *child == before));
                    parent.children.insert(at.unwrap_or(parent.children.len()), id);
                }
                Op::Remove { parent, id } => {
                    if let Some(parent) = self.nodes.get_mut(&parent) {
                        parent.children.retain(|child| *child != id);
                    }
                }
                Op::Drop { id } => {
                    self.detach(id);
                    self.nodes.remove(&id);
                }
            }
        }
        Ok(())
    }

    /// Takes `id` out of wherever it is, as a node inserted again moves.
    fn detach(&mut self, id: Id) {
        for node in self.nodes.values_mut() {
            node.children.retain(|child| *child != id);
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
    /// generic containers are, and so is what's `aria-hidden`.
    pub fn semantics(&self, id: Id) -> Vec<Value> {
        let Some(node) = self.nodes.get(&id) else { return Vec::new() };
        if node.style.display.as_deref() == Some("none") || node.str("aria-hidden") == Some("true") {
            return Vec::new();
        }
        let text = matches!(node.kind, Kind::Text | Kind::RawText);
        let children: Vec<Value> =
            if text { Vec::new() } else { node.children.iter().flat_map(|child| self.semantics(*child)).collect() };
        let role = node.str("role").map(str::to_owned).or_else(|| match node.kind {
            Kind::Text | Kind::RawText => Some("text".into()),
            Kind::Image if node.str("alt").is_some_and(|alt| !alt.is_empty()) => Some("img".into()),
            Kind::Input if node.str("type") == Some("search") => Some("searchbox".into()),
            Kind::Input => Some("textbox".into()),
            _ => None,
        });
        let name = node
            .str("aria-label")
            .or_else(|| node.str("alt"))
            .map(str::to_owned)
            .or_else(|| text.then(|| self.text(id)))
            .or_else(|| (node.str("role") == Some("button") || node.str("role") == Some("radio")).then(|| self.label(id)));
        let actions: Vec<&str> =
            ["click", "input", "focus", "blur"].into_iter().filter(|event| node.listens(event)).collect();
        if role.is_none() && actions.is_empty() && node.str("aria-label").is_none() {
            return children;
        }
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        if let Some(role) = role {
            out.insert("role".into(), json!(role));
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            out.insert("name".into(), json!(name));
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
            if let Some(value) = node.prop(prop) {
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

    /// What's written in `id`, its texts' words one after another: a button's name where it has no `aria-label`.
    fn label(&self, id: Id) -> String {
        let Some(node) = self.nodes.get(&id) else { return String::new() };
        match node.kind {
            Kind::Text | Kind::RawText => self.text(id),
            _ => node.children.iter().map(|child| self.label(*child)).filter(|words| !words.is_empty()).collect::<Vec<_>>().join(" "),
        }
    }
}
