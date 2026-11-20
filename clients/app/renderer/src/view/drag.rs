//! A node a pointer or a finger drags, as Motion's `drag` has it: pressed and moved past a few points, it follows the
//! pointer here, where the input arrives, with no turn of its page's JS between, along the way its `drag` says ("x",
//! "y", or `true`, both), held in by `dragConstraints` (pixels it may go up, left, right and down from where its style
//! puts it) past which it gives as a rubber band, as loose as `dragElastic` says (0 none, Motion's default 0.5). Let go,
//! its page hears `dragend` once, with how far it went (`offset`) and how fast it was going (`velocity`, pixels a
//! second), and it springs from where it is, at that speed, to where its style puts it, wherever its page moves that
//! meanwhile (`transition::Transitions::release`).

use serde_json::{Value, json};

use super::{Phase, Screen};
use crate::geometry::{Point, point};
use crate::motion::{Velocity, rubber_band};
use crate::tree::Id;

/// How far a pointer moves before it drags, in points: Motion's three for a mouse, and for a finger, UIScrollView's
/// slop, past which a phone's window decides whether it scrolls instead (`Screen::pans`).
const MOUSE: f32 = 3.0;
const FINGER: f32 = 10.0;

/// How loose a node is past its constraints where its `dragElastic` says nothing: Motion's.
const ELASTIC: f32 = 0.5;

/// A press on a node that drags.
pub(super) struct Drag {
    id: Id,
    along: bool,
    down: bool,
    start: Point<f32>,
    began: bool,
    velocity: Velocity,
}

/// A trackpad's fingers dragging what drags: where they'd be as a finger (`at`), while they're on it, or, once they've
/// lifted, that what the system scrolls on with is theirs (`lifted`).
#[derive(Default)]
pub(super) enum Trackpad {
    #[default]
    None,
    On(Point<f32>),
    Lifted,
}

/// Which ways node props say it drags: along, down.
fn ways(drag: &Value) -> (bool, bool) {
    match drag {
        Value::String(way) if way == "x" => (true, false),
        Value::String(way) if way == "y" => (false, true),
        Value::Bool(true) => (true, true),
        _ => (false, false),
    }
}

impl Screen {
    /// Whether the innermost node at `at` that drags, drags the way a finger moved `by`.
    pub(super) fn drags(&self, id: Id, by: Point<f32>) -> Option<bool> {
        let (along, down) = ways(self.tree.nodes.get(&id)?.prop("drag")?);
        if !along && !down {
            return None;
        }
        Some(if by.x.abs() > by.y.abs() { along } else { down })
    }

    /// A press went down at `at` on what's there (`chain`, the innermost first): the innermost that drags may.
    pub(super) fn drag_down(&mut self, chain: &[Id], at: Point<f32>) {
        self.drag = chain.iter().find_map(|id| {
            let (along, down) = ways(self.tree.nodes.get(id)?.prop("drag")?);
            let mut velocity = Velocity::default();
            velocity.add(at, self.input_time());
            (along || down).then_some(Drag { id: *id, along, down, start: at, began: false, velocity })
        });
    }

    /// A finger scrolling what's at `at` by `by` (as `drag_scroll` takes it), now at `finger`: where what it scrolls is
    /// at its start and the finger pulls on past it the way something it's in drags (a sheet pulled down), that takes
    /// the finger from here, as an iPhone's sheet takes the pull of its content scrolled to its top. → whether it did.
    pub fn hand_over(&mut self, at: Point<f32>, by: Point<f32>, finger: Point<f32>) -> bool {
        let along = by.x.abs() > by.y.abs();
        let pulled = if along { by.x } else { by.y };
        if pulled >= 0.0 {
            return false;
        }
        let chain = self.under(at);
        let Some(at_start) = chain.iter().position(|id| {
            self.placed(*id).and_then(|placed| placed.scroll).is_some_and(|(_, _, its_along)| its_along == along)
        }) else {
            return false;
        };
        let scroller = chain[at_start];
        let now = self.scrolled.get(&scroller).copied().unwrap_or(point(0.0, 0.0));
        if (if along { now.x } else { now.y }) > 0.0 {
            return false;
        }
        let Some(id) = chain[at_start + 1..].iter().copied().find(|id| self.drags(*id, by) == Some(true)) else { return false };
        let Some((along, down)) = self.tree.nodes.get(&id).and_then(|node| node.prop("drag")).map(ways) else { return false };
        self.coasts.remove(&scroller);
        let mut velocity = Velocity::default();
        velocity.add(finger, self.input_time());
        self.drag = Some(Drag { id, along, down, start: finger, began: true, velocity });
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("dragstart")) {
            self.tell(id, "dragstart", json!({"clientX": finger.x, "clientY": finger.y}));
        }
        true
    }

    /// A trackpad scrolled by `delta` at `at`, in `phase`: where the system says when its fingers touch and lift (a
    /// Mac's), they drag what drags under them as a finger's press would, as an iPad's trackpad moves what a finger
    /// does (a launcher's pages swiped with two fingers), from where they touch, the way they go (`delta` is the
    /// scroll's, against the fingers), let go at their speed as they lift; what the system scrolls on with after is
    /// theirs too, so nothing else scrolls by it. → whether they're its, which nothing else hears.
    pub(super) fn drag_wheel(&mut self, at: Point<f32>, delta: Point<f32>, phase: Option<Phase>) -> bool {
        let Some(phase) = phase else { return false };
        if phase.momentum {
            return matches!(self.trackpad, Trackpad::Lifted);
        }
        match (phase.fingers, std::mem::take(&mut self.trackpad)) {
            (Some("began"), _) => {
                let chain = self.under(at);
                self.drag_down(&chain, at);
                if self.drag.is_none() {
                    return false;
                }
                self.trackpad = Trackpad::On(at);
                true
            }
            (Some("changed"), Trackpad::On(was)) => {
                let finger = point(was.x - delta.x, was.y - delta.y);
                self.drag_moved(finger);
                // Gone the other way, they're not its: what scrolls that way, or the page, has them.
                if self.drag.is_none() {
                    return false;
                }
                self.trackpad = Trackpad::On(finger);
                true
            }
            (Some(lifted @ ("ended" | "cancelled")), Trackpad::On(was)) => {
                let finger = point(was.x - delta.x, was.y - delta.y);
                self.drag_up(finger, lifted == "cancelled");
                self.trackpad = Trackpad::Lifted;
                true
            }
            _ => false,
        }
    }

    /// The press moved to `at`: past a few points, its node follows it, the way it drags, and gives past its
    /// constraints.
    pub(super) fn drag_moved(&mut self, at: Point<f32>) {
        let (threshold, time) = (if self.touch { FINGER } else { MOUSE }, self.input_time());
        let Some(drag) = self.drag.as_mut() else { return };
        drag.velocity.add(at, time);
        let by = point(if drag.along { at.x - drag.start.x } else { 0.0 }, if drag.down { at.y - drag.start.y } else { 0.0 });
        if !drag.began {
            let moved = point(at.x - drag.start.x, at.y - drag.start.y);
            if moved.x.hypot(moved.y) < threshold {
                return;
            }
            // It's dragged only where the press goes mostly its way, as Motion's `dragDirectionLock`: one dragged along
            // isn't moved by a swipe up and down (an owner's pages under a swipe between owners), which is left be.
            if (moved.x.abs() > moved.y.abs() && !drag.along) || (moved.y.abs() > moved.x.abs() && !drag.down) {
                self.drag = None;
                return;
            }
            drag.began = true;
            // Once it's dragged, it's not pressed.
            self.states.pressed = None;
            let id = drag.id;
            if self.tree.nodes.get(&id).is_some_and(|node| node.listens("dragstart")) {
                self.tell(id, "dragstart", json!({"clientX": at.x, "clientY": at.y}));
            }
        }
        self.hold_at(by);
    }

    /// Where the device predicts the press is going (iOS's `predictedTouches`, Android's `MotionPredictor`, the web's
    /// `getPredictedEvents`), at `at`, a little after its last move: what's dragged is drawn there, as far ahead of the
    /// last move as the frame takes to show, so it keeps up with the finger. How fast it was going stays the real
    /// moves'.
    pub fn predicted(&mut self, at: Point<f32>) {
        // A press held in a frame is its page's.
        if let Some(frame) = self.captured_frame {
            let local = self.local(frame, at);
            if let Some(child) = self.frames.get_mut(&frame) {
                child.screen.predicted(local);
            }
            return;
        }
        let Some(drag) = self.drag.as_ref().filter(|drag| drag.began) else { return };
        let by = point(if drag.along { at.x - drag.start.x } else { 0.0 }, if drag.down { at.y - drag.start.y } else { 0.0 });
        self.hold_at(by);
    }

    /// The node dragged is held `by` from where it was pressed, as its constraints let it, giving past them as a
    /// rubber band.
    fn hold_at(&mut self, by: Point<f32>) {
        let Some(id) = self.drag.as_ref().map(|drag| drag.id) else { return };
        let Some(node) = self.tree.nodes.get(&id) else { return self.drag = None };
        let constraints = node.prop("dragConstraints");
        let bound = |side: &str| constraints.and_then(|constraints| constraints[side].as_f64()).map(|px| px as f32);
        let elastic = node.prop("dragElastic").and_then(Value::as_f64).map_or(ELASTIC, |elastic| elastic as f32);
        let size = self.rects.get(&id).map_or((0.0, 0.0), |rect| (rect.width, rect.height));
        let give = |by: f32, least: Option<f32>, most: Option<f32>, size: f32| match (least.map(|least| -least), most) {
            (Some(least), _) if by < least => least + rubber_band(by - least, size, elastic),
            (_, Some(most)) if by > most => most + rubber_band(by - most, size, elastic),
            _ => by,
        };
        let shown = point(give(by.x, bound("left"), bound("right"), size.0), give(by.y, bound("top"), bound("bottom"), size.1));
        self.transitions.hold(id, shown);
        self.changed();
    }

    /// The press came up at `at`, or the system took it (`cancelled`): a node dragged is let go, springing on at the
    /// speed it had, its page told how far it went and how fast.
    pub(super) fn drag_up(&mut self, at: Point<f32>, cancelled: bool) {
        let Some(mut drag) = self.drag.take().filter(|drag| drag.began) else { return };
        let (time, now) = (self.input_time(), self.js.clock().now());
        drag.velocity.add(at, time);
        let going = if cancelled { point(0.0, 0.0) } else { drag.velocity.get(time) };
        let velocity = point(if drag.along { going.x } else { 0.0 }, if drag.down { going.y } else { 0.0 });
        self.transitions.release(drag.id, velocity, now);
        self.changed();
        if self.tree.nodes.get(&drag.id).is_some_and(|node| node.listens("dragend")) {
            let offset = json!({"x": if drag.along { at.x - drag.start.x } else { 0.0 }, "y": if drag.down { at.y - drag.start.y } else { 0.0 }});
            let rect = self.rects.get(&drag.id).map(|rect| json!({"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}));
            self.tell(drag.id, "dragend", json!({"clientX": at.x, "clientY": at.y, "rect": rect, "offset": offset, "velocity": {"x": velocity.x, "y": velocity.y}, "canceled": cancelled}));
        }
    }
}
