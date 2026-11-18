//! How a node looks and is laid out, from what its page says, resolved against the tokens (`tokens`): its layout as
//! Taffy's style, its box (`Look`) and its text (`Text`), together `Computed`. A page written for version 2 names its
//! styles by id (`Styles` in core/ui/src/strict/renderer.ts), each a style table's, merged in order, each property's
//! value by the node's state (`State`); one written for 1 sends one style object (`tree::Style`). The one place that
//! knows how styles are written.

use taffy::prelude::*;
use taffy::style::{GridTemplateComponent, Overflow, RepetitionCount};

use std::collections::HashMap;
use std::sync::RwLock;

use serde_json::{Map, Value};

use crate::Cursor;
use crate::geometry::{Corners, Edges, Hsla};
use crate::text::TextStyle;
use crate::tokens::{Conditions, Radius, Shadow, table, tokens, type_style};
use crate::tree::{Length, Node, Style};

/// A colour token, in light or dark.
pub fn colour(name: &str, dark: bool) -> Hsla {
    tokens().colour(name, dark).map(Hsla::from).unwrap_or_default()
}

/// A space or a size in pixels: a number, or a token (`gap`, `gutter`, `control`, `icon`, `readable`).
fn pixels(length: &Length, touch: bool) -> Option<f32> {
    match length {
        Length::Px(px) => Some(*px),
        Length::Word(word) => tokens().length(word, touch),
    }
}

fn percent_of(word: &str) -> Option<f32> {
    word.strip_suffix('%')?.parse::<f32>().ok().map(|percent| percent / 100.0)
}

fn dimension(length: &Length, touch: bool) -> Dimension {
    match length {
        Length::Word(word) if word == "auto" => Dimension::auto(),
        Length::Word(word) if word.ends_with('%') => Dimension::percent(percent_of(word).unwrap_or(0.0)),
        length => Dimension::length(pixels(length, touch).unwrap_or(0.0)),
    }
}

fn auto_or(length: &Length, touch: bool) -> LengthPercentageAuto {
    match length {
        Length::Word(word) if word == "auto" => LengthPercentageAuto::auto(),
        Length::Word(word) if word.ends_with('%') => LengthPercentageAuto::percent(percent_of(word).unwrap_or(0.0)),
        length => LengthPercentageAuto::length(pixels(length, touch).unwrap_or(0.0)),
    }
}

fn definite(length: &Length, touch: bool) -> LengthPercentage {
    match length {
        Length::Word(word) if word.ends_with('%') => LengthPercentage::percent(percent_of(word).unwrap_or(0.0)),
        length => LengthPercentage::length(pixels(length, touch).unwrap_or(0.0)),
    }
}

/// Which sides of the box have a 1-pixel border.
pub fn borders(style: &Style) -> Edges<f32> {
    let all = style.border_width == Some(1);
    let side = |width: Option<u8>| if all || width == Some(1) { 1.0 } else { 0.0 };
    Edges { top: side(style.border_top_width), right: side(style.border_right_width), bottom: side(style.border_bottom_width), left: side(style.border_left_width) }
}

/// How a node is laid out: flex, by default a column, as React Native's views are, and as GPUI's `div` with
/// `flex_col` was.
pub fn layout(style: &Style, touch: bool) -> taffy::Style {
    let mut out = taffy::Style {
        display: match style.display.as_deref() {
            Some("none") => Display::None,
            Some("grid") => Display::Grid,
            _ => Display::Flex,
        },
        flex_direction: match style.flex_direction.as_deref() {
            Some("row") => FlexDirection::Row,
            Some("row-reverse") => FlexDirection::RowReverse,
            Some("column-reverse") => FlexDirection::ColumnReverse,
            _ => FlexDirection::Column,
        },
        ..Default::default()
    };
    if style.flex_wrap.as_deref() == Some("wrap") {
        out.flex_wrap = FlexWrap::Wrap;
    }
    if let Some(grow) = style.flex_grow {
        out.flex_grow = grow;
    }
    if let Some(shrink) = style.flex_shrink {
        out.flex_shrink = shrink;
    }
    if let Some(basis) = &style.flex_basis {
        out.flex_basis = dimension(basis, touch);
    }
    if let Some(align) = match style.align_items.as_deref() {
        Some("flex-start") => Some(AlignItems::FLEX_START),
        Some("center") => Some(AlignItems::CENTER),
        Some("flex-end") => Some(AlignItems::FLEX_END),
        Some("baseline") => Some(AlignItems::BASELINE),
        Some("stretch") => Some(AlignItems::STRETCH),
        _ => None,
    } {
        out.align_items = align;
    }
    out.align_self = match style.align_self.as_deref() {
        Some("flex-start") => Some(AlignSelf::FLEX_START),
        Some("center") => Some(AlignSelf::CENTER),
        Some("flex-end") => Some(AlignSelf::FLEX_END),
        Some("stretch") => Some(AlignSelf::STRETCH),
        _ => None,
    };
    if let Some(justify) = match style.justify_content.as_deref() {
        Some("center") => Some(JustifyContent::CENTER),
        Some("flex-end") => Some(JustifyContent::FLEX_END),
        Some("space-between") => Some(JustifyContent::SPACE_BETWEEN),
        Some("space-around") => Some(JustifyContent::SPACE_AROUND),
        Some("space-evenly") => Some(JustifyContent::SPACE_EVENLY),
        Some("flex-start") => Some(JustifyContent::FLEX_START),
        _ => None,
    } {
        out.justify_content = justify;
    }
    // `repeat(n, 1fr)`, or `repeat(auto-fill, minmax(<px>px, 1fr))`: as many columns as fit, as CSS has it.
    if let Some(columns) = style.grid_template_columns.as_deref() {
        let inner = columns.strip_prefix("repeat(").and_then(|rest| rest.strip_suffix(')')).unwrap_or_default();
        if let Some((count, track)) = inner.split_once(',') {
            if let Ok(count) = count.trim().parse::<u16>() {
                out.grid_template_columns = evenly_sized_tracks(count);
            } else if count.trim() == "auto-fill" {
                let min = track.trim().strip_prefix("minmax(").and_then(|rest| rest.split_once("px")).and_then(|(min, _)| min.trim().parse::<f32>().ok());
                if let Some(min) = min {
                    out.grid_template_columns =
                        vec![GridTemplateComponent::Repeat(taffy::style::GridTemplateRepetition {
                            count: RepetitionCount::AutoFill,
                            tracks: vec![minmax(MinTrackSizingFunction::length(min), MaxTrackSizingFunction::fr(1.0))],
                            line_names: Vec::new(),
                        })];
                }
            }
        }
    }
    if let Some(span) = style.grid_column.as_deref().and_then(|span| span.strip_prefix("span ")?.parse::<u16>().ok()) {
        out.grid_column = Line { start: taffy::style_helpers::span(span), end: GridPlacement::Auto };
    }
    let gap = style.gap.as_ref().and_then(|length| pixels(length, touch));
    let row_gap = style.row_gap.as_ref().and_then(|length| pixels(length, touch)).or(gap);
    let column_gap = style.column_gap.as_ref().and_then(|length| pixels(length, touch)).or(gap);
    out.gap = Size { width: LengthPercentage::length(column_gap.unwrap_or(0.0)), height: LengthPercentage::length(row_gap.unwrap_or(0.0)) };

    let mut padding = Rect::<Option<&Length>> { left: None, right: None, top: None, bottom: None };
    for (length, sides) in [
        (&style.padding, [true, true, true, true]),
        (&style.padding_inline, [true, true, false, false]),
        (&style.padding_block, [false, false, true, true]),
        (&style.padding_left, [true, false, false, false]),
        (&style.padding_right, [false, true, false, false]),
        (&style.padding_top, [false, false, true, false]),
        (&style.padding_bottom, [false, false, false, true]),
    ] {
        if let Some(length) = length {
            for (side, on) in [&mut padding.left, &mut padding.right, &mut padding.top, &mut padding.bottom].into_iter().zip(sides) {
                if on {
                    *side = Some(length);
                }
            }
        }
    }
    out.padding = Rect {
        left: padding.left.map_or(LengthPercentage::length(0.0), |length| definite(length, touch)),
        right: padding.right.map_or(LengthPercentage::length(0.0), |length| definite(length, touch)),
        top: padding.top.map_or(LengthPercentage::length(0.0), |length| definite(length, touch)),
        bottom: padding.bottom.map_or(LengthPercentage::length(0.0), |length| definite(length, touch)),
    };
    let (inline, block) = (style.margin_inline.as_ref().or(style.margin.as_ref()), style.margin_block.as_ref().or(style.margin.as_ref()));
    let zero = LengthPercentageAuto::length(0.0);
    out.margin = Rect {
        left: inline.map_or(zero, |length| auto_or(length, touch)),
        right: inline.map_or(zero, |length| auto_or(length, touch)),
        top: block.map_or(zero, |length| auto_or(length, touch)),
        bottom: block.map_or(zero, |length| auto_or(length, touch)),
    };
    if let Some(width) = &style.width {
        out.size.width = dimension(width, touch);
    }
    if let Some(height) = &style.height {
        out.size.height = dimension(height, touch);
    }
    if let Some(width) = &style.min_width {
        out.min_size.width = auto_or(width, touch);
    }
    if let Some(height) = &style.min_height {
        out.min_size.height = auto_or(height, touch);
    }
    if let Some(width) = &style.max_width {
        out.max_size.width = auto_or(width, touch);
    }
    if let Some(height) = &style.max_height {
        out.max_size.height = auto_or(height, touch);
    }
    out.aspect_ratio = style.aspect_ratio;
    out.overflow = match style.overflow.as_deref() {
        Some("hidden") => taffy::geometry::Point { x: Overflow::Hidden, y: Overflow::Hidden },
        Some("scroll") | Some("auto") => taffy::geometry::Point { x: Overflow::Hidden, y: Overflow::Scroll },
        _ => taffy::geometry::Point { x: Overflow::Visible, y: Overflow::Visible },
    };
    out.scrollbar_width = 0.0;
    if matches!(style.position.as_deref(), Some("absolute")) {
        out.position = Position::Absolute;
    } else if matches!(style.position.as_deref(), Some("relative")) {
        out.position = Position::Relative;
    }
    let inset = |side: Option<f32>| side.map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length);
    out.inset = Rect { left: inset(style.left), right: inset(style.right), top: inset(style.top), bottom: inset(style.bottom) };
    let border = borders(style);
    out.border = Rect {
        left: LengthPercentage::length(border.left),
        right: LengthPercentage::length(border.right),
        top: LengthPercentage::length(border.top),
        bottom: LengthPercentage::length(border.bottom),
    };
    out
}

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
            cursor: None,
            fill: Paint::Current,
            stroke: Paint::None,
            stroke_width: 1.0,
            animation: None,
        }
    }
}

pub fn look(style: &Style, conditions: Conditions) -> Look {
    let dark = conditions.dark;
    let radius = match &style.border_radius {
        Some(Length::Word(full)) if full == "full" => 9999.0,
        Some(Length::Word(token)) => tokens().length(&format!("radius-{token}"), conditions.touch).unwrap_or(0.0),
        Some(Length::Px(px)) => *px,
        None => 0.0,
    };
    Look {
        background: style.background_color.as_deref().map(|name| colour(name, dark)),
        border_color: colour(style.border_color.as_deref().unwrap_or("border"), dark),
        borders: borders(style),
        radii: Corners::all(Radius::Px(radius)),
        opacity: style.opacity.unwrap_or(1.0),
        clips: matches!(style.overflow.as_deref(), Some("hidden") | Some("scroll") | Some("auto")),
        scrolls: matches!(style.overflow.as_deref(), Some("scroll") | Some("auto")),
        translate: (Offset::Px(style.translate_x.unwrap_or(0.0)), Offset::Px(style.translate_y.unwrap_or(0.0))),
        scale: style.scale.unwrap_or(1.0),
        rotate: style.rotate.unwrap_or(0.0),
        z_index: style.z_index.unwrap_or(0.0) as i32,
        pointer_events: style.pointer_events.as_deref().map(|events| events != "none"),
        ..Look::default()
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
    /// The body's, as a page's `body` is (ui.css).
    pub fn body(conditions: Conditions) -> Text {
        Text { style: text_style("body", false, conditions), color: colour("foreground", conditions.dark), align: 0.0, wrap: true, underline: false }
    }

    /// A heading's, as the DOM draws a `Text` that's one, an `<h1>` to `<h3>`: a title's weight and tracking (ui.css),
    /// where its type style doesn't say its own; a type style's weight always does (`[data-font]`).
    pub fn heading(self, typed: bool, conditions: Conditions) -> Text {
        let title = text_style("title", false, conditions);
        let mut out = self;
        if !typed {
            out.style.weight = title.weight;
        }
        if out.style.tracking == 0.0 {
            out.style.tracking = title.tracking;
        }
        out
    }

    /// What `style` makes of the text it's given: its own type style and colour, or what it inherits. A `text`
    /// with no type style of its own is body text, as the UI library's are.
    pub fn of(self, style: &Style, text: bool, conditions: Conditions) -> Text {
        let dark = conditions.dark;
        let mut out = self;
        if let Some(color) = &style.color {
            out.color = colour(color, dark);
        }
        if style.font.is_some() || text || style.font_variant_numeric.is_some() {
            out.style = text_style(style.font.as_deref().unwrap_or("body"), style.font_variant_numeric.as_deref() == Some("tabular-nums"), conditions);
        }
        match style.text_align.as_deref() {
            Some("center") => out.align = 0.5,
            Some("end") | Some("right") => out.align = 1.0,
            Some("start") | Some("left") => out.align = 0.0,
            _ => {}
        }
        out
    }
}

fn text_style(font: &str, tabular: bool, conditions: Conditions) -> TextStyle {
    let font = type_style(font, conditions);
    TextStyle {
        size: font.size,
        weight: font.weight.round() as u16,
        tracking: font.tracking,
        line_height: (font.size * font.line_height).round(),
        tabular: font.tabular || tabular,
    }
}

// Version 2.

/// A node's state, which a style's value may depend on, as `State` in core/ui/src/strict/renderer.ts has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub hover: bool,
    pub pressed: bool,
    pub focused: bool,
    pub entering: bool,
}

/// Every style table's styles, by id: the UI library's and each app's, kept in one, as their ids are their contents'.
static STYLES: RwLock<Option<HashMap<String, Value>>> = RwLock::new(None);
static ROOT: RwLock<Option<String>> = RwLock::new(None);

/// Takes a style table (`Styles`): the UI library's (`/ui/client/styles.json`), or an app's (`styles.json` beside
/// its page). → the runtime version it needs.
pub fn load_styles(json: &str) -> anyhow::Result<u32> {
    let parsed: Value = serde_json::from_str(json)?;
    let mut styles = STYLES.write().expect("nothing panics holding it");
    let styles = styles.get_or_insert_with(HashMap::new);
    for (id, style) in parsed["styles"].as_object().into_iter().flatten() {
        styles.insert(id.clone(), style.clone());
    }
    if let Some(root) = parsed["root"].as_str() {
        *ROOT.write().expect("nothing panics holding it") = Some(root.to_owned());
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
}

#[derive(Default)]
pub(crate) struct TextChange {
    style: Option<TextStyle>,
    weight: Option<u16>,
    pub(crate) color: Option<Hsla>,
    align: Option<f32>,
    wrap: Option<bool>,
    underline: Option<bool>,
}

impl Text {
    /// What a version 2 node makes of the text it inherits: its own type style, weight and colour where it says,
    /// as CSS inherits them.
    pub fn with(self, computed: &Computed) -> Text {
        let change = &computed.text;
        let mut out = self;
        if let Some(style) = change.style {
            out.style = style;
        }
        if let Some(weight) = change.weight {
            out.style.weight = weight;
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

    /// The root's, as its page's root style says (`Styles.root`), or as the DOM's `body` is (ui.css): the
    /// background's colour, `label` and `type-body`.
    pub fn root(conditions: Conditions) -> (Text, Option<Hsla>) {
        let root = ROOT.read().expect("nothing panics holding it").clone();
        let computed = match root {
            Some(root) => compute(&[Value::String(root)], State::default(), conditions),
            None => {
                let mut style = Map::new();
                style.insert("backgroundColor".into(), serde_json::json!({"token": "background"}));
                style.insert("color".into(), serde_json::json!({"token": "label"}));
                style.insert("font".into(), serde_json::json!({"token": "type-body"}));
                computed(&style, conditions)
            }
        };
        let base = Text { style: TextStyle { size: 16.0, weight: 400, tracking: 0.0, line_height: 24.0, tabular: false }, color: Hsla::default(), align: 0.0, wrap: true, underline: false };
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

fn ref_colour(value: &Value, conditions: Conditions) -> Option<Hsla> {
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

/// A resolved style's properties (`Properties` in core/ui/src/strict/renderer.ts) as layout, look and text.
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
    });
    let text = TextChange {
        style: type_style,
        weight: get("fontWeight").and_then(token_of).and_then(|(token, _)| table().weight(token, conditions)).map(|weight| weight.round() as u16),
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
        Some(motion) if !properties.is_empty() && motion.duration > 0.0 => Some(Transition { properties, duration: motion.duration, easing: motion.easing, spring: motion.spring }),
        _ => None,
    };
    Computed { layout, look, text, line_clamp: number("lineClamp").map(|lines| lines as usize), fixed, transition }
}

/// How node `node` is styled in `state`: its version 2 styles by id, or its version 1 style object.
pub fn of(node: &Node, state: State, conditions: Conditions) -> Computed {
    match &node.styles {
        Some(ids) => compute(ids, state, conditions),
        None => Computed {
            layout: layout(&node.style, conditions.touch),
            look: look(&node.style, conditions),
            text: TextChange::default(),
            line_clamp: node.style.line_clamp,
            fixed: false,
            transition: None,
        },
    }
}
