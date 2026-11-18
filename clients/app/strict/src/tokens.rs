//! The design tokens, read from the UI library's own stylesheet (`core/ui/src/tokens.css`) as it's built in, so a
//! native screen draws with exactly what a web one does: its colours, light and dark, its sizes and its type styles.
//! Only `:root`'s first rule is read, which is a computer's (the touch and iPhone rules after it are a phone's), and
//! `:root.dark`'s `--lighter`.

use std::collections::HashMap;
use std::sync::LazyLock;

use gpui::Rgba;

const CSS: &str = include_str!("../../../../core/ui/src/tokens.css");

/// What CSS has as 1rem, in pixels.
const REM: f32 = 16.0;

/// The tokens, light and dark.
pub struct Tokens {
    declared: HashMap<String, String>,
    lighter: f32,
}

pub static TOKENS: LazyLock<Tokens> = LazyLock::new(|| Tokens::parse(CSS));

impl Tokens {
    fn parse(css: &str) -> Tokens {
        let rule = |selector: &str| -> HashMap<String, String> {
            let Some(start) = css.find(&format!("\n{selector} {{")) else { return HashMap::new() };
            let body = &css[start..];
            let body = &body[body.find('{').unwrap_or(0) + 1..body.find("\n}").unwrap_or(body.len())];
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
    let mut parts = lch.split_whitespace().map(|part| part.parse::<f32>());
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
    let tokens = &*TOKENS;
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
