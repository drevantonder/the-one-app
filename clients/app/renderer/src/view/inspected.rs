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
        let acts = matches!(request.op.as_str(), "tap" | "type" | "ime" | "key" | "scroll" | "drag");
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
                    "focused": self.focus.focused,
                    "leaving": self.tree.going().count(),
                    "moving": self.transitions.moving().len(),
                    "blank": self.pictures.blank + self.frames.values().map(|child| child.screen.pictures.blank).sum::<usize>(),
                    "picture_ms": self.frames.values().map(|child| child.screen.pictures.slowest).fold(self.pictures.slowest, Duration::max).as_millis() as u64,
                })
            }
            "tree" => self.inspected_tree(),
            "query" => {
                let mut found = inspect::query(&self.tree, &self.rects, request);
                // An input's editing state as the host has it, which is ahead of what its app set.
                let window = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: crate::geometry::Size { width: self.room.width, height: self.room.height } };
                for node in found["nodes"].as_array_mut().into_iter().flatten() {
                    let Some(id) = node["node"].as_u64() else { continue };
                    if let Some(field) = self.editing.fields.get(&id) {
                        node["editing"] = json!(field.value());
                    }
                    // What a person sees of it: how opaque it's drawn, and how much of it is in the window, not
                    // clipped away.
                    if let Some(placed) = self.placed(id) {
                        let rect = self.rects.get(&id).map_or(0.0, |rect| rect.width * rect.height);
                        let shown = placed.shown.intersect(&window);
                        let seen = if rect > 0.0 { (shown.size.width.max(0.0) * shown.size.height.max(0.0) / rect).min(1.0) } else { 0.0 };
                        node["opacity"] = json!((placed.opacity * 1000.0).round() / 1000.0);
                        node["seen"] = json!((seen * 1000.0).round() / 1000.0);
                    }
                    node["focused"] = json!(self.focus.focused == Some(id));
                }
                found
            }
            "tap" => {
                // As a test clicks: the point, or a node's middle where that reaches it as the pointer finds it
                // there; the pointer moves there, goes down and comes up, as a mouse's does, or a finger on a window for
                // one, through what every phone's host takes a finger by (`touch`), so what takes it is what a person's
                // press would reach. A node out of sight (scrolled away, or clipped) is pressed itself, as it was asked
                // for by name; one under something in sight isn't, as a person's press there reaches what's over it,
                // which it says.
                let at = match request.selects() {
                    true => {
                        let node = inspect::one(&self.tree, request)?;
                        let at = self.rects.get(&node).map(|rect| rect.centre()).map(|(x, y)| crate::geometry::point(x, y));
                        let reached = at.map(|at| self.under(at)).unwrap_or_default();
                        let seen = at.zip(self.placed(node)).is_some_and(|(at, placed)| placed.shown.contains(&at));
                        let other = self.pressed_in(&reached) != self.pressed_in(&self.chain_from(node));
                        if !reached.contains(&node) && (reached.is_empty() || other) {
                            if seen && !reached.is_empty() {
                                let over = reached.first().and_then(|id| self.tree.nodes.get(id)).and_then(|over| over.str("aria-label").or(over.str("role")).map(str::to_owned));
                                bail!("its middle is under something else{}, and a press there reaches {}", over.map_or(String::new(), |over| format!(" ({over})")), self.pressed_in(&reached).map_or("nothing".into(), |id| format!("node {id}")));
                            }
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
                if self.touch {
                    let time = self.js.clock().now();
                    self.touch(&super::touch::Fingers::PHONE, 0, at, super::touch::Phase::Down, time);
                    self.touch(&super::touch::Fingers::PHONE, 0, at, super::touch::Phase::Up, time + 80.0);
                } else {
                    self.pointer_moved(at);
                    self.pointer_down(at, 0, request.bool("shift"));
                    self.pointer_up(at, 0);
                }
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
            "hover" => {
                let (x, y) = match request.selects() {
                    true => self.rects.get(&inspect::one(&self.tree, request)?).map(|rect| rect.centre()).context("that node isn't drawn")?,
                    false => request.point().context("say where to hover: a node, by a selector, or a point, x and y")?,
                };
                self.pointer_moved(crate::geometry::point(x, y));
                json!({"hovered": {"x": x, "y": y}})
            }
            "scroll" => {
                let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
                // What scrolls around the node selected, or at a point, or the window.
                let at = if request.selects() { self.rects.get(&inspect::one(&self.tree, request)?).map(|rect| rect.centre()) } else { request.point() };
                let (x, y) = at.unwrap_or((0.0, 0.0));
                // The trackpad's phase, as a Mac's says it, where it's given.
                let fingers = request.inputs.get("phase").and_then(Value::as_str).and_then(|named| super::Phase::FINGERS.into_iter().find(|one| *one == named));
                let momentum = request.inputs.get("momentum").and_then(Value::as_bool);
                let phase = (fingers.is_some() || momentum.is_some()).then(|| super::Phase { fingers, momentum: momentum.unwrap_or(false) });
                let scrolled = self.scroll(crate::geometry::point(x, y), crate::geometry::point(dx, dy), phase).and_then(|id| self.scrolled.get(&id).copied()).unwrap_or_default();
                json!({"scrolled": {"x": scrolled.x, "y": scrolled.y}})
            }
            "drag" => {
                // `steps` even moves over `ms`, each at its own time, the clock stepped to each where it's paused.
                let start = self.start_of(request)?;
                let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
                let steps = request.inputs.get("steps").and_then(Value::as_u64).unwrap_or(10).max(1);
                let ms = request.f64("ms").unwrap_or(200.0).max(0.0);
                let trace = super::finger::Trace { start, moves: vec![(dx, dy, ms)], lift: true };
                self.play(&trace, ms / steps as f64, self.hz, None, 0.0, None, None)?;
                json!({"dragged": {"x": dx, "y": dy}})
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
            op => bail!("there's no op {op}: window, tree, query, snap, profile, replay, tap, type, ime, key, scroll, drag, clock and settle are"),
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

    /// The inspector's `profile`: the clock paused, then stepped to each time `at` says (ms from now, in order; 0, 80,
    /// 200 and 400 unless it says), and the frame there drawn by `draw`, as the window draws one, with what it cost
    /// (`cost`). The clock stays paused, as `clock` leaves it.
    pub fn profile(&mut self, request: &Request, draw: &mut dyn FnMut(&mut Screen) -> anyhow::Result<Value>) -> anyhow::Result<Value> {
        let at: Vec<f64> = match request.inputs.get("at") {
            None => vec![0.0, 80.0, 200.0, 400.0],
            Some(at) => serde_json::from_value(at.clone()).context("say `at` as a list of ms from now: [0, 80, 200, 400]")?,
        };
        if at.windows(2).any(|pair| pair[1] < pair[0]) || at.first().is_some_and(|first| *first < 0.0) {
            bail!("the clock goes forward only: say `at` in order, from 0");
        }
        let clock = self.js.clock().clone();
        clock.pause();
        self.idle();
        // Its first frame's cost is what's done from here.
        self.cost = Default::default();
        let start = clock.now();
        let mut frames = Vec::new();
        for ms in at {
            self.step(start + ms - clock.now());
            let mut frame = draw(self)?;
            frame["at"] = json!(ms);
            frames.push(frame);
        }
        // The window's own scene is older than what was drawn here, so its next frame is painted afresh.
        self.seen = None;
        Ok(json!({"frames": frames, "time": clock.now(), "paused": true}))
    }

    /// The inspector's `replay`: a press goes down where it says (a selector, or `x`, `y`), moves along `moves` (each
    /// `{dx, dy, ms}`, a stretch with neither holding still), sampled `input_hz` times a second (`hz` unless it says),
    /// and lifts unless `lift` is false, with the clock paused and stepped to each move and each frame of a display
    /// `hz` times a second (60 unless it says), each frame drawn by `draw` as the window draws one, until `after` ms
    /// past its end (2000 unless it says) or nothing moves. → each frame's cost, where `track` (a selector) was drawn
    /// (`x`, `y`), how far what scrolls under its start shows scrolled (`scroll`, its stretch past an end with it), and
    /// the frame's budget (`budget_ms`), and how many frames took longer to make (`over`, `inspect::over`). A frame where
    /// what `track` selects isn't drawn says it's `gone`. With `frame_ms`, its
    /// frames are drawn only that often, as a device others load draws them, and pressed as that presses a window
    /// (each frame's `pressure`). `leaving` is how much that was taken out is still drawn going at its end.
    pub fn replay(&mut self, request: &Request, draw: &mut dyn FnMut(&mut Screen) -> anyhow::Result<Value>) -> anyhow::Result<Value> {
        let start = self.start_of(request)?;
        let moves: Vec<Value> = request.inputs.get("moves").and_then(Value::as_array).cloned().context("say how it moves: `moves`, each {dx, dy, ms}")?;
        let moves = moves.iter().map(|one| (one["dx"].as_f64().unwrap_or(0.0) as f32, one["dy"].as_f64().unwrap_or(0.0) as f32, one["ms"].as_f64().unwrap_or(0.0).max(0.0))).collect();
        let lift = request.inputs.get("lift").and_then(Value::as_bool) != Some(false);
        let hz = request.f64("hz").unwrap_or(60.0).clamp(1.0, 480.0);
        let every = 1000.0 / request.f64("input_hz").unwrap_or(hz).clamp(1.0, 1000.0);
        let after = request.f64("after").unwrap_or(2000.0).max(0.0);
        let track = match request.inputs.get("track") {
            Some(Value::Object(selector)) => Some(Request { id: Value::Null, op: "query".into(), inputs: selector.clone() }),
            _ => None,
        };
        let trace = super::finger::Trace { start, moves, lift };
        let paced = request.f64("frame_ms").map(|ms| ms.max(1.0));
        let frames = self.play(&trace, every, hz, paced, after, track.as_ref(), Some(draw))?;
        let budget = 1000.0 / hz;
        let over = inspect::over(&frames, budget);
        // What's been taken out but is still drawn going, which `after` should have seen go.
        let leaving = self.tree.going().count();
        Ok(json!({"hz": hz, "budget_ms": (budget * 100.0).round() / 100.0, "over": over, "leaving": leaving, "frames": frames, "time": self.js.clock().now(), "paused": true}))
    }

    /// What its frames cost since this was last asked, as `profile` says it: the mutations its JS made that it applied
    /// (`mutations`), how long its JS thread ran (`script_ms`), how long it took to lay out (`layout_ms`) and to style,
    /// lay out and paint all told (`paint_ms`), how many times it painted (`painted`: none where nothing changed, more
    /// than one where it painted again before the display's refresh), its thread's CPU time drawing those frames for
    /// the GPU (`draw_ms`), and, as `renderer` last drew it, its draw calls (`draws`), what its GPU did (`work`,
    /// `Renderer::work`) and what its atlas holds (`atlas`).
    pub fn cost(&mut self, renderer: &crate::render::renderer::Renderer) -> Value {
        let ms = |time: Duration| (time.as_secs_f64() * 100_000.0).round() / 100.0;
        let cost = std::mem::take(&mut self.cost);
        json!({
            "mutations": cost.mutations,
            "script_ms": ms(cost.script),
            "layout_ms": ms(cost.layout),
            "paint_ms": ms(cost.paint),
            "painted": cost.painted,
            "draw_ms": ms(renderer.drawing()),
            "draws": renderer.draws(),
            "work": renderer.work(),
            "atlas": renderer.sprite_atlas().usage(),
        })
    }

    /// Moves the clock on by `ms`, paused, running each timer due in that time at its own time, in order, each once
    /// what came before it has run, and while its page asks for frames, each as a display's would come, on its clock:
    /// for an inspector's request, which no display ticks for. This waits on the app's JS thread.
    pub(super) fn step(&mut self, ms: f64) {
        self.moved(ms, true);
    }

    /// Moves the clock on by `ms`, paused, running each timer due in that time at its own time, in order: for a host
    /// whose display ticks of its own (the fake platform's), which this waits on the app's JS thread for. → whether
    /// its page changed anything, which its window draws.
    pub fn advance(&mut self, ms: f64) -> bool {
        self.moved(ms, false)
    }

    /// Whether what it shows is still on its way somewhere: a transition, a node let go springing back, a list
    /// flung, what its page asked a frame for, or for a harness, a picture decoded that's there only later on its clock,
    /// or its page busy (`seed`).
    /// A loop (a spinner) never arrives, so isn't.
    pub fn arriving(&self) -> bool {
        self.transitions.arriving() || self.coasting() || self.js.framing() || self.js.holding().is_some() || self.late.as_ref().is_some_and(super::Late::holding) || self.pictures.due().is_some()
    }

    /// Its page's thread as a busy device's, as `seed` says (`Thread::seed`): what it's called with together now and then
    /// runs a turn apart. A browser's page runs on its own thread, the web's, which nothing seeds.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn seed(&mut self, seed: u64) {
        self.js.seed(seed);
    }

    /// Where the pointer is on it, if it's on it.
    pub fn pointer(&self) -> Option<crate::geometry::Point<f32>> {
        self.pointer
    }

    /// When its page's next timer is due, or a picture it decoded is there for a harness, in ms from now on its clock.
    pub fn timer_due(&self) -> Option<f64> {
        // A picture decoded is there at its time, for a harness (`Pictures::due`), as a timer is.
        self.js.due().into_iter().chain(self.pictures.due()).chain(self.frames.values().filter_map(|child| child.screen.pictures.due())).reduce(f64::min)
    }

    fn moved(&mut self, ms: f64, frames: bool) -> bool {
        let frame_ms = 1000.0 / self.hz;
        let clock = self.js.clock().clone();
        clock.pause();
        let mut came = self.idle();
        let mut left = ms;
        loop {
            // The next frame's, at least a microsecond on, so a clock a hair short of one isn't stepped by nothing.
            let frame = (frames && self.framing()).then(|| frame_ms - clock.now().rem_euclid(frame_ms)).map(|next| if next < 0.001 { next + frame_ms } else { next });
            let Some(next) = self.js.due().into_iter().chain(frame).reduce(f64::min).filter(|next| *next <= left) else { break };
            let framed = frame == Some(next);
            let next = next.max(0.0);
            clock.advance(next);
            left -= next;
            if framed {
                self.frame(web_time::Instant::now());
            }
            self.js.fire();
            came |= self.idle();
        }
        clock.advance(left);
        self.js.fire();
        came | self.idle()
    }

    /// Tells each `settle` waiting that it's settled, if it is, or what's pending once its deadline has come.
    pub(super) fn tell_settled(&mut self) {
        if self.settling.is_empty() {
            return;
        }
        // Put away or in the background, it's given no frames: what moves a frame at a time, or by the renderer, can't end
        // by itself till it's shown, when it's where its time has taken it.
        let (moving, animating) = if self.visible { (self.transitions.len(), self.js.framing() || self.coasting()) } else { (0, false) };
        let mut pending = inspect::pending(&self.unanswered, &self.js.timers(), moving, animating, self.changed.is_some(), self.busy());
        // A page busy with something else (`Thread::seed`) runs what's held for it once it's free, as a timer does.
        if let Some(free) = self.js.holding() {
            pending.push(json!({"kind": "busy", "in": free}));
        }
        // What a frame's page asked and hasn't heard back is this one's to wait for too.
        for child in self.frames.values() {
            pending.extend(inspect::pending(&child.screen.unanswered, &[], 0, child.screen.js.framing(), false, false));
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
