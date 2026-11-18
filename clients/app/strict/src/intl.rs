//! What ECMA-402's `Intl` formats with, for the sandbox's own `Intl` (intl.js): ICU4X's formatters, with the data for
//! every locale built in, each held by a class intl.js makes once its options are resolved as ECMA-402 says, and the
//! patterns CLDR has for the units ICU4X doesn't format (units.json, from units.js). Every number reaches it as a
//! number or a decimal string, never NaN or an infinity, which intl.js says itself.

use std::fmt;
use std::sync::OnceLock;

use fixed_decimal::{Decimal, FloatPrecision, SignDisplay, SignedRoundingMode, UnsignedRoundingMode};
use icu::collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu::collator::{Collator, CollatorBorrowed};
use icu::datetime::fieldbag::{DateTimeFieldBag, field};
use icu::datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu::datetime::fieldsets::enums::CompositeFieldSet;
use icu::datetime::options::{Length, TimePrecision};
use icu::datetime::DateTimeFormatter;
use icu::decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu::decimal::{CompactDecimalFormatter, DecimalFormatter};
use icu::list::ListFormatter;
use icu::list::options::{ListFormatterOptions, ListLength};
use icu::locale::{Locale, LocaleExpander};
use icu::plurals::{PluralCategory, PluralRuleType, PluralRules, PluralRulesOptions};
use icu::segmenter::{GraphemeClusterSegmenter, SentenceSegmenter, WordSegmenter};
use icu_experimental::dimension::currency::formatter::CurrencyFormatter;
use icu_experimental::dimension::percent::formatter::PercentFormatter;
use icu_experimental::relativetime::options::Numeric;
use icu_experimental::relativetime::{RelativeTimeFormatter, RelativeTimeFormatterOptions};
use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, Result, Value};
use serde::Deserialize;
use writeable::{Part, PartsWrite, Writeable};

/// Adds what intl.js makes its `Intl` from to `globalThis.__intl`.
pub fn init(ctx: &Ctx, time_zone: &str) -> Result<()> {
    let intl = Object::new(ctx.clone())?;
    Class::<Numbers>::define(&intl)?;
    Class::<Dates>::define(&intl)?;
    Class::<Plurals>::define(&intl)?;
    Class::<Relative>::define(&intl)?;
    Class::<Lists>::define(&intl)?;
    Class::<Collation>::define(&intl)?;
    Class::<Segments>::define(&intl)?;
    intl.set("canonicalize", rquickjs::prelude::Func::from(|ctx: Ctx, tag: String| locale(&ctx, &tag).map(|locale| locale.to_string())))?;
    intl.set("maximize", rquickjs::prelude::Func::from(|ctx: Ctx, tag: String| {
        let mut locale = locale(&ctx, &tag)?;
        LocaleExpander::new_common().maximize(&mut locale.id);
        Ok::<_, rquickjs::Error>(locale.to_string())
    }))?;
    intl.set("minimize", rquickjs::prelude::Func::from(|ctx: Ctx, tag: String| {
        let mut locale = locale(&ctx, &tag)?;
        LocaleExpander::new_common().minimize(&mut locale.id);
        Ok::<_, rquickjs::Error>(locale.to_string())
    }))?;
    intl.set("timeZone", rquickjs::prelude::Func::from(|ctx: Ctx, name: String| zone(&ctx, &name).map(|zone| zone_name(&zone))))?;
    intl.set("hostTimeZone", jiff::tz::TimeZone::get(time_zone).map_or_else(|_| "UTC".into(), |zone| zone_name(&zone)))?;
    ctx.globals().set("__intl", intl)
}

/// The host's time zone, by its IANA name.
pub fn host_time_zone() -> String {
    iana_time_zone::get_timezone().ok().filter(|name| jiff::tz::TimeZone::get(name).is_ok()).unwrap_or_else(|| "UTC".into())
}

/// A time zone's name as ECMA-402 gives it: its IANA name, where UTC's is `UTC`, not one of its links (`Etc/UTC`).
fn zone_name(zone: &jiff::tz::TimeZone) -> String {
    match zone.iana_name() {
        Some("Etc/UTC" | "Etc/GMT" | "GMT" | "Etc/UCT" | "UCT" | "Etc/Universal" | "Universal" | "Etc/Zulu" | "Zulu") | None => "UTC".into(),
        Some(name) => name.into(),
    }
}

fn range(ctx: &Ctx, message: impl fmt::Display) -> rquickjs::Error {
    Exception::throw_range(ctx, &message.to_string())
}

fn locale(ctx: &Ctx, tag: &str) -> Result<Locale> {
    let mut locale: Locale = tag.parse().map_err(|_| range(ctx, format!("Incorrect locale information provided: {tag}")))?;
    icu::locale::LocaleCanonicalizer::new_extended().canonicalize(&mut locale);
    Ok(locale)
}

fn zone(ctx: &Ctx, name: &str) -> Result<jiff::tz::TimeZone> {
    if name.eq_ignore_ascii_case("utc") {
        return Ok(jiff::tz::TimeZone::UTC);
    }
    jiff::tz::TimeZone::get(name).map_err(|_| range(ctx, format!("Invalid time zone specified: {name}")))
}

/// A number as ECMA-402 formats it: a JS number, or a decimal string (a BigInt, or a string it was given).
fn decimal(ctx: &Ctx, value: &Value) -> Result<Decimal> {
    if let Some(number) = value.as_number() {
        return Decimal::try_from_f64(number, FloatPrecision::RoundTrip).map_err(|_| range(ctx, "Invalid number"));
    }
    let text: String = value.get()?;
    Decimal::try_from_str(text.trim()).map_err(|_| range(ctx, format!("Invalid number: {text}")))
}

/// What ECMA-402 calls a number format's digit options, resolved (SetNumberFormatDigitOptions).
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Digits {
    minimum_integer_digits: i16,
    minimum_fraction_digits: Option<i16>,
    maximum_fraction_digits: Option<i16>,
    minimum_significant_digits: Option<i16>,
    maximum_significant_digits: Option<i16>,
    /// `auto`, `morePrecision` or `lessPrecision`.
    rounding_priority: String,
    rounding_mode: String,
    /// `auto` or `stripIfInteger`.
    trailing_zero_display: String,
}

impl Digits {
    fn mode(&self) -> SignedRoundingMode {
        use SignedRoundingMode::*;
        use UnsignedRoundingMode::*;
        match self.rounding_mode.as_str() {
            "ceil" => Ceil,
            "floor" => Floor,
            "expand" => Unsigned(Expand),
            "trunc" => Unsigned(Trunc),
            "halfCeil" => HalfCeil,
            "halfFloor" => HalfFloor,
            "halfTrunc" => Unsigned(HalfTrunc),
            "halfEven" => Unsigned(HalfEven),
            _ => Unsigned(HalfExpand),
        }
    }

    /// `value` rounded and padded as these say (FormatNumericToString).
    fn apply(&self, mut value: Decimal) -> Decimal {
        let significant = |mut value: Decimal| {
            let (min, max) = (self.minimum_significant_digits.unwrap_or(1), self.maximum_significant_digits.unwrap_or(21));
            let start = value.absolute.nonzero_magnitude_start().max(0).max(value.absolute.nonzero_magnitude_start());
            let top = if value.absolute.is_zero() { 0 } else { start };
            value.round_with_mode(top - max + 1, self.mode());
            value.absolute.trim_end();
            value.absolute.pad_end(top - min + 1);
            value
        };
        let fraction = |mut value: Decimal| {
            let (min, max) = (self.minimum_fraction_digits.unwrap_or(0), self.maximum_fraction_digits.unwrap_or(3));
            value.round_with_mode(-max, self.mode());
            value.absolute.trim_end();
            value.absolute.pad_end(-min);
            value
        };
        let has_significant = self.minimum_significant_digits.is_some() || self.maximum_significant_digits.is_some();
        let has_fraction = self.minimum_fraction_digits.is_some() || self.maximum_fraction_digits.is_some();
        value = match self.rounding_priority.as_str() {
            "morePrecision" | "lessPrecision" => {
                let (by_significant, by_fraction) = (significant(value.clone()), fraction(value));
                let fraction_digits = |value: &Decimal| -*value.absolute.magnitude_range().start();
                let more = fraction_digits(&by_significant) >= fraction_digits(&by_fraction);
                if (self.rounding_priority == "morePrecision") == more { by_significant } else { by_fraction }
            }
            _ if has_significant => significant(value),
            _ if has_fraction || !has_significant => fraction(value),
            _ => value,
        };
        if self.trailing_zero_display == "stripIfInteger" && *value.absolute.magnitude_range().start() < 0 {
            let mut integer = value.clone();
            integer.absolute.trim_end();
            if *integer.absolute.magnitude_range().start() >= 0 {
                value = integer;
            }
        }
        value.absolute.pad_start(self.minimum_integer_digits.max(1));
        value
    }
}

/// What a formatter wrote, in ECMA-402's parts: each written as a part of ICU4X's by that part's name, the rest a
/// literal, which `named` names where it's a currency's, a unit's or a percent sign. A number's parts are its innermost
/// (an integer in a relative time), a date's its outermost (a day, not the integer it's written with).
#[derive(Default)]
struct Parts {
    parts: Vec<(&'static str, String)>,
    current: Option<&'static str>,
    outermost: bool,
}

impl fmt::Write for Parts {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let kind = self.current.unwrap_or("literal");
        match self.parts.last_mut() {
            Some((last, written)) if *last == kind => written.push_str(text),
            _ => self.parts.push((kind, text.to_owned())),
        }
        Ok(())
    }
}

impl PartsWrite for Parts {
    type SubPartsWrite = Parts;

    fn with_part(&mut self, part: Part, mut f: impl FnMut(&mut Self::SubPartsWrite) -> fmt::Result) -> fmt::Result {
        let was = self.current;
        if !(self.outermost && was.is_some()) {
            self.current = Some(part.value);
        }
        let written = f(self);
        self.current = was;
        written
    }
}

impl Parts {
    fn of(written: &impl Writeable) -> Parts {
        Parts::written(written, false)
    }

    fn written(written: &impl Writeable, outermost: bool) -> Parts {
        let mut parts = Parts { outermost, ..Parts::default() };
        let _ = written.write_to_parts(&mut parts);
        parts.parts.retain(|(_, text)| !text.is_empty());
        parts
    }

    /// Its literals' words named `kind`, their spaces left literals.
    fn named(mut self, kind: &'static str) -> Parts {
        self.parts = self
            .parts
            .into_iter()
            .flat_map(|(was, text)| {
                if was != "literal" {
                    return vec![(was, text)];
                }
                let mut split: Vec<(&'static str, String)> = vec![];
                for character in text.chars() {
                    let kind = if character.is_whitespace() || character == '\u{200f}' || character == '\u{200e}' { "literal" } else { kind };
                    match split.last_mut() {
                        Some((last, written)) if *last == kind => written.push(character),
                        _ => split.push((kind, character.to_string())),
                    }
                }
                split
            })
            .collect();
        self
    }

    fn text(&self) -> String {
        self.parts.iter().map(|(_, text)| text.as_str()).collect()
    }

    fn into_js<'js>(self, ctx: &Ctx<'js>) -> Result<rquickjs::Array<'js>> {
        let array = rquickjs::Array::new(ctx.clone())?;
        for (at, (kind, value)) in self.parts.into_iter().enumerate() {
            let part = Object::new(ctx.clone())?;
            part.set("type", kind)?;
            part.set("value", value)?;
            array.set(at, part)?;
        }
        Ok(array)
    }
}

/// A number format's options as intl.js resolved them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NumberOptions {
    style: String,
    currency: Option<String>,
    currency_display: String,
    unit: Option<String>,
    unit_display: String,
    notation: String,
    compact_display: String,
    sign_display: String,
    /// `always`, `auto`, `min2` or `false`.
    use_grouping: String,
    #[serde(flatten)]
    digits: Digits,
}

enum Style {
    Decimal,
    Percent(PercentFormatter<DecimalFormatter>),
    Currency(CurrencyFormatter<DecimalFormatter>),
    CompactCurrency(CurrencyFormatter<CompactDecimalFormatter>),
    Unit { plurals: PluralRules, patterns: Patterns },
    Compact(CompactDecimalFormatter),
    Scientific { engineering: bool },
}

/// Intl.NumberFormat's formatting.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Numbers {
    #[qjs(skip_trace)]
    decimals: DecimalFormatter,
    #[qjs(skip_trace)]
    style: Style,
    #[qjs(skip_trace)]
    options: NumberOptions,
}

#[rquickjs::methods]
impl Numbers {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, options: String) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let options: NumberOptions = serde_json::from_str(&options).map_err(|error| range(&ctx, error))?;
        let mut decimal_options = DecimalFormatterOptions::default();
        decimal_options.grouping_strategy = Some(match options.use_grouping.as_str() {
            "always" => GroupingStrategy::Always,
            "min2" => GroupingStrategy::Min2,
            "false" => GroupingStrategy::Never,
            _ => GroupingStrategy::Auto,
        });
        let failed = |error: icu_provider::DataError| range(&ctx, error);
        let decimals = DecimalFormatter::try_new((&locale).into(), decimal_options).map_err(failed)?;
        let compact = options.notation == "compact";
        let style = match (options.style.as_str(), options.notation.as_str()) {
            ("currency", _) => {
                let code = options.currency.as_deref().unwrap_or("usd").to_ascii_lowercase();
                let code = icu_experimental::dimension::currency::CurrencyType::try_from_str(&code)
                    .map_err(|_| range(&ctx, format!("Invalid currency code: {code}")))?;
                let prefs = (&locale).into();
                let display = options.currency_display.as_str();
                if compact {
                    let long = options.compact_display == "long";
                    Style::CompactCurrency(
                        match display {
                            "narrowSymbol" if long => CurrencyFormatter::try_new_compact_long_symbol_narrow(prefs, code, Default::default()),
                            "narrowSymbol" => CurrencyFormatter::try_new_compact_symbol_narrow(prefs, code, Default::default()),
                            "code" if long => CurrencyFormatter::try_new_compact_long_code(prefs, code, Default::default()),
                            "code" => CurrencyFormatter::try_new_compact_code(prefs, code, Default::default()),
                            "name" if long => CurrencyFormatter::try_new_compact_long_name(prefs, code),
                            "name" => CurrencyFormatter::try_new_compact_name(prefs, code),
                            _ if long => CurrencyFormatter::try_new_compact_long_symbol(prefs, code, Default::default()),
                            _ => CurrencyFormatter::try_new_compact_symbol(prefs, code, Default::default()),
                        }
                        .map_err(failed)?,
                    )
                } else {
                    Style::Currency(
                        match display {
                            "narrowSymbol" => CurrencyFormatter::try_new_symbol_narrow(prefs, code, Default::default()),
                            "code" => CurrencyFormatter::try_new_code(prefs, code, Default::default()),
                            "name" => CurrencyFormatter::try_new_name(prefs, code),
                            _ => CurrencyFormatter::try_new_symbol(prefs, code, Default::default()),
                        }
                        .map_err(failed)?,
                    )
                }
            }
            ("percent", _) => Style::Percent(
                PercentFormatter::try_new_with_decimal_formatter((&locale).into(), decimals.clone(), Default::default()).map_err(failed)?,
            ),
            ("unit", _) => {
                let unit = options.unit.clone().unwrap_or_default();
                let width = match options.unit_display.as_str() {
                    "long" => "long",
                    "narrow" => "narrow",
                    _ => "short",
                };
                let patterns = units(&locale, width, &unit).ok_or_else(|| range(&ctx, format!("Invalid unit argument: {unit}")))?;
                Style::Unit { plurals: PluralRules::try_new_cardinal((&locale).into()).map_err(failed)?, patterns }
            }
            (_, "compact") => {
                let prefs = (&locale).into();
                Style::Compact(
                    if options.compact_display == "long" {
                        CompactDecimalFormatter::try_new_long(prefs, Default::default())
                    } else {
                        CompactDecimalFormatter::try_new_short(prefs, Default::default())
                    }
                    .map_err(failed)?,
                )
            }
            (_, "scientific") => Style::Scientific { engineering: false },
            (_, "engineering") => Style::Scientific { engineering: true },
            _ => Style::Decimal,
        };
        Ok(Numbers { decimals, style, options })
    }

    pub fn format(&self, ctx: Ctx<'_>, value: Value<'_>) -> Result<String> {
        Ok(self.parts(&ctx, &value)?.text())
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<rquickjs::Array<'js>> {
        self.parts(&ctx, &value)?.into_js(&ctx)
    }
}

impl Numbers {
    fn signed(&self, mut value: Decimal) -> Decimal {
        value.apply_sign_display(match self.options.sign_display.as_str() {
            "always" => SignDisplay::Always,
            "exceptZero" => SignDisplay::ExceptZero,
            "negative" => SignDisplay::Negative,
            "never" => SignDisplay::Never,
            _ => SignDisplay::Auto,
        });
        value
    }

    fn parts(&self, ctx: &Ctx, value: &Value) -> Result<Parts> {
        let mut value = decimal(ctx, value)?;
        let digits = &self.options.digits;
        Ok(match &self.style {
            Style::Decimal => Parts::of(&self.decimals.format(&self.signed(digits.apply(value)))),
            Style::Percent(percent) => {
                value.multiply_pow10(2);
                let value = self.signed(digits.apply(value));
                Parts::of(&percent.format(&value)).named("percentSign")
            }
            Style::Currency(currency) => {
                let value = self.signed(digits.apply(value));
                Parts::of(&currency.format_fixed_decimal(&value)).named("currency")
            }
            Style::CompactCurrency(currency) => {
                let value = self.signed(value);
                Parts::of(&currency.format_fixed_decimal(&value)).named("currency")
            }
            Style::Compact(compact) => {
                let value = self.signed(value);
                Parts::of(&compact.format(&value)).named("compact")
            }
            Style::Unit { plurals, patterns } => {
                let value = self.signed(digits.apply(value));
                let category = plurals.category_for(&value);
                let number = Parts::of(&self.decimals.format(&value));
                let pattern = patterns.get(category);
                let (before, after) = pattern.split_once("{0}").unwrap_or(("", pattern));
                let literal = |text: &str| Parts { parts: vec![("literal", text.to_owned())], ..Parts::default() }.named("unit");
                let mut parts = literal(before);
                parts.parts.extend(number.parts);
                parts.parts.extend(literal(after).parts);
                parts.parts.retain(|(_, text)| !text.is_empty());
                parts
            }
            Style::Scientific { engineering } => {
                let start = if value.absolute.is_zero() { 0 } else { value.absolute.nonzero_magnitude_start() };
                let exponent = if *engineering { start.div_euclid(3) * 3 } else { start };
                value.multiply_pow10(-exponent);
                let mut parts = Parts::of(&self.decimals.format(&self.signed(digits.apply(value))));
                parts.parts.push(("exponentSeparator", "E".into()));
                if exponent < 0 {
                    parts.parts.push(("exponentMinusSign", "-".into()));
                }
                parts.parts.push(("exponentInteger", exponent.unsigned_abs().to_string()));
                parts
            }
        })
    }
}

/// A unit's patterns in a locale and width, by plural category: `{0} kB`.
struct Patterns {
    patterns: Vec<(String, String)>,
}

impl Patterns {
    fn get(&self, category: PluralCategory) -> &str {
        let name = match category {
            PluralCategory::Zero => "zero",
            PluralCategory::One => "one",
            PluralCategory::Two => "two",
            PluralCategory::Few => "few",
            PluralCategory::Many => "many",
            PluralCategory::Other => "other",
        };
        let find = |name: &str| self.patterns.iter().find(|(was, _)| was == name).map(|(_, pattern)| pattern.as_str());
        find(name).or_else(|| find("other")).unwrap_or("{0}")
    }
}

/// CLDR's unit patterns (units.json): each locale's, where they differ from its parent's.
#[derive(Deserialize)]
struct UnitsData {
    parents: std::collections::HashMap<String, String>,
    locales: std::collections::HashMap<String, std::collections::HashMap<String, std::collections::HashMap<String, serde_json::Value>>>,
}

fn units_data() -> &'static UnitsData {
    static DATA: OnceLock<UnitsData> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(include_str!("units.json")).expect("units.js writes it"))
}

/// What `locale` says of `unit` in `width`, from it or the nearest locale it falls back to; a compound one
/// (`kilometer-per-hour`) that CLDR doesn't name is made of its two, as CLDR says (`per`).
fn units(locale: &Locale, width: &str, unit: &str) -> Option<Patterns> {
    let data = units_data();
    let mut chain = vec![];
    let mut at = locale.id.to_string();
    loop {
        chain.push(at.clone());
        if at == "und" {
            break;
        }
        at = data.parents.get(&at).cloned().unwrap_or_else(|| at.rsplit_once('-').map_or_else(|| "und".into(), |(parent, _)| parent.to_owned()));
    }
    let find = |key: &str| chain.iter().find_map(|locale| data.locales.get(locale)?.get(width)?.get(key)).cloned();
    if let Some(serde_json::Value::Object(patterns)) = find(unit) {
        let patterns = patterns.into_iter().filter(|(name, _)| name != "per").filter_map(|(name, pattern)| Some((name, pattern.as_str()?.to_owned()))).collect();
        return Some(Patterns { patterns });
    }
    let (numerator, denominator) = unit.split_once("-per-")?;
    let Patterns { patterns } = units(locale, width, numerator)?;
    let per_unit = find(denominator).and_then(|denominator| denominator.get("per")?.as_str().map(str::to_owned));
    let combined = match per_unit {
        Some(per_unit) => patterns.into_iter().map(|(name, pattern)| (name, per_unit.replace("{0}", &pattern))).collect(),
        None => {
            let compound = find("per").and_then(|per| per.as_str().map(str::to_owned)).unwrap_or_else(|| "{0}/{1}".into());
            let one = units(locale, width, denominator)?;
            let name = one.get(PluralCategory::One).replace("{0}", "").trim().to_owned();
            patterns.into_iter().map(|(category, pattern)| (category, compound.replace("{0}", &pattern).replace("{1}", &name))).collect()
        }
    };
    Some(Patterns { patterns: combined })
}

/// A date and time format's options as intl.js resolved them: a style, or the fields to show.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DateOptions {
    time_zone: String,
    date_style: Option<String>,
    time_style: Option<String>,
    era: Option<String>,
    year: Option<String>,
    month: Option<String>,
    day: Option<String>,
    weekday: Option<String>,
    hour: Option<String>,
    minute: Option<String>,
    second: Option<String>,
    fractional_second_digits: Option<u8>,
    time_zone_name: Option<String>,
}

/// Intl.DateTimeFormat's formatting.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Dates {
    #[qjs(skip_trace)]
    formatter: DateTimeFormatter<CompositeFieldSet>,
    #[qjs(skip_trace)]
    zone: jiff::tz::TimeZone,
    #[qjs(skip_trace)]
    locale: Locale,
}

#[rquickjs::methods]
impl Dates {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, options: String) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let options: DateOptions = serde_json::from_str(&options).map_err(|error| range(&ctx, error))?;
        let zone = zone(&ctx, &options.time_zone)?;
        let fields = match (&options.date_style, &options.time_style) {
            (None, None) => fields(&options),
            (date, time) => styles(date.as_deref(), time.as_deref()).build_composite().map_err(|error| range(&ctx, format!("{error:?}")))?,
        };
        let formatter = DateTimeFormatter::try_new((&locale).into(), fields).map_err(|error| range(&ctx, error))?;
        Ok(Dates { formatter, zone, locale })
    }

    /// `ms`, since 1970, as it's shown.
    pub fn format(&self, ctx: Ctx<'_>, ms: f64) -> Result<String> {
        Ok(self.parts(&ctx, ms)?.text())
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, ms: f64) -> Result<rquickjs::Array<'js>> {
        self.parts(&ctx, ms)?.into_js(&ctx)
    }

    /// The hour cycle its locale shows hours in: `h12`, `h23`, `h11` or `h24`.
    #[qjs(rename = "hourCycle")]
    pub fn hour_cycle(&self, ctx: Ctx<'_>) -> Result<String> {
        let mut bag = DateTimeFieldBag::default();
        bag.hour = Some(field::Hour::Numeric);
        let hours = DateTimeFormatter::try_new((&self.locale).into(), bag.to_composite_field_set()).map_err(|error| range(&ctx, error))?;
        let at = |hour: i8| hours.format(&jiff::civil::date(2026, 1, 1).at(hour, 0, 0, 0).to_zoned(jiff::tz::TimeZone::UTC).expect("a valid time")).to_string();
        let (midnight, afternoon) = (at(0), at(13));
        Ok(match (afternoon.contains("13"), midnight.contains("12"), midnight.contains("24")) {
            (true, _, true) => "h24",
            (true, _, _) => "h23",
            (false, true, _) => "h12",
            _ => "h11",
        }
        .into())
    }
}

impl Dates {
    fn parts(&self, ctx: &Ctx, ms: f64) -> Result<Parts> {
        let at = jiff::Timestamp::from_millisecond(ms.floor() as i64).map_err(|_| range(ctx, "Invalid time value"))?;
        Ok(Parts::written(&self.formatter.format(&at.to_zoned(self.zone.clone())), true))
    }
}

/// ECMA-402's fields, as ICU4X's field bag, which mirrors them.
fn fields(options: &DateOptions) -> CompositeFieldSet {
    use field::*;
    let width = |value: &Option<String>| value.as_deref().map(|value| value == "2-digit");
    let mut bag = DateTimeFieldBag::default();
    bag.era = options.era.as_deref().map(|era| match era {
        "long" => Era::Long,
        "narrow" => Era::Narrow,
        _ => Era::Short,
    });
    bag.year = width(&options.year).map(|two| if two { Year::TwoDigit } else { Year::Numeric });
    bag.month = options.month.as_deref().map(|month| match month {
        "2-digit" => Month::TwoDigit,
        "long" => Month::Long,
        "short" => Month::Short,
        "narrow" => Month::Narrow,
        _ => Month::Numeric,
    });
    bag.day = width(&options.day).map(|two| if two { Day::TwoDigit } else { Day::Numeric });
    bag.weekday = options.weekday.as_deref().map(|weekday| match weekday {
        "long" => Weekday::Long,
        "narrow" => Weekday::Narrow,
        _ => Weekday::Short,
    });
    bag.hour = width(&options.hour).map(|two| if two { Hour::TwoDigit } else { Hour::Numeric });
    bag.minute = width(&options.minute).map(|two| if two { Minute::TwoDigit } else { Minute::Numeric });
    bag.second = width(&options.second).map(|two| if two { Second::TwoDigit } else { Second::Numeric });
    bag.fractional_second_digits = options.fractional_second_digits.map(|digits| match digits {
        1 => FractionalSecondDigits::F1,
        2 => FractionalSecondDigits::F2,
        _ => FractionalSecondDigits::F3,
    });
    bag.time_zone_name = options.time_zone_name.as_deref().map(|name| match name {
        "long" | "longGeneric" => TimeZoneName::LongSpecific,
        "shortOffset" => TimeZoneName::ShortOffset,
        "longOffset" => TimeZoneName::LongOffset,
        _ => TimeZoneName::ShortSpecific,
    });
    bag.to_composite_field_set()
}

/// ECMA-402's `dateStyle` and `timeStyle`, as CLDR's lengths.
fn styles(date: Option<&str>, time: Option<&str>) -> FieldSetBuilder {
    let length = |style: &str| match style {
        "full" | "long" => Length::Long,
        "medium" => Length::Medium,
        _ => Length::Short,
    };
    let mut builder = FieldSetBuilder::new();
    builder.length = date.or(time).map(length);
    builder.date_fields = date.map(|style| if style == "full" { DateFields::YMDE } else { DateFields::YMD });
    builder.time_precision = time.map(|style| if style == "short" { TimePrecision::Minute } else { TimePrecision::Second });
    builder.zone_style = time.and_then(|style| match style {
        "full" => Some(ZoneStyle::SpecificLong),
        "long" => Some(ZoneStyle::SpecificShort),
        _ => None,
    });
    builder
}

/// Intl.PluralRules' category for a number, rounded as its options say.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Plurals {
    #[qjs(skip_trace)]
    rules: PluralRules,
    #[qjs(skip_trace)]
    digits: Digits,
}

#[rquickjs::methods]
impl Plurals {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, ordinal: bool, digits: String) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let kind = if ordinal { PluralRuleType::Ordinal } else { PluralRuleType::Cardinal };
        let rules = PluralRules::try_new((&locale).into(), PluralRulesOptions::default().with_type(kind)).map_err(|error| range(&ctx, error))?;
        Ok(Plurals { rules, digits: serde_json::from_str(&digits).map_err(|error| range(&ctx, error))? })
    }

    pub fn select(&self, ctx: Ctx<'_>, value: Value<'_>) -> Result<String> {
        Ok(name(self.rules.category_for(&self.digits.apply(decimal(&ctx, &value)?))).into())
    }

    pub fn categories(&self) -> Vec<String> {
        self.rules.categories().map(|category| name(category).into()).collect()
    }
}

fn name(category: PluralCategory) -> &'static str {
    match category {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

/// Intl.RelativeTimeFormat's formatting, for one unit.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Relative {
    #[qjs(skip_trace)]
    formatter: RelativeTimeFormatter,
}

#[rquickjs::methods]
impl Relative {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, unit: String, style: String, numeric: String) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let prefs = (&locale).into();
        let mut options = RelativeTimeFormatterOptions::default();
        options.numeric = Some(if numeric == "auto" { Numeric::Auto } else { Numeric::Always });
        use RelativeTimeFormatter as F;
        let formatter = match (style.as_str(), unit.as_str()) {
            ("short", "second") => F::try_new_short_second(prefs, options),
            ("short", "minute") => F::try_new_short_minute(prefs, options),
            ("short", "hour") => F::try_new_short_hour(prefs, options),
            ("short", "day") => F::try_new_short_day(prefs, options),
            ("short", "week") => F::try_new_short_week(prefs, options),
            ("short", "month") => F::try_new_short_month(prefs, options),
            ("short", "quarter") => F::try_new_short_quarter(prefs, options),
            ("short", "year") => F::try_new_short_year(prefs, options),
            ("narrow", "second") => F::try_new_narrow_second(prefs, options),
            ("narrow", "minute") => F::try_new_narrow_minute(prefs, options),
            ("narrow", "hour") => F::try_new_narrow_hour(prefs, options),
            ("narrow", "day") => F::try_new_narrow_day(prefs, options),
            ("narrow", "week") => F::try_new_narrow_week(prefs, options),
            ("narrow", "month") => F::try_new_narrow_month(prefs, options),
            ("narrow", "quarter") => F::try_new_narrow_quarter(prefs, options),
            ("narrow", "year") => F::try_new_narrow_year(prefs, options),
            (_, "second") => F::try_new_long_second(prefs, options),
            (_, "minute") => F::try_new_long_minute(prefs, options),
            (_, "hour") => F::try_new_long_hour(prefs, options),
            (_, "day") => F::try_new_long_day(prefs, options),
            (_, "week") => F::try_new_long_week(prefs, options),
            (_, "month") => F::try_new_long_month(prefs, options),
            (_, "quarter") => F::try_new_long_quarter(prefs, options),
            (_, "year") => F::try_new_long_year(prefs, options),
            _ => return Err(range(&ctx, format!("Invalid unit argument: {unit}"))),
        }
        .map_err(|error| range(&ctx, error))?;
        Ok(Relative { formatter })
    }

    pub fn format(&self, ctx: Ctx<'_>, value: Value<'_>) -> Result<String> {
        Ok(self.parts(&ctx, &value)?.text())
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> Result<rquickjs::Array<'js>> {
        self.parts(&ctx, &value)?.into_js(&ctx)
    }
}

impl Relative {
    fn parts(&self, ctx: &Ctx, value: &Value) -> Result<Parts> {
        let digits = Digits { maximum_fraction_digits: Some(3), ..Default::default() };
        Ok(Parts::of(&self.formatter.format(digits.apply(decimal(ctx, value)?))))
    }
}

/// Intl.ListFormat's formatting.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Lists {
    #[qjs(skip_trace)]
    formatter: ListFormatter,
}

#[rquickjs::methods]
impl Lists {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, kind: String, style: String) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let length = match style.as_str() {
            "short" => ListLength::Short,
            "narrow" => ListLength::Narrow,
            _ => ListLength::Wide,
        };
        let options = ListFormatterOptions::default().with_length(length);
        let prefs = (&locale).into();
        let formatter = match kind.as_str() {
            "disjunction" => ListFormatter::try_new_or(prefs, options),
            "unit" => ListFormatter::try_new_unit(prefs, options),
            _ => ListFormatter::try_new_and(prefs, options),
        }
        .map_err(|error| range(&ctx, error))?;
        Ok(Lists { formatter })
    }

    pub fn format(&self, items: Vec<String>) -> String {
        Parts::of(&self.formatter.format(items.iter())).text()
    }

    #[qjs(rename = "formatToParts")]
    pub fn format_to_parts<'js>(&self, ctx: Ctx<'js>, items: Vec<String>) -> Result<rquickjs::Array<'js>> {
        Parts::of(&self.formatter.format(items.iter())).into_js(&ctx)
    }
}

/// Intl.Collator's comparison.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Collation {
    #[qjs(skip_trace)]
    collator: CollatorBorrowed<'static>,
}

#[rquickjs::methods]
impl Collation {
    /// `tag` carries its `kn` (numeric) and `kf` (case first) as its own Unicode extension keys.
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, tag: String, sensitivity: String, ignore_punctuation: bool) -> Result<Self> {
        let locale = locale(&ctx, &tag)?;
        let mut options = CollatorOptions::default();
        options.strength = Some(match sensitivity.as_str() {
            "base" | "case" => Strength::Primary,
            "accent" => Strength::Secondary,
            _ => Strength::Tertiary,
        });
        if sensitivity == "case" {
            options.case_level = Some(CaseLevel::On);
        }
        if ignore_punctuation {
            options.alternate_handling = Some(AlternateHandling::Shifted);
        }
        let collator = Collator::try_new((&locale).into(), options).map_err(|error| range(&ctx, error))?;
        Ok(Collation { collator })
    }

    pub fn compare(&self, a: String, b: String) -> i32 {
        self.collator.compare(&a, &b) as i32
    }
}

/// Intl.Segmenter's breaks, as UTF-16 indices, as a JS string counts.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct Segments {
    #[qjs(skip_trace)]
    granularity: String,
}

#[rquickjs::methods]
impl Segments {
    #[qjs(constructor)]
    pub fn new(granularity: String) -> Self {
        Segments { granularity }
    }

    /// Where each segment of `text` ends (`ends`), and for words whether each is one (`words`, `isWordLike`).
    pub fn segment<'js>(&self, ctx: Ctx<'js>, text: String) -> Result<Object<'js>> {
        let (ends, words) = self.ends(&text);
        let segments = Object::new(ctx)?;
        segments.set("ends", ends)?;
        segments.set("words", words)?;
        Ok(segments)
    }
}

impl Segments {
    fn ends(&self, text: &str) -> (Vec<usize>, Vec<bool>) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        match self.granularity.as_str() {
            "word" => {
                let segmenter = WordSegmenter::new_for_non_complex_scripts(Default::default());
                let mut breaks = segmenter.segment_utf16(&utf16);
                let (mut ends, mut words) = (vec![], vec![]);
                while let Some(end) = breaks.next() {
                    if end > 0 {
                        ends.push(end);
                        words.push(breaks.is_word_like());
                    }
                }
                (ends, words)
            }
            "sentence" => (SentenceSegmenter::new(Default::default()).segment_utf16(&utf16).filter(|end| *end > 0).collect(), vec![]),
            _ => (GraphemeClusterSegmenter::new().segment_utf16(&utf16).filter(|end| *end > 0).collect(), vec![]),
        }
    }
}
