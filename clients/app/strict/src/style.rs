//! How a node looks and is laid out, from what its page says (version 1's `style`), resolved against the tokens
//! (`tokens`): the one place that knows how styles are written, which the strict protocol's style table replaces
//! here alone.

use gpui::prelude::*;
use gpui::{AbsoluteLength, DefiniteLength, FontFeatures, FontWeight, Hsla, Length as GpuiLength, TextAlign, px, relative};
use std::sync::Arc;

use crate::tokens::{tokens, type_style};
use crate::tree::{Length, Style};
use crate::view::FONT;

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

fn size(length: &Length) -> GpuiLength {
    match length {
        Length::Word(word) if word == "auto" => GpuiLength::Auto,
        Length::Word(word) if word.ends_with('%') => {
            relative(word.trim_end_matches('%').parse::<f32>().unwrap_or(0.0) / 100.0).into()
        }
        length => px(pixels(length).unwrap_or(0.0)).into(),
    }
}

fn definite(length: &Length) -> DefiniteLength {
    match length {
        Length::Word(word) if word.ends_with('%') => relative(word.trim_end_matches('%').parse::<f32>().unwrap_or(0.0) / 100.0),
        length => px(pixels(length).unwrap_or(0.0)).into(),
    }
}

/// How a node is laid out and its box looks: flex, by default a column, as React Native's views are.
pub(crate) fn layout<E: Styled>(mut element: E, style: &Style) -> E {
    element = match style.display.as_deref() {
        Some("grid") => element.grid(),
        _ => element.flex(),
    };
    element = match style.flex_direction.as_deref() {
        Some("row") => element.flex_row(),
        Some("row-reverse") => element.flex_row_reverse(),
        Some("column-reverse") => element.flex_col_reverse(),
        _ => element.flex_col(),
    };
    if style.flex_wrap.as_deref() == Some("wrap") {
        element = element.flex_wrap();
    }
    if let Some(grow) = style.flex_grow {
        element.style().flex_grow = Some(grow);
    }
    if let Some(shrink) = style.flex_shrink {
        element.style().flex_shrink = Some(shrink);
    }
    if let Some(basis) = &style.flex_basis {
        element = element.flex_basis(size(basis));
    }
    element = match style.align_items.as_deref() {
        Some("flex-start") => element.items_start(),
        Some("center") => element.items_center(),
        Some("flex-end") => element.items_end(),
        Some("baseline") => element.items_baseline(),
        _ => element,
    };
    element = match style.align_self.as_deref() {
        Some("flex-start") => element.self_start(),
        Some("center") => element.self_center(),
        Some("flex-end") => element.self_end(),
        Some("stretch") => element.self_stretch(),
        _ => element,
    };
    element = match style.justify_content.as_deref() {
        Some("center") => element.justify_center(),
        Some("flex-end") => element.justify_end(),
        Some("space-between") => element.justify_between(),
        Some("space-around") => element.justify_around(),
        Some("space-evenly") => element.justify_evenly(),
        Some("flex-start") => element.justify_start(),
        _ => element,
    };
    // `repeat(n, 1fr)`; `repeat(auto-fill, …)` is a column until step 4 measures its room.
    if let Some(columns) = style.grid_template_columns.as_deref().and_then(|columns| {
        columns.strip_prefix("repeat(")?.split(',').next()?.trim().parse::<u16>().ok()
    }) {
        element = element.grid_cols(columns);
    }
    if let Some(span) = style.grid_column.as_deref().and_then(|span| span.strip_prefix("span ")?.parse::<u16>().ok()) {
        element = element.col_span(span);
    }
    if let Some(gap) = style.gap.as_ref().and_then(pixels) {
        element = element.gap(px(gap));
    }
    if let Some(gap) = style.row_gap.as_ref().and_then(pixels) {
        element = element.gap_y(px(gap));
    }
    if let Some(gap) = style.column_gap.as_ref().and_then(pixels) {
        element = element.gap_x(px(gap));
    }
    for (length, set) in [
        (&style.padding, &(|e: E, l| e.p(l)) as &dyn Fn(E, DefiniteLength) -> E),
        (&style.padding_inline, &|e: E, l| e.px(l)),
        (&style.padding_block, &|e: E, l| e.py(l)),
        (&style.padding_top, &|e: E, l| e.pt(l)),
        (&style.padding_right, &|e: E, l| e.pr(l)),
        (&style.padding_bottom, &|e: E, l| e.pb(l)),
        (&style.padding_left, &|e: E, l| e.pl(l)),
    ] {
        if let Some(length) = length {
            element = set(element, definite(length));
        }
    }
    for (length, set) in [
        (&style.margin, &(|e: E, l| e.m(l)) as &dyn Fn(E, GpuiLength) -> E),
        (&style.margin_inline, &|e: E, l| e.mx(l)),
        (&style.margin_block, &|e: E, l| e.my(l)),
    ] {
        if let Some(length) = length {
            element = set(element, size(length));
        }
    }
    for (length, set) in [
        (&style.width, &(|e: E, l| e.w(l)) as &dyn Fn(E, GpuiLength) -> E),
        (&style.height, &|e: E, l| e.h(l)),
        (&style.min_width, &|e: E, l| e.min_w(l)),
        (&style.min_height, &|e: E, l| e.min_h(l)),
        (&style.max_width, &|e: E, l| e.max_w(l)),
        (&style.max_height, &|e: E, l| e.max_h(l)),
    ] {
        if let Some(length) = length {
            element = set(element, size(length));
        }
    }
    if let Some(ratio) = style.aspect_ratio {
        element = element.aspect_ratio(ratio);
    }
    if matches!(style.overflow.as_deref(), Some("hidden") | Some("scroll")) {
        element = element.overflow_hidden();
    }
    if let Some(opacity) = style.opacity {
        element = element.opacity(opacity);
    }
    element
}

/// A node's colours, border, corners and text: its type style, if it has one or is text, and what it inherits
/// otherwise.
pub(crate) fn text_style<E: Styled>(mut element: E, style: &Style, dark: bool, text: bool) -> E {
    if let Some(background) = &style.background_color {
        element = element.bg(colour(background, dark));
    }
    if let Some(foreground) = &style.color {
        element = element.text_color(colour(foreground, dark));
    }
    let any_border = [style.border_width, style.border_top_width, style.border_right_width, style.border_bottom_width, style.border_left_width]
        .iter()
        .any(|width| *width == Some(1));
    if any_border {
        element = element.border_color(colour(style.border_color.as_deref().unwrap_or("border"), dark));
        let all = style.border_width == Some(1);
        if all || style.border_top_width == Some(1) {
            element = element.border_t_1();
        }
        if all || style.border_right_width == Some(1) {
            element = element.border_r_1();
        }
        if all || style.border_bottom_width == Some(1) {
            element = element.border_b_1();
        }
        if all || style.border_left_width == Some(1) {
            element = element.border_l_1();
        }
    }
    if let Some(radius) = &style.border_radius {
        let radius = match radius {
            Length::Word(full) if full == "full" => 9999.0,
            Length::Word(token) => tokens().length(&format!("radius-{token}")).unwrap_or(0.0),
            Length::Px(px) => *px,
        };
        element = element.rounded(px(radius));
    }
    if style.font.is_some() || text || style.font_variant_numeric.is_some() {
        let font = type_style(style.font.as_deref().unwrap_or("body"), dark);
        element = element
            .font_family(FONT)
            .text_size(AbsoluteLength::Pixels(px(font.size)))
            .font_weight(FontWeight(font.weight))
            .line_height(relative(font.line_height));
        if font.tabular || style.font_variant_numeric.as_deref() == Some("tabular-nums") {
            element = element.font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
        }
    }
    element = match style.text_align.as_deref() {
        Some("center") => element.text_align(TextAlign::Center),
        Some("end") => element.text_align(TextAlign::Right),
        _ => element,
    };
    if let Some(lines) = style.line_clamp {
        element = element.line_clamp(lines).text_ellipsis();
    }
    element
}
