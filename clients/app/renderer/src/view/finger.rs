//! A press that moves, replayed from a trace (the inspector's `drag` and `replay`): a pointer's, as a mouse's goes down,
//! moves and comes up; or a finger's, as a phone's host takes one (`platform/ios`'s `touch_at`): past the slop it pans
//! what scrolls that way under it, stretching past its end as a rubber band, and lifted it flings or springs back at
//! the speed it had over its last tenth of a second; otherwise it's what it's on's to follow (a node that drags, a row
//! swiped). Each move is at its own time, as a device's events carry theirs, so how fast it went is the trace's, not
//! how quickly it was replayed.

use anyhow::Context as _;
use serde_json::{Value, json};

use super::Screen;
use crate::geometry::{Point, point};
use crate::inspect::{self, Request};
use crate::motion::Velocity;
use crate::tree::Id;

/// How far a finger moves before it pans, in points: UIScrollView's slop, as the phones' hosts have it.
const SLOP: f32 = 10.0;

/// A press being replayed.
pub(super) struct Finger {
    start: Point<f32>,
    last: Point<f32>,
    moved: bool,
    panning: bool,
    /// It landed on what was flung or springing back, which it only stopped.
    stopped: bool,
    velocity: Velocity,
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

    /// How far what scrolls under `at` shows scrolled, with its stretch past an end, if something scrolls there.
    pub fn scrolled_under(&self, at: Point<f32>) -> Option<Point<f32>> {
        let scrolled = self.under(at).into_iter().find_map(|id| self.scrolled.get(&id).copied())?;
        let stretched = self.stretched();
        let stretch = self.under(at).into_iter().find_map(|id| stretched.get(&id).copied()).unwrap_or(point(0.0, 0.0));
        Some(point(scrolled.x + stretch.x, scrolled.y + stretch.y))
    }

    /// A press goes down at `at`, at `time` (ms on its clock).
    pub(super) fn finger_down(&mut self, at: Point<f32>, time: f64) -> Finger {
        let mut velocity = Velocity::default();
        velocity.add(at, time);
        let stopped = self.touch && self.catch_scroll(at);
        if !self.touch {
            self.pointer_moved(at);
        }
        if !stopped {
            self.pointer_down(at, 0, false);
        }
        Finger { start: at, last: at, moved: false, panning: false, stopped, velocity }
    }

    /// The press moved to `at`, at `time`.
    pub(super) fn finger_moved(&mut self, finger: &mut Finger, at: Point<f32>, time: f64) {
        finger.velocity.add(at, time);
        if !self.touch {
            finger.last = at;
            return self.pointer_moved(at);
        }
        let by = point(at.x - finger.start.x, at.y - finger.start.y);
        if !finger.moved && (by.x.abs() > SLOP || by.y.abs() > SLOP) {
            finger.moved = true;
            finger.panning = finger.stopped || self.pans(finger.start, by);
            if finger.panning && !finger.stopped {
                self.pointer_cancel(at);
            } else if !finger.panning {
                self.dragged();
            }
        }
        if finger.panning {
            self.drag_scroll(finger.start, point(finger.last.x - at.x, finger.last.y - at.y));
        } else if !finger.stopped {
            self.pointer_moved(at);
        }
        finger.last = at;
    }

    /// The press lifts where it last was, at `time`: a pan flings or springs back at the speed it had.
    pub(super) fn finger_up(&mut self, mut finger: Finger, time: f64) {
        let at = finger.last;
        finger.velocity.add(at, time);
        if finger.panning {
            let going = finger.velocity.get(time);
            return self.release_scroll(finger.start, point(-going.x, -going.y));
        }
        if !finger.stopped {
            self.pointer_up(at, 0);
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
    pub(super) fn play(&mut self, trace: &Trace, every: f64, hz: f64, paced: Option<f64>, after: f64, track: Option<Id>, mut draw: Option<&mut dyn FnMut(&mut Screen) -> anyhow::Result<Value>>) -> anyhow::Result<Vec<Value>> {
        let clock = self.js.clock().clone();
        if draw.is_some() {
            clock.pause();
            self.idle();
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
            if let Some(rect) = track.and_then(|id| self.rects.get(&id)) {
                drawn["x"] = json!((rect.x * 100.0).round() / 100.0);
                drawn["y"] = json!((rect.y * 100.0).round() / 100.0);
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
        // The window's own scene is older than what was drawn here, so its next frame is painted afresh.
        if draw.is_some() {
            self.seen = None;
        }
        self.pressure = pressure;
        Ok(frames)
    }
}
