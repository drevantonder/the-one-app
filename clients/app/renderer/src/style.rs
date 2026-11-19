//! How a node looks and is laid out, from what its page says, resolved against the tokens (`tokens`): its layout as
//! Taffy's style, its box (`Look`) and its text (`Text`), together `Computed`. A page names its styles by id (`Styles`
//! in core/ui/src/drawing-primitives/renderer.ts), each a style table's, merged in order, each property's value by the
//! node's state (`State`). The one place that knows how styles are written.

use taffy::prelude::*;
use taffy::style::{GridTemplateComponent, Overflow, RepetitionCount};

use std::collections::HashMap;
use std::sync::RwLock;

use serde_json::{Map, Value};

use crate::Cursor;

/// The one way a finger pans (`Look::touch_action`): along, or down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pan {
    X,
    Y,
}
use crate::geometry::{Corners, Edges, Hsla, Rgba};
use crate::text::TextStyle;
use crate::tokens::{Conditions, Radius, Shadow, table};
use crate::tree::Node;

/// A node's box: its fill, border and corners, its shadows and what shows through it, how opaque it is, where it's
/// drawn from where it's laid out, and how it takes the pointer.
#[derive(Clone, Debug)]
pub struct Look {
    pub background: Option<Hsla>,
    pub border_color: Hsla,
    pub borders: Edges<f32>,
    pub dashed: bool,
    pub radii: Corners<Radius>,
    pub shadows: Vec<Shadow>,
    /// What's behind it, blurred so far and saturated so much, under its background, and whether it's the system's own
    /// material, where the host has one.
    pub material: Option<(f32, f32, bool)>,
    pub opacity: f32,
    pub clips: bool,
    pub scrolls: bool,
    /// Moved along and down, each pixels or a part of its own size, grown, and turned (degrees), around its middle.
    pub translate: (Offset, Offset),
    pub scale: f32,
    pub rotate: f32,
    pub z_index: i32,
    /// Whether it takes the pointer, as its style says (`pointerEvents`), or as what it's in does (`None`), as CSS's
    /// `pointer-events` is inherited: a layer that says `none` lets presses through, and what in it says `auto` takes them.
    pub pointer_events: Option<bool>,
    /// Whether a press moves its window (`appRegion: "drag"`), or doesn't though what it's in does (`no-drag`).
    pub app_region: Option<bool>,
    /// Which way a finger on it the device scrolls by itself where it listens for its pointer, as CSS's `touch-action`
    /// (`auto`, `pan-x`, `pan-y`): the way it pans, or every way (`None` for `auto`, the default). Otherwise a node
    /// that listens for its pointer keeps a finger that lands on it (`Screen::pans`).
    pub touch_action: Option<Pan>,
    /// It's about to move, grow or fade (`willChange` names one of those), so it's drawn ahead as a layer of its own.
    pub will_change: bool,
    pub cursor: Option<Cursor>,
    /// A path's paint, where it says, and its stroke's width.
    pub fill: Paint,
    pub stroke: Paint,
    pub stroke_width: f32,
    /// A motion that repeats while it's drawn: `spin` or `pulse`, and how long one takes, in milliseconds.
    pub animation: Option<(String, f32)>,
}

/// A path's paint: a colour, the text's colour, or none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Colour(Hsla),
    Current,
    None,
}

/// A length a node is moved by: pixels, or a part of its own size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Offset {
    Px(f32),
    Percent(f32),
}

impl Offset {
    pub fn of(self, size: f32) -> f32 {
        match self {
            Offset::Px(px) => px,
            Offset::Percent(percent) => size * percent / 100.0,
        }
    }
}

impl Default for Look {
    fn default() -> Look {
        Look {
            background: None,
            border_color: Hsla::default(),
            borders: Edges::default(),
            dashed: false,
            radii: Corners::all(Radius::Px(0.0)),
            shadows: Vec::new(),
            material: None,
            opacity: 1.0,
            clips: false,
            scrolls: false,
            translate: (Offset::Px(0.0), Offset::Px(0.0)),
            scale: 1.0,
            rotate: 0.0,
            z_index: 0,
            pointer_events: None,
            touch_action: None,
            will_change: false,
            app_region: None,
            cursor: None,
            fill: Paint::Current,
            stroke: Paint::None,
            stroke_width: 1.0,
            animation: None,
        }
    }
}

/// How text is set where it is: what a node's children inherit, as CSS's text properties are inherited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Text {
    pub style: TextStyle,
    pub color: Hsla,
    /// Where a line goes along its box: 0 start, 0.5 centre, 1 end.
    pub align: f32,
    /// Whether lines wrap at its box's width, or run on (`whiteSpace: nowrap`).
    pub wrap: bool,
    pub underline: bool,
}

impl Text {
    /// A heading's, as the DOM draws a `Text` that's one, an `<h1>` to `<h3>`, with no styles of its own: a title's
    /// weight and tracking (ui.css).
    pub fn heading(self, conditions: Conditions) -> Text {
        let title = table().type_style("type-title", conditions).unwrap_or_default();
        let mut out = self;
        out.style.weight = title.weight.round() as u16;
        if out.style.tracking == 0.0 {
            out.style.tracking = title.tracking;
        }
        out
    }
}

/// A node's state, which a style's value may depend on, as `State` in core/ui/src/drawing-primitives/renderer.ts has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub hover: bool,
    pub pressed: bool,
    pub focused: bool,
    pub entering: bool,
}

/// Every style table's styles, by id: the UI library's and each app's, kept in one, as their ids are their contents'.
static STYLES: RwLock<Option<HashMap<String, Value>>> = RwLock::new(None);

/// Takes a style table (`Styles`): the UI library's (`/ui/client/styles.json`), or an app's (`styles.json` beside
/// its page). → the runtime version it needs.
pub fn load_styles(json: &str) -> anyhow::Result<u32> {
    let parsed: Value = serde_json::from_str(json)?;
    let mut styles = STYLES.write().expect("nothing panics holding it");
    let styles = styles.get_or_insert_with(HashMap::new);
    for (id, style) in parsed["styles"].as_object().into_iter().flatten() {
        styles.insert(id.clone(), style.clone());
    }
    Ok(parsed["v"].as_u64().unwrap_or(0) as u32)
}

/// Whether a value is split by state: an object of `default` and states, and nothing else.
fn by_state(value: &Value) -> Option<&Map<String, Value>> {
    let object = value.as_object()?;
    let states = ["default", "hover", "pressed", "focused", "entering"];
    (!object.is_empty() && object.keys().all(|key| states.contains(&key.as_str()))).then_some(object)
}

/// Styles merged in order, each by id or an object of its own (its lengths pixels), the last to say a property
/// winning, all of it, so one that says a property only in a state leaves it unset otherwise; then each value as it
/// is in `state`: `default`, then each state that holds, the later winning.
pub fn resolve(list: &[Value], state: State) -> Map<String, Value> {
    let styles = STYLES.read().expect("nothing panics holding it");
    let mut said = Map::new();
    let empty = HashMap::new();
    let table = styles.as_ref().unwrap_or(&empty);
    let picked = list.iter().filter_map(|style| match style {
        Value::String(id) => table.get(id),
        inline => Some(inline),
    });
    for style in picked.filter_map(Value::as_object) {
        for (property, value) in style {
            said.insert(property.clone(), value.clone());
        }
    }
    let mut out = Map::new();
    for (property, value) in said {
        let value = match by_state(&value) {
            Some(by) => {
                let mut picked = by.get("default");
                for (name, holds) in [("hover", state.hover), ("pressed", state.pressed), ("focused", state.focused), ("entering", state.entering)] {
                    if holds && let Some(value) = by.get(name) {
                        picked = Some(value);
                    }
                }
                match picked {
                    Some(value) => value.clone(),
                    None => continue,
                }
            }
            None => value,
        };
        out.insert(property, value);
    }
    out
}

/// What a node's style comes to: how it's laid out, how its box looks, and what it makes of its text.
pub struct Computed {
    pub layout: taffy::Style,
    pub look: Look,
    /// Its own text properties, which change what it inherits (`Text::with`).
    pub(crate) text: TextChange,
    pub line_clamp: Option<usize>,
    /// Placed in the window, over everything, not in its parent (`position: fixed`).
    pub fixed: bool,
    /// What changes of it move rather than jump, and how (`transitionProperty` and `transition`).
    pub transition: Option<Transition>,
}

/// Which properties of a node move to a new value rather than jump, over how long, in milliseconds, with which easing
/// (a cubic Bézier's two points), as CSS's `transition`, or by a spring, which keeps its speed as it's turned.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub properties: Vec<String>,
    pub duration: f32,
    pub easing: [f32; 4],
    pub spring: Option<crate::tokens::Spring>,
    /// How fast a spring sets off, as a fraction of the way a second (`transitionVelocity`), where the style says.
    pub velocity: Option<f32>,
}

#[derive(Default)]
pub(crate) struct TextChange {
    style: Option<TextStyle>,
    weight: Option<u16>,
    mono: Option<bool>,
    pub(crate) color: Option<Hsla>,
    align: Option<f32>,
    wrap: Option<bool>,
    underline: Option<bool>,
}

impl Text {
    /// What a node makes of the text it inherits: its own type style, weight and colour where it says,
    /// as CSS inherits them.
    pub fn with(self, computed: &Computed) -> Text {
        let change = &computed.text;
        let mut out = self;
        if let Some(style) = change.style {
            // A type style says size, weight and spacing; the family is inherited, as CSS's is.
            out.style = TextStyle { mono: out.style.mono, ..style };
        }
        if let Some(weight) = change.weight {
            out.style.weight = weight;
        }
        if let Some(mono) = change.mono {
            out.style.mono = mono;
        }
        if let Some(color) = change.color {
            out.color = color;
        }
        if let Some(align) = change.align {
            out.align = align;
        }
        if let Some(wrap) = change.wrap {
            out.wrap = wrap;
        }
        if let Some(underline) = change.underline {
            out.underline = underline;
        }
        out
    }

    /// The root's, as the DOM's `body` is (ui.css): on the background's colour, in `label` and `type-body`.
    pub fn root(conditions: Conditions) -> (Text, Option<Hsla>) {
        let root = serde_json::json!({"backgroundColor": {"token": "background"}, "color": {"token": "label"}, "font": {"token": "type-body"}});
        let computed = computed(root.as_object().expect("it's an object"), conditions);
        let base = Text { style: TextStyle { size: 16.0, weight: 400, tracking: 0.0, line_height: 24.0, tabular: false, mono: false }, color: Hsla::default(), align: 0.0, wrap: true, underline: false };
        (base.with(&computed), computed.look.background)
    }
}

/// Styles `list` in `state`, where `conditions` hold.
pub fn compute(list: &[Value], state: State, conditions: Conditions) -> Computed {
    computed(&resolve(list, state), conditions)
}

/// A `Ref`'s token and whether it's negative.
fn token_of(value: &Value) -> Option<(&str, bool)> {
    Some((value["token"].as_str()?, value["negative"] == true))
}

/// A length a `Ref` says, or a number, in pixels: `0`, or an object of a node's own style's.
fn ref_length(value: &Value, conditions: Conditions) -> Option<f32> {
    if let Some(px) = value.as_f64() {
        return Some(px as f32);
    }
    let (token, negative) = token_of(value)?;
    let px = table().length(token, conditions)?;
    Some(if negative { -px } else { px })
}

/// A colour as data has it, hex: `#f80`, `#ff8800`, `#f808` or `#ff880080` (what a dynamic style takes).
fn hex(text: &str) -> Option<Rgba> {
    let digits = text.strip_prefix('#')?;
    let channel = |at: usize, width: usize| {
        let part = digits.get(at..at + width)?;
        let value = u8::from_str_radix(part, 16).ok()?;
        Some(if width == 1 { f32::from(value * 17) } else { f32::from(value) } / 255.0)
    };
    match digits.len() {
        3 | 4 => Some(Rgba { r: channel(0, 1)?, g: channel(1, 1)?, b: channel(2, 1)?, a: if digits.len() == 4 { channel(3, 1)? } else { 1.0 } }),
        6 | 8 => Some(Rgba { r: channel(0, 2)?, g: channel(2, 2)?, b: channel(4, 2)?, a: if digits.len() == 8 { channel(6, 2)? } else { 1.0 } }),
        _ => None,
    }
}

fn ref_colour(value: &Value, conditions: Conditions) -> Option<Hsla> {
    if let Some(text) = value.as_str() {
        return hex(text).map(Hsla::from);
    }
    let (token, _) = token_of(value)?;
    table().colour(token, conditions).map(Hsla::from)
}

/// A `Dimension`: a token's length, a percentage of the parent's, `auto`, or `0`.
enum Dim {
    Px(f32),
    Percent(f32),
    Auto,
}

fn dim(value: &Value, conditions: Conditions) -> Option<Dim> {
    match value {
        Value::String(word) if word == "auto" => Some(Dim::Auto),
        Value::String(word) => word.strip_suffix('%')?.parse::<f32>().ok().map(|percent| Dim::Percent(percent / 100.0)),
        value => ref_length(value, conditions).map(Dim::Px),
    }
}

fn to_dimension(value: &Value, conditions: Conditions) -> Option<Dimension> {
    Some(match dim(value, conditions)? {
        Dim::Px(px) => Dimension::length(px),
        Dim::Percent(part) => Dimension::percent(part),
        Dim::Auto => Dimension::auto(),
    })
}

fn to_auto(value: &Value, conditions: Conditions) -> Option<LengthPercentageAuto> {
    Some(match dim(value, conditions)? {
        Dim::Px(px) => LengthPercentageAuto::length(px),
        Dim::Percent(part) => LengthPercentageAuto::percent(part),
        Dim::Auto => LengthPercentageAuto::auto(),
    })
}

fn align_items(word: &str) -> Option<AlignItems> {
    Some(match word {
        "flex-start" => AlignItems::FLEX_START,
        "center" => AlignItems::CENTER,
        "flex-end" => AlignItems::FLEX_END,
        "stretch" => AlignItems::STRETCH,
        "baseline" => AlignItems::BASELINE,
        _ => return None,
    })
}

fn align_content(word: &str) -> Option<AlignContent> {
    Some(match word {
        "flex-start" => AlignContent::FLEX_START,
        "center" => AlignContent::CENTER,
        "flex-end" => AlignContent::FLEX_END,
        "stretch" => AlignContent::STRETCH,
        "space-between" => AlignContent::SPACE_BETWEEN,
        "space-around" => AlignContent::SPACE_AROUND,
        "space-evenly" => AlignContent::SPACE_EVENLY,
        _ => return None,
    })
}

/// A resolved style's properties (`Properties` in core/ui/src/drawing-primitives/renderer.ts) as layout, look and text.
fn computed(style: &Map<String, Value>, conditions: Conditions) -> Computed {
    let get = |name: &str| style.get(name).filter(|value| !value.is_null());
    let word = |name: &str| get(name).and_then(Value::as_str);
    let number = |name: &str| get(name).and_then(Value::as_f64).map(|number| number as f32);
    let length = |name: &str| get(name).and_then(|value| ref_length(value, conditions));

    let mut layout = taffy::Style {
        display: match word("display") {
            Some("none") => Display::None,
            Some("grid") => Display::Grid,
            _ => Display::Flex,
        },
        flex_direction: match word("flexDirection") {
            Some("row") => FlexDirection::Row,
            Some("row-reverse") => FlexDirection::RowReverse,
            Some("column-reverse") => FlexDirection::ColumnReverse,
            _ => FlexDirection::Column,
        },
        flex_wrap: match word("flexWrap") {
            Some("wrap") => FlexWrap::Wrap,
            Some("wrap-reverse") => FlexWrap::WrapReverse,
            _ => FlexWrap::NoWrap,
        },
        scrollbar_width: 0.0,
        ..Default::default()
    };
    if let Some(grow) = number("flexGrow") {
        layout.flex_grow = grow;
    }
    if let Some(shrink) = number("flexShrink") {
        layout.flex_shrink = shrink;
    }
    if let Some(basis) = get("flexBasis").and_then(|value| to_dimension(value, conditions)) {
        layout.flex_basis = basis;
    }
    if let Some(align) = word("alignItems").and_then(align_items) {
        layout.align_items = align;
    }
    if let Some(align) = word("alignSelf").and_then(align_items) {
        layout.align_self = Some(align);
    }
    if let Some(align) = word("alignContent").and_then(align_content) {
        layout.align_content = align;
    }
    if let Some(justify) = word("justifyContent").and_then(align_content) {
        layout.justify_content = justify;
    }
    if let Some(columns) = get("gridTemplateColumns") {
        if let Some(count) = columns["repeat"].as_u64() {
            layout.grid_template_columns = evenly_sized_tracks(count as u16);
        } else if let Some(min) = ref_length(&columns["fill"], conditions) {
            layout.grid_template_columns = vec![GridTemplateComponent::Repeat(taffy::style::GridTemplateRepetition {
                count: RepetitionCount::AutoFill,
                tracks: vec![minmax(MinTrackSizingFunction::length(min), MaxTrackSizingFunction::fr(1.0))],
                line_names: Vec::new(),
            })];
        }
    }
    if let Some(span) = get("gridColumn").and_then(|column| column["span"].as_u64()) {
        layout.grid_column = Line { start: taffy::style_helpers::span(span as u16), end: GridPlacement::Auto };
    }
    layout.gap = Size {
        width: LengthPercentage::length(length("columnGap").unwrap_or(0.0)),
        height: LengthPercentage::length(length("rowGap").unwrap_or(0.0)),
    };
    let side = |name: &str| LengthPercentage::length(length(name).unwrap_or(0.0));
    layout.padding = Rect { left: side("paddingStart"), right: side("paddingEnd"), top: side("paddingTop"), bottom: side("paddingBottom") };
    let margin = |name: &str| get(name).and_then(|value| to_auto(value, conditions)).unwrap_or(LengthPercentageAuto::length(0.0));
    layout.margin = Rect { left: margin("marginStart"), right: margin("marginEnd"), top: margin("marginTop"), bottom: margin("marginBottom") };
    let borders = Edges {
        top: length("borderTopWidth").unwrap_or(0.0),
        right: length("borderRightWidth").or_else(|| length("borderEndWidth")).unwrap_or(0.0),
        bottom: length("borderBottomWidth").unwrap_or(0.0),
        left: length("borderLeftWidth").or_else(|| length("borderStartWidth")).unwrap_or(0.0),
    };
    layout.border = Rect {
        left: LengthPercentage::length(borders.left),
        right: LengthPercentage::length(borders.right),
        top: LengthPercentage::length(borders.top),
        bottom: LengthPercentage::length(borders.bottom),
    };
    if let Some(width) = get("width").and_then(|value| to_dimension(value, conditions)) {
        layout.size.width = width;
    }
    if let Some(height) = get("height").and_then(|value| to_dimension(value, conditions)) {
        layout.size.height = height;
    }
    if let Some(width) = get("minWidth").and_then(|value| to_auto(value, conditions)) {
        layout.min_size.width = width;
    }
    if let Some(height) = get("minHeight").and_then(|value| to_auto(value, conditions)) {
        layout.min_size.height = height;
    }
    if let Some(width) = get("maxWidth").and_then(|value| to_auto(value, conditions)) {
        layout.max_size.width = width;
    }
    if let Some(height) = get("maxHeight").and_then(|value| to_auto(value, conditions)) {
        layout.max_size.height = height;
    }
    layout.aspect_ratio = number("aspectRatio");
    // Fixed is placed in the window, as absolute is in its nearest parent that's relative or absolute (`view` makes
    // the window a fixed one's parent). What says neither is static, as CSS's, Taffy's default.
    let fixed = word("position") == Some("fixed");
    if fixed || word("position") == Some("absolute") {
        layout.position = Position::Absolute;
    } else if word("position") == Some("relative") {
        layout.position = Position::Relative;
    }
    // `left` and `right` are CSS's own, as a page's inline style says them; `start` and `end` the line's, which run
    // left to right here (`direction`).
    let inset = |names: &[&str]| names.iter().find_map(|name| get(name)).and_then(|value| to_auto(value, conditions)).unwrap_or(LengthPercentageAuto::auto());
    layout.inset = Rect { left: inset(&["left", "start"]), right: inset(&["right", "end"]), top: inset(&["top"]), bottom: inset(&["bottom"]) };
    let clips = word("overflow") == Some("hidden");
    if clips {
        layout.overflow = taffy::geometry::Point { x: Overflow::Hidden, y: Overflow::Hidden };
    }

    let radius = |name: &str| {
        let value = get(name)?;
        if let Some(px) = value.as_f64() {
            return Some(Radius::Px(px as f32));
        }
        table().radius(token_of(value)?.0, conditions)
    };
    let offset = |name: &str| match get(name).and_then(|value| dim(value, conditions)) {
        Some(Dim::Px(px)) => Offset::Px(px),
        Some(Dim::Percent(part)) => Offset::Percent(part * 100.0),
        _ => Offset::Px(0.0),
    };
    let paint = |name: &str, otherwise: Paint| match get(name) {
        Some(Value::String(word)) if word == "none" => Paint::None,
        Some(Value::String(word)) if word == "currentColor" => Paint::Current,
        Some(value) => ref_colour(value, conditions).map_or(otherwise, Paint::Colour),
        None => otherwise,
    };
    let look = Look {
        background: get("backgroundColor").and_then(|value| ref_colour(value, conditions)),
        border_color: get("borderColor").and_then(|value| ref_colour(value, conditions)).unwrap_or_default(),
        borders,
        dashed: word("borderStyle") == Some("dashed"),
        radii: Corners {
            top_left: radius("borderTopStartRadius").unwrap_or(Radius::Px(0.0)),
            top_right: radius("borderTopEndRadius").unwrap_or(Radius::Px(0.0)),
            bottom_right: radius("borderBottomEndRadius").unwrap_or(Radius::Px(0.0)),
            bottom_left: radius("borderBottomStartRadius").unwrap_or(Radius::Px(0.0)),
        },
        shadows: get("boxShadow").and_then(token_of).map(|(token, _)| table().shadows(token, conditions)).unwrap_or_default(),
        material: get("backdropFilter").and_then(token_of).and_then(|(token, _)| table().material(token, conditions)),
        opacity: number("opacity").unwrap_or(1.0),
        clips,
        scrolls: false,
        translate: (offset("translateX"), offset("translateY")),
        scale: number("scale").unwrap_or(1.0),
        rotate: number("rotate").unwrap_or(0.0),
        z_index: number("zIndex").unwrap_or(0.0) as i32,
        pointer_events: word("pointerEvents").map(|events| events != "none"),
        app_region: word("appRegion").map(|region| region == "drag"),
        touch_action: match word("touchAction") {
            Some("pan-x") => Some(Pan::X),
            Some("pan-y") => Some(Pan::Y),
            _ => None,
        },
        will_change: get("willChange").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).any(|property| matches!(property, "translateX" | "translateY" | "scale" | "opacity")),
        cursor: match word("cursor") {
            Some("pointer") => Some(Cursor::Pointer),
            Some("text") => Some(Cursor::Text),
            Some(_) => Some(Cursor::Arrow),
            None => None,
        },
        fill: paint("fill", Paint::Current),
        stroke: paint("stroke", Paint::None),
        stroke_width: number("strokeWidth").unwrap_or(1.0),
        animation: match word("animationName") {
            Some(name @ ("spin" | "pulse")) => {
                let duration = get("animation").and_then(token_of).and_then(|(token, _)| table().motion(token, conditions)).map_or(1000.0, |motion| motion.duration);
                Some((name.to_owned(), duration))
            }
            _ => None,
        },
    };

    let type_style = get("font").and_then(token_of).and_then(|(token, _)| table().type_style(token, conditions)).map(|font| TextStyle {
        size: font.size,
        weight: font.weight.round() as u16,
        tracking: font.tracking,
        line_height: font.line_height.round(),
        tabular: font.tabular,
        mono: false,
    });
    let text = TextChange {
        style: type_style,
        weight: get("fontWeight").and_then(token_of).and_then(|(token, _)| table().weight(token, conditions)).map(|weight| weight.round() as u16),
        mono: get("fontFamily").and_then(token_of).and_then(|(token, _)| table().monospace(token, conditions)),
        color: get("color").and_then(|value| ref_colour(value, conditions)),
        align: match word("textAlign") {
            Some("center") => Some(0.5),
            Some("end") => Some(1.0),
            Some("start") => Some(0.0),
            _ => None,
        },
        wrap: word("whiteSpace").map(|white_space| white_space != "nowrap"),
        underline: word("textDecorationLine").map(|line| line == "underline"),
    };
    let properties: Vec<String> = get("transitionProperty").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let motion = get("transition").and_then(token_of).and_then(|(token, _)| table().motion(token, conditions));
    let transition = match motion {
        Some(motion) if !properties.is_empty() && motion.duration > 0.0 => Some(Transition { properties, duration: motion.duration, easing: motion.easing, spring: motion.spring, velocity: number("transitionVelocity") }),
        _ => None,
    };
    Computed { layout, look, text, line_clamp: number("lineClamp").map(|lines| lines as usize), fixed, transition }
}

/// How node `node` is styled in `state`: its styles by id, merged.
pub fn of(node: &Node, state: State, conditions: Conditions) -> Computed {
    compute(node.styles.as_deref().unwrap_or_default(), state, conditions)
}
