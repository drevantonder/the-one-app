//! How a node looks and is laid out, from what its page says (version 1's `style`), resolved against the tokens
//! (`tokens`): its layout as Taffy's style, its box (`Look`) and its text (`Text`). The one place that knows how
//! styles are written, which the strict protocol's style table replaces here alone.

use taffy::prelude::*;
use taffy::style::{GridTemplateComponent, Overflow, RepetitionCount};

use crate::geometry::{Edges, Hsla};
use crate::text::TextStyle;
use crate::tokens::{tokens, type_style};
use crate::tree::{Length, Style};

/// A colour token, in light or dark.
pub fn colour(name: &str, dark: bool) -> Hsla {
    tokens().colour(name, dark).map(Hsla::from).unwrap_or_default()
}

/// A space or a size in pixels: a number, or a token (`gap`, `gutter`, `control`, `icon`, `readable`).
fn pixels(length: &Length) -> Option<f32> {
    match length {
        Length::Px(px) => Some(*px),
        Length::Word(word) => tokens().length(word),
    }
}

fn percent_of(word: &str) -> Option<f32> {
    word.strip_suffix('%')?.parse::<f32>().ok().map(|percent| percent / 100.0)
}

fn dimension(length: &Length) -> Dimension {
    match length {
        Length::Word(word) if word == "auto" => Dimension::auto(),
        Length::Word(word) if word.ends_with('%') => Dimension::percent(percent_of(word).unwrap_or(0.0)),
        length => Dimension::length(pixels(length).unwrap_or(0.0)),
    }
}

fn auto_or(length: &Length) -> LengthPercentageAuto {
    match length {
        Length::Word(word) if word == "auto" => LengthPercentageAuto::auto(),
        Length::Word(word) if word.ends_with('%') => LengthPercentageAuto::percent(percent_of(word).unwrap_or(0.0)),
        length => LengthPercentageAuto::length(pixels(length).unwrap_or(0.0)),
    }
}

fn definite(length: &Length) -> LengthPercentage {
    match length {
        Length::Word(word) if word.ends_with('%') => LengthPercentage::percent(percent_of(word).unwrap_or(0.0)),
        length => LengthPercentage::length(pixels(length).unwrap_or(0.0)),
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
pub fn layout(style: &Style) -> taffy::Style {
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
        out.flex_basis = dimension(basis);
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
    let gap = style.gap.as_ref().and_then(pixels);
    let row_gap = style.row_gap.as_ref().and_then(pixels).or(gap);
    let column_gap = style.column_gap.as_ref().and_then(pixels).or(gap);
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
        left: padding.left.map_or(LengthPercentage::length(0.0), definite),
        right: padding.right.map_or(LengthPercentage::length(0.0), definite),
        top: padding.top.map_or(LengthPercentage::length(0.0), definite),
        bottom: padding.bottom.map_or(LengthPercentage::length(0.0), definite),
    };
    let (inline, block) = (style.margin_inline.as_ref().or(style.margin.as_ref()), style.margin_block.as_ref().or(style.margin.as_ref()));
    let zero = LengthPercentageAuto::length(0.0);
    out.margin = Rect {
        left: inline.map_or(zero, auto_or),
        right: inline.map_or(zero, auto_or),
        top: block.map_or(zero, auto_or),
        bottom: block.map_or(zero, auto_or),
    };
    if let Some(width) = &style.width {
        out.size.width = dimension(width);
    }
    if let Some(height) = &style.height {
        out.size.height = dimension(height);
    }
    if let Some(width) = &style.min_width {
        out.min_size.width = auto_or(width);
    }
    if let Some(height) = &style.min_height {
        out.min_size.height = auto_or(height);
    }
    if let Some(width) = &style.max_width {
        out.max_size.width = auto_or(width);
    }
    if let Some(height) = &style.max_height {
        out.max_size.height = auto_or(height);
    }
    out.aspect_ratio = style.aspect_ratio;
    out.overflow = match style.overflow.as_deref() {
        Some("hidden") => taffy::geometry::Point { x: Overflow::Hidden, y: Overflow::Hidden },
        Some("scroll") | Some("auto") => taffy::geometry::Point { x: Overflow::Hidden, y: Overflow::Scroll },
        _ => taffy::geometry::Point { x: Overflow::Visible, y: Overflow::Visible },
    };
    out.scrollbar_width = 0.0;
    let border = borders(style);
    out.border = Rect {
        left: LengthPercentage::length(border.left),
        right: LengthPercentage::length(border.right),
        top: LengthPercentage::length(border.top),
        bottom: LengthPercentage::length(border.bottom),
    };
    out
}

/// A node's box: its fill, border and corners, how opaque it is, and whether it clips or scrolls what's in it.
pub struct Look {
    pub background: Option<Hsla>,
    pub border_color: Hsla,
    pub borders: Edges<f32>,
    pub radius: f32,
    pub opacity: f32,
    pub clips: bool,
    pub scrolls: bool,
}

pub fn look(style: &Style, dark: bool) -> Look {
    let radius = match &style.border_radius {
        Some(Length::Word(full)) if full == "full" => 9999.0,
        Some(Length::Word(token)) => tokens().length(&format!("radius-{token}")).unwrap_or(0.0),
        Some(Length::Px(px)) => *px,
        None => 0.0,
    };
    Look {
        background: style.background_color.as_deref().map(|name| colour(name, dark)),
        border_color: colour(style.border_color.as_deref().unwrap_or("border"), dark),
        borders: borders(style),
        radius,
        opacity: style.opacity.unwrap_or(1.0),
        clips: matches!(style.overflow.as_deref(), Some("hidden") | Some("scroll") | Some("auto")),
        scrolls: matches!(style.overflow.as_deref(), Some("scroll") | Some("auto")),
    }
}

/// How text is set where it is: what a node's children inherit, as CSS's text properties are inherited.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub style: TextStyle,
    pub color: Hsla,
    /// Where a line goes along its box: 0 start, 0.5 centre, 1 end.
    pub align: f32,
}

impl Text {
    /// The body's, as a page's `body` is (ui.css).
    pub fn body(dark: bool) -> Text {
        Text { style: text_style("body", false, dark), color: colour("foreground", dark), align: 0.0 }
    }

    /// What `style` makes of the text it's given: its own type style and colour, or what it inherits. A `text`
    /// with no type style of its own is body text, as the UI library's are.
    pub fn of(self, style: &Style, text: bool, dark: bool) -> Text {
        let mut out = self;
        if let Some(color) = &style.color {
            out.color = colour(color, dark);
        }
        if style.font.is_some() || text || style.font_variant_numeric.is_some() {
            out.style = text_style(style.font.as_deref().unwrap_or("body"), style.font_variant_numeric.as_deref() == Some("tabular-nums"), dark);
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

fn text_style(font: &str, tabular: bool, dark: bool) -> TextStyle {
    let font = type_style(font, dark);
    TextStyle {
        size: font.size,
        weight: font.weight.round() as u16,
        tracking: font.tracking,
        line_height: (font.size * font.line_height).round(),
        tabular: font.tabular || tabular,
    }
}
