//! `insights` as someone reads it: a few lines, added up as the Insights screen adds them, from what the tool gives. It
//! knows OTEL's shapes (a span's kind, a histogram's buckets, a trace's ids) and no particular signal.

use std::collections::BTreeMap;

use serde_json::Value;

/// The most lines a list has unless asked for more.
pub const LISTED: usize = 20;
/// What a fingerprint's `status` is when something broke, which is all that's listed unless other's asked for.
pub const ERROR: &str = "error";
/// A histogram's bucket bounds, OTEL's defaults and, for what's over them all, a minute to draw it as. The same as the
/// screen's (`src/screen/settings/quantile.js`), held to it by tests/contracts.rs.
pub const BOUNDS: [f64; 16] = [0.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 2500.0, 5000.0, 7500.0, 10000.0, 60000.0];

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// The quantile of histograms added together, interpolated within its bucket as Prometheus does.
pub fn quantile(rows: &[&Value], q: f64) -> Option<f64> {
    let counts: Vec<u64> = (0..BOUNDS.len()).map(|i| rows.iter().map(|r| r["buckets"][i].as_u64().unwrap_or(0)).sum()).collect();
    let total = counts.iter().sum::<u64>() as f64;
    if total == 0.0 {
        return None;
    }
    let (mut seen, mut low) = (0.0, 0.0);
    for (count, high) in counts.iter().map(|c| *c as f64).zip(BOUNDS) {
        if count > 0.0 && seen + count >= q * total {
            return Some(low + (q * total - seen) / count * (high - low));
        }
        seen += count;
        low = high;
    }
    Some(low)
}

/// What requests added up to, by the hour.
fn requests(rows: &Value) -> Vec<&Value> {
    rows["requests"].as_array().into_iter().flatten().collect()
}

/// What requests add up to: a line of a table, its requests, error rate and P95.
struct Line {
    name: String,
    requests: u64,
    errors: u64,
    p95: Option<f64>,
}

fn lines(rows: Vec<&Value>, name: fn(&Value) -> String) -> Vec<Line> {
    let mut by: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for row in rows {
        by.entry(name(row)).or_default().push(row);
    }
    let mut lines: Vec<Line> = by.into_iter().map(|(name, rows)| Line { name, requests: rows.iter().map(|r| r["count"].as_u64().unwrap_or(0)).sum(), errors: rows.iter().map(|r| r["errors"].as_u64().unwrap_or(0)).sum(), p95: quantile(&rows, 0.95) }).collect();
    lines.sort_by_key(|l| std::cmp::Reverse(l.requests));
    lines
}

fn plural(n: u64, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

fn rate(line: &Line) -> String {
    match (line.requests, line.errors) {
        (0, _) => "–".into(),
        (_, 0) => "0%".into(),
        (requests, errors) => format!("{:.1}% ({})", errors as f64 * 100.0 / requests as f64, plural(errors, "error")),
    }
}

fn took(ms: Option<f64>) -> String {
    match ms {
        None => "–".into(),
        Some(ms) if ms >= 60_000.0 => format!("{:.1}m", ms / 60_000.0),
        Some(ms) if ms >= 1000.0 => format!("{:.1}s", ms / 1000.0),
        Some(ms) => format!("{ms:.0}ms"),
    }
}

fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        d if d < MINUTE => "just now".into(),
        d if d < HOUR => format!("{}m ago", d / MINUTE),
        d if d < DAY => format!("{}h ago", d / HOUR),
        d => format!("{}d ago", d / DAY),
    }
}

/// `some`'s lines first, then how many are left out.
fn listed<T>(some: &[T], limit: usize, line: impl Fn(&T) -> String) -> String {
    let mut out: Vec<String> = some.iter().take(limit).map(|x| format!("  {}", line(x))).collect();
    if some.len() > limit {
        out.push(format!("  and {} more", some.len() - limit));
    }
    out.join("\n")
}

/// What failed with `status`: real errors unless asked for what wasn't one.
fn errors(rows: &Value, status: &str, since: u64, now: u64, limit: usize) -> Option<String> {
    let mut failed: Vec<&Value> = rows["errors"].as_array().into_iter().flatten().filter(|p| p["status"] == status && p["last"].as_u64() >= Some(since)).collect();
    if failed.is_empty() {
        return None;
    }
    failed.sort_by_key(|p| std::cmp::Reverse(p["count"].as_u64()));
    let text = |row: &Value, key: &str| row[key].as_str().unwrap_or("").to_string();
    let lines = listed(&failed, limit, |p| {
        let said = p["message"].as_str().map(|m| format!("\n    {m}")).unwrap_or_default();
        format!("{} {}: {} at {}, {}, last {}{said}", text(p, "service"), text(p, "name"), text(p, "type"), text(p, "frame"), plural(p["count"].as_u64().unwrap_or(0), "time"), ago(now, p["last"].as_u64().unwrap_or(0)))
    });
    Some(format!("{}:\n{lines}", if status == ERROR { "Top errors" } else { "Not errors" }))
}

fn since_said(since: u64, now: u64) -> String {
    format!("the last {}", ago(now, since).trim_end_matches(" ago").replace("just now", "moment"))
}

fn table(lines: &[Line], limit: usize, label: &str) -> String {
    let width = lines.iter().take(limit).map(|l| l.name.len()).max().unwrap_or(0).max(label.len());
    let rows = listed(lines, limit, |l| format!("{:<width$}  {:>7}  {:<16}  {}", l.name, l.requests, rate(l), took(l.p95)));
    format!("  {label:<width$}  {:>7}  {:<16}  P95\n{rows}", "requests", "errors")
}

/// What's going on across the network: each service, and the top errors.
pub fn overview(rows: &Value, status: &str, since: u64, now: u64, limit: usize) -> String {
    let services = lines(requests(rows), |r| r["service"].as_str().unwrap_or("").to_string());
    if services.is_empty() {
        return format!("Nothing recorded in {}.", since_said(since, now));
    }
    let (count, failed) = services.iter().fold((0, 0), |(c, f), l| (c + l.requests, f + l.errors));
    let mut out = format!("{} in {}, {:.1}% failed.\n\n{}", plural(count, "request"), since_said(since, now), failed as f64 * 100.0 / count.max(1) as f64, table(&services, limit, "service"));
    if let Some(errors) = errors(rows, status, since, now, limit) {
        out.push_str("\n\n");
        out.push_str(&errors);
    }
    out.push_str("\n\nMore: insights --service <service>, then --trace-id <id>.");
    out
}

/// One service: its operations, and its failing and slow traces.
pub fn service(rows: &Value, service: &str, status: &str, since: u64, now: u64, limit: usize) -> String {
    let operations = lines(requests(rows), |r| r["name"].as_str().unwrap_or("").to_string());
    if operations.is_empty() {
        return format!("{service} answered no requests in {}.", since_said(since, now));
    }
    let mut out = format!("{service}, {}:\n\n{}", since_said(since, now), table(&operations, limit, "operation"));
    if let Some(errors) = errors(rows, status, since, now, limit) {
        out.push_str("\n\n");
        out.push_str(&errors);
    }
    let worth: Vec<&Value> = rows["spans"].as_array().into_iter().flatten().filter(|s| s["kind"] == "server" && (s["status"] == ERROR || s["duration"].as_u64().is_some_and(|d| d >= super::SLOW_MS))).collect();
    if !worth.is_empty() {
        let shown = listed(&worth, limit, |s| format!("{}  {}  {}  {}  {}", s["trace"].as_str().unwrap_or(""), s["name"].as_str().unwrap_or(""), took(s["duration"].as_f64()), s["status"].as_str().unwrap_or(""), ago(now, s["time"].as_u64().unwrap_or(0))));
        out.push_str(&format!("\n\nFailing or slow:\n{shown}"));
    }
    out
}

/// One trace, each span under the one that made it.
pub fn trace(rows: &Value, id: &str, limit: usize) -> String {
    let mut spans: Vec<&Value> = rows["spans"].as_array().into_iter().flatten().filter(|s| s["trace"] == id).collect();
    if spans.is_empty() {
        return format!("No trace {id}: nothing you may see has that id, or it hasn't arrived yet.");
    }
    spans.sort_by_key(|s| s["time"].as_u64());
    let ids: Vec<&str> = spans.iter().filter_map(|s| s["span"].as_str()).collect();
    let is_root = |s: &&Value| !s["parent"].as_str().is_some_and(|p| ids.contains(&p));
    let roots: Vec<&Value> = spans.iter().copied().filter(|s| is_root(s)).collect();
    let mut out = vec![format!("Trace {id}:")];
    for root in roots {
        add_span(root, &spans, 0, limit, &mut out);
    }
    if out.len() > limit {
        let more = out.len() - limit - 1;
        out.truncate(limit + 1);
        out.push(format!("  and {more} more"));
    }
    out.join("\n")
}

fn add_span(span: &Value, spans: &[&Value], depth: usize, limit: usize, out: &mut Vec<String>) {
    if out.len() > limit + 1 {
        return;
    }
    let attr = |key: &str| span["attrs"][key].as_str().map(str::to_string);
    let mut said = vec![took(span["duration"].as_f64()), span["status"].as_str().unwrap_or("").to_string()];
    said.extend(span["kind"].as_str().map(str::to_string));
    said.extend(attr("error.type"));
    said.extend(attr("one.result"));
    out.push(format!("{}{} {} ({})", "  ".repeat(depth + 1), span["service"].as_str().unwrap_or(""), span["name"].as_str().unwrap_or(""), [said.join(", "), span["device"].as_str().unwrap_or("").to_string()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(", on ")));
    let mine = span["span"].as_str();
    for child in spans.iter().filter(|s| s["parent"].as_str() == mine) {
        add_span(child, spans, depth + 1, limit, out);
    }
}
