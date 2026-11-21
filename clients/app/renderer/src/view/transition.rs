//! What moves rather than jumps as a node's style changes, as CSS's transitions do: each property its style names
//! (`transitionProperty`) goes from what's drawn now to its new value over its motion token's time, with its easing,
//! or by its spring, which a change on the way turns from where it is at the speed it has, as SwiftUI's and Motion's do,
//! setting off its `transitionDelay` after the change, and with only a delay, jumping then, as CSS's do.
//! A node that's just been added starts from its `entering` state's values, as from CSS's `@starting-style`. Where the
//! person asked for less motion, what would slide, grow or turn goes straight to where it's going, and a node coming
//! in fades in instead, as Apple's reduced motion has it. A node a finger or a pointer drags (`drag`, as Motion's) follows
//! it here, where the input arrives, and let go, springs from where it is at the finger's speed to where its style
//! puts it, wherever that's moved to meanwhile.

use std::collections::{HashMap, HashSet};

use crate::geometry::{Bounds, Hsla, Point, Rgba, Size};
use crate::motion::Spring;
use crate::style::{Computed, Offset};
use crate::tokens::{Radius, Shadow};
use crate::tree::Id;

/// A property's value, as it moves: a number, a length, a colour, the shadows under it, or what shows through it
/// (`Look::material`).
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(f32),
    Px(f32),
    Percent(f32),
    Colour(Option<Rgba>),
    Shadows(Vec<Shadow>),
    Material(Option<(f32, f32, bool)>),
}

/// Two colours `part` of the way from one to the other: through Oklch, premultiplied, its hue the shorter way round, as
/// CSS Color 4 mixes colours, so a blue going to a red passes through neither grey nor a muddy middle.
fn mix_colour(a: Rgba, b: Rgba, part: f32) -> Rgba {
    let dynamic = |c: Rgba| color::DynamicColor::from_alpha_color(color::AlphaColor::<color::Srgb>::new([c.r, c.g, c.b, c.a]));
    let [r, g, b, a] = dynamic(a).interpolate(dynamic(b), color::ColorSpaceTag::Oklch, color::HueDirection::Shorter).eval(part).to_alpha_color::<color::Srgb>().components;
    Rgba { r: r.clamp(0.0, 1.0), g: g.clamp(0.0, 1.0), b: b.clamp(0.0, 1.0), a: a.clamp(0.0, 1.0) }
}

impl Value {
    fn mix(&self, to: &Value, part: f32) -> Option<Value> {
        let between = |from: f32, to: f32| from + (to - from) * part;
        Some(match (self.clone(), to.clone()) {
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
                Value::Colour(Some(mix_colour(a, b, part)))
            }
            // Shadow by shadow, the fewer made up with the other's, clear and nowhere, as CSS's `box-shadow` moves; an
            // inset one and one under it don't move into each other.
            (Value::Shadows(a), Value::Shadows(b)) => {
                let none = |other: &Shadow| Shadow { x: 0.0, y: 0.0, blur: 0.0, spread: 0.0, colour: Rgba { a: 0.0, ..other.colour }, inset: other.inset };
                let mut mixed = Vec::new();
                for at in 0..a.len().max(b.len()) {
                    let (one, other) = match (a.get(at), b.get(at)) {
                        (Some(one), Some(other)) => (*one, *other),
                        (Some(one), None) => (*one, none(one)),
                        (None, Some(other)) => (none(other), *other),
                        (None, None) => unreachable!("at is within the longer"),
                    };
                    if one.inset != other.inset {
                        return None;
                    }
                    mixed.push(Shadow {
                        x: between(one.x, other.x),
                        y: between(one.y, other.y),
                        blur: between(one.blur, other.blur),
                        spread: between(one.spread, other.spread),
                        colour: mix_colour(one.colour, other.colour, part),
                        inset: one.inset,
                    });
                }
                Value::Shadows(mixed)
            }
            // What shows through blurs in from none, as CSS's `backdrop-filter` moves from `none`; on its way, it's the
            // renderer's own blur, which fades as the system's can't, and the system's once it's there.
            (Value::Material(a), Value::Material(b)) => {
                let (one, other) = match (a, b) {
                    (None, None) => return Some(Value::Material(None)),
                    (Some(one), None) => (one, (0.0, 1.0, false)),
                    (None, Some(other)) => ((0.0, 1.0, false), other),
                    (Some(one), Some(other)) => (one, other),
                };
                Value::Material(Some((between(one.0, other.0), between(one.1, other.1), if part >= 1.0 { other.2 } else { false })))
            }
            _ => return None,
        })
    }
}

/// The properties that move, and whether each moves something (which reduced motion doesn't), as against fading.
const PROPERTIES: [(&str, bool); 16] = [
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
    ("borderTopStartRadius", false),
    ("borderTopEndRadius", false),
    ("borderBottomEndRadius", false),
    ("borderBottomStartRadius", false),
    ("boxShadow", false),
    ("backdropFilter", false),
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
    let radius = |radius: Radius| match radius {
        Radius::Px(px) => Value::Px(px),
        Radius::Percent(percent) => Value::Percent(percent),
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
        "borderTopStartRadius" => radius(look.radii.top_left),
        "borderTopEndRadius" => radius(look.radii.top_right),
        "borderBottomEndRadius" => radius(look.radii.bottom_right),
        "borderBottomStartRadius" => radius(look.radii.bottom_left),
        "boxShadow" => Value::Shadows(look.shadows.clone()),
        "backdropFilter" => Value::Material(look.material),
        _ => return None,
    })
}

fn set(computed: &mut Computed, property: &str, value: Value) {
    let look = &mut computed.look;
    let offset = |value: Value| match value {
        Value::Percent(percent) => Offset::Percent(percent),
        Value::Px(px) | Value::Number(px) => Offset::Px(px),
        _ => Offset::Px(0.0),
    };
    let size = |value: Value| match value {
        Value::Percent(part) => taffy::style::Dimension::percent(part),
        Value::Px(px) | Value::Number(px) => taffy::style::Dimension::length(px),
        _ => taffy::style::Dimension::auto(),
    };
    let radius = |value: Value| match value {
        Value::Percent(percent) => Radius::Percent(percent),
        Value::Px(px) | Value::Number(px) => Radius::Px(px),
        _ => Radius::Px(0.0),
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
        ("borderTopStartRadius", value) => look.radii.top_left = radius(value),
        ("borderTopEndRadius", value) => look.radii.top_right = radius(value),
        ("borderBottomEndRadius", value) => look.radii.bottom_right = radius(value),
        ("borderBottomStartRadius", value) => look.radii.bottom_left = radius(value),
        ("boxShadow", Value::Shadows(shadows)) => look.shadows = shadows,
        ("backdropFilter", Value::Material(material)) => look.material = material,
        _ => {}
    }
}

/// A property on its way from one value to another.
struct Moving {
    from: Value,
    to: Value,
    curve: Curve,
}

/// How a property moves: eased from when it set off, on the screen's clock, over `duration` milliseconds; or by a
/// spring from when it set off at `velocity` (in its value's unit a second), which a change on the way keeps.
enum Curve {
    Eased { start: f64, duration: f32, easing: [f32; 4] },
    Sprung { start: f64, velocity: f64, spring: Spring },
}

/// A value's number, where it's one: what a spring moves in its own unit.
fn number(value: &Value) -> Option<f32> {
    match *value {
        Value::Number(number) | Value::Px(number) | Value::Percent(number) => Some(number),
        _ => None,
    }
}

/// Whether two values are numbers in the same unit, which a spring moves between, keeping its speed.
fn alike(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Number(_), Value::Number(_)) | (Value::Px(_), Value::Px(_)) | (Value::Percent(_), Value::Percent(_)))
}

/// Of a value this close to where it's going, and moving no faster than ten times this a second, a spring has arrived:
/// a thousandth of a scale or an opacity, a tenth of a pixel.
fn near(value: &Value) -> f64 {
    match value {
        Value::Px(_) => 0.1,
        Value::Percent(_) => 0.01,
        _ => 0.001,
    }
}

/// `value`'s unit, at `number`.
fn with(value: &Value, number: f32) -> Value {
    match value {
        Value::Number(_) => Value::Number(number),
        Value::Px(_) => Value::Px(number),
        Value::Percent(_) => Value::Percent(number),
        other => other.clone(),
    }
}

impl Moving {
    /// Where it is at `now`, and how fast it's moving, in its unit a second, where it's a number a spring moves.
    fn state(&self, now: f64) -> (Value, f64) {
        match self.curve {
            Curve::Eased { start, duration, easing } => {
                let part = if duration > 0.0 { ((now - start) / f64::from(duration)).clamp(0.0, 1.0) as f32 } else { f32::from(u8::from(now >= start)) };
                (self.from.mix(&self.to, ease(easing, part)).unwrap_or_else(|| self.to.clone()), 0.0)
            }
            Curve::Sprung { start, velocity, spring } => {
                let seconds = ((now - start) / 1000.0).max(0.0);
                match (number(&self.from), number(&self.to)) {
                    (Some(from), Some(to)) if alike(&self.from, &self.to) => {
                        let (off, speed) = spring.at(f64::from(from - to), velocity, seconds);
                        (with(&self.to, (f64::from(to) + off) as f32), speed)
                    }
                    // Colours, shadows, or a number between units, by how far the spring has gone, from rest.
                    _ => {
                        let (off, _) = spring.at(1.0, 0.0, seconds);
                        (self.from.mix(&self.to, (1.0 - off as f32).clamp(0.0, 1.0)).unwrap_or_else(|| self.to.clone()), 0.0)
                    }
                }
            }
        }
    }

    fn at(&self, now: f64) -> Value {
        self.state(now).0
    }

    fn done(&self, now: f64) -> bool {
        match self.curve {
            Curve::Eased { start, duration, .. } => now - start >= f64::from(duration),
            Curve::Sprung { start, velocity, spring } => match (number(&self.from), number(&self.to)) {
                (Some(_), Some(_)) if alike(&self.from, &self.to) => {
                    let (at, speed) = self.state(now);
                    number(&at).zip(number(&self.to)).is_some_and(|(at, to)| Spring::arrived(f64::from(at - to), speed, near(&self.to)))
                }
                // What a spring moves by how far it's gone has arrived where it has.
                _ => {
                    let (off, speed) = spring.at(1.0, if alike(&self.from, &self.to) { velocity } else { 0.0 }, ((now - start) / 1000.0).max(0.0));
                    Spring::arrived(off, speed, 0.001)
                }
            },
        }
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
    /// The nodes in the tree as it's drawn: one that isn't yet is entering.
    drawn: HashSet<Id>,
    /// Each node dragged, while it's held and as it springs back once let go.
    dragged: HashMap<Id, Dragged>,
    /// The nodes moving by their keyframes (`Look::animation`), which are styled again each frame they're drawn.
    animated: HashSet<Id>,
    /// Where each node that moves as it's laid out again (`layout`) was laid out last, in the window, unscrolled, and
    /// each such node on its way from where it was to there; and where what shares a place (`layoutId`) was last laid
    /// out, by what and in which frame.
    laid: HashMap<Id, Bounds<f32>>,
    flips: HashMap<Id, Flip>,
    shared: HashMap<String, (Id, Bounds<f32>, u64)>,
}

/// A node on its way from where it was laid out to where it is, as Motion's layout animations and the FLIP technique:
/// how far from it its box is drawn, along, down, wider and taller, and how fast that was going at `start`.
struct Flip {
    off: [f64; 4],
    velocity: [f64; 4],
    start: f64,
}

impl Flip {
    fn state(&self, now: f64) -> [(f64, f64); 4] {
        let seconds = ((now - self.start) / 1000.0).max(0.0);
        std::array::from_fn(|at| DRAGGED.at(self.off[at], self.velocity[at], seconds))
    }
}

/// A node dragged: how far from where its style puts it, along and down, held where the finger has it or springing
/// back once let go.
struct Dragged {
    held: bool,
    /// Let go, but its page hasn't yet answered where it goes (`dragend`): it carries on at the speed it was let go at,
    /// till the frame that draws where it now goes (`answered`), from which it springs there.
    coasting: bool,
    answered: bool,
    axes: [Axis; 2],
    spring: Option<Spring>,
}

/// A dragged node's way along or down: how far it was from where its style puts it (`rest`, in pixels, as last
/// drawn), and how fast it was going, at `start` on the screen's clock, from which its spring goes on.
#[derive(Clone, Copy, Default)]
struct Axis {
    off: f64,
    velocity: f64,
    start: f64,
    rest: Option<f32>,
}

impl Dragged {
    /// How far from where it rests, and how fast, at `now`, along each way.
    fn state(&self, now: f64) -> [(f64, f64); 2] {
        let seconds = |axis: &Axis| ((now - axis.start) / 1000.0).max(0.0);
        self.axes.map(|axis| match (self.held, self.coasting, self.spring) {
            (false, true, _) => (axis.off + axis.velocity * seconds(&axis), axis.velocity),
            (false, false, Some(spring)) => spring.at(axis.off, axis.velocity, seconds(&axis)),
            _ => (axis.off, 0.0),
        })
    }

    fn done(&self, now: f64) -> bool {
        !self.held && !self.coasting && self.spring.is_some() && self.state(now).iter().all(|(off, velocity)| Spring::arrived(*off, *velocity, 0.1))
    }
}

/// How a node let go springs back where its style says nothing of how it moves: SwiftUI's default spring.
const DRAGGED: Spring = Spring { stiffness: 157.9, damping: 25.13 };

impl Transitions {
    /// Whether node `id` is being added this frame, so it starts from its `entering` state.
    pub fn entering(&self, id: Id) -> bool {
        !self.drawn.contains(&id)
    }

    /// Node `id`'s style as it's drawn at `now` (ms on the screen's clock): each property its transition names on its
    /// way from where it was, or from `entering`'s value for a node just added, to `computed`'s, setting off when it
    /// changed (`since`). `reduced`: the person asked for less motion. `unseen`: the window isn't shown, so it's where
    /// it's going at once, as nobody sees it on its way, and nothing waits for it to get there. `size`: how big it was
    /// last laid out, which a part of it it's moved by is of.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(&mut self, id: Id, computed: &mut Computed, entering: Option<&Computed>, now: f64, since: f64, reduced: bool, unseen: bool, size: Size<f32>) {
        self.drawn.insert(id);
        let dragged = self.drag(id, computed, now, size);
        let Some(transition) = computed.transition.clone() else { return };
        for (property, moves) in PROPERTIES {
            let key = (id, property);
            // Where it's dragged, it's where the drag has it.
            if dragged && matches!(property, "translateX" | "translateY") {
                self.moving.remove(&key);
                continue;
            }
            let to = get(computed, property).filter(|_| transition.properties.iter().any(|named| named == property));
            let Some(to) = to else {
                self.values.remove(&key);
                self.moving.remove(&key);
                continue;
            };
            let was = self.values.insert(key, to.clone());
            if unseen {
                self.moving.remove(&key);
                continue;
            }
            // What it's drawn as now, and how fast it's going: where a move has it, else where it was going, else where
            // it entered from, at rest.
            let (from, velocity) = match self.moving.get(&key) {
                Some(moving) if moving.to == to => {
                    set(computed, property, moving.at(now));
                    continue;
                }
                Some(moving) => moving.state(since),
                None => match was.or_else(|| get(entering?, property)) {
                    Some(from) => (from, 0.0),
                    None => continue,
                },
            };
            if from == to || from.mix(&to, 0.0).is_none() || (reduced && moves) {
                self.moving.remove(&key);
                continue;
            }
            let curve = match transition.spring {
                // A spring sets off at the speed its style says, of the way a second, or turned, keeps its speed, where
                // it's a number in the same unit.
                Some(spring) => {
                    let said = transition.velocity.zip(number(&from).zip(number(&to))).map(|(velocity, (from, to))| f64::from(velocity * (to - from)));
                    Curve::Sprung { start: since + f64::from(transition.delay), velocity: said.unwrap_or(if alike(&from, &to) { velocity } else { 0.0 }), spring }
                }
                None => Curve::Eased { start: since + f64::from(transition.delay), duration: transition.duration, easing: transition.easing },
            };
            let moving = Moving { from, to, curve };
            set(computed, property, moving.at(now));
            self.moving.insert(key, moving);
        }
        // Coming in, with less motion: what would have slid or grown in fades in instead.
        let moved_in = entering.is_some_and(|entering| PROPERTIES.iter().any(|(property, moves)| *moves && get(entering, property) != get(computed, property)));
        if reduced && !unseen && moved_in && !self.moving.contains_key(&(id, "opacity")) {
            let curve = Curve::Eased { start: since, duration: transition.duration, easing: transition.easing };
            let moving = Moving { from: Value::Number(0.0), to: Value::Number(computed.look.opacity), curve };
            set(computed, "opacity", moving.at(now));
            self.moving.insert((id, "opacity"), moving);
        }
    }

    /// Node `id` as its keyframes have it at `now` (ms on the screen's clock), where it has them: each property one
    /// names, between the keyframes either side of where it is in its time through, eased, from the node's own where
    /// it has no keyframe at its start or its end, as CSS's `@keyframes` are.
    pub fn animate(&mut self, id: Id, computed: &mut Computed, now: f64, conditions: crate::tokens::Conditions) {
        let Some(animation) = computed.look.animation.clone() else {
            self.animated.remove(&id);
            return;
        };
        self.animated.insert(id);
        let part = ((now % f64::from(animation.duration)) / f64::from(animation.duration)) as f32;
        let frames: Vec<(f32, Computed)> = animation.frames.iter().map(|(at, style)| (*at, crate::style::compute(&[serde_json::Value::Object(style.clone())], Default::default(), conditions))).collect();
        for (property, _) in PROPERTIES {
            let Some(own) = get(computed, property) else { continue };
            let mut stops: Vec<(f32, Value)> = animation.frames.iter().zip(&frames).filter(|((_, style), _)| style.contains_key(property)).filter_map(|((at, _), (_, frame))| Some((*at, get(frame, property)?))).collect();
            if stops.is_empty() {
                continue;
            }
            if stops.first().is_none_or(|(at, _)| *at > 0.0) {
                stops.insert(0, (0.0, own.clone()));
            }
            if stops.last().is_none_or(|(at, _)| *at < 1.0) {
                stops.push((1.0, own));
            }
            let after = stops.iter().position(|(at, _)| *at > part).unwrap_or(stops.len() - 1).max(1);
            let ((from, a), (to, b)) = (&stops[after - 1], &stops[after]);
            let between = if to > from { ((part - from) / (to - from)).clamp(0.0, 1.0) } else { 1.0 };
            if let Some(value) = a.mix(b, ease(animation.easing, between)) {
                set(computed, property, value);
            }
        }
    }

    /// Node `id`, which moves as it's laid out again, is laid out at `laid` in `frame`: where that's moved from where it
    /// was, or, new, from where what it shares its place with (`shared`) was in the frame before, it's drawn going
    /// there from where it's drawn now, at the speed it has. `reduced`: it jumps.
    pub fn laid_out(&mut self, id: Id, shared: Option<&str>, laid: Bounds<f32>, now: f64, frame: u64, reduced: bool) {
        let was = self.laid.insert(id, laid).or_else(|| shared.and_then(|key| self.shared.get(key)).filter(|(owner, _, seen)| *owner != id && seen + 1 >= frame).map(|(_, bounds, _)| *bounds));
        if let Some(key) = shared {
            self.shared.insert(key.to_owned(), (id, laid, frame));
        }
        let Some(was) = was.filter(|was| *was != laid && !reduced) else { return };
        let now_at = self.flips.get(&id).map(|flip| flip.state(now)).unwrap_or_default();
        let to = [laid.origin.x, laid.origin.y, laid.size.width, laid.size.height];
        let from = [was.origin.x, was.origin.y, was.size.width, was.size.height];
        let off = std::array::from_fn(|at| f64::from(from[at] - to[at]) + now_at[at].0);
        let velocity = std::array::from_fn(|at| now_at[at].1);
        self.flips.insert(id, Flip { off, velocity, start: now });
    }

    /// How far from where it's laid out node `id` is drawn at `now`, on its way there: along, down, wider and taller.
    pub fn flip(&self, id: Id, now: f64) -> Option<[f32; 4]> {
        self.flips.get(&id).map(|flip| flip.state(now).map(|(off, _)| off as f32))
    }

    /// Node `id` is dragged `offset` pixels, along and down, from where its style puts it, held there.
    pub fn hold(&mut self, id: Id, offset: Point<f32>) {
        let dragged = self.dragged.entry(id).or_insert(Dragged { held: true, coasting: false, answered: false, axes: [Axis::default(); 2], spring: None });
        (dragged.held, dragged.coasting, dragged.answered) = (true, false, false);
        dragged.spring = None;
        for (axis, by) in dragged.axes.iter_mut().zip([offset.x, offset.y]) {
            axis.off = f64::from(by);
            axis.velocity = 0.0;
        }
    }

    /// Node `id` is let go at `now` going `velocity` pixels a second, along and down: it springs from where it is to
    /// where its style puts it, or where its page is yet to say where it goes (`answered`), carries on at that speed
    /// till it does, so it's never pulled back toward where it was before its page has moved it.
    pub fn release(&mut self, id: Id, velocity: Point<f32>, now: f64, answered: bool) {
        let Some(dragged) = self.dragged.get_mut(&id).filter(|dragged| dragged.held) else { return };
        (dragged.held, dragged.coasting, dragged.answered) = (false, !answered, false);
        for (axis, speed) in dragged.axes.iter_mut().zip([velocity.x, velocity.y]) {
            (axis.velocity, axis.start) = (f64::from(speed), now);
        }
    }

    /// Node `id`'s page has said where it goes, let go: from the next frame, it springs there from where it's got to,
    /// at the speed it has.
    pub fn answered(&mut self, id: Id) {
        if let Some(dragged) = self.dragged.get_mut(&id) {
            dragged.answered = true;
        }
    }

    /// Draws node `id` where it's dragged: where its style puts it, in pixels, and how far it's held or springing
    /// from there, its spring set off again from where it is where its style moved meanwhile, so it goes on at the
    /// speed it had. → whether it's dragged.
    fn drag(&mut self, id: Id, computed: &mut Computed, now: f64, size: Size<f32>) -> bool {
        let Some(dragged) = self.dragged.get_mut(&id) else { return false };
        if dragged.spring.is_none() && !dragged.held {
            let transition = computed.transition.as_ref();
            dragged.spring = Some(transition.and_then(|transition| transition.spring).or(transition.map(|transition| Spring::new(transition.duration, 0.0))).unwrap_or(DRAGGED));
        }
        // Where its style puts it is where it's let go to, not a change to move to once it's arrived; where it was
        // drawn nowhere as it was held (let go within a frame), it was held from where its style last put it.
        let px = |value: Value, size: f32| match value {
            Value::Px(px) | Value::Number(px) => Some(px),
            Value::Percent(percent) => Some(size * percent / 100.0),
            _ => None,
        };
        let mut styled = [None; 2];
        for ((property, along), was) in [("translateX", size.width), ("translateY", size.height)].into_iter().zip(&mut styled) {
            if let Some(now) = get(computed, property) {
                *was = self.values.insert((id, property), now).and_then(|value| px(value, along));
            }
        }
        let rests = [computed.look.translate.0.of(size.width), computed.look.translate.1.of(size.height)];
        // Its page has answered: it springs from where it's coasted to, now.
        if dragged.coasting && dragged.answered {
            let state = dragged.state(now);
            dragged.coasting = false;
            for (axis, (off, velocity)) in dragged.axes.iter_mut().zip(state) {
                (axis.off, axis.velocity, axis.start) = (off, velocity, now);
            }
        }
        let state = dragged.state(now);
        for (((axis, rest), (off, velocity)), styled) in dragged.axes.iter_mut().zip(rests).zip(state).zip(styled) {
            if let Some(was) = axis.rest.or(styled).filter(|was| *was != rest && !dragged.held) {
                (axis.off, axis.velocity, axis.start) = (off + f64::from(was - rest), velocity, now);
            }
            axis.rest = Some(rest);
        }
        let [(x, _), (y, _)] = dragged.state(now);
        computed.look.translate = (Offset::Px(rests[0] + x as f32), Offset::Px(rests[1] + y as f32));
        true
    }

    /// Node `id` is no longer in the tree as it's drawn: put back, it enters again.
    pub fn forget(&mut self, id: Id) {
        self.dragged.remove(&id);
        self.laid.remove(&id);
        self.flips.remove(&id);
        self.animated.remove(&id);
        if self.drawn.remove(&id) {
            for (property, _) in PROPERTIES {
                self.values.remove(&(id, property));
                self.moving.remove(&(id, property));
            }
        }
    }

    /// Whether something's on its way somewhere it'll arrive at: what moves but a loop (`animation`), which never does.
    pub fn arriving(&self) -> bool {
        !self.moving.is_empty() || !self.dragged.is_empty() || !self.flips.is_empty()
    }

    /// The nodes with something on its way, which are styled again each frame until it's arrived.
    pub fn moving(&self) -> HashSet<Id> {
        self.on_the_way().chain(self.animated.iter().copied()).collect()
    }

    /// How many nodes move by a loop and nothing else (`animation`, a spinner), which never arrives: it's at rest as
    /// a test's end has it, as Flutter's tests pump a while rather than wait for an endless animation to end.
    pub fn repeating(&self) -> usize {
        let arriving: HashSet<Id> = self.on_the_way().collect();
        self.animated.iter().filter(|id| !arriving.contains(id)).count()
    }

    fn on_the_way(&self) -> impl Iterator<Item = Id> + '_ {
        self.moving.keys().map(|(id, _)| *id).chain(self.dragged.keys().copied()).chain(self.flips.keys().copied())
    }

    /// The nodes moving only by their own move, growth and fade (`translateX`, `translateY`, `scale`, `opacity`),
    /// which change nothing in them, so what's in them can be drawn once and moved (`view::Layer`).
    pub fn moving_whole(&self) -> HashSet<Id> {
        let whole = |property: &str| matches!(property, "translateX" | "translateY" | "scale" | "opacity");
        let mut ids: HashSet<Id> = self.moving.keys().filter(|(_, property)| whole(property)).map(|(id, _)| *id).chain(self.dragged.keys().copied()).collect();
        for (id, _) in self.moving.keys().filter(|(_, property)| !whole(property)) {
            ids.remove(id);
        }
        ids
    }

    /// Ends a frame: forgets what's arrived. → whether anything's still moving.
    pub fn finish(&mut self, now: f64) -> bool {
        self.moving.retain(|_, moving| !moving.done(now));
        self.dragged.retain(|_, dragged| !dragged.done(now));
        self.flips.retain(|_, flip| !flip.state(now).iter().all(|(off, velocity)| Spring::arrived(*off, *velocity, 0.1)));
        !self.moving.is_empty() || !self.dragged.is_empty() || !self.animated.is_empty() || !self.flips.is_empty()
    }

    /// How many properties are still on their way, for the inspector's `settle`.
    pub fn len(&self) -> usize {
        self.moving.len() + self.dragged.len() + self.flips.len()
    }
}

