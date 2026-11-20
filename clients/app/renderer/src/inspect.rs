//! The inspector: how an agent or a test reads a native window and acts on it, as Rune's inspector protocol has it
//! (github.com/anomalyco/rune, PLAN.md §6). Every native window serves it on a local socket of its own
//! (`clients/app/src/native.rs`), and the headless harness (`examples/screenshot.rs`) answers the same requests, so
//! whatever drives one drives the other.
//!
//! # Protocol, version 1
//!
//! JSON, one object a line. A request is `{"v": 1, "id": <any>, "op": "<op>", ...its inputs}`, and its answer
//! `{"v": 1, "id": <the same>, "result": {...}}` or `{"v": 1, "id": <the same>, "error": "<why>"}`, in the order asked.
//! A request of another `v` is refused, saying which this window speaks.
//!
//! A node is found by a selector, the inputs `node` (its id in the tree), `role` (ARIA's: `button`, `searchbox`),
//! `name` (its accessible name, exactly), `text` (words in its name, any case) and `key` (its `id` prop, as the DOM's
//! `id` names an element), every one given matching, in the tree's order; `nth` picks a later match than the first.
//! A point is `x` and `y`, in CSS pixels from the window's top left, as `rect`s are.
//!
//! - `window` → `{entrypoint, width, height, scale, time, paused, failed, focused, leaving, moving, blank,
//!   picture_ms}`: what it shows, its size, its clock, why it can't draw, if it can't (its JS threw, or sent what this
//!   host doesn't speak), the node with the keyboard, how many nodes taken out are still drawn going, how many are on
//!   their way somewhere, how many pictures the frame drawn last drew empty (decoding, or that didn't decode or come),
//!   and the longest a picture took from being asked for to its window having it to draw.
//! - `tree` → `{tree}`: the semantic tree (`Tree::semantics`), as a screen reader reads it, each node with its `rect`.
//! - `query` (a selector) → `{nodes}`: each node that matches, with its `node`, `kind`, `role`, `name`, `rect`, the
//!   `style` and `class` it was given, its colours and lengths `resolved` from the design tokens, `source`, the
//!   `file:line` of the JSX that made it, where its build says, an input's `editing` state as the host has it, what a
//!   person sees of it as the last frame drew it (`opacity`, with what it's in, and `seen`, how much of its box is in
//!   the window, not clipped away), and whether it has the keyboard (`focused`).
//! - `snap` → `{png, width, height}`: the window as drawn, a PNG in base64, at its scale.
//! - `profile` (`at`, ms from now, in order: `[0, 80, 200, 400]` unless given; `snap`: false for no pictures) →
//!   `{frames, time, paused}`: the clock paused and stepped to each time, as `clock` steps it, and the frame there
//!   drawn, each `{at, mutations, script_ms, layout_ms, paint_ms, draws, work, atlas, png, width, height}`: the
//!   mutations its JS made that it applied since the frame before, how long its JS thread ran meanwhile, how long laying it out took and painting it all told (styling,
//!   layout and its scene), its draw calls, what its GPU did, the same on any GPU (`work`: the device pixels of
//!   `quads`, `shadows`, `sprites`, `backdrops` and `rasters` it drew, as often as each is drawn over, the pixels it
//!   `copied` of the frames before, and the bytes it `uploaded`), and its glyph and picture atlas's `tiles`,
//!   `textures`, `bytes` and how much of them is `filled`. The first is painted whole, the rest as the window paints a frame, from what changed; one where
//!   nothing changed costs nothing. The clock stays paused.
//! - `replay` (a selector, or a point, where a press goes down; `moves`, each `{dx, dy, ms}`, a stretch with neither
//!   holding still; `lift`, true unless given; `hz`, the display's frames a second, 60 unless given; `input_hz`, its
//!   moves a second, `hz` unless given; `after`, ms to keep drawing past its end, 2000 unless given, stopping once
//!   nothing moves; `track`, a selector of a node to follow; `snap`: true for each frame's picture) → `{hz, budget_ms,
//!   over, frames, time, paused}`: the press played as `drag` plays it, each move at its own time, and every frame of
//!   the display drawn as the window draws one, each with what `profile`'s says, `at` (ms since it went down), where
//!   `track` was drawn (`x`, `y`) and how far what scrolls under its start shows scrolled (`scroll`, with its stretch
//!   past an end); `over` counts the frames whose painting took longer than `budget_ms`, a frame's time at `hz`.
//!   With `frame_ms`, a frame is drawn only that often, as a device others load draws them, each a late one that
//!   presses it as a window's do (each frame's `pressure`); `leaving` is how much it took out is still drawn going at
//!   its end.
//! - `host` → `{ok, checks, paced, refresh_hz, ticks, presented, inputs}`, after a second and a half: whether its host
//!   keeps the renderer's contract with it (`platform`'s), its window kept moving for a second, then still, each check
//!   `{check, ok, said}`: a tick each refresh, a frame presented each tick, nothing while still, and inputs timed by
//!   the device, in order. A native window answers it; the headless harness, which has no host, doesn't.
//! - `hover` (a selector, or a point) → `{hovered}`: the pointer moves to the point, or the node's middle, pressing
//!   nothing, as Playwright's `hover`: what's under it is hovered and told the pointer moved.
//! - `tap` (a selector, or a point) → `{tapped, editing}`: the pointer moves to the point, or the node's middle, and
//!   presses there as a mouse does, so what takes it is what a person's press reaches, as the window drew it (what
//!   doesn't take the pointer, `pointerEvents: "none"`, and what's clipped away are passed over): an input takes the
//!   keyboard, its caret there, and what listens for `click` gets one. A node whose middle doesn't reach it, scrolled
//!   out of sight or under something, is pressed itself. `clicks` 2 selects the word there and 3 its line, and `shift`
//!   extends the selection.
//! - `type` (`text`, and a selector but for `text`, or the input with the keyboard) → `{typed, value, editing}`: the
//!   text typed into it, a character at a time, each an `input` event.
//! - `ime` (`select`, a `[start, end]` in UTF-16 units, then `preedit`, with its `cursor` in UTF-16 units, or
//!   `commit`, or `finish`) → `{node, editing}`: what an input method selects to replace, composes, commits or
//!   finishes in the input with the keyboard, as a platform's input method says it (`editing::Ime`).
//! - `key` (`key`, as the DOM's `KeyboardEvent.key` names it: `Enter`, `Escape`, `Tab`, `Backspace`, `a`; and
//!   `shift`, `alt`, `ctrl`, `meta`) → `{node}`: pressed where the keyboard is. Tab moves it, Enter and Space press
//!   what has it, what listens for `keydown` hears it, and an input edits by it as a Linux field does (Ctrl+C, X, V,
//!   Z, Shift+Z, Y and A, Ctrl+arrows by word, Shift to select, Home and End), with a clipboard of the window's own
//!   in the headless harness.
//! - `scroll` (`dx`, `dy` in CSS pixels, and a selector of what's in what scrolls, or a point; `phase`, a trackpad's fingers' by
//!   AppKit's names, and `momentum`, as a Mac says them, `view::Phase`) → `{scrolled: {x, y}}`.
//! - `drag` (a selector, or a point, where it starts; `dx`, `dy` in CSS pixels; `steps`, 10 unless given, and `ms`,
//!   200) → `{dragged}`: the pointer goes down there, moves `dx`, `dy` in `steps` even moves, and comes up, as
//!   Playwright's mouse drags, or in a window for a finger, as a phone's finger does, scrolling what scrolls that way
//!   under it, past its end as a rubber band, and flinging it as it lifts; with the clock paused, it's stepped `ms`
//!   over them, so what's let go has their speed.
//! - `clock` (`pause`: true or false, `step`: ms, `seek`: ms since it started) → `{time, paused}`. The app's clock
//!   (`Date`, `performance.now`, its timers) and the host's animations run on it. Paused, nothing is due until it's
//!   stepped; a step runs each timer due in it at its own time, in order.
//! - `settle` (`deadline`, ms, 5000 unless given) → `{settled, ms, pending}`: waits until nothing's pending that ends
//!   by itself, or the deadline, and says what still is: `request`s the app asked its client that aren't answered
//!   (`method`, `tool`, `ms`), `timer`s (`in`, ms, and `every` for an interval, which never ends by itself so is
//!   listed but not waited for), `transition`s on their way (`properties`, how many), and a `frame` that's due as
//!   what it draws changed, which what changed starts moving in. While the clock's paused only requests and frames
//!   are waited for.

use std::collections::HashMap;

use anyhow::{Context as _, bail};
use serde_json::{Map, Value, json};

use crate::tree::{Id, Kind, Tree};

/// The inspector protocol's version, which every request names.
pub const VERSION: u64 = 1;

/// The inputs that select a node.
const SELECTOR: [&str; 5] = ["node", "role", "name", "text", "key"];

/// A rectangle, in CSS pixels from the window's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    fn json(&self) -> Value {
        let round = |n: f32| (n * 100.0).round() / 100.0;
        json!({"x": round(self.x), "y": round(self.y), "width": round(self.width), "height": round(self.height)})
    }
}

/// Where each node was drawn, by its id, as the last frame laid it out.
pub type Rects = HashMap<Id, Rect>;

/// One request, read: its id, its op, and its inputs.
pub struct Request {
    pub id: Value,
    pub op: String,
    pub inputs: Map<String, Value>,
}

impl Request {
    /// Reads a line. A request this can't read is answered with why, by the id it gave, if it gave one.
    pub fn parse(line: &str) -> Result<Request, Value> {
        let mut inputs: Map<String, Value> = match serde_json::from_str(line) {
            Ok(Value::Object(inputs)) => inputs,
            _ => return Err(answer(&Value::Null, Err(anyhow::anyhow!("a request is a JSON object, one a line")))),
        };
        let id = inputs.remove("id").unwrap_or(Value::Null);
        let v = inputs.remove("v");
        if v.as_ref().and_then(Value::as_u64) != Some(VERSION) {
            return Err(answer(&id, Err(anyhow::anyhow!("this window speaks version {VERSION} of the inspector protocol, not {}", v.unwrap_or(Value::Null)))));
        }
        let Some(Value::String(op)) = inputs.remove("op") else {
            return Err(answer(&id, Err(anyhow::anyhow!("say which op"))));
        };
        Ok(Request { id, op, inputs })
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.inputs.get(name).and_then(Value::as_str)
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        self.inputs.get(name).and_then(Value::as_f64).map(|n| n as f32)
    }

    pub fn f64(&self, name: &str) -> Option<f64> {
        self.inputs.get(name).and_then(Value::as_f64)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.inputs.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    /// The inputs that select a node: all of `SELECTOR` but for `type`, whose `text` is what's typed.
    fn selector(&self) -> impl Iterator<Item = &'static str> + '_ {
        SELECTOR.into_iter().filter(|input| !(self.op == "type" && *input == "text"))
    }

    /// Whether it names a node, by a selector.
    pub fn selects(&self) -> bool {
        self.selector().any(|input| self.inputs.contains_key(input))
    }

    /// One of its selector's inputs.
    fn wants(&self, name: &str) -> Option<&str> {
        self.selector().any(|input| input == name).then(|| self.str(name)).flatten()
    }

    /// The point it names, if it names one.
    pub fn point(&self) -> Option<(f32, f32)> {
        Some((self.f32("x")?, self.f32("y")?))
    }
}

/// Whether a request that draws frames reads each back as a picture: `snap` always, `profile` unless it says `snap`
/// false, and `replay` only where it says `snap` true.
pub fn snaps(request: &Request) -> bool {
    let said = request.inputs.get("snap").and_then(Value::as_bool);
    match request.op.as_str() {
        "snap" => true,
        "replay" => said == Some(true),
        _ => said != Some(false),
    }
}

/// The answer to `id`: its result, or why it has none.
pub fn answer(id: &Value, result: anyhow::Result<Value>) -> Value {
    match result {
        Ok(result) => json!({"v": VERSION, "id": id, "result": result}),
        Err(error) => json!({"v": VERSION, "id": id, "error": format!("{error:#}")}),
    }
}

/// A picture as `snap` gives it: a PNG, in base64, of `rgba`, `width` by `height` pixels.
pub fn png(width: u32, height: u32, rgba: Vec<u8>) -> anyhow::Result<Value> {
    let mut png = Vec::new();
    image::RgbaImage::from_raw(width, height, rgba)
        .context("its pixels don't fill it")?
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(json!({"png": data_encoding::BASE64.encode(&png), "width": width, "height": height}))
}

/// Each node's semantics as `Tree::semantics` gives them, by its id, without its children.
fn semantics(tree: &Tree) -> HashMap<Id, Map<String, Value>> {
    fn walk(nodes: &[Value], into: &mut HashMap<Id, Map<String, Value>>) {
        for node in nodes {
            let Value::Object(node) = node else { continue };
            let mut node = node.clone();
            if let Some(Value::Array(children)) = node.remove("children") {
                walk(&children, into);
            }
            if let Some(id) = node.remove("id").and_then(|id| id.as_u64()) {
                into.insert(id, node);
            }
        }
    }
    let mut described = HashMap::new();
    walk(&tree.semantics(0), &mut described);
    described
}

/// Every node in the tree's order, from the root's first child: what's drawn first first, as a page's document order.
pub fn order(tree: &Tree) -> Vec<Id> {
    fn walk(tree: &Tree, id: Id, into: &mut Vec<Id>) {
        let Some(node) = tree.nodes.get(&id) else { return };
        for child in &node.children {
            into.push(*child);
            walk(tree, *child, into);
        }
    }
    let mut order = Vec::new();
    walk(tree, 0, &mut order);
    order
}

/// The nodes the request's selector matches, in the tree's order.
pub fn select(tree: &Tree, request: &Request) -> Vec<Id> {
    let described = semantics(tree);
    let want = |name: &str| request.wants(name);
    let text = want("text").map(str::to_lowercase);
    order(tree)
        .into_iter()
        .filter(|id| {
            let node = &tree.nodes[id];
            let said = described.get(id);
            let said = |name: &str| said.and_then(|said| said.get(name)).and_then(Value::as_str);
            request.inputs.get("node").is_none_or(|node| node.as_u64() == Some(*id))
                && want("role").is_none_or(|role| said("role") == Some(role))
                && want("name").is_none_or(|name| said("name") == Some(name))
                && text.as_ref().is_none_or(|text| said("name").is_some_and(|name| name.to_lowercase().contains(text)))
                && want("key").is_none_or(|key| node.str("id") == Some(key))
        })
        .collect()
}

/// The one node the request's selector names: its `nth` match, the first unless it says.
pub fn one(tree: &Tree, request: &Request) -> anyhow::Result<Id> {
    let nth = request.inputs.get("nth").and_then(Value::as_u64).unwrap_or(0) as usize;
    let matched = select(tree, request);
    matched.get(nth).copied().with_context(|| match matched.len() {
        0 => "no node matches that".to_owned(),
        found => format!("only {found} nodes match that, so there's no nth {nth}"),
    })
}

/// Whether a press does something to it: it's an input, or it listens for `click` and isn't disabled.
pub fn pressable(tree: &Tree, id: Id) -> bool {
    tree.nodes.get(&id).is_some_and(|node| {
        node.kind == Kind::Input || (node.listens("click") && !node.is("disabled") && !node.is("aria-disabled"))
    })
}

/// The semantic tree, each node with where it was drawn.
pub fn tree(tree: &Tree, rects: &Rects) -> Value {
    fn place(nodes: &mut [Value], rects: &Rects) {
        for node in nodes {
            let Value::Object(node) = node else { continue };
            if let Some(rect) = node.get("id").and_then(Value::as_u64).and_then(|id| rects.get(&id)) {
                node.insert("rect".into(), rect.json());
            }
            if let Some(Value::Array(children)) = node.get_mut("children") {
                place(children, rects);
            }
        }
    }
    let mut nodes = tree.semantics(0);
    place(&mut nodes, rects);
    json!({"tree": nodes})
}

/// The nodes a `query` matches, each with all the inspector knows of it.
pub fn query(tree: &Tree, rects: &Rects, request: &Request) -> Value {
    let described = semantics(tree);
    let nodes: Vec<Value> = select(tree, request)
        .into_iter()
        .map(|id| {
            let node = &tree.nodes[&id];
            let mut out = described.get(&id).cloned().unwrap_or_default();
            out.insert("node".into(), json!(id));
            out.insert("kind".into(), serde_json::to_value(node.kind).unwrap_or(Value::Null));
            if let Some(rect) = rects.get(&id) {
                out.insert("rect".into(), rect.json());
            }
            // Its styles by id, and what they come to, merged, as it's drawn with nothing hovered.
            if let Some(ids) = &node.styles {
                out.insert("style".into(), json!(ids));
                out.insert("resolved".into(), Value::Object(crate::style::resolve(ids, crate::style::State::default())));
            }
            for prop in ["class", "id"] {
                if let Some(value) = node.prop(prop) {
                    out.insert(if prop == "id" { "key".into() } else { prop.into() }, value.clone());
                }
            }
            if let Some(source) = node.str("data-source") {
                out.insert("source".into(), json!(source));
            }
            out.insert("children".into(), json!(node.children));
            Value::Object(out)
        })
        .collect();
    json!({"nodes": nodes})
}

/// A request the app posted its client that hasn't been answered yet.
pub struct Asked {
    pub method: String,
    pub tool: Option<String>,
    pub since: web_time::Instant,
}

/// What's still pending, as `settle` lists it, from what the app asked and hasn't heard back, its timers
/// (`Thread::timers`, each `{in, every?}`), how many properties are on their way (`moving`), whether what it draws
/// changed since the frame drawn last (`changed`), so a frame is due, which what changed starts moving in, and whether
/// its JS thread is still running what it was sent (`running`), or starting.
pub fn pending(asked: &HashMap<String, Asked>, timers: &[Value], moving: usize, animating: bool, changed: bool, running: bool) -> Vec<Value> {
    let mut pending: Vec<Value> = asked
        .values()
        .map(|asked| json!({"kind": "request", "method": asked.method, "tool": asked.tool, "ms": asked.since.elapsed().as_millis() as u64}))
        .collect();
    pending.extend(timers.iter().map(|timer| {
        let mut timer = timer.clone();
        timer["kind"] = json!("timer");
        timer
    }));
    if moving > 0 {
        pending.push(json!({"kind": "transition", "properties": moving}));
    }
    // What its page moves a frame at a time (a spring on `requestAnimationFrame`), or a list flung.
    if animating {
        pending.push(json!({"kind": "animation"}));
    }
    if changed {
        pending.push(json!({"kind": "frame"}));
    }
    if running {
        pending.push(json!({"kind": "script"}));
    }
    pending
}

/// How many of a replay's or a profile's `frames` took longer than `budget` ms to make: their JS, laying out and
/// painting, and their GPU's time where it was waited for, all of which the frame waits on before the display's refresh
/// shows it.
pub fn over(frames: &[Value], budget: f64) -> usize {
    let ms = |frame: &Value, name: &str| frame[name].as_f64().unwrap_or(0.0);
    frames.iter().filter(|frame| ms(frame, "script_ms") + ms(frame, "paint_ms") + ms(frame, "gpu_ms") > budget).count()
}

/// Whether nothing's pending that ends by itself: no request unanswered, no frame due, and, while the clock runs, no
/// timer but intervals and nothing on its way.
pub fn settled(pending: &[Value], paused: bool) -> bool {
    pending.iter().all(|pending| match pending["kind"].as_str() {
        Some("timer") => paused || pending.get("every").is_some_and(|every| !every.is_null()),
        Some("transition" | "animation") => paused,
        _ => false,
    })
}

/// What a `key` request presses, as the DOM's `KeyboardEvent` has it.
pub struct Key {
    pub key: String,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
}

impl Key {
    pub fn of(request: &Request) -> anyhow::Result<Key> {
        let Some(key) = request.str("key") else { bail!("say which key, as the DOM names it: Enter, Escape, Tab, a") };
        Ok(Key { key: key.to_owned(), shift: request.bool("shift"), alt: request.bool("alt"), ctrl: request.bool("ctrl"), meta: request.bool("meta") })
    }

    /// What its `keydown` event says of it.
    pub fn told(&self) -> Value {
        json!({"key": self.key, "shiftKey": self.shift, "altKey": self.alt, "ctrlKey": self.ctrl, "metaKey": self.meta})
    }
}
