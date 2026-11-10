//! A batch written to a scope's node database, and what's past its time taken out in the same write.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::database::stored::execute;

use super::{Batch, RAW_KEPT_MS, ROLLUPS_KEPT_MS};

/// Writes `batch`. A fingerprint goes once its build is out of `live`, the builds and app bundles still running, or a
/// migrated one, which names none, as a rollup would. With nothing known to be live, none goes.
pub fn write(c: &Connection, batch: &Batch, now: u64, live: &[String]) -> Result<()> {
    let ex = |sql: &str, params: &[Value]| execute(c, sql, params).map(|_| ());
    for s in &batch.signals {
        ex(
            "insert into signals values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[json!(s.time), json!(s.device), json!(s.kind.as_str()), json!(s.name), json!(s.app), json!(s.trace), json!(s.span), json!(s.parent), json!(s.ms), json!(if s.ok { "ok" } else { "error" }), json!(s.fault.map(|f| f.as_str())), json!(s.attrs.json().to_string())],
        )?;
    }
    for ((hour, name, app, key), r) in &batch.rollups {
        ex(
            "insert into rollups values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) on conflict (hour, name, app, key) do update set
               count = count + excluded.count, errors = errors + excluded.errors, ms = ms + excluded.ms, under_10 = under_10 + excluded.under_10,
               under_100 = under_100 + excluded.under_100, under_1000 = under_1000 + excluded.under_1000, under_10000 = under_10000 + excluded.under_10000, over = over + excluded.over",
            &[json!(hour), json!(name), json!(app), json!(key), json!(r.count), json!(r.errors), json!(r.ms), json!(r.under[0]), json!(r.under[1]), json!(r.under[2]), json!(r.under[3]), json!(r.under[4])],
        )?;
    }
    for ((app, tool, id), p) in &batch.prints {
        ex(
            "insert into fingerprints values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) on conflict (app, tool, fingerprint) do update set
               count = count + excluded.count, first = min(first, excluded.first), last = max(last, excluded.last)",
            &[json!(app), json!(tool), json!(id), json!(p.print.kind), json!(p.print.frame), json!(p.print.build), json!(p.print.fault.as_str()), json!(p.count), json!(p.first), json!(p.last)],
        )?;
    }
    ex("delete from signals where time < ?", &[json!(now.saturating_sub(RAW_KEPT_MS))])?;
    ex("delete from rollups where hour < ?", &[json!(now.saturating_sub(ROLLUPS_KEPT_MS) / 3_600_000)])?;
    ex("delete from fingerprints where build = '' and last < ?", &[json!(now.saturating_sub(ROLLUPS_KEPT_MS))])?;
    if !live.is_empty() {
        let holes = vec!["?"; live.len()].join(", ");
        ex(&format!("delete from fingerprints where build != '' and build not in ({holes})"), &live.iter().map(|b| json!(b)).collect::<Vec<_>>())?;
    }
    Ok(())
}
