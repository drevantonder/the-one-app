//! What moves rather than jumps as a node's style changes, as CSS's transitions do: each property its style names
//! (`transitionProperty`) goes from what's drawn now to its new value over its motion token's time, with its easing.
//! A node that's just been added starts from its `entering` state's values, as from CSS's `@starting-style`. Where the
//! person asked for less motion, what would slide, grow or turn goes straight to where it's going, and a node coming
//! in fades in instead, as Apple's reduced motion has it.

use std::collections::{HashMap, HashSet};

use crate::geometry::{Hsla, Rgba};
use crate::style::{Computed, Offset};
use crate::tree::Id;

/// A property's value, as it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Number(f32),
    Px(f32),
    Percent(f32),
    Colour(Option<Rgba>),
}

impl Value {
    fn mix(self, to: Value, part: f32) -> Option<Value> {
        let between = |from: f32, to: f32| from + (to - from) * part;
        Some(match (self, to) {
            (Value::Number(a), Value::Number(b)) => Value::Number(between(a, b)),
            // Nothing is nothing in either unit, as CSS has `0` move to `100%`.
            (Value::Px(0.0), Value::Percent(b)) => Value::Percent(between(0.0, b)),
            (Value::Percent(a), Value::Px(0.0)) => Value::Percent(between(a, 0.0)),
            (Value::Percent(0.0), Value::Px(b)) => Value::Px(between(0.0, b)),
            (Value::Px(a), Value::Percent(0.0)) => Value::Px(between(a, 0.0)),
            (Value::Px(a), Value::Px(b)) => Value::Px(between(a, b)),
            (Value::Percent(a), Value::Percent(b)) => Value::Percent(between(a, b)),
            // No colour is the other's, clear, as CSS's `transparent` moves.
            (Value::Colour(a), Value::Colour(b)) => {
                let (a, b) = match (a, b) {
                    (None, None) => return Some(Value::Colour(None)),
                    (Some(a), None) => (a, Rgba { a: 0.0, ..a }),
                    (None, Some(b)) => (Rgba { a: 0.0, ..b }, b),
                    (Some(a), Some(b)) => (a, b),
                };
                Value::Colour(Some(Rgba { r: between(a.r, b.r), g: between(a.g, b.g), b: between(a.b, b.b), a: between(a.a, b.a) }))
            }
            _ => return None,
        })
    }
}

/// The properties that move, and whether each moves something (which reduced motion doesn't), as against fading.
const PROPERTIES: [(&str, bool); 10] = [
    ("opacity", false),
    ("scale", true),
    ("rotate", true),
    ("translateX", true),
    ("translateY", true),
    ("width", true),
    ("height", true),
    ("backgroundColor", false),
    ("borderColor", false),
    ("color", false),
];

fn get(computed: &Computed, property: &str) -> Option<Value> {
    let look = &computed.look;
    let offset = |offset: Offset| match offset {
        Offset::Px(px) => Value::Px(px),
        Offset::Percent(percent) => Value::Percent(percent),
    };
    let size = |dimension: taffy::style::Dimension| match dimension.expand() {
        taffy::style::ExpandedDimension::Length(px) => Some(Value::Px(px)),
        taffy::style::ExpandedDimension::Percent(part) => Some(Value::Percent(part)),
        _ => None,
    };
    Some(match property {
        "opacity" => Value::Number(look.opacity),
        "scale" => Value::Number(look.scale),
        "rotate" => Value::Number(look.rotate),
        "translateX" => offset(look.translate.0),
        "translateY" => offset(look.translate.1),
        "width" => size(computed.layout.size.width)?,
        "height" => size(computed.layout.size.height)?,
        "backgroundColor" => Value::Colour(look.background.map(Rgba::from)),
        "borderColor" => Value::Colour(Some(Rgba::from(look.border_color))),
        "color" => Value::Colour(computed.text.color.map(Rgba::from)),
        _ => return None,
    })
}

fn set(computed: &mut Computed, property: &str, value: Value) {
    let look = &mut computed.look;
    let offset = |value: Value| match value {
        Value::Percent(percent) => Offset::Percent(percent),
        Value::Px(px) | Value::Number(px) => Offset::Px(px),
        Value::Colour(_) => Offset::Px(0.0),
    };
    let size = |value: Value| match value {
        Value::Percent(part) => taffy::style::Dimension::percent(part),
        Value::Px(px) | Value::Number(px) => taffy::style::Dimension::length(px),
        Value::Colour(_) => taffy::style::Dimension::auto(),
    };
    match (property, value) {
        ("opacity", Value::Number(number)) => look.opacity = number,
        ("scale", Value::Number(number)) => look.scale = number,
        ("rotate", Value::Number(number)) => look.rotate = number,
        ("translateX", value) => look.translate.0 = offset(value),
        ("translateY", value) => look.translate.1 = offset(value),
        ("width", value) => computed.layout.size.width = size(value),
        ("height", value) => computed.layout.size.height = size(value),
        ("backgroundColor", Value::Colour(colour)) => look.background = colour.map(Hsla::from),
        ("borderColor", Value::Colour(colour)) => look.border_color = colour.map(Hsla::from).unwrap_or_default(),
        ("color", Value::Colour(colour)) => computed.text.color = colour.map(Hsla::from),
        _ => {}
    }
}

/// A property on its way from one value to another.
struct Moving {
    from: Value,
    to: Value,
    /// When it set off, on the screen's clock, and how long it takes, in milliseconds.
    start: f64,
    duration: f32,
    easing: [f32; 4],
}

impl Moving {
    fn at(&self, now: f64) -> Value {
        let part = ((now - self.start) / f64::from(self.duration.max(1.0))).clamp(0.0, 1.0) as f32;
        self.from.mix(self.to, ease(self.easing, part)).unwrap_or(self.to)
    }

    fn done(&self, now: f64) -> bool {
        now - self.start >= f64::from(self.duration)
    }
}

/// A cubic Bézier easing's progress at `x`, its time: the curve's `y` where its `x` is, found by halving, as its `x`
/// only grows.
fn ease([x1, y1, x2, y2]: [f32; 4], x: f32) -> f32 {
    let bezier = |t: f32, a: f32, b: f32| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if bezier(middle, x1, x2) < x {
            low = middle;
        } else {
            high = middle;
        }
    }
    bezier((low + high) / 2.0, y1, y2)
}

/// Every node's properties as they move, kept from frame to frame.
#[derive(Default)]
pub(crate) struct Transitions {
    /// Where each property that moves was last going.
    values: HashMap<(Id, &'static str), Value>,
    moving: HashMap<(Id, &'static str), Moving>,
    /// The nodes drawn last frame and this one: one that wasn't is entering.
    drawn: HashSet<Id>,
    drawing: HashSet<Id>,
}

impl Transitions {
    /// Whether node `id` is being added this frame, so it starts from its `entering` state.
    pub fn entering(&self, id: Id) -> bool {
        !self.drawn.contains(&id)
    }

    /// Node `id`'s style as it's drawn at `now` (ms on the screen's clock): each property its transition names on its
    /// way from where it was, or from `entering`'s value for a node just added, to `computed`'s. `reduced`: the person
    /// asked for less motion.
    pub fn apply(&mut self, id: Id, computed: &mut Computed, entering: Option<&Computed>, now: f64, reduced: bool) {
        self.drawing.insert(id);
        let Some(transition) = computed.transition.clone() else { return };
        for (property, moves) in PROPERTIES {
            let key = (id, property);
            let to = get(computed, property).filter(|_| transition.properties.iter().any(|named| named == property));
            let Some(to) = to else {
                self.values.remove(&key);
                self.moving.remove(&key);
                continue;
            };
            let was = self.values.insert(key, to);
            // What it's drawn as now: where a move has it, else where it was going, else where it entered from.
            let from = match self.moving.get(&key) {
                Some(moving) if moving.to == to => {
                    set(computed, property, moving.at(now));
                    continue;
                }
                Some(moving) => moving.at(now),
                None => match was.or_else(|| get(entering?, property)) {
                    Some(from) => from,
                    None => continue,
                },
            };
            if from == to || from.mix(to, 0.0).is_none() || (reduced && moves) {
                self.moving.remove(&key);
                continue;
            }
            set(computed, property, from);
            self.moving.insert(key, Moving { from, to, start: now, duration: transition.duration, easing: transition.easing });
        }
        // Coming in, with less motion: what would have slid or grown in fades in instead.
        let moved_in = entering.is_some_and(|entering| PROPERTIES.iter().any(|(property, moves)| *moves && get(entering, property) != get(computed, property)));
        if reduced && moved_in && !self.moving.contains_key(&(id, "opacity")) {
            let to = Value::Number(computed.look.opacity);
            computed.look.opacity = 0.0;
            self.moving.insert((id, "opacity"), Moving { from: Value::Number(0.0), to, start: now, duration: transition.duration, easing: transition.easing });
        }
    }

    /// Ends a frame: forgets what's no longer drawn and what's arrived. → whether anything's still moving.
    pub fn finish(&mut self, now: f64) -> bool {
        self.drawn = std::mem::take(&mut self.drawing);
        let drawn = &self.drawn;
        self.values.retain(|(id, _), _| drawn.contains(id));
        self.moving.retain(|(id, _), moving| drawn.contains(id) && !moving.done(now));
        !self.moving.is_empty()
    }

    /// How many properties are still on their way, for the inspector's `settle`.
    pub fn len(&self) -> usize {
        self.moving.len()
    }
}
