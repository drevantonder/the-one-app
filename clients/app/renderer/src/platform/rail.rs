//! The rail over a phone's app windows (DESIGN.md, The rail): the home line its screen draws on the right edge
//! (`Screen::rail`), which a finger pulls in to go home, as the window page's line does over an app in a frame
//! (rail.js's `pull` and move-machine.js's `moves`, whose numbers these are). Pulled in far enough to say so, the window follows the
//! finger, its right edge under it, shrinking to a card over the Home Screen, dimmed; let go past `PULLED` or flicked
//! in, a spring carries it on into the icon it was opened from, or with none, it shrinks away, and every app's window
//! closes; short of that, it springs back. A tap does nothing, and the first finger to come in takes it: another on the
//! line meanwhile, as the hand holding the phone touches its edge, is ignored, as iOS's swipe home ignores one. For
//! whoever asked for less motion, the window only fades.
//!
//! An app's window opens as the reverse of that move, on the same spring: it grows out of its icon, straight to the
//! whole screen, as an iPhone's app zooms out of its icon (`Pull::opening`). A finger on the line as it opens takes
//! it from where it's got to, and lets it go home or on open as a pull does.

use web_time::Instant;

use crate::geometry::{Bounds, Point, Size, point};
use crate::motion::{Spring, Velocity};

/// How far through the pull its first part goes, where the window is a card, how small, and its corners' radius, in
/// the card's points.
const HELD: f32 = 0.4;
const CARD: f32 = 0.8;
const CORNER: f32 = 40.0;
/// How far into its icon the window has gone as it starts to fade into it.
const FADES: f32 = 0.75;
/// How far in, as a share of the window's width, past which a finger let go goes home, and how fast in (widths a
/// second) a flick that goes home is.
const PULLED: f32 = 0.18;
const FLICKED: f32 = 0.6;
/// How far a finger moves, in points, before it says which way it goes.
const SAYS: f32 = 8.0;
/// The spring: critically damped, landing in 400 ms from still, as the page's move's.
const SPRING: Spring = Spring { stiffness: 27.0 * 27.0, damping: 2.0 * 27.0 };


/// How a window is drawn as it's pulled: grown by `scale` about its middle, then moved by `x`, `y` points, as
/// opaque as `opacity`, with corners of `corner` points (before it's grown), and what's under it dimmed by `dim`
/// (the black over it, 0 to 1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Moved {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
    pub corner: f32,
    pub dim: f32,
}

impl Moved {
    /// Where it is when nothing moves it.
    pub const STILL: Moved = Moved { scale: 1.0, x: 0.0, y: 0.0, opacity: 1.0, corner: 0.0, dim: 0.0 };
    /// A window under the one pulled, which doesn't show as it goes: the Home Screen shows behind it.
    pub const HIDDEN: Moved = Moved { opacity: 0.0, ..Moved::STILL };
}

/// A finger on a window's line, and the window's move home it drives once it's come in.
pub struct Pull {
    finger: u64,
    start: Point<f32>,
    /// It's come in far enough to say so, and the window follows it.
    pulling: bool,
    /// How fast it's been coming in lately, in widths a second.
    velocity_in: Velocity,
    /// Where the move is, 0 the app and 1 home, where it's going and how fast, once let go.
    p: f32,
    target: Option<f32>,
    velocity: f32,
    stepped: Instant,
    /// It's an app's window opening out of its icon (`opening`), on the straight way between them rather than through
    /// the card, and where it had got to as a finger took it.
    straight: bool,
    from: f32,
}

/// No finger: an opening's, until one takes it.
const NO_FINGER: u64 = u64::MAX;

/// What a finger's move or lift did to a pull.
pub enum Pulled {
    /// It's still the line's, and the window is drawn as it says, if it moves.
    Kept(Option<Moved>),
    /// It wasn't a pull (up or down the line, or out), so it's let go, and nothing moved.
    Dropped,
}

impl Pull {
    pub fn new(finger: u64, start: Point<f32>) -> Pull {
        Pull { finger, start, pulling: false, velocity_in: Velocity::default(), p: 0.0, target: None, velocity: 0.0, stepped: super::now(), straight: false, from: 0.0 }
    }

    /// An app's window opening out of its icon: home, on its way to the app, from the moment it's first shown.
    pub fn opening() -> Pull {
        Pull { finger: NO_FINGER, start: point(0.0, 0.0), pulling: true, velocity_in: Velocity::default(), p: 1.0, target: Some(0.0), velocity: 0.0, stepped: super::now(), straight: true, from: 1.0 }
    }

    /// Whether it's an opening no finger has taken yet.
    pub fn opens(&self) -> bool {
        self.finger == NO_FINGER
    }

    /// Finger `finger`, landing on the line at `at`, takes an opening from where it's got to, as iOS's swipe home
    /// takes an app still zooming open: it follows the finger from there.
    #[cfg_attr(not(any(target_os = "android", target_os = "ios", feature = "fake")), allow(dead_code))]
    pub fn take(&mut self, finger: u64, at: Point<f32>) {
        (self.finger, self.start, self.from, self.target, self.velocity) = (finger, at, self.p, None, 0.0);
        self.velocity_in = Velocity::default();
    }

    /// The spring starts now, as its window first shows something.
    pub fn shown(&mut self) {
        self.stepped = super::now();
    }

    pub fn finger(&self) -> u64 {
        self.finger
    }

    /// Whether the window has started to follow it.
    pub fn pulling(&self) -> bool {
        self.pulling
    }

    /// Whether it's let go and springing home or back.
    pub fn springing(&self) -> bool {
        self.target.is_some()
    }

    /// The finger is at `at` on a window `size` big: the window as it then is, if it's begun to follow.
    pub fn moved(&mut self, at: Point<f32>, size: Size<f32>, icon: Option<Bounds<f32>>, reduced: bool) -> Pulled {
        if self.target.is_some() {
            return Pulled::Kept(None);
        }
        let (along, down) = (at.x - self.start.x, at.y - self.start.y);
        if !self.pulling {
            // Too little yet to say which way it goes; up or down isn't a pull.
            if along.hypot(down) < SAYS {
                return Pulled::Kept(None);
            }
            if !(along < 0.0 && along.abs() > down.abs()) {
                return Pulled::Dropped;
            }
            self.pulling = true;
        }
        let inward = (-along / size.width).max(0.0);
        // It follows the finger, and past the card (`HELD`) slows to a stop, as a rubber band does; an opening taken
        // follows it from where it had got to.
        self.p = if self.straight { (self.from + inward).min(1.0) } else { HELD * (inward / HELD).tanh() };
        self.velocity_in.add(point(inward, 0.0), crate::motion::now());
        Pulled::Kept(Some(moved(self.p, size, icon, reduced, self.straight)))
    }

    /// The finger's let go, or the system took it (`cancelled`): it goes home if it came in past `PULLED` or was
    /// flicked in, and otherwise springs back. → whether it's springing; a finger that never pulled is dropped.
    pub fn let_go(&mut self, cancelled: bool) -> bool {
        if !self.pulling {
            return false;
        }
        let (inward, velocity) = (self.velocity_in.last().map_or(0.0, |at| at.x), self.velocity_in.get(crate::motion::now()).x);
        // An opening taken goes home where it's let go nearer home than the app.
        let home = !cancelled && (inward >= PULLED || velocity > FLICKED || (self.straight && self.p > 0.5 && velocity >= 0.0));
        self.target = Some(if home { 1.0 } else { 0.0 });
        self.velocity = if home { velocity.max(0.0) } else { 0.0 };
        self.stepped = super::now();
        true
    }

    /// Steps the spring to now: → the window as it then is, and whether it's still, where it was going (1 home).
    pub fn step(&mut self, size: Size<f32>, icon: Option<Bounds<f32>>, reduced: bool) -> (Moved, Option<f32>) {
        let Some(target) = self.target else { return (moved(self.p, size, icon, reduced, self.straight), None) };
        let now = super::now();
        let dt = now.duration_since(self.stepped).as_secs_f32();
        self.stepped = now;
        let still;
        (self.p, self.velocity, still) = spring(self.p, self.velocity, target, dt);
        (moved(self.p, size, icon, reduced, self.straight), still.then_some(target))
    }
}

/// Steps the rail's spring at `p`, going `velocity` a second, `dt` seconds on toward `target`, as the page's moves
/// have it (move-machine.js's `spring`): → where it is, how fast it goes, and whether it's still there. A step late, as when
/// the app was in the background, is taken as a frame's, so nothing jumps.
pub fn spring(p: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32, bool) {
    let (off, velocity) = SPRING.at(f64::from(p - target), f64::from(velocity), f64::from(dt.min(1.0 / 30.0)));
    if Spring::arrived(off, velocity, 0.001) { (target, 0.0, true) } else { (target + off as f32, velocity as f32, false) }
}

/// The window at `p` of its way home, on a window `size` big, into `icon` where it opened from one, as the page's
/// `moves` has it: whole at 0, a card at `HELD`, its right edge `HELD` of the width in, and in its icon, or shrunk
/// away, at 1, fading out over its last part; what's under it dimmed at first. `straight` goes from the whole screen to
/// its icon with no card between, as an app opening out of its icon does, its corners rounding as it leaves the whole
/// screen.
fn moved(p: f32, size: Size<f32>, icon: Option<Bounds<f32>>, reduced: bool, straight: bool) -> Moved {
    if reduced {
        return Moved { opacity: 1.0 - p, ..Moved::STILL };
    }
    let (width, height) = (size.width, size.height);
    let held = (CARD, (width / 2.0) * (1.0 - CARD) - HELD * width, 0.0);
    // The app in its icon: its middle on the icon's, as large as fits in it.
    let gone = match icon {
        Some(icon) => (
            (icon.size.width / width).min(icon.size.height / height),
            icon.origin.x + icon.size.width / 2.0 - width / 2.0,
            icon.origin.y + icon.size.height / 2.0 - height / 2.0,
        ),
        None => (0.5, -0.3 * width, 0.0),
    };
    let between = |from: (f32, f32, f32), to: (f32, f32, f32), t: f32| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, from.2 + (to.2 - from.2) * t);
    let (scale, x, y) = match straight {
        true => between((1.0, 0.0, 0.0), gone, p),
        false if p <= HELD => between((1.0, 0.0, 0.0), held, p / HELD),
        false => between(held, gone, (p - HELD) / (1.0 - HELD)),
    };
    let fades = if icon.is_some() { HELD + (1.0 - HELD) * FADES } else { HELD };
    let opacity = if p <= fades { 1.0 } else { 1.0 - (p - fades) / (1.0 - fades) };
    let corner = if straight { CORNER / CARD * (p / HELD).min(1.0) } else { CORNER / CARD };
    Moved { scale, x, y, opacity, corner, dim: 0.3 * (1.0 - p) }
}
