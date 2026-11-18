//! What the inspector (`crate::inspect`) asks of a drawn screen: where it drew each node and how it takes input. What's read from the tree alone is `inspect`'s.

use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use serde_json::{Value, json};

use super::{Edit, Screen};
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
        reply(inspect::answer(&request.id, result))
    }

    fn act(&mut self, request: &Request) -> anyhow::Result<Value> {
        let dark = self.dark();
        Ok(match request.op.as_str() {
            "window" => {
                let (size, scale) = self.painted;
                json!({
                    "width": size.width,
                    "height": size.height,
                    "scale": scale,
                    "time": self.sandbox.clock().now(),
                    "paused": self.sandbox.clock().paused(),
                    "failed": self.failed,
                })
            }
            "tree" => inspect::tree(&self.tree, &self.rects),
            "query" => inspect::query(&self.tree, &self.rects, dark, request),
            "tap" => {
                let target = if request.selects() {
                    inspect::one(&self.tree, request)?
                } else {
                    let (x, y) = request.point().context("say what to tap: a node, by a selector, or a point, x and y")?;
                    inspect::at(&self.tree, &self.rects, x, y).with_context(|| format!("nothing is drawn at {x}, {y}"))?
                };
                let tapped = self.press(target)?;
                json!({"tapped": tapped})
            }
            "type" => {
                let text = request.str("text").context("say what to type, as text")?.to_owned();
                let input = match request.selects() {
                    true => {
                        let id = inspect::one(&self.tree, request)?;
                        self.press(id)?
                    }
                    false => self.focused.context("nothing has the keyboard: say which input, by a selector")?,
                };
                if self.tree.nodes.get(&input).is_none_or(|node| node.kind != Kind::Input) {
                    bail!("node {input} isn't an input, so it takes no text");
                }
                let mut buffer = [0; 4];
                for typed in text.chars() {
                    self.edit(input, Edit::Type(typed.encode_utf8(&mut buffer)));
                }
                json!({"typed": input, "value": self.typed.get(&input)})
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
                let scrolled = self.scroll(crate::geometry::point(x, y), crate::geometry::point(dx, dy)).and_then(|id| self.scrolled.get(&id).copied()).unwrap_or_default();
                json!({"scrolled": {"x": scrolled.x, "y": scrolled.y}})
            }
            "clock" => {
                let clock = self.sandbox.clock().clone();
                match request.inputs.get("pause").and_then(Value::as_bool) {
                    Some(true) => clock.pause(),
                    Some(false) => {
                        clock.resume();
                        self.pump();
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
            op => bail!("there's no op {op}: window, tree, query, snap, tap, type, key, scroll, clock and settle are"),
        })
    }

    /// Presses node `id`, as a finger or a pointer would: what takes the press is it or what it's in, an input
    /// taking the keyboard, and what listens for `click` getting one. → what took it.
    fn press(&mut self, id: Id) -> anyhow::Result<Id> {
        let mut at = Some(id);
        while let Some(node) = at.filter(|node| !inspect::pressable(&self.tree, *node)) {
            at = inspect::parent(&self.tree, node).filter(|parent| *parent != 0);
        }
        let pressed = at.with_context(|| format!("node {id} takes no press, and neither does what it's in"))?;
        self.focus(Some(pressed));
        if self.tree.nodes[&pressed].kind != Kind::Input {
            self.event(pressed, "click", None);
        }
        Ok(pressed)
    }

    /// Moves the clock on by `ms`, paused, running each timer due in that time at its own time, in order.
    fn step(&mut self, ms: f64) {
        let clock = self.sandbox.clock().clone();
        clock.pause();
        let mut left = ms;
        while let Some(due) = self.sandbox.due().filter(|due| *due <= left) {
            let due = due.max(0.0);
            clock.advance(due);
            left -= due;
            self.fire();
        }
        clock.advance(left);
        self.pump();
    }

    /// Tells each `settle` waiting that it's settled, if it is, or what's pending once its deadline has come.
    pub(super) fn tell_settled(&mut self) {
        if self.settling.is_empty() {
            return;
        }
        let pending = inspect::pending(&self.unanswered, &self.sandbox.timers(), self.transitions.len());
        let settled = inspect::settled(&pending, self.sandbox.clock().paused());
        let now = Instant::now();
        let (told, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.settling).into_iter().partition(|(until, _)| settled || *until <= now);
        self.settling = waiting;
        for (_, tell) in told {
            tell(json!({"settled": settled, "pending": pending}));
        }
    }
}
