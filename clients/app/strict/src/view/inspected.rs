//! What the inspector (`crate::inspect`) asks of a drawn screen: what the window draws and how it takes input, which a
//! renderer of the host's own answers in its place. What's read from the tree alone is `inspect`'s.

use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use gpui::{Context, Window, point, px};
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
    pub fn inspect(&mut self, request: Request, reply: Reply, window: &mut Window, cx: &mut Context<Screen>) {
        if request.op == "settle" {
            let deadline = Duration::from_secs_f64(request.f64("deadline").unwrap_or(5000.0).max(0.0) / 1000.0);
            let started = Instant::now();
            let id = request.id.clone();
            self.settling.push((started + deadline, Box::new(move |result: Value| {
                let mut result = result;
                result["ms"] = json!(started.elapsed().as_millis() as u64);
                reply(inspect::answer(&id, Ok(result)))
            })));
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(deadline).await;
                let _ = this.update(cx, |this, _| this.tell_settled());
            })
            .detach();
            return self.tell_settled();
        }
        let result = self.act(&request, window, cx);
        reply(inspect::answer(&request.id, result))
    }

    fn act(&mut self, request: &Request, window: &mut Window, cx: &mut Context<Screen>) -> anyhow::Result<Value> {
        let dark = self.dark(window);
        Ok(match request.op.as_str() {
            "window" => {
                let size = window.viewport_size();
                json!({
                    "width": f32::from(size.width),
                    "height": f32::from(size.height),
                    "scale": window.scale_factor(),
                    "time": self.sandbox.clock().now(),
                    "paused": self.sandbox.clock().paused(),
                    "failed": self.failed,
                })
            }
            "tree" => inspect::tree(&self.tree, &self.rects.borrow()),
            "query" => inspect::query(&self.tree, &self.rects.borrow(), dark, request),
            "tap" => {
                let target = if request.selects() {
                    inspect::one(&self.tree, request)?
                } else {
                    let (x, y) = request.point().context("say what to tap: a node, by a selector, or a point, x and y")?;
                    inspect::at(&self.tree, &self.rects.borrow(), x, y).with_context(|| format!("nothing is drawn at {x}, {y}"))?
                };
                let tapped = self.press(target, window, cx)?;
                json!({"tapped": tapped})
            }
            "type" => {
                let text = request.str("text").context("say what to type, as text")?.to_owned();
                let input = match request.selects() {
                    true => {
                        let id = inspect::one(&self.tree, request)?;
                        self.press(id, window, cx)?
                    }
                    false => self.focused(window).context("nothing has the keyboard: say which input, by a selector")?,
                };
                if self.tree.nodes.get(&input).is_none_or(|node| node.kind != Kind::Input) {
                    bail!("node {input} isn't an input, so it takes no text");
                }
                let mut buffer = [0; 4];
                for typed in text.chars() {
                    self.edit(input, Edit::Type(typed.encode_utf8(&mut buffer)), window, cx);
                }
                json!({"typed": input, "value": self.typed.get(&input)})
            }
            "key" => {
                let key = Key::of(request)?;
                let node = self.key(&key, window, cx);
                json!({"node": node})
            }
            "scroll" => {
                let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
                let (offset, most) = (self.scroll.offset(), self.scroll.max_offset());
                // GPUI's offset is how far the content has moved: up and left are negative.
                let x = (f32::from(offset.x) - dx).clamp(-f32::from(most.x), 0.0);
                let y = (f32::from(offset.y) - dy).clamp(-f32::from(most.y), 0.0);
                self.scroll.set_offset(point(px(x), px(y)));
                cx.notify();
                json!({"scrolled": {"x": -x, "y": -y}})
            }
            "clock" => {
                let clock = self.sandbox.clock().clone();
                match request.inputs.get("pause").and_then(Value::as_bool) {
                    Some(true) => {
                        clock.pause();
                        self.timer = None;
                    }
                    Some(false) => {
                        clock.resume();
                        self.pump(window, cx);
                    }
                    None => {}
                }
                if let Some(at) = request.f64("seek") {
                    let ms = at - clock.now();
                    if ms < 0.0 {
                        bail!("the clock goes forward only: it's at {} ms", clock.now());
                    }
                    self.step(ms, window, cx);
                }
                if let Some(ms) = request.f64("step") {
                    self.step(ms.max(0.0), window, cx);
                }
                json!({"time": clock.now(), "paused": clock.paused()})
            }
            op => bail!("there's no op {op}: window, tree, query, snap, tap, type, key, scroll, clock and settle are"),
        })
    }

    /// Presses node `id`, as a finger or a pointer would: what takes the press is it or what it's in, an input
    /// taking the keyboard, and what listens for `click` getting one. → what took it.
    fn press(&mut self, id: Id, window: &mut Window, cx: &mut Context<Screen>) -> anyhow::Result<Id> {
        let mut at = Some(id);
        while let Some(node) = at.filter(|node| !inspect::pressable(&self.tree, *node)) {
            at = inspect::parent(&self.tree, node).filter(|parent| *parent != 0);
        }
        let pressed = at.with_context(|| format!("node {id} takes no press, and neither does what it's in"))?;
        if let Some((handle, _)) = self.focus.get(&pressed) {
            window.focus(&handle.clone(), cx);
        }
        if self.tree.nodes[&pressed].kind != Kind::Input {
            self.event(pressed, "click", None, window, cx);
        }
        Ok(pressed)
    }

    /// What has the keyboard.
    fn focused(&self, window: &Window) -> Option<Id> {
        self.focus.iter().find(|(_, (handle, _))| handle.is_focused(window)).map(|(id, _)| *id)
    }

    /// Presses a key where the keyboard is, as the window's own keys do. → what had the keyboard.
    fn key(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Screen>) -> Option<Id> {
        let focused = self.focused(window);
        if key.key == "Tab" {
            if key.shift { window.focus_prev(cx) } else { window.focus_next(cx) }
            return focused;
        }
        let id = focused?;
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("keydown")) {
            if let Err(error) = self.sandbox.send("event", &key.event(id)) {
                self.failed = Some(format!("{error:#}"));
            }
            self.pump(window, cx);
        }
        let input = self.tree.nodes.get(&id).is_some_and(|node| node.kind == Kind::Input);
        match key.key.as_str() {
            "Enter" if input => drop(self.edit(id, Edit::Enter, window, cx)),
            "Backspace" if input => drop(self.edit(id, Edit::Backspace, window, cx)),
            "Enter" | " " if !input => self.event(id, "click", None, window, cx),
            typed if input && typed.chars().count() == 1 && !key.ctrl && !key.meta => drop(self.edit(id, Edit::Type(typed), window, cx)),
            _ => {}
        }
        Some(id)
    }

    /// Moves the clock on by `ms`, paused, running each timer due in that time at its own time, in order.
    fn step(&mut self, ms: f64, window: &mut Window, cx: &mut Context<Screen>) {
        let clock = self.sandbox.clock().clone();
        clock.pause();
        self.timer = None;
        let mut left = ms;
        while let Some(due) = self.sandbox.due().filter(|due| *due <= left) {
            let due = due.max(0.0);
            clock.advance(due);
            left -= due;
            self.fire(window, cx);
        }
        clock.advance(left);
        self.pump(window, cx);
    }

    /// Tells each `settle` waiting that it's settled, if it is, or what's pending once its deadline has come.
    pub(super) fn tell_settled(&mut self) {
        if self.settling.is_empty() {
            return;
        }
        let pending = inspect::pending(&self.unanswered, &self.sandbox.timers());
        let settled = inspect::settled(&pending, self.sandbox.clock().paused());
        let now = Instant::now();
        let (told, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.settling).into_iter().partition(|(until, _)| settled || *until <= now);
        self.settling = waiting;
        for (_, tell) in told {
            tell(json!({"settled": settled, "pending": pending}));
        }
    }
}
