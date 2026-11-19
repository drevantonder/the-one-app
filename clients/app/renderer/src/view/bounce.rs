//! What scrolls under a finger, as UIScrollView's does: dragged past its end, it gives as a rubber band; let go there,
//! it springs back; flung, it slows as UIScrollView's deceleration has it and bounces off its end. The screen does it
//! all, where the touch arrives, each frame on its own clock (`motion`), and draws how far it's stretched past its end
//! as its content moved (`Screen::stretched`), never as how far it's scrolled, which its page is told.

use serde_json::json;

use super::Screen;
use crate::geometry::{Point, point};
use crate::motion::{Spring, decay, rubber_band};
use crate::tree::Id;

/// How loose the rubber band is: UIScrollView's.
const ELASTIC: f32 = 0.55;
/// How it springs back to its end: critically damped, as UIScrollView's bounce.
const BACK: Spring = Spring { stiffness: 228.0, damping: 30.2 };
/// The speed, in points a second, below which a fling stops, and the least a lift has to be one.
const STOPPED: f32 = 20.0;
const FLUNG: f32 = 200.0;

/// A scroll's motion of its own, along or down (`along`): held stretched past its end by a finger (`pulled`, how far
/// the finger's gone past it), springing back from `stretch` at `velocity`, or flung at `velocity`; each a second,
/// stepped last at `stepped` (ms on the screen's clock).
pub(super) struct Coast {
    along: bool,
    held: bool,
    flung: bool,
    pulled: f32,
    stretch: f32,
    velocity: f32,
    stepped: f64,
}

impl Screen {
    /// The innermost thing at `at` that scrolls the way `by` goes, or `along` where it's said: its id, whether it
    /// scrolls along, how far it's scrolled that way, how far it can, and how long it is that way.
    fn scroller(&self, at: Point<f32>, by: Point<f32>) -> Option<(Id, bool, f32, f32, f32)> {
        let along = by.x.abs() > by.y.abs();
        self.under(at).into_iter().find_map(|id| {
            let placed = self.placed(id)?;
            let (most_y, most_x, its_along) = placed.scroll?;
            (its_along == along).then_some(())?;
            let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
            Some(if along { (id, true, now.x, most_x, placed.shown.size.width) } else { (id, false, now.y, most_y, placed.shown.size.height) })
        })
    }

    /// A finger on what scrolls at `at` moved it `by` points (down and right positive, as `pan`): it scrolls, and past
    /// its end, stretches, less the further it's pulled; pulled back, it gives up its stretch before it scrolls.
    pub fn drag_scroll(&mut self, at: Point<f32>, by: Point<f32>) -> Option<Id> {
        let held = self.under(at).into_iter().find(|id| self.coasts.get(id).is_some_and(|coast| coast.held));
        let target = match held {
            Some(id) => Some(id),
            None => {
                if let Some(scrolled) = self.pan(at, by) {
                    return Some(scrolled);
                }
                let (id, along, _, _, _) = self.scroller(at, by)?;
                let now = self.js.clock().now();
                self.coasts.insert(id, Coast { along, held: true, flung: false, pulled: 0.0, stretch: 0.0, velocity: 0.0, stepped: now });
                Some(id)
            }
        };
        let id = target?;
        let size = self.placed(id).map_or(0.0, |placed| if self.coasts[&id].along { placed.shown.size.width } else { placed.shown.size.height });
        let coast = self.coasts.get_mut(&id).expect("it's held");
        let pulled = coast.pulled + if coast.along { by.x } else { by.y };
        // Pulled back past its end, what's left scrolls.
        if pulled == 0.0 || pulled.signum() != coast.pulled.signum() && coast.pulled != 0.0 {
            let along = coast.along;
            self.coasts.remove(&id);
            if pulled != 0.0 {
                self.pan(at, if along { point(pulled, 0.0) } else { point(0.0, pulled) });
            }
        } else {
            coast.pulled = pulled;
            coast.stretch = rubber_band(pulled, size, ELASTIC);
        }
        Some(id)
    }

    /// The finger on what scrolls at `at` lifted going `velocity` points a second (as a scroll goes): stretched, it
    /// springs back at that speed; otherwise, fast enough, it's flung.
    pub fn release_scroll(&mut self, at: Point<f32>, velocity: Point<f32>) {
        let now = self.js.clock().now();
        if let Some(id) = self.under(at).into_iter().find(|id| self.coasts.get(id).is_some_and(|coast| coast.held)) {
            let coast = self.coasts.get_mut(&id).expect("it's held");
            coast.velocity = if coast.along { velocity.x } else { velocity.y };
            (coast.held, coast.stepped) = (false, now);
            return self.changed();
        }
        let Some((id, along, _, _, _)) = self.scroller(at, velocity) else { return };
        let speed = if along { velocity.x } else { velocity.y };
        if speed.abs() > FLUNG {
            self.coasts.insert(id, Coast { along, held: false, flung: true, pulled: 0.0, stretch: 0.0, velocity: speed, stepped: now });
            self.changed();
        }
    }

    /// A finger landed at `at`: what's flung or springing back under it stops where it is, held there. → whether
    /// something was moving, which the finger only stops.
    pub fn catch_scroll(&mut self, at: Point<f32>) -> bool {
        let caught: Vec<Id> = self.under(at).into_iter().filter(|id| self.coasts.get(id).is_some_and(|coast| !coast.held)).collect();
        for id in &caught {
            let size = self.placed(*id).map_or(0.0, |placed| if self.coasts[id].along { placed.shown.size.width } else { placed.shown.size.height });
            let coast = self.coasts.get_mut(id).expect("it's coasting");
            if coast.stretch == 0.0 {
                self.coasts.remove(id);
                continue;
            }
            // Held where it's stretched to, as far past its end as a finger would have pulled it to stretch it so.
            let shown = (coast.stretch.abs() / size.max(1.0)).min(0.99);
            coast.pulled = coast.stretch.signum() * size * (1.0 / (1.0 - shown) - 1.0) / ELASTIC;
            (coast.held, coast.flung, coast.velocity) = (true, false, 0.0);
        }
        !caught.is_empty()
    }

    /// Steps what's flung or springing back to the screen's clock: a fling scrolls on, slowing, and bounces off an end
    /// it reaches at the speed it has; a stretch springs back.
    pub(super) fn coast(&mut self) {
        let now = self.js.clock().now();
        let ids: Vec<Id> = self.coasts.keys().copied().collect();
        for id in ids {
            let coast = self.coasts.get_mut(&id).expect("it's there");
            if coast.held {
                continue;
            }
            let seconds = ((now - coast.stepped) / 1000.0).max(0.0);
            coast.stepped = now;
            if coast.flung {
                let (by, velocity) = decay(f64::from(coast.velocity), seconds);
                let (along, velocity) = (coast.along, velocity as f32);
                let Some((most, was)) = self.placed(id).and_then(|placed| placed.scroll).map(|(most_y, most_x, _)| {
                    let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
                    if along { (most_x, now.x) } else { (most_y, now.y) }
                }) else {
                    self.coasts.remove(&id);
                    continue;
                };
                let to = (was + by as f32).clamp(0.0, most);
                self.scroll_to(id, along, to);
                let coast = self.coasts.get_mut(&id).expect("it's there");
                coast.velocity = velocity;
                // Reaching an end going on, it bounces off it; slow enough, it stops.
                if (to == 0.0 && velocity < 0.0) || (to == most && velocity > 0.0) {
                    coast.flung = false;
                } else if velocity.abs() < STOPPED {
                    self.coasts.remove(&id);
                }
                continue;
            }
            let (stretch, velocity) = BACK.at(f64::from(coast.stretch), f64::from(coast.velocity), seconds);
            if Spring::arrived(stretch, velocity, 0.5) {
                self.coasts.remove(&id);
            } else {
                (coast.stretch, coast.velocity) = (stretch as f32, velocity as f32);
            }
        }
    }

    /// Scrolls `id` to `to`, along or down, telling its page where it listens.
    fn scroll_to(&mut self, id: Id, along: bool, to: f32) {
        let now = self.scrolled.entry(id).or_insert(point(0.0, 0.0));
        if along {
            now.x = to;
        } else {
            now.y = to;
        }
        let scrolled = *now;
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("scroll")) {
            self.tell(id, "scroll", json!({"scrollTop": scrolled.y, "scrollLeft": scrolled.x}));
        }
    }

    /// How far past its end each scroll is stretched, which its content is drawn moved by.
    pub(super) fn stretched(&self) -> std::collections::HashMap<Id, Point<f32>> {
        self.coasts.iter().filter(|(_, coast)| coast.stretch != 0.0).map(|(id, coast)| (*id, if coast.along { point(coast.stretch, 0.0) } else { point(0.0, coast.stretch) })).collect()
    }

    /// Whether anything scrolls by itself, which a frame steps.
    pub(super) fn coasting(&self) -> bool {
        self.coasts.values().any(|coast| !coast.held)
    }
}
