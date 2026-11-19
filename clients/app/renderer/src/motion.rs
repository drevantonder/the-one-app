//! How things move by themselves, the one place it's worked out: a spring, as SwiftUI's and Motion's (a style's
//! transition, a window let go), a fling slowing down, as UIScrollView's deceleration, the rubber band past an edge, as
//! UIScrollView's bounce, and how fast a finger was going as it let go.

use web_time::Instant;

use crate::geometry::{Point, point};

/// A spring with a mass of 1, as SwiftUI's `.spring(duration:bounce:)` gives one: how stiff it is and how damped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    /// SwiftUI's: `duration` in milliseconds, and `bounce` from 0, none, to 1 (below 0, more damped than none).
    pub fn new(duration: f32, bounce: f32) -> Spring {
        let response = (duration / 1000.0).max(0.01);
        let fraction = if bounce >= 0.0 { 1.0 - bounce } else { 1.0 / (1.0 + bounce) };
        Spring { stiffness: (2.0 * std::f32::consts::PI / response).powi(2), damping: 4.0 * std::f32::consts::PI * fraction / response }
    }

    /// Where something `from` past where it's going, moving at `velocity` (each a second), is `seconds` later, and how
    /// fast it moves then: the damped spring's own solution, so it's the same at any frame rate.
    pub fn at(&self, from: f64, velocity: f64, seconds: f64) -> (f64, f64) {
        let (k, c, t) = (f64::from(self.stiffness), f64::from(self.damping), seconds);
        let natural = k.sqrt();
        let ratio = c / (2.0 * natural);
        if (ratio - 1.0).abs() < 1e-4 {
            let e = (-natural * t).exp();
            let b = velocity + natural * from;
            ((from + b * t) * e, (velocity - natural * b * t) * e)
        } else if ratio < 1.0 {
            let damped = natural * (1.0 - ratio * ratio).sqrt();
            let (a, b) = (from, (velocity + ratio * natural * from) / damped);
            let (e, (sin, cos)) = ((-ratio * natural * t).exp(), (damped * t).sin_cos());
            (e * (a * cos + b * sin), e * ((b * damped - ratio * natural * a) * cos - (a * damped + ratio * natural * b) * sin))
        } else {
            let root = natural * (ratio * ratio - 1.0).sqrt();
            let (r1, r2) = (-ratio * natural + root, -ratio * natural - root);
            let a = (velocity - r2 * from) / (r1 - r2);
            let b = from - a;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }

    /// Whether something `off` from where it's going, at `velocity`, has arrived: within `near`, and moving no faster
    /// than ten times that a second.
    pub fn arrived(off: f64, velocity: f64, near: f64) -> bool {
        off.abs() < near && velocity.abs() < near * 10.0
    }
}

/// How much of its speed a fling keeps each millisecond: UIScrollView's normal deceleration rate.
const DECELERATION: f64 = 0.998;

/// A fling going `velocity` (each a second), `seconds` later: how far it's gone, and how fast it goes then, as
/// UIScrollView's deceleration has it.
pub fn decay(velocity: f64, seconds: f64) -> (f64, f64) {
    let kept = DECELERATION.powf(seconds * 1000.0);
    (velocity * (kept - 1.0) / DECELERATION.ln() / 1000.0, velocity * kept)
}

/// How far something pulled `past` an edge shows past it, in something `size` long: less the further it's pulled,
/// never as far as `size`, as UIScrollView's rubber band. `elastic` is how loose it is: iOS's is 0.55.
pub fn rubber_band(past: f32, size: f32, elastic: f32) -> f32 {
    if size <= 0.0 || elastic <= 0.0 {
        return 0.0;
    }
    (1.0 - 1.0 / (past.abs() * elastic / size + 1.0)) * size * past.signum()
}

/// Where a finger has been lately, for how fast it was going as it lets go, by whatever clock its caller keeps, in
/// milliseconds: a screen's, which an agent's paused clock steps, or the system's (`now`).
#[derive(Default)]
pub struct Velocity {
    lately: Vec<(f64, Point<f32>)>,
}

/// How far back a finger's speed is measured, in milliseconds: UIScrollView's pan measures it over its last tenth of
/// a second.
const LATELY: f64 = 100.0;

/// The system's clock, in milliseconds since this process first asked.
pub fn now() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

impl Velocity {
    /// The finger is at `at` at `time`.
    pub fn add(&mut self, at: Point<f32>, time: f64) {
        self.lately.retain(|(when, _)| time - when <= LATELY);
        self.lately.push((time, at));
    }

    /// Where it was last.
    pub fn last(&self) -> Option<Point<f32>> {
        self.lately.last().map(|(_, at)| *at)
    }

    /// How fast it's been going over its last tenth of a second until `time`, in its points a second, along and down:
    /// a finger that stopped before it let go was going slower, and one still all that time not at all.
    pub fn get(&self, time: f64) -> Point<f32> {
        let first = self.lately.iter().find(|(when, _)| time - when <= LATELY);
        let (Some(first), Some(last)) = (first, self.lately.last()) else { return point(0.0, 0.0) };
        let seconds = ((time - first.0) / 1000.0).max(0.008) as f32;
        point((last.1.x - first.1.x) / seconds, (last.1.y - first.1.y) / seconds)
    }
}
