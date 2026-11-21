//! The design tokens, which aren't the host's: they come with the UI library, over the air, as the app's page does,
//! as its token table (`load_table`), read as a window opens. The host knows only kinds of token (a colour, light and
//! dark, a length, a number), never their names or values, but for the few it draws its own window's title and
//! notices with.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::geometry::Rgba;
use crate::motion::Spring;

/// What CSS has as 1rem, in hundredths of a pixel, where the device has no text size of its own.
const REM: u16 = 1600;

/// `oklch(L C H)` or `oklch(L C H / A%)`, as sRGB (Björn Ottosson's OKLab, which CSS Color 4 uses).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn oklch(value: &str) -> Option<Rgba> {
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

// The token table (`Tokens` in core/ui/src/drawing-primitives/renderer.ts), which comes with the UI library as
// /ui/client/tokens.json. A host knows each kind of token and the conditions a value varies by, never a token's name.

/// What a token's value may depend on, as the window is now, in the order they win.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub dark: bool,
    pub touch: bool,
    pub phone: bool,
    pub reduced_motion: bool,
    /// What the window keeps for itself at its top, right, bottom and left, in whole pixels (`Screen::insets`, which
    /// its page is told as `safeAreaInsets`): the UI library's `safe-top`, `safe-right`, `safe-bottom` and `safe-left`
    /// lengths are these, as its tokens say the host sets them.
    pub insets: [u16; 4],
    /// Whose own tokens it draws with, as `load_page` gave them: its page's, where it has any (0, where it has none).
    pub page: u32,
    /// Where its title row has room for its page's top bar, which is in it (a unified toolbar): from the window's start,
    /// after its controls and title, and from its end, before its controls there. The UI library's
    /// `title-bar-start` and `title-bar-end` lengths are these, where they're not 0, and else its own.
    pub title_bar: [u16; 2],
    /// What 1rem is, in hundredths of a pixel: a phone's own text size, as an iPhone's browser has it
    /// (`font: -apple-system-body`), where `phone` holds, and CSS's 16px elsewhere.
    pub rem: u16,
}

impl Default for Conditions {
    fn default() -> Conditions {
        Conditions { dark: false, touch: false, phone: false, reduced_motion: false, insets: [0; 4], page: 0, title_bar: [0; 2], rem: REM }
    }
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

/// Each page's own tokens, and those of the table it names new values for (`Styles.tokens`), by the number
/// `load_page` gave it, from 1: kept apart, as each is its window's alone.
static PAGES: std::sync::RwLock<Vec<(String, std::sync::Arc<HashMap<String, serde_json::Value>>)>> = std::sync::RwLock::new(Vec::new());

/// Takes a page's own tokens, which come with its styles (`styles.json`), as its window opens. → its number, for its
/// `Conditions`, which is 0 for a page with none; the same tokens, again, are the same page.
pub fn load_page(json: &str) -> anyhow::Result<u32> {
    let parsed: serde_json::Value = serde_json::from_str(json)?;
    let Some(tokens) = parsed["tokens"].as_object().filter(|tokens| !tokens.is_empty()) else { return Ok(0) };
    let key = serde_json::to_string(tokens)?;
    let mut pages = PAGES.write().expect("nothing panics holding it");
    if let Some(at) = pages.iter().position(|(known, _)| *known == key) {
        return Ok(at as u32 + 1);
    }
    pages.push((key, std::sync::Arc::new(tokens.iter().map(|(id, token)| (id.clone(), token.clone())).collect())));
    Ok(pages.len() as u32)
}

/// The tokens of the apps whose components a page draws (Settings' sections), as their `styles.json` came with them.
/// An id starts with what its app makes it from, so no two apps' are the same, and these are every window's.
static COMPONENTS: std::sync::RwLock<Option<HashMap<String, serde_json::Value>>> = std::sync::RwLock::new(None);

/// Takes another app's own tokens, which its component brings to a page that isn't its own.
pub fn load_component(json: &str) -> anyhow::Result<()> {
    let parsed: serde_json::Value = serde_json::from_str(json)?;
    let mut components = COMPONENTS.write().expect("nothing panics holding it");
    let components = components.get_or_insert_with(HashMap::new);
    for (id, token) in parsed["tokens"].as_object().into_iter().flatten() {
        components.insert(id.clone(), token.clone());
    }
    Ok(())
}

static TABLE: std::sync::RwLock<Option<std::sync::Arc<Table>>> = std::sync::RwLock::new(None);

/// Takes the token table the UI library came with (`tokens.json`). → the runtime version it needs.
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

/// A `Length` (`{px}` or `{rem}`), in pixels, as 1rem is where `conditions` hold.
fn length_of(value: &serde_json::Value, conditions: Conditions) -> Option<f32> {
    value["px"].as_f64().or_else(|| value["rem"].as_f64().map(|rem| rem * f64::from(conditions.rem) / 100.0)).map(|px| px as f32)
}

fn rgba_of(value: &serde_json::Value) -> Option<Rgba> {
    let channel = |at: usize| value.get(at).and_then(serde_json::Value::as_f64).map(|channel| channel as f32);
    Some(Rgba { r: channel(0)?, g: channel(1)?, b: channel(2)?, a: channel(3).unwrap_or(1.0) })
}

impl Table {
    /// Token `id`'s value, if it's of `kind`, as it is where `conditions` hold.
    fn value(&self, id: &str, kind: &str, conditions: Conditions) -> Option<Cow<'_, serde_json::Value>> {
        if let Some(at) = conditions.page.checked_sub(1) {
            let page = PAGES.read().expect("nothing panics holding it").get(at as usize).map(|(_, tokens)| tokens.clone());
            if let Some(token) = page.as_ref().and_then(|tokens| tokens.get(id)) {
                return (token["kind"] == kind).then(|| Cow::Owned(varies(&token["value"], conditions).clone()));
            }
        }
        if let Some(token) = COMPONENTS.read().expect("nothing panics holding it").as_ref().and_then(|tokens| tokens.get(id)) {
            return (token["kind"] == kind).then(|| Cow::Owned(varies(&token["value"], conditions).clone()));
        }
        let token = self.tokens.get(id).filter(|token| token["kind"] == kind)?;
        Some(Cow::Borrowed(varies(&token["value"], conditions)))
    }

    pub fn kind(&self, id: &str) -> Option<&str> {
        self.tokens.get(id)?["kind"].as_str()
    }

    pub fn colour(&self, id: &str, conditions: Conditions) -> Option<Rgba> {
        let value = self.value(id, "colour", conditions)?;
        rgba_of(&value)
    }

    pub fn length(&self, id: &str, conditions: Conditions) -> Option<f32> {
        let inset = ["safe-top", "safe-right", "safe-bottom", "safe-left"].iter().position(|safe| *safe == id);
        if let Some(side) = inset {
            return Some(f32::from(conditions.insets[side]));
        }
        let title_bar = ["title-bar-start", "title-bar-end"].iter().position(|edge| *edge == id);
        if let Some(edge) = title_bar.filter(|edge| conditions.title_bar[*edge] > 0) {
            return Some(f32::from(conditions.title_bar[edge]));
        }
        let value = self.value(id, "length", conditions)?;
        length_of(&value, conditions)
    }

    pub fn radius(&self, id: &str, conditions: Conditions) -> Option<Radius> {
        let value = self.value(id, "radius", conditions)?;
        match value["percent"].as_f64() {
            Some(percent) => Some(Radius::Percent(percent as f32)),
            None => length_of(&value, conditions).map(Radius::Px),
        }
    }

    pub fn shadows(&self, id: &str, conditions: Conditions) -> Vec<Shadow> {
        let value = self.value(id, "shadow", conditions);
        let Some(shadows) = value.as_deref().and_then(serde_json::Value::as_array) else { return Vec::new() };
        shadows
            .iter()
            .filter_map(|shadow| {
                Some(Shadow {
                    x: length_of(&shadow["x"], conditions).unwrap_or(0.0),
                    y: length_of(&shadow["y"], conditions).unwrap_or(0.0),
                    blur: length_of(&shadow["blur"], conditions).unwrap_or(0.0),
                    spread: length_of(&shadow["spread"], conditions).unwrap_or(0.0),
                    colour: rgba_of(&shadow["colour"])?,
                    inset: shadow["inset"] == true,
                })
            })
            .collect()
    }

    /// A material: how far what's behind is blurred, in pixels, how saturated it is, and whether it's the system's own
    /// where the host has one.
    pub fn material(&self, id: &str, conditions: Conditions) -> Option<(f32, f32, bool)> {
        let value = self.value(id, "material", conditions)?;
        Some((length_of(&value["blur"], conditions)?, value["saturate"].as_f64().unwrap_or(1.0) as f32, value["system"] == true))
    }

    pub fn weight(&self, id: &str, conditions: Conditions) -> Option<f32> {
        self.value(id, "weight", conditions)?.as_f64().map(|weight| weight as f32)
    }

    /// Whether a font's families end in the generic `monospace`, as CSS's `font-family` falls back to one.
    pub fn monospace(&self, id: &str, conditions: Conditions) -> Option<bool> {
        let value = self.value(id, "font", conditions)?;
        Some(value.as_array()?.last()?.as_str()? == "monospace")
    }

    pub fn type_style(&self, id: &str, conditions: Conditions) -> Option<Type> {
        let value = self.value(id, "type", conditions)?;
        let size = length_of(varies(&value["size"], conditions), conditions)?;
        Some(Type {
            size,
            line_height: size * value["lineHeight"].as_f64().unwrap_or(1.5) as f32,
            weight: value["weight"].as_str().and_then(|weight| self.weight(weight, conditions)).unwrap_or(400.0),
            tracking: value["tracking"].as_f64().unwrap_or(0.0) as f32,
            tabular: value["tabular"] == true,
        })
    }

    /// A motion: how long it takes, in milliseconds, and its easing, a cubic Bézier's two points, or a spring's.
    pub fn motion(&self, id: &str, conditions: Conditions) -> Option<Motion> {
        let value = self.value(id, "motion", conditions)?;
        let duration = varies(&value["duration"], conditions).as_f64()? as f32;
        let easing = varies(&value["easing"], conditions);
        let point = |at: usize| easing.get(at).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
        // With less motion it's an ease of its (reduced) duration, a fade where it would have moved: no spring.
        let spring = value["bounce"].as_f64().filter(|_| !conditions.reduced_motion).map(|bounce| Spring::new(duration, bounce as f32));
        Some(Motion { duration, easing: [point(0), point(1), point(2), point(3)], spring })
    }
}

/// How a change moves: over `duration` milliseconds by `easing`, a cubic Bézier's two points, or by `spring`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub duration: f32,
    pub easing: [f32; 4],
    pub spring: Option<Spring>,
}
