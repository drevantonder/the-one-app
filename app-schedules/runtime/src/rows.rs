//! How schedules ran, as devices say it: each schedule's latest runs in a scope. Rows of that scope's node database
//! `app-schedules`, changed in place, not steps of their own: written often, and needed only as they are now. Each device
//! reads them back into its `State`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use api::Caller;
use api::call::Event;
use api::runtime::prelude::*;
use key::{Key, hex, unhex};
use scope::scopes::Scope;

use crate::State;

/// Its database, in each scope, the runtime's own.
const DATABASE: &str = "app-schedules";

/// Its schema, as its changes, oldest first.
const SCHEMA: &[&str] = &["
create table runs (app text, schedule text, slot integer, time integer not null, device text not null, user text not null,
                   ok integer not null, error text, ms integer not null, spec text, primary key (app, schedule, slot));
"];

/// How many runs of each schedule it keeps.
const KEPT: usize = 20;

/// How often it reads every scope's runs again, for what other devices said.
const READ_EVERY: u64 = 60_000;

/// A scope's runs, as this device last read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Runs {
    /// Each schedule's latest runs, oldest first, by (app, schedule).
    pub runs: BTreeMap<(String, String), Vec<Run>>,
    /// When a schedule's runs first followed a different definition, by (app, schedule): (was, is, when).
    pub changed: BTreeMap<(String, String), (Value, Value, u64)>,
}

/// One run of a schedule: the slot it ran for, when, on which device, as whom, how it went, and the definition it
/// followed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub slot: u64,
    pub time: u64,
    pub device: Key,
    pub user: String,
    pub ok: bool,
    pub error: Option<String>,
    pub ms: u64,
    pub spec: Value,
}

/// Writes a schedule's run in its scope, then reads the scope's runs back.
pub(crate) async fn ran(rt: &Rt, scope: &Scope, app: &str, schedule: &str, run: &Run) -> Result<()> {
    let me = rt.caller()?;
    copy_once(rt, &me, scope).await;
    let ex = |c: &rusqlite::Connection, sql: &str, params: &[Value]| database::stored::execute(c, sql, params).map(|_| ());
    let mut change = |c: &rusqlite::Connection| -> Result<()> {
        ex(c, "insert or replace into runs values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", &[json!(app), json!(schedule), json!(run.slot), json!(run.time), json!(hex(&run.device)), json!(run.user), json!(run.ok), json!(run.error), json!(run.ms), json!(run.spec.to_string())])?;
        ex(c, "delete from runs where app = ? and schedule = ? and slot not in (select slot from runs where app = ? and schedule = ? order by slot desc limit ?)", &[json!(app), json!(schedule), json!(app), json!(schedule), json!(KEPT)])
    };
    rt.write(&me, scope, key::APP, DATABASE, SCHEMA, &mut change).await?;
    read(rt, scope).await
}

/// Reads every scope's runs this device holds again, once a while has passed: for what other devices said.
pub(crate) async fn read_all(rt: &Rt) {
    let now = custom_build::now_ms();
    {
        let mut read = rt.state::<State>().read.lock().unwrap();
        if now < *read + READ_EVERY {
            return;
        }
        *read = now;
    }
    let scopes = rt.read_or(Vec::new(), |r| Ok(r.db().scopes()?.into_iter().filter(|s| r.holds(s)).collect::<Vec<_>>()));
    for scope in scopes {
        if let Err(err) = read(rt, &scope).await {
            tracing::debug!("reading {scope}'s schedules' runs: {err:#}");
        }
    }
}

/// Reads a scope's runs as the network has them now into its state, telling whoever watches if they changed.
async fn read(rt: &Rt, scope: &Scope) -> Result<()> {
    let me = rt.caller()?;
    copy_once(rt, &me, scope).await;
    let db = rt.open(&me, scope, key::APP, DATABASE, SCHEMA).await?;
    let rows = db.lock().await.query("select * from runs order by slot", &[])?;
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let mut s = Runs::default();
    for r in rows {
        let key = (text(&r["app"]), text(&r["schedule"]));
        let run = Run {
            slot: n(&r["slot"]),
            time: n(&r["time"]),
            device: unhex(r["device"].as_str().unwrap_or("")).context("a device in a schedule's runs isn't a key: they can't be read")?,
            user: text(&r["user"]),
            ok: r["ok"] == 1,
            error: r["error"].as_str().map(str::to_string),
            ms: n(&r["ms"]),
            spec: serde_json::from_str(r["spec"].as_str().unwrap_or("null")).context("a schedule's run's definition isn't JSON: its runs can't be read")?,
        };
        let runs = s.runs.entry(key.clone()).or_default();
        if let Some(last) = runs.last().filter(|l| !l.spec.is_null() && !run.spec.is_null() && l.spec != run.spec) {
            s.changed.insert(key, (last.spec.clone(), run.spec.clone(), run.time));
        }
        runs.push(run);
    }
    let changed = rt.state::<State>().rows.lock().unwrap().insert(scope.clone(), s.clone()).is_none_or(|was| was != s);
    if changed {
        rt.emit(Event::Changed { apps: Vec::new() });
    }
    Ok(())
}

/// Builds before these rows had a database of their own kept them in the node's `main`: the first device on this build
/// copies a scope's here, once, keeping any already here.
async fn copy_once(rt: &Rt, me: &Caller, scope: &Scope) {
    if rt.state::<State>().copied.lock().unwrap().contains(scope) {
        return;
    }
    let copied = async {
        let ours = rt.open(me, scope, key::APP, DATABASE, SCHEMA).await?;
        if !ours.lock().await.query("select 1 from runs limit 1", &[])?.is_empty() {
            return Ok(());
        }
        let main = rt.open(me, scope, key::APP, key::APP_DATABASE, &[]).await?;
        let old = main.lock().await.query("select * from runs", &[]).unwrap_or_default();
        if old.is_empty() {
            return Ok(());
        }
        let mut change = |c: &rusqlite::Connection| -> Result<()> {
            for r in &old {
                let columns: Vec<&str> = r.keys().map(String::as_str).collect();
                let sql = format!("insert or ignore into runs ({}) values ({})", columns.join(", "), vec!["?"; columns.len()].join(", "));
                database::stored::execute(c, &sql, &r.values().cloned().collect::<Vec<_>>())?;
            }
            Ok(())
        };
        rt.write(me, scope, key::APP, DATABASE, SCHEMA, &mut change).await
    };
    match copied.await {
        Ok(()) => drop(rt.state::<State>().copied.lock().unwrap().insert(scope.clone())),
        Err(err) => tracing::debug!("copying {scope}'s schedules' runs from the node's main database: {err:#}"),
    }
}
