//! A press that moves, replayed from a trace (the inspector's `drag` and `replay`): a pointer's, as a mouse's goes down,
//! moves and comes up; or a finger's, as every phone's host takes one (`touch`). Each move is at its own time, as a
//! device's events carry theirs, so how fast it went is the trace's, not how quickly it was replayed.

use anyhow::Context as _;
use serde_json::{Value, json};

use super::Screen;
use super::touch::{Fingers, Phase};
use crate::geometry::{Point, point};
use crate::inspect::{self, Request};

/// A press being replayed, where it last was.
pub(super) struct Finger {
    last: Point<f32>,
}

/// A trace: where a press goes down, then each stretch it moves (`dx`, `dy`, over `ms`; a stretch with neither holds
/// still), and whether it lifts at the end.
pub(super) struct Trace {
    pub start: Point<f32>,
    pub moves: Vec<(f32, f32, f64)>,
    pub lift: bool,
}

impl Trace {
    /// How long it is, in ms.
    pub fn ms(&self) -> f64 {
        self.moves.iter().map(|(_, _, ms)| ms).sum()
    }

    /// Where it is `t` ms in.
    pub fn at(&self, t: f64) -> Point<f32> {
        let (mut at, mut since) = (self.start, 0.0);
        for (dx, dy, ms) in &self.moves {
            let part = if *ms <= 0.0 { 1.0 } else { ((t - since) / ms).clamp(0.0, 1.0) as f32 };
            at = point(at.x + dx * part, at.y + dy * part);
            if t < since + ms {
                break;
            }
            since += ms;
        }
        at
    }

    /// Its moves, each `every` ms and its end: when (ms in) and where.
    pub fn samples(&self, every: f64) -> Vec<(f64, Point<f32>)> {
        let ms = self.ms();
        let count = (ms / every.max(0.1)).ceil() as usize;
        (1..=count).map(|n| (n as f64 * every).min(ms)).map(|t| (t, self.at(t))).collect()
    }
}

impl Screen {
    /// Where a request's press starts: the middle of the node it selects, or its point.
    pub(super) fn start_of(&self, request: &Request) -> anyhow::Result<Point<f32>> {
        let (x, y) = match request.selects() {
            true => self.rects.get(&inspect::one(&self.tree, request)?).map(|rect| rect.centre()).context("that node isn't drawn")?,
            false => request.point().context("say where it starts: a node, by a selector, or a point, x and y")?,
        };
        Ok(point(x, y))
    }

    /// How far what scrolls under `at` shows scrolled, with its stretch past an end, if something scrolls there: on the
    /// fake platform, in the frame its window last presented (`presented`).
    pub fn scrolled_under(&self, at: Point<f32>) -> Option<Point<f32>> {
        let (scrolled, stretched) = (&self.scrolled, self.stretched());
        #[cfg(feature = "fake")]
        let (scrolled, stretched) = self.shown.as_ref().map_or((scrolled, stretched), |(_, scrolled, stretched)| (scrolled, stretched.clone()));
        let shown = self.under(at).into_iter().find_map(|id| scrolled.get(&id).copied())?;
        let stretch = self.under(at).into_iter().find_map(|id| stretched.get(&id).copied()).unwrap_or(point(0.0, 0.0));
        Some(point(shown.x + stretch.x, shown.y + stretch.y))
    }

    /// Its window presented the frame it last painted: where each node was drawn in it, and how far what scrolls showed
    /// scrolled, are what's on the screen till the next (`shown`).
    #[cfg(feature = "fake")]
    pub fn presented(&mut self) {
        if let Some(seen) = &self.seen {
            self.shown = Some((self.rects.clone(), seen.scrolled.clone(), seen.stretched.iter().map(|(id, at)| (*id, *at)).collect()));
        }
    }

    /// Where node `id` is drawn in the frame its window last presented (`presented`), or the last painted where none has
    /// been.
    #[cfg(feature = "fake")]
    pub fn shown_rect(&self, id: crate::tree::Id) -> Option<inspect::Rect> {
        self.shown.as_ref().map_or(&self.rects, |(rects, ..)| rects).get(&id).copied()
    }

    /// A press goes down at `at`, at `time` (ms on its clock): a finger's as a phone's host hands it each touch
    /// (`touch`), or the pointer's.
    pub(super) fn finger_down(&mut self, at: Point<f32>, time: f64) -> Finger {
        if self.touch {
            self.touch(&Fingers::PHONE, 0, at, Phase::Down, time);
        } else {
            self.pointer_moved(at);
            self.pointer_down(at, 0, false);
        }
        Finger { last: at }
    }

    /// The press moved to `at`, at `time`.
    pub(super) fn finger_moved(&mut self, finger: &mut Finger, at: Point<f32>, time: f64) {
        finger.last = at;
        if self.touch {
            self.touch(&Fingers::PHONE, 0, at, Phase::Moved, time);
        } else {
            self.pointer_moved(at);
        }
    }

    /// The press lifts where it last was, at `time`.
    pub(super) fn finger_up(&mut self, finger: Finger, time: f64) {
        if self.touch {
            self.touch(&Fingers::PHONE, 0, finger.last, Phase::Up, time);
        } else {
            self.pointer_up(finger.last, 0);
        }
    }

    /// Plays `trace` with moves each `every` ms, the clock stepped to each where it's paused, and with `draw`, the
    /// frames of a display `hz` times a second drawn, from its start until `after` ms past its end or nothing's moved
    /// for a frame: each with what it cost, where `track` was drawn and how far what scrolls under its start has
    /// scrolled. → those frames.
    /// `paced`: ms between the frames it draws, where its device draws them less often than its display refreshes
    /// (a machine others load, as Telemetry measures one): each is a long one, which presses it as a window's do
    /// (`platform::Deadlines`), and it draws as pressed, as a window would, its pressure as it was again after.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn play(&mut self, trace: &Trace, every: f64, hz: f64, paced: Option<f64>, after: f64, track: Option<&Request>, mut draw: Option<&mut dyn FnMut(&mut Screen) -> anyhow::Result<Value>>) -> anyhow::Result<Vec<Value>> {
        let clock = self.js.clock().clone();
        if draw.is_some() {
            clock.pause();
            self.idle();
            // Its first frame's cost is what's done from here.
            self.cost = Default::default();
        }
        let paused = clock.paused();
        let refresh = 1000.0 / hz;
        let frame_ms = paced.unwrap_or(refresh).max(refresh);
        self.hz = 1000.0 / frame_ms;
        let begun = clock.now();
        let (ms, samples) = (trace.ms(), trace.samples(every));
        let (pressure, mut deadlines, origin) = (self.pressure, crate::platform::Deadlines::default(), web_time::Instant::now());
        let long = crate::platform::Timing::long(std::time::Duration::from_secs_f64(frame_ms / 1000.0), std::time::Duration::from_secs_f64(refresh / 1000.0));
        let mut finger = Some(self.finger_down(trace.start, begun));
        let (mut next, mut frames) = (0, Vec::new());
        let mut followed: Option<crate::tree::Id> = None;
        // As many frames as its length and `after` take, and for a trace with no frames drawn, as its moves take.
        let last = ((ms + after) / frame_ms).ceil() as u64 + 1;
        for n in 1..=last.max(1) {
            let frame = n as f64 * frame_ms;
            // The moves before the frame, each at its own time, as a device delivers them just before a frame.
            while let Some((t, at)) = samples.get(next).copied().filter(|(t, _)| draw.is_none() || *t <= frame) {
                if paused {
                    self.step(begun + t - clock.now());
                }
                if let Some(finger) = finger.as_mut() {
                    self.finger_moved(finger, at, begun + t);
                }
                next += 1;
            }
            if next == samples.len() && trace.lift && let Some(finger) = finger.take() {
                self.finger_up(finger, begun + ms);
            }
            let Some(draw) = draw.as_mut() else { break };
            self.step(begun + frame - clock.now());
            if paced.is_some() {
                let at = origin + std::time::Duration::from_secs_f64(frame / 1000.0);
                deadlines.frame_at(at, long);
                self.pressure = pressure.max(deadlines.pressure_at(at));
            }
            let mut drawn = draw(self)?;
            drawn["at"] = json!((frame * 100.0).round() / 100.0);
            drawn["pressure"] = json!(self.pressure.name());
            // What's tracked, where it's drawn, or that it's gone, as a page that's left takes it with it: the node the
            // selector first found, not another it finds once that's gone (an App Library's Notes, as Home's goes).
            if let Some(selector) = track {
                // The first frame it's there, it must be one node: another that matches later isn't it.
                if followed.is_none() && let Some(found) = self.matched(selector) {
                    followed = Some(found.context("what it follows (`track`)")?);
                }
                match followed.filter(|id| self.tree.nodes.contains_key(id)).and_then(|id| self.rects.get(&id)) {
                    Some(rect) => {
                        drawn["x"] = json!((rect.x * 100.0).round() / 100.0);
                        drawn["y"] = json!((rect.y * 100.0).round() / 100.0);
                    }
                    None => drawn["gone"] = json!(true),
                }
            }
            if let Some(scrolled) = self.scrolled_under(trace.start) {
                drawn["scroll"] = json!({"x": scrolled.x, "y": scrolled.y});
            }
            frames.push(drawn);
            let still = !self.animating && !self.framing() && !self.coasting() && self.transitions.len() == 0;
            if frame >= ms && next == samples.len() && still {
                break;
            }
        }
        // What's tracked was there in some frame, or it's a selector that matches nothing, which isn't a node that's gone.
        if let (Some(selector), None) = (track, followed) {
            anyhow::bail!("what it follows (`track`): {}", self.unmatched(selector));
        }
        // The window's own scene is older than what was drawn here, so its next frame is painted afresh.
        if draw.is_some() {
            self.seen = None;
        }
        self.pressure = pressure;
        Ok(frames)
    }
}
