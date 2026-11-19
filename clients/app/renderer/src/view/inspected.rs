//! What the inspector (`crate::inspect`) asks of a drawn screen: where it drew each node and how it takes input. What's read from the tree alone is `inspect`'s.

use std::time::Duration;
use web_time::Instant;

use anyhow::{Context as _, bail};
use serde_json::{Value, json};

use super::Screen;
use crate::editing::{self, Ime};
use crate::inspect::{self, Key, Request};
use crate::tree::{Id, Kind};

/// What tells the inspector's client the answer, once it has one.
pub type Reply = Box<dyn FnOnce(Value)>;

impl Screen {
    /// Answers an inspector request, but `snap`, which is the window's to answer, as it has its pixels. It reads where
    /// each node was drawn as the last frame laid it out, so it's asked once a frame has been drawn after what changed
    /// last. `reply` is told once, at once but for `settle`.
    pub fn inspect(&mut self, request: Request, reply: Reply) {
        if request.op == "settle" {
            let deadline = Duration::from_secs_f64(request.f64("deadline").unwrap_or(5000.0).max(0.0) / 1000.0);
            let started = Instant::now();
            let id = request.id.clone();
            self.settling.push((started + deadline, Box::new(move |result: Value| {
                let mut result = result;
                result["ms"] = json!(started.elapsed().as_millis() as u64);
                reply(inspect::answer(&id, Ok(result)))
            })));
            // The window calls `fire` at the deadline, as `due` says.
            return self.tell_settled();
        }
        let result = self.act(&request);
        let answer = inspect::answer(&request.id, result);
        // What acts is told once the app's JS has run what it did, as a test's click resolves once its handlers have
        // run; what reads, at once.
        let acts = matches!(request.op.as_str(), "tap" | "type" | "ime" | "key" | "scroll");
        match acts && self.js.busy() {
            true => self.replies.push((answer, reply)),
            false => reply(answer),
        }
    }

    fn act(&mut self, request: &Request) -> anyhow::Result<Value> {
        Ok(match request.op.as_str() {
            "window" => {
                let (size, scale) = self.painted;
                json!({
                    "width": size.width,
                    "height": size.height,
                    "scale": scale,
                    "time": self.js.clock().now(),
                    "paused": self.js.clock().paused(),
                    "failed": self.failed,
                })
            }
            "tree" => self.inspected_tree(),
            "query" => {
                let mut found = inspect::query(&self.tree, &self.rects, request);
                // An input's editing state as the host has it, which is ahead of what its app set.
                for node in found["nodes"].as_array_mut().into_iter().flatten() {
                    if let Some(field) = node["node"].as_u64().and_then(|id| self.editing.fields.get(&id)) {
                        node["editing"] = json!(field.value());
                    }
                }
                found
            }
            "tap" => {
                // As a test clicks: the point, or a node's middle where that reaches it as the pointer finds it
                // there; the pointer moves there, goes down and comes up, as a mouse's does, so what takes it is what
                // a person's press would reach. A node whose middle doesn't reach it (scrolled out of sight, or under
                // something) is pressed itself, as it was asked for by name.
                let at = match request.selects() {
                    true => {
                        let node = inspect::one(&self.tree, request)?;
                        let at = self.rects.get(&node).map(|rect| rect.centre()).map(|(x, y)| crate::geometry::point(x, y));
                        let reached = at.map(|at| self.under(at)).unwrap_or_default();
                        if !reached.contains(&node) && (reached.is_empty() || self.pressed_in(&reached) != self.pressed_in(&self.chain_from(node))) {
                            let tapped = self.press(node)?;
                            return Ok(json!({"tapped": tapped, "editing": self.editing.fields.get(&tapped).map(editing::Field::value)}));
                        }
                        at.expect("a node reached is drawn")
                    }
                    false => {
                        let (x, y) = request.point().context("say what to tap: a node, by a selector, or a point, x and y")?;
                        crate::geometry::point(x, y)
                    }
                };
                // In a frame, what its page has there takes it, which the frame passes on. Where nothing takes a
                // press, it's clicked all the same, as a person's click there is: what had the keyboard loses it.
                let tapped = match self.frame_at(at) {
                    Some((frame, _)) => Some(frame),
                    None => self.pressed_in(&self.under(at)),
                };
                self.pointer_moved(at);
                self.pointer_down(at, 0, request.bool("shift"));
                self.pointer_up(at, 0);
                // A point in an input: `clicks` 2 selects a word there, 3 its line.
                let clicks = request.inputs.get("clicks").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3) as u8;
                if let Some(tapped) = tapped.filter(|tapped| clicks > 1 && self.editing.fields.contains_key(tapped)) {
                    self.press_input(tapped, at, clicks, request.bool("shift"));
                }
                json!({"tapped": tapped, "focused": self.focus.focused, "editing": tapped.and_then(|tapped| self.editing.fields.get(&tapped)).map(editing::Field::value)})
            }
            "type" => {
                let text = request.str("text").context("say what to type, as text")?.to_owned();
                let input = match request.selects() {
                    true => {
                        let id = inspect::one(&self.tree, request)?;
                        self.press(id)?
                    }
                    false => {
                        // An input in a frame's page with the keyboard takes it there.
                        if let Some(child) = self.focused_frame() {
                            let input = child.focus.focused.filter(|id| child.editing.fields.contains_key(id)).context("nothing in the frame with the keyboard takes text")?;
                            child.type_text(input, &text);
                            let editing = child.editing.fields.get(&input).map(editing::Field::value);
                            return Ok(json!({"typed": input, "value": editing.as_ref().map(|editing| &editing.text), "editing": editing}));
                        }
                        self.focus.focused.context("nothing has the keyboard: say which input, by a selector")?
                    }
                };
                if self.tree.nodes.get(&input).is_none_or(|node| node.kind != Kind::Input) {
                    bail!("node {input} isn't an input, so it takes no text");
                }
                self.type_text(input, &text);
                let editing = self.editing.fields.get(&input).map(editing::Field::value);
                json!({"typed": input, "value": editing.as_ref().map(|editing| &editing.text), "editing": editing})
            }
            "ime" => {
                let input = self.focus.focused.filter(|id| self.editing.fields.contains_key(id)).context("no input has the keyboard: tap one first")?;
                if let Some([start, end]) = request.inputs.get("select").and_then(|range| serde_json::from_value::<[usize; 2]>(range.clone()).ok()) {
                    self.ime(Ime::Select(start..end));
                }
                match (request.str("commit"), request.str("preedit")) {
                    _ if request.bool("finish") => self.ime(Ime::Finish),
                    (Some(commit), _) => self.ime(Ime::Commit(commit.to_owned())),
                    (None, Some(preedit)) => {
                        let cursor = request.inputs.get("cursor").and_then(Value::as_u64).map(|units| editing::byte(preedit, units as usize));
                        self.ime(Ime::Preedit(preedit.to_owned(), cursor))
                    }
                    (None, None) if request.inputs.contains_key("select") => {}
                    (None, None) => bail!("say what the input method composes (`preedit`, and its `cursor`), commits (`commit`), selects (`select`) or finishes (`finish`)"),
                }
                json!({"node": input, "editing": self.editing.fields.get(&input).map(editing::Field::value)})
            }
            "key" => {
                let key = Key::of(request)?;
                let node = self.key(&key);
                json!({"node": node})
            }
            "scroll" => {
                let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
                // What scrolls around the node selected, or the window.
                let at = if request.selects() { self.rects.get(&inspect::one(&self.tree, request)?).map(|rect| rect.centre()) } else { None };
                let (x, y) = at.unwrap_or((0.0, 0.0));
                // The trackpad's phase, as a Mac's says it, where it's given.
                let fingers = request.inputs.get("phase").and_then(Value::as_str).and_then(|named| super::Phase::FINGERS.into_iter().find(|one| *one == named));
                let momentum = request.inputs.get("momentum").and_then(Value::as_bool);
                let phase = (fingers.is_some() || momentum.is_some()).then(|| super::Phase { fingers, momentum: momentum.unwrap_or(false) });
                let scrolled = self.scroll(crate::geometry::point(x, y), crate::geometry::point(dx, dy), phase).and_then(|id| self.scrolled.get(&id).copied()).unwrap_or_default();
                json!({"scrolled": {"x": scrolled.x, "y": scrolled.y}})
            }
            "clock" => {
                let clock = self.js.clock().clone();
                match request.inputs.get("pause").and_then(Value::as_bool) {
                    Some(true) => clock.pause(),
                    Some(false) => {
                        clock.resume();
                        self.js.fire();
                    }
                    None => {}
                }
                if let Some(at) = request.f64("seek") {
                    let ms = at - clock.now();
                    if ms < 0.0 {
                        bail!("the clock goes forward only: it's at {} ms", clock.now());
                    }
                    self.step(ms);
                }
                if let Some(ms) = request.f64("step") {
                    self.step(ms.max(0.0));
                }
                json!({"time": clock.now(), "paused": clock.paused()})
            }
            op => bail!("there's no op {op}: window, tree, query, snap, tap, type, ime, key, scroll, clock and settle are"),
        })
    }

    /// Presses node `id`, as a finger or a pointer would: what takes the press is it or what it's in, an input
    /// taking the keyboard, and what listens for `click` getting one. → what took it.
    pub(crate) fn press(&mut self, id: Id) -> anyhow::Result<Id> {
        if self.press_chrome(id) {
            return Ok(id);
        }
        let mut at = Some(id);
        while let Some(node) = at.filter(|node| !inspect::pressable(&self.tree, *node)) {
            at = self.tree.parent(node).filter(|parent| *parent != 0);
        }
        let pressed = at.with_context(|| format!("node {id} takes no press, and neither does what it's in"))?;
        self.click_focus(&self.chain_from(pressed));
        if self.tree.nodes[&pressed].kind != Kind::Input {
            self.click(pressed, None);
        }
        Ok(pressed)
    }

    /// Moves the clock on by `ms`, paused, running each timer due in that time at its own time, in order, each once
    /// what came before it has run: this waits on the app's JS thread.
    fn step(&mut self, ms: f64) {
        // While its page asks for frames, they come as a 60 Hz display's would, on its clock.
        const FRAME: f64 = 1000.0 / 60.0;
        let clock = self.js.clock().clone();
        clock.pause();
        self.idle();
        let mut left = ms;
        loop {
            // The next frame's, at least a microsecond on, so a clock a hair short of one isn't stepped by nothing.
            let frame = self.framing().then(|| FRAME - clock.now().rem_euclid(FRAME)).map(|next| if next < 0.001 { next + FRAME } else { next });
            let Some(next) = self.js.due().into_iter().chain(frame).reduce(f64::min).filter(|next| *next <= left) else { break };
            let framed = frame == Some(next);
            let next = next.max(0.0);
            clock.advance(next);
            left -= next;
            if framed {
                self.frame(web_time::Instant::now());
            }
            self.js.fire();
            self.idle();
        }
        clock.advance(left);
        self.js.fire();
        self.idle();
    }

    /// Tells each `settle` waiting that it's settled, if it is, or what's pending once its deadline has come.
    pub(super) fn tell_settled(&mut self) {
        if self.settling.is_empty() {
            return;
        }
        let mut pending = inspect::pending(&self.unanswered, &self.js.timers(), self.transitions.len(), self.changed.is_some(), self.busy());
        // What a frame's page asked and hasn't heard back is this one's to wait for too.
        for child in self.frames.values() {
            pending.extend(inspect::pending(&child.screen.unanswered, &[], 0, false, false));
        }
        // A picture decoding in the background is drawn once it's ready.
        let pictures = self.pictures.pending() + self.frames.values().map(|child| child.screen.pictures.pending()).sum::<usize>();
        if pictures > 0 {
            pending.push(json!({"kind": "picture", "decoding": pictures}));
        }
        let settled = inspect::settled(&pending, self.js.clock().paused());
        let now = Instant::now();
        let (told, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.settling).into_iter().partition(|(until, _)| settled || *until <= now);
        self.settling = waiting;
        for (_, tell) in told {
            tell(json!({"settled": settled, "pending": pending}));
        }
    }
}
