//! What the network's use of Cloudflare costs, estimated when it's read from the usage the telemetry Worker records each
//! hour (insights/worker/usage.js). The one place Cloudflare's prices are.

use std::collections::BTreeMap;

use serde_json::Value;

/// The service Cloudflare's usage is recorded as: only an admin's to read.
pub const CLOUDFLARE: &str = "cloudflare";

/// What one of Cloudflare's metrics is billed at: `each` dollars a unit past `free` units a month, where a row's unit is
/// `counts` of one, by its attrs. A `gauge`'s row is an hour of what it held, billed by the month.
pub struct Price {
    pub product: &'static str,
    pub metric: &'static str,
    pub counts: fn(&Value) -> f64,
    each: f64,
    free: f64,
    gauge: bool,
}

const fn price(product: &'static str, metric: &'static str, counts: fn(&Value) -> f64, each: f64, free: f64, gauge: bool) -> Price {
    Price { product, metric, counts, each, free, gauge }
}

const MILLION: f64 = 1e6;
const GB: f64 = 1e9;
/// A Durable Object is billed as 128 MB for as long as it's active: a GB-s is 8 of its seconds.
const DO_SECONDS_A_GB_S: f64 = 8.0;

fn all(_: &Value) -> f64 {
    1.0
}

fn class_a(attrs: &Value) -> f64 {
    f64::from(attrs["cloudflare.r2.class"] == "A")
}

fn class_b(attrs: &Value) -> f64 {
    f64::from(attrs["cloudflare.r2.class"] == "B")
}

/// An incoming WebSocket message is billed as a twentieth of a request.
fn do_requests(attrs: &Value) -> f64 {
    if attrs["cloudflare.durable_objects.type"] == "hibernation" { 1.0 / 20.0 } else { 1.0 }
}

/// Cloudflare's published unit prices and the monthly allowances the Workers Paid plan includes, as they were on
/// cloudflare.com on 2026-11-10 (developers.cloudflare.com/{workers,r2,durable-objects,basin-pipelines,basin-sql,
/// basin-catalog}/…/pricing). Workers Paid's own $5 a month isn't usage, and isn't here.
pub const PRICES: &[Price] = &[
    price("Workers", "cloudflare.workers.requests", all, 0.30 / MILLION, 10.0 * MILLION, false),
    price("Workers", "cloudflare.workers.cpu_time", all, 0.02 / MILLION, 30.0 * MILLION, false),
    price("R2", "cloudflare.r2.operations", class_a, 4.50 / MILLION, MILLION, false),
    price("R2", "cloudflare.r2.operations", class_b, 0.36 / MILLION, 10.0 * MILLION, false),
    price("R2", "cloudflare.r2.storage", all, 0.015 / GB, 10.0 * GB, true),
    price("Durable Objects", "cloudflare.durable_objects.requests", do_requests, 0.15 / MILLION, MILLION, false),
    price("Durable Objects", "cloudflare.durable_objects.duration", all, 12.50 / MILLION / DO_SECONDS_A_GB_S, 400_000.0 * DO_SECONDS_A_GB_S, false),
    price("Basin Pipelines", "cloudflare.pipelines.processed", all, 0.04 / GB, 50.0 * GB, false),
    price("Basin Pipelines", "cloudflare.pipelines.delivered", all, 0.06 / GB, 50.0 * GB, false),
    price("Basin SQL", "cloudflare.sql.scanned", all, 0.0025 / GB, 10.0 * GB, false),
    price("Basin Catalog", "cloudflare.catalog.operations", all, 9.00 / MILLION, MILLION, false),
];

const HOUR: f64 = 3_600_000.0;
/// Cloudflare's billing month.
const MONTH: f64 = 30.0 * 24.0 * HOUR;

/// The hours `metrics` (the `metrics` rows of the service `CLOUDFLARE`) recorded: what an estimate is of, as an hour
/// that wasn't recorded is one nothing's known of.
fn hours(metrics: &[Value]) -> usize {
    metrics.iter().filter_map(|r| r["hour"].as_u64()).collect::<std::collections::BTreeSet<_>>().len()
}

/// What each product cost in the hours `metrics` recorded, at list prices less those hours' share of each monthly
/// allowance, most first: `(product, dollars)`.
pub fn estimate(metrics: &[Value]) -> Vec<(&'static str, f64)> {
    let range = hours(metrics) as f64 * HOUR;
    let mut products: BTreeMap<&str, f64> = BTreeMap::new();
    for p in PRICES {
        let rows: Vec<&Value> = metrics.iter().filter(|r| r["name"] == p.metric).collect();
        if rows.is_empty() {
            continue;
        }
        let used = rows.iter().map(|r| r["sum"].as_f64().unwrap_or(0.0) * (p.counts)(&r["attrs"])).sum::<f64>() * if p.gauge { HOUR / MONTH } else { 1.0 };
        *products.entry(p.product).or_default() += (used - p.free * range / MONTH).max(0.0) * p.each;
    }
    let mut out: Vec<(&str, f64)> = products.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

/// `estimate` as lines to read, honest that it is one, or none if nothing was recorded in `range` (as it's said: "the last 1d").
pub fn said(metrics: &[Value], range: &str) -> Option<String> {
    let products = estimate(metrics);
    let hours = hours(metrics);
    if products.is_empty() {
        return None;
    }
    let a_month = MONTH / (hours as f64 * HOUR);
    let width = products.iter().map(|(p, _)| p.len()).max().unwrap_or(0);
    let lines: Vec<String> = products.iter().map(|(product, dollars)| format!("  {product:<width$}  ${dollars:.2}  (${:.2} a month at this rate)", dollars * a_month)).collect();
    Some(format!("Cloudflare, estimated from the {hours} hour{} of {range} recorded, at list prices less their share of the free allowances (the account's other use shares them too):\n{}", if hours == 1 { "" } else { "s" }, lines.join("\n")))
}

/// What AI Gateway's calls spent, from what its token metric (`insights::GEN_AI_TOKENS`) recorded, by app, user and
/// model: each priced when it's read, at its hour's list price, by `genai-prices`, Pydantic's table of every company's
/// prices, embedded at its release. A model on one of the network's devices costs nothing; one the table doesn't know
/// has its tokens and no price, never a guess.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiSpend {
    pub app: String,
    pub user: String,
    pub model: String,
    pub input: f64,
    pub output: f64,
    /// None for a model on the network's devices (`local`) or one with no price known.
    pub dollars: Option<f64>,
    pub on_devices: bool,
}

/// The table's company for a model id's first part, and the model's id there.
fn priced_as(model: &str) -> Option<(&'static str, &str)> {
    let (first, rest) = model.split_once('/')?;
    let company = match first {
        "openai" => "openai",
        "google" => "google",
        "anthropic" => "anthropic",
        "openrouter" => "openrouter",
        "workers-ai" => "cloudflare",
        _ => return None,
    };
    Some((company, rest))
}

/// What `input` and `output` tokens of `model` cost in the hour `hour` (hours since 1970), at `tier`.
fn ai_price(model: &str, tier: Option<&str>, hour: u64, input: f64, output: f64) -> Option<f64> {
    use genai_prices::{PriceRequest, Usage, UsageKey};
    let (company, id) = priced_as(model)?;
    let usage = Usage::from([(UsageKey::INPUT_TOKENS, input), (UsageKey::OUTPUT_TOKENS, output)]);
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_millis(hour * HOUR as u64);
    let mut asked = PriceRequest::new(id, &usage).with_provider_id(company).with_timestamp(at);
    if let Some(tier) = tier {
        asked = asked.with_price_context("service_tier", tier);
    }
    genai_prices::calculate(&asked).ok().map(|p| p.total_price)
}

/// `metrics`' AI token rows (any service's), priced: one entry for each app, user and model, most spent first.
pub fn ai(metrics: &[Value]) -> Vec<AiSpend> {
    let mut by: BTreeMap<(String, String, String), AiSpend> = BTreeMap::new();
    for row in metrics.iter().filter(|r| r["name"] == crate::GEN_AI_TOKENS.name) {
        let attrs = &row["attrs"];
        let text = |key: &str| attrs[key].as_str().unwrap_or("").to_string();
        let (app, user, model) = (row["service"].as_str().unwrap_or("").to_string(), text("enduser.id"), text("gen_ai.request.model"));
        let n = row["sum"].as_f64().unwrap_or(0.0);
        let (input, output) = if attrs["gen_ai.token.type"] == "output" { (0.0, n) } else { (n, 0.0) };
        let on_devices = attrs["one.paid"] == "device";
        let dollars = if on_devices { None } else { ai_price(&model, attrs["one.tier"].as_str(), row["hour"].as_u64().unwrap_or(0), input, output) };
        let spend = by.entry((app.clone(), user.clone(), model.clone())).or_insert_with(|| AiSpend { app, user, model, on_devices, ..AiSpend::default() });
        spend.input += input;
        spend.output += output;
        if let Some(d) = dollars {
            spend.dollars = Some(spend.dollars.unwrap_or(0.0) + d);
        }
    }
    let mut out: Vec<AiSpend> = by.into_values().collect();
    out.sort_by(|a, b| b.dollars.unwrap_or(0.0).total_cmp(&a.dollars.unwrap_or(0.0)).then(b.input.total_cmp(&a.input)));
    out
}

/// `ai` as lines to read, by app, by model and by user (named by `name`), or none if no call was recorded in `range`.
pub fn ai_said(metrics: &[Value], range: &str, name: impl Fn(&str) -> String) -> Option<String> {
    let spent = ai(metrics);
    if spent.is_empty() {
        return None;
    }
    let tokens = |n: f64| if n >= 1e6 { format!("{:.1}M", n / 1e6) } else if n >= 1e3 { format!("{:.0}k", n / 1e3) } else { format!("{n:.0}") };
    let group = |title: &str, key: &dyn Fn(&AiSpend) -> String| {
        let mut sums: BTreeMap<String, (f64, f64, Option<f64>, bool)> = BTreeMap::new();
        for s in &spent {
            let e = sums.entry(key(s)).or_insert((0.0, 0.0, None, true));
            e.0 += s.input;
            e.1 += s.output;
            if let Some(d) = s.dollars {
                e.2 = Some(e.2.unwrap_or(0.0) + d);
            }
            e.3 &= s.on_devices;
        }
        let mut rows: Vec<(String, (f64, f64, Option<f64>, bool))> = sums.into_iter().collect();
        rows.sort_by(|a, b| b.1.2.unwrap_or(0.0).total_cmp(&a.1.2.unwrap_or(0.0)));
        let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        let lines: Vec<String> = rows
            .iter()
            .map(|(k, (i, o, d, devices))| {
                let cost = match (d, devices) {
                    (_, true) => "on your devices".to_string(),
                    (Some(d), _) => format!("${d:.2}"),
                    (None, _) => "no price known".to_string(),
                };
                format!("    {k:<width$}  {cost}  ({} in, {} out)", tokens(*i), tokens(*o))
            })
            .collect();
        format!("  {title}\n{}", lines.join("\n"))
    };
    let parts = [
        group("by app", &|s| s.app.clone()),
        group("by model", &|s| s.model.clone()),
        group("by user", &|s| if s.user.is_empty() { "(none)".to_string() } else { name(&s.user) }),
    ];
    Some(format!("AI, at list prices when each call was made ({range}; free allowances, like Workers AI's daily neurons, not taken off):\n{}", parts.join("\n")))
}
