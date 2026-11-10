//! What the network's use of Cloudflare costs, estimated when it's read from the usage the telemetry Worker records each
//! hour (cloudflare/telemetry/usage.js). The one place Cloudflare's prices are.

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

/// What each product cost over the `since`–`now` range of `metrics` (the `metrics` rows of the service `CLOUDFLARE`),
/// at list prices less that range's share of each monthly allowance, most first: `(product, dollars)`.
pub fn estimate(metrics: &[Value], since: u64, now: u64) -> Vec<(&'static str, f64)> {
    let range = now.saturating_sub(since).max(1) as f64;
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

/// `estimate` as lines to read, honest that it is one, or none if there's no usage in the range.
pub fn said(metrics: &[Value], since: u64, now: u64, range: &str) -> Option<String> {
    let products = estimate(metrics, since, now);
    if products.is_empty() {
        return None;
    }
    let a_month = MONTH / now.saturating_sub(since).max(1) as f64;
    let width = products.iter().map(|(p, _)| p.len()).max().unwrap_or(0);
    let lines: Vec<String> = products.iter().map(|(product, dollars)| format!("  {product:<width$}  ${dollars:.2}  (${:.2} a month at this rate)", dollars * a_month)).collect();
    Some(format!("Cloudflare, estimated for {range} from what the network used, at list prices less its share of the free allowances (the account's other use shares them too):\n{}", lines.join("\n")))
}
