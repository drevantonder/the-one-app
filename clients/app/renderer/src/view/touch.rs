//! A finger on a phone's screen, taken the same by every host that has one (an iPhone's, Android's, a phone's
//! browser's, the fake platform's and the inspector's replays), so what the harness checks is what a phone does:
//! - It presses what it lands on, but an input, which it gives the keyboard only as it lifts, as a phone's own fields
//!   do, so one that lands there to scroll doesn't; and a fling, which it only stops.
//! - Held still on an input (`Fingers::hold`), it selects the word there, and moved on, the words to where it is; two
//!   taps select a word too, and a handle of what it selected drags that end (`text_selection`).
//! - Past the slop, it pans what scrolls that way under it, and what it pressed isn't pressed; where nothing there
//!   scrolls that way, or what it's on keeps it, it's that's to follow (`Screen::pans`), as a browser decides by
//!   `touch-action`. Panning what's scrolled to its start and pulled on past it, what it's in that drags that way (a
//!   sheet) takes it (`Screen::hand_over`), as an iPhone's sheet does.
//! - Lifted, a pan carries on as a fling at the speed it had, or springs back from past its end.
//! - From the left edge, where the host leaves it to the app (`Fingers::edge`), it drags the window back, as
//!   UINavigationController's swipe does: let go far enough or flicked, through what's open over its page first.
//!
//! A host says where its system differs (`Fingers`), and does what's its own (`Touched`): sliding its window, flinging
//! where it flings itself, bringing up its keyboard.

use super::Screen;
use super::text_selection::End;
use crate::geometry::{Point, point};
use crate::motion::Velocity;

/// How far a finger moves before it pans rather than presses, in points: UIScrollView's.
pub const SLOP: f32 = 10.0;
/// How long a finger is held still before it's a long press, in ms: UILongPressGestureRecognizer's.
pub const HOLD: f64 = 500.0;
/// How far apart a finger's taps land and still count as a double tap, in points: Android's (`ViewConfiguration`'s
/// double-tap slop), as a fingertip lands nowhere near as exactly as a pointer.
pub const DOUBLE_TAP_SLOP: f32 = 100.0;
/// How near the left edge a finger lands to swipe back, in points, as UINavigationController's.
const EDGE: f32 = 20.0;
/// How fast a flick back goes, in points a second, and how far across a swipe back goes, as a share of the width.
const FLICKED: f32 = 500.0;
const ACROSS: f32 = 1.0 / 3.0;

/// Where a finger is, as its platform says.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Down,
    Moved,
    Up,
    Cancelled,
}

impl Phase {
    /// The phase named so: `down`, `moved`, `up` or `cancelled`, as a trace says it.
    pub fn named(name: &str) -> Option<Phase> {
        [Phase::Down, Phase::Moved, Phase::Up, Phase::Cancelled].into_iter().find(|phase| format!("{phase:?}").eq_ignore_ascii_case(name))
    }
}

/// What a host's system does with a finger itself, where systems differ.
#[derive(Clone, Copy)]
pub struct Fingers {
    /// How far a finger moves before it pans, in points: Android's own (`ViewConfiguration`'s), UIScrollView's elsewhere.
    pub slop: f32,
    /// The host flings what a pan let go of (Android's `OverScroller`, which stretches at the ends rather than going
    /// past them), and says when a finger stopped its fling (`stopped`); elsewhere the renderer carries a pan on and
    /// springs it back from past its end, as UIScrollView does.
    pub flings: bool,
    pub stopped: bool,
    /// Where a finger from the left edge swipes the window back, as an iPhone's apps draw that themselves (Android's
    /// and a browser's is the system's own, and the Home Screen has nothing to go back to): how wide the window is, and
    /// how far across it's slid now, where it's springing from a swipe let go, which a finger landing catches.
    pub edge: Option<Edge>,
    /// How long a finger is held still before it's a long press, in ms: Android's own (`ViewConfiguration`'s, which
    /// its accessibility settings lengthen), UIKit's elsewhere.
    pub hold: f64,
}

#[derive(Clone, Copy)]
pub struct Edge {
    pub width: f32,
    pub slid: Option<f32>,
}

impl Fingers {
    /// A phone's, as an iPhone's are, with no edge to swipe from: the inspector's replays'.
    pub const PHONE: Fingers = Fingers { slop: SLOP, flings: false, stopped: false, edge: None, hold: HOLD };
}

/// The finger a screen follows: the last to land, as one hand holds the phone and the other's finger does what's done.
pub(super) struct Finger {
    id: u64,
    start: Point<f32>,
    pub(super) last: Point<f32>,
    /// When it landed, on the page's clock, and how long it's held still before it's a long press.
    landed: f64,
    hold: f64,
    /// It was held still on an input long enough: it selects by the word as it moves (`text_selection`).
    pub(super) held: bool,
    /// It landed on a handle of what a finger selected, whose end it drags.
    pub(super) handle: Option<End>,
    /// Whether the system's menu showed as it landed, which hides it: a tap on the caret shows it again or not.
    menu: bool,
    /// It's moved past the slop, so it's decided whether it pans or is what it's on's.
    moved: bool,
    panning: bool,
    on_input: bool,
    stopped: bool,
    swipe: Option<Swipe>,
    velocity: Velocity,
}

/// What a finger from the left edge swipes back.
#[derive(Clone, Copy, PartialEq)]
enum Swipe {
    /// The window, which follows it across.
    Window,
    /// A screen of the page's over its first, which the page goes back from as it's let go, the window staying put.
    Screen,
}

/// What its host does, as a finger moved.
#[derive(Debug, PartialEq)]
pub enum Touched {
    /// Nothing.
    Done,
    /// It pans what's under where it landed: a host that flings flings it as it lifts.
    Panning,
    /// The window's drawn so many points across, following it from the edge.
    Slide(f32),
    /// Let go from the edge: the window springs from where it's slid at `velocity` points a second, off to the right
    /// and closed where it's `out`, or back.
    Slid { velocity: f32, out: bool },
    /// It tapped the field that has the keyboard: the keyboard comes up again, if it went.
    Keyboard,
}

impl Screen {
    /// Finger `id` at `at`, in the window's points, at `time` on the page's clock, as its host says it.
    pub fn touch(&mut self, fingers: &Fingers, id: u64, at: Point<f32>, phase: Phase, time: f64) -> Touched {
        self.touch = true;
        // A finger landing takes over from one whose end never came.
        if phase == Phase::Down {
            if let Some(finger) = self.finger.take() {
                self.touch_cancelled(fingers, finger, at);
            }
            return self.touch_down(fingers, id, at, time);
        }
        let Some(mut finger) = self.finger.take().filter(|finger| finger.id == id) else { return Touched::Done };
        finger.velocity.add(at, time);
        if phase == Phase::Moved {
            let touched = self.touch_moved(fingers, &mut finger, at);
            self.finger = Some(finger);
            return touched;
        }
        let touched = match phase {
            Phase::Up => self.touch_up(fingers, finger, at, time),
            _ => self.touch_cancelled(fingers, finger, at),
        };
        // A finger that's lifted is over nothing, as a phone's browser has it: nothing under it is hovered.
        self.pointer_left();
        touched
    }

    /// Whether finger `id`, lifted, presses what it's on, as it does unless it panned, swiped, stopped a fling or moved a selection: what its
    /// lift does is then its page's, which waits for the page to have run what came before (`Windows::touch`).
    pub fn lift_presses(&self, id: u64) -> bool {
        self.finger.as_ref().is_some_and(|finger| finger.id == id && !finger.panning && !finger.stopped && finger.swipe.is_none() && !finger.held && finger.handle.is_none())
    }

    /// How long until the finger it follows, held still on an input, is a long press (`Fingers::hold`).
    pub(super) fn finger_hold(&self) -> Option<std::time::Duration> {
        let finger = self.finger.as_ref().filter(|finger| finger.on_input && !finger.moved && !finger.held)?;
        let left = finger.landed + finger.hold - self.js.clock().now();
        Some(std::time::Duration::from_secs_f64(left.max(0.0) / 1000.0))
    }

    /// The finger it follows has been held still on an input long enough: the word under it is selected, with its
    /// handles and the system's menu as it lifts, and as it moves on, the words to where it is.
    pub(super) fn finger_held(&mut self) {
        if self.finger_hold() != Some(std::time::Duration::ZERO) {
            return;
        }
        let Some(at) = self.finger.as_ref().map(|finger| finger.last) else { return };
        self.select_word(at);
        if let Some(finger) = &mut self.finger {
            finger.held = true;
        }
    }

    fn touch_down(&mut self, fingers: &Fingers, id: u64, at: Point<f32>, time: f64) -> Touched {
        let stopped = fingers.stopped || self.catch_scroll(at);
        // The system's menu goes as a finger lands, as UIKit's and Android's do.
        let menu = self.editing.overlay.is_some_and(|overlay| overlay.menu);
        self.text_menu_closed();
        // One that lands on a handle of what a finger selected drags that end of it.
        let handle = fingers.edge.and_then(|edge| edge.slid).is_none().then(|| self.handle_at(at)).flatten();
        if handle.is_some() {
            let mut velocity = Velocity::default();
            velocity.add(at, time);
            #[rustfmt::skip]
            let finger = Finger { id, start: at, last: at, landed: time, hold: fingers.hold, held: false, handle, menu, moved: false, panning: false, on_input: false, stopped: false, swipe: None, velocity };
            self.finger = Some(finger);
            return Touched::Done;
        }
        // One that lands as the window springs from the edge takes it where it is. What's open over a page (a sheet, a
        // menu) takes no swipe back, as iOS's modals don't.
        let covered = self.covered();
        let swipe = fingers.edge.and_then(|edge| match edge.slid {
            Some(_) => Some(Swipe::Window),
            None if at.x < EDGE && covered != Some(true) => Some(if covered == Some(false) { Swipe::Screen } else { Swipe::Window }),
            None => None,
        });
        let slid = fingers.edge.and_then(|edge| edge.slid).unwrap_or(0.0);
        let on_input = swipe.is_none() && self.input_at(at);
        let mut velocity = Velocity::default();
        velocity.add(at, time);
        #[rustfmt::skip]
        let finger = Finger { id, start: point(at.x - slid, at.y), last: at, landed: time, hold: fingers.hold, held: false, handle: None, menu, moved: false, panning: false, on_input, stopped, swipe, velocity };
        self.finger = Some(finger);
        if swipe.is_none() && !on_input && !stopped {
            self.pointer_down(at, 0, false);
        }
        match swipe {
            Some(Swipe::Window) => Touched::Slide(slid),
            _ => Touched::Done,
        }
    }

    fn touch_moved(&mut self, fingers: &Fingers, finger: &mut Finger, at: Point<f32>) -> Touched {
        match finger.swipe {
            Some(Swipe::Window) => return Touched::Slide((at.x - finger.start.x).max(0.0)),
            Some(Swipe::Screen) => return Touched::Done,
            None => {}
        }
        // Held on a selection or on a handle, it moves the selection.
        if finger.held || finger.handle.is_some() {
            finger.last = at;
            match finger.handle {
                Some(end) => self.drag_handle(end, at),
                None => self.drag_word(at),
            }
            return Touched::Done;
        }
        let by = point(at.x - finger.start.x, at.y - finger.start.y);
        if !finger.moved && (by.x.abs() > fingers.slop || by.y.abs() > fingers.slop) {
            finger.moved = true;
            finger.panning = finger.on_input || finger.stopped || self.pans(finger.start, by);
            if finger.panning && !finger.on_input && !finger.stopped {
                self.pointer_cancel(at);
            } else if !finger.panning {
                self.dragged();
            }
        }
        let step = point(finger.last.x - at.x, finger.last.y - at.y);
        finger.last = at;
        // Scrolled to its start and pulled on, what it's in that drags that way (a sheet) takes the finger.
        if finger.panning && !finger.on_input && self.hand_over(finger.start, step, at) {
            finger.panning = false;
        }
        if finger.panning {
            match fingers.flings {
                true => drop(self.pan(finger.start, step)),
                false => drop(self.drag_scroll(finger.start, step)),
            }
            return Touched::Panning;
        }
        if !finger.on_input && !finger.stopped {
            self.pointer_moved(at);
        }
        Touched::Done
    }

    fn touch_up(&mut self, fingers: &Fingers, finger: Finger, at: Point<f32>, time: f64) -> Touched {
        // How fast it was going, along the finger.
        let going = finger.velocity.get(time);
        let width = fingers.edge.map_or(0.0, |edge| edge.width);
        let far = at.x - finger.start.x > width * ACROSS || going.x > FLICKED;
        match finger.swipe {
            // Over a screen of the page's, it goes back a screen.
            Some(Swipe::Screen) => {
                if far {
                    self.back();
                }
                return Touched::Done;
            }
            // Let go far enough, or flicked, it goes back: through what's open over the page or its own screens first,
            // as iOS's navigation does, the window springing back as the page goes back a screen; from its first, the
            // window slides off and closes. Short of that, it springs back.
            Some(Swipe::Window) => {
                let out = far && !self.back();
                return Touched::Slid { velocity: if far { going.x.max(0.0) } else { 0.0 }, out };
            }
            None => {}
        }
        // Lifted from moving a selection, the system's menu shows for it.
        if finger.held || finger.handle.is_some() {
            self.show_text_menu(true);
            return Touched::Done;
        }
        if finger.panning {
            // Scrolled, the menu shows again if it showed.
            self.show_text_menu(finger.menu);
            if fingers.flings {
                return Touched::Panning;
            }
            self.release_scroll(finger.start, point(-going.x, -going.y));
            return Touched::Done;
        }
        if finger.stopped {
            return Touched::Done;
        }
        let had = self.focused();
        let caret = had.and_then(|id| self.editing.fields.get(&id)).filter(|field| field.selected().is_empty()).map(crate::editing::Field::caret);
        if finger.on_input {
            self.pointer_down(at, 0, false);
        }
        self.pointer_up(at, 0);
        if finger.on_input {
            self.tapped_input(had, caret, finger.menu);
        }
        match finger.on_input && had.is_some() && self.focused() == had {
            true => Touched::Keyboard,
            false => Touched::Done,
        }
    }

    fn touch_cancelled(&mut self, fingers: &Fingers, finger: Finger, at: Point<f32>) -> Touched {
        match finger.swipe {
            Some(Swipe::Window) => return Touched::Slid { velocity: 0.0, out: false },
            Some(Swipe::Screen) => return Touched::Done,
            None => {}
        }
        if finger.held || finger.handle.is_some() {
            self.show_text_menu(true);
        } else if finger.panning {
            self.show_text_menu(finger.menu);
            if !fingers.flings {
                self.release_scroll(finger.start, point(0.0, 0.0));
            }
        } else if !finger.on_input && !finger.stopped {
            self.pointer_cancel(at);
        }
        Touched::Done
    }
}
