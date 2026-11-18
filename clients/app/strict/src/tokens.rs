//! The design tokens, which aren't the host's: they come with the UI library, over the air, as the app's page does,
//! and are read as it opens (`load`). The host knows only kinds of token (a colour, light and dark, a length, a
//! number), never their names or values. Until the strict protocol defines the token table (core/ui/src/strict/),
//! it's read from the UI library's stylesheet (`/ui/client/ui.css`): `:root`'s first rule, which is a computer's (the
//! touch and iPhone rules after it are a phone's), and `:root.dark`'s `--lighter`. This module is the only one that
//! knows that, so the table replaces it here alone.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::geometry::Rgba;

/// What CSS has as 1rem, in pixels.
const REM: f32 = 16.0;

/// The tokens, light and dark.
pub struct Tokens {
    declared: HashMap<String, String>,
    lighter: f32,
}

static LOADED: OnceLock<Tokens> = OnceLock::new();

/// Takes the tokens the UI library came with, as the window opens, before anything is drawn.
pub fn load(css: &str) {
    let _ = LOADED.set(Tokens::parse(css));
}

/// The tokens loaded, or none (`load`), where every lookup is `None`.
pub fn tokens() -> &'static Tokens {
    LOADED.get_or_init(|| Tokens::parse(""))
}

impl Tokens {
    fn parse(css: &str) -> Tokens {
        // A rule's declarations, by its selector exactly, as written or minified (`:root {` or `:root{`).
        let rule = |selector: &str| -> HashMap<String, String> {
            let Some(start) = [format!("{selector} {{"), format!("{selector}{{")].iter().filter_map(|open| {
                css.match_indices(open.as_str()).find(|(at, _)| *at == 0 || matches!(css.as_bytes()[at - 1], b'\n' | b'}'))
            }).map(|(at, _)| at).min() else { return HashMap::new() };
            let body = &css[start..];
            let body = &body[body.find('{').unwrap_or(0) + 1..body.find('}').unwrap_or(body.len())];
            uncommented(body)
                .split(';')
                .filter_map(|declaration| {
                    let (name, value) = declaration.trim().split_once(':')?;
                    Some((name.trim().strip_prefix("--")?.to_owned(), value.split_whitespace().collect::<Vec<_>>().join(" ")))
                })
                .collect()
        };
        let declared = rule(":root");
        let lighter = rule(":root.dark").get("lighter").and_then(|value| value.parse().ok()).unwrap_or(0.0);
        Tokens { declared, lighter }
    }

    /// A colour token (`muted-foreground`), as it is in light or dark.
    pub fn colour(&self, name: &str, dark: bool) -> Option<Rgba> {
        let value = self.declared.get(name)?;
        let value = match value.strip_prefix("light-dark(").and_then(|value| value.strip_suffix(')')) {
            Some(both) => {
                let split = split_top_level(both);
                if dark { split.1 } else { split.0 }
            }
            None => value.as_str(),
        };
        oklch(value.trim())
    }

    /// A length token (`gutter`, `radius-sm`, `text-base`), in pixels.
    pub fn length(&self, name: &str) -> Option<f32> {
        self.evaluate(self.declared.get(name)?, false)
    }

    /// A number token (`body-weight`), as it is in light or dark: a weight is lighter in dark.
    pub fn number(&self, name: &str, dark: bool) -> Option<f32> {
        self.evaluate(self.declared.get(name)?, dark)
    }

    /// A tracking token (`title-tracking`), in em.
    pub fn em(&self, name: &str) -> Option<f32> {
        self.declared.get(name)?.strip_suffix("em")?.parse().ok()
    }

    /// What `calc()` the tokens use comes to: lengths and numbers, `var()`s of other tokens, added and taken away.
    fn evaluate(&self, value: &str, dark: bool) -> Option<f32> {
        let value = value.trim();
        let value = value.strip_prefix("calc(").and_then(|value| value.strip_suffix(')')).unwrap_or(value);
        let mut total = 0.0;
        let mut sign = 1.0;
        for term in value.split_whitespace() {
            match term {
                "+" => sign = 1.0,
                "-" => sign = -1.0,
                term => {
                    let amount = if let Some(name) = term.strip_prefix("var(--").and_then(|term| term.strip_suffix(')')) {
                        if name == "lighter" {
                            if dark { self.lighter } else { 0.0 }
                        } else {
                            self.evaluate(self.declared.get(name)?, dark)?
                        }
                    } else if let Some(rem) = term.strip_suffix("rem") {
                        rem.parse::<f32>().ok()? * REM
                    } else if let Some(px) = term.strip_suffix("px") {
                        px.parse().ok()?
                    } else {
                        term.parse().ok()?
                    };
                    total += sign * amount;
                }
            }
        }
        Some(total)
    }
}

fn uncommented(css: &str) -> String {
    let mut out = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start..].find("*/").map_or("", |end| &rest[start + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// `a, b` at the top level of a `light-dark()`, where each may have commas of its own in brackets.
fn split_top_level(both: &str) -> (&str, &str) {
    let mut depth = 0;
    for (at, character) in both.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return (both[..at].trim(), both[at + 1..].trim()),
            _ => {}
        }
    }
    (both, both)
}

/// `oklch(L C H)` or `oklch(L C H / A%)`, as sRGB (Björn Ottosson's OKLab, which CSS Color 4 uses).
fn oklch(value: &str) -> Option<Rgba> {
    let inner = value.strip_prefix("oklch(")?.strip_suffix(')')?;
    let (lch, alpha) = match inner.split_once('/') {
        Some((lch, alpha)) => (lch, alpha.trim()),
        None => (inner, "1"),
    };
    let alpha = match alpha.strip_suffix('%') {
        Some(percent) => percent.parse::<f32>().ok()? / 100.0,
        None => alpha.parse().ok()?,
    };
    let mut parts = lch.split_whitespace().map(|part| match part.strip_suffix('%') {
        Some(percent) => percent.parse::<f32>().map(|percent| percent / 100.0),
        None => part.parse::<f32>(),
    });
    let (l, c, h) = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?);
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let linear = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ];
    let gamma = |channel: f32| {
        let channel = channel.clamp(0.0, 1.0);
        if channel <= 0.003_130_8 { 12.92 * channel } else { 1.055 * channel.powf(1.0 / 2.4) - 0.055 }
    };
    Some(Rgba { r: gamma(linear[0]), g: gamma(linear[1]), b: gamma(linear[2]), a: alpha })
}

/// A type style (DESIGN.md: text is one of a few), as the UI library's stylesheet draws it (`[data-font]` in ui.css),
/// each line 1.5 times its size, as `body`'s, but a title's and a figure's.
pub struct TypeStyle {
    pub size: f32,
    pub weight: f32,
    /// Tracking, in em.
    pub tracking: f32,
    pub line_height: f32,
    pub tabular: bool,
}

/// `body`, `label`, `title`, `figure`, `subheadline`, `footnote`, `caption-1` or `caption-2`.
pub fn type_style(font: &str, dark: bool) -> TypeStyle {
    let tokens = tokens();
    let weight = |name: &str| tokens.number(name, dark).unwrap_or(450.0);
    let size = |name: &str| tokens.length(name).unwrap_or(14.0);
    let (size, weight, tracking, line_height, tabular) = match font {
        "label" => (size("text-base"), weight("label-weight"), 0.0, 1.5, false),
        "title" => (size("text-title"), weight("title-weight"), tokens.em("title-tracking").unwrap_or(0.0), 1.2, false),
        "figure" => (size("text-title"), weight("figure-weight"), tokens.em("figure-tracking").unwrap_or(0.0), 1.2, true),
        "subheadline" => (size("text-subheadline"), weight("body-weight"), 0.0, 1.5, false),
        "footnote" => (size("text-footnote"), weight("body-weight"), 0.0, 1.5, false),
        "caption-1" => (size("text-caption-1"), weight("body-weight"), 0.0, 1.5, false),
        "caption-2" => (size("text-caption-2"), weight("body-weight"), 0.0, 1.5, false),
        _ => (size("text-base"), weight("body-weight"), 0.0, 1.5, false),
    };
    TypeStyle { size, weight, tracking, line_height, tabular }
}

// Version 2's token table (`Tokens` in core/ui/src/strict/protocol.ts), which comes with the UI library as
// /ui/client/tokens.json. A host knows each kind of token and the conditions a value varies by, never a token's name.

/// What a token's value may depend on, as the window is now, in the order they win.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions {
    pub dark: bool,
    pub touch: bool,
    pub phone: bool,
    pub reduced_motion: bool,
}

impl Conditions {
    fn holds(&self, condition: &str) -> bool {
        match condition {
            "dark" => self.dark,
            "touch" => self.touch,
            "phone" => self.phone,
            "reducedMotion" => self.reduced_motion,
            _ => false,
        }
    }
}

/// A corner's radius: pixels, or a part of its box's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    Px(f32),
    Percent(f32),
}

/// One shadow of a `shadow` token, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub colour: Rgba,
    pub inset: bool,
}

/// A `type` token resolved: its size and line height in pixels, its weight, tracking in em, and whether its figures
/// line up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: f32,
    pub line_height: f32,
    pub weight: f32,
    pub tracking: f32,
    pub tabular: bool,
}

/// The token table: each token's JSON, by id.
#[derive(Default)]
pub struct Table {
    tokens: HashMap<String, serde_json::Value>,
}

static TABLE: std::sync::RwLock<Option<std::sync::Arc<Table>>> = std::sync::RwLock::new(None);

/// Takes the token table the UI library came with (`tokens.json`). → the version of the protocol it needs.
pub fn load_table(json: &str) -> anyhow::Result<u32> {
    let parsed: serde_json::Value = serde_json::from_str(json)?;
    let v = parsed["v"].as_u64().unwrap_or(0) as u32;
    let tokens = parsed["tokens"].as_object().map(|tokens| tokens.iter().map(|(id, token)| (id.clone(), token.clone())).collect()).unwrap_or_default();
    *TABLE.write().expect("nothing panics holding it") = Some(std::sync::Arc::new(Table { tokens }));
    Ok(v)
}

/// The token table loaded, or an empty one.
pub fn table() -> std::sync::Arc<Table> {
    TABLE.read().expect("nothing panics holding it").clone().unwrap_or_default()
}

/// A value, or a value by condition: `default`, then each condition that holds, in order, the later winning.
fn varies<'a>(value: &'a serde_json::Value, conditions: Conditions) -> &'a serde_json::Value {
    let Some(by) = value.as_object().filter(|by| by.contains_key("default")) else { return value };
    let mut out = &by["default"];
    for condition in ["dark", "touch", "phone", "reducedMotion"] {
        if conditions.holds(condition)
            && let Some(value) = by.get(condition)
        {
            out = value;
        }
    }
    out
}

/// A `Length` (`{px}` or `{rem}`), in pixels.
fn length_of(value: &serde_json::Value) -> Option<f32> {
    value["px"].as_f64().or_else(|| value["rem"].as_f64().map(|rem| rem * f64::from(REM))).map(|px| px as f32)
}

fn rgba_of(value: &serde_json::Value) -> Option<Rgba> {
    let channel = |at: usize| value.get(at).and_then(serde_json::Value::as_f64).map(|channel| channel as f32);
    Some(Rgba { r: channel(0)?, g: channel(1)?, b: channel(2)?, a: channel(3).unwrap_or(1.0) })
}

impl Table {
    /// Token `id`'s value, if it's of `kind`, as it is where `conditions` hold.
    fn value(&self, id: &str, kind: &str, conditions: Conditions) -> Option<&serde_json::Value> {
        let token = self.tokens.get(id).filter(|token| token["kind"] == kind)?;
        Some(varies(&token["value"], conditions))
    }

    pub fn kind(&self, id: &str) -> Option<&str> {
        self.tokens.get(id)?["kind"].as_str()
    }

    pub fn colour(&self, id: &str, conditions: Conditions) -> Option<Rgba> {
        rgba_of(self.value(id, "colour", conditions)?)
    }

    pub fn length(&self, id: &str, conditions: Conditions) -> Option<f32> {
        length_of(self.value(id, "length", conditions)?)
    }

    pub fn radius(&self, id: &str, conditions: Conditions) -> Option<Radius> {
        let value = self.value(id, "radius", conditions)?;
        match value["percent"].as_f64() {
            Some(percent) => Some(Radius::Percent(percent as f32)),
            None => length_of(value).map(Radius::Px),
        }
    }

    pub fn shadows(&self, id: &str, conditions: Conditions) -> Vec<Shadow> {
        let Some(shadows) = self.value(id, "shadow", conditions).and_then(serde_json::Value::as_array) else { return Vec::new() };
        shadows
            .iter()
            .filter_map(|shadow| {
                Some(Shadow {
                    x: length_of(&shadow["x"]).unwrap_or(0.0),
                    y: length_of(&shadow["y"]).unwrap_or(0.0),
                    blur: length_of(&shadow["blur"]).unwrap_or(0.0),
                    spread: length_of(&shadow["spread"]).unwrap_or(0.0),
                    colour: rgba_of(&shadow["colour"])?,
                    inset: shadow["inset"] == true,
                })
            })
            .collect()
    }

    /// A material: how far what's behind is blurred, in pixels, and how saturated it is.
    pub fn material(&self, id: &str, conditions: Conditions) -> Option<(f32, f32)> {
        let value = self.value(id, "material", conditions)?;
        Some((length_of(&value["blur"])?, value["saturate"].as_f64().unwrap_or(1.0) as f32))
    }

    pub fn weight(&self, id: &str, conditions: Conditions) -> Option<f32> {
        self.value(id, "weight", conditions)?.as_f64().map(|weight| weight as f32)
    }

    pub fn type_style(&self, id: &str, conditions: Conditions) -> Option<Type> {
        let value = self.value(id, "type", conditions)?;
        let size = length_of(varies(&value["size"], conditions))?;
        Some(Type {
            size,
            line_height: size * value["lineHeight"].as_f64().unwrap_or(1.5) as f32,
            weight: value["weight"].as_str().and_then(|weight| self.weight(weight, conditions)).unwrap_or(400.0),
            tracking: value["tracking"].as_f64().unwrap_or(0.0) as f32,
            tabular: value["tabular"] == true,
        })
    }

    /// A motion: how long it takes, in milliseconds, and its easing, a cubic Bézier's two points.
    pub fn motion(&self, id: &str, conditions: Conditions) -> Option<(f32, [f32; 4])> {
        let value = self.value(id, "motion", conditions)?;
        let duration = varies(&value["duration"], conditions).as_f64()? as f32;
        let easing = varies(&value["easing"], conditions);
        let point = |at: usize| easing.get(at).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
        Some((duration, [point(0), point(1), point(2), point(3)]))
    }
}
