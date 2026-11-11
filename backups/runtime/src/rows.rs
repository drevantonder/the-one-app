//! How backups went, as devices say it: each run to each place, when admins were told of a place's trouble, and each
//! restore check. Rows of `global`'s app runtime database `backups`, changed in place, not steps of their own: written often,
//! and needed only as they are now. Each device reads them back into its `State`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use api::Caller;
use api::call::Event;
use api::runtime::prelude::*;
use key::{Key, hex, unhex};
use scope::scopes::Scope;

use crate::State;

/// Its database, in `global`, the runtime's own.
const DATABASE: &str = "backups";

/// Its schema, as its changes, oldest first.
const SCHEMA: &[&str] = &["
create table backups (place text, device text, time integer, error text, trouble text, snapshots text not null,
                      primary key (place, device, time));
create table alerts (place text primary key, time integer not null);
create table checks (place text, device text, time integer, error text, report text not null, primary key (place, device, time));
", "
alter table backups add column missed text;
"];

/// How many backup runs to each place it keeps, besides each scope's last that backed it up.
const BACKUP_RUNS_KEPT: usize = 100;
const CHECKS_KEPT: usize = 20;

/// The rows, as this device last read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rows {
    /// The latest backup runs, oldest first.
    pub backups: Vec<BackupRun>,
    /// By (place, scope), the newest step's time a snapshot there holds, and when it was written.
    pub covered: BTreeMap<(String, String), (u64, u64)>,
    /// When admins were last told of a place's trouble, by place.
    pub alerted: BTreeMap<String, u64>,
    /// The latest restore checks, oldest first.
    pub checks: Vec<CheckRun>,
}

/// One device's backup run to one place: the snapshots it wrote, or what went wrong with the place; and each scope it
/// couldn't back up while it backed up the rest (`missed`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BackupRun {
    /// The place's Vault item.
    pub place: String,
    pub device: Key,
    pub time: u64,
    pub error: Option<String>,
    /// What kind of trouble the error is, for its fix: `bucket::Trouble`'s name.
    pub trouble: Option<String>,
    pub snapshots: Vec<BackedUp>,
    pub missed: Vec<Missed>,
}

/// A scope a run couldn't back up, and why: one this device couldn't open (it wasn't given its newest key), say.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Missed {
    pub scope: String,
    pub error: String,
}

/// A scope's snapshot in a run: when it holds the scope as of, and what it added.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct BackedUp {
    pub scope: String,
    pub upto: u64,
    pub blobs: u64,
    pub bytes: u64,
}

/// One device's restore check of one place: what came back (`engine::check`'s report, in short), or what's wrong.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CheckRun {
    pub place: String,
    pub device: Key,
    pub time: u64,
    pub error: Option<String>,
    pub report: Value,
}

pub(crate) enum Said {
    BackedUp(BackupRun),
    Alerted { place: String },
    Checked(CheckRun),
}

/// Writes what it says, reads the rows back, and tells whoever watches (Backups on Settings).
pub(crate) async fn say(rt: &Rt, said: Said) -> Result<()> {
    let now = custom_build::now_ms();
    let ex = |c: &rusqlite::Connection, sql: &str, params: &[Value]| database::stored::execute(c, sql, params).map(|_| ());
    let mut change = |c: &rusqlite::Connection| -> Result<()> {
        match &said {
            Said::BackedUp(r) => {
                let missed = (!r.missed.is_empty()).then(|| serde_json::to_string(&r.missed)).transpose()?;
                ex(c, "insert or replace into backups values (?, ?, ?, ?, ?, ?, ?)", &[json!(r.place), json!(hex(&r.device)), json!(r.time), json!(r.error), json!(r.trouble), json!(serde_json::to_string(&r.snapshots)?), json!(missed)])?;
                // Beyond the newest, each scope's last run that backed it up stays, as what's covered is read from
                // them: a scope that hasn't changed would otherwise look a day late once busier ones fill these.
                ex(
                    c,
                    "delete from backups where place = ? and time not in (select time from backups where place = ? order by time desc limit ?) \
                     and time not in (select max(b.time) from backups b, json_each(b.snapshots) j where b.place = ? and b.error is null group by json_extract(j.value, '$.scope'))",
                    &[json!(r.place), json!(r.place), json!(BACKUP_RUNS_KEPT), json!(r.place)],
                )
            }
            Said::Alerted { place } => ex(c, "insert or replace into alerts values (?, ?)", &[json!(place), json!(now)]),
            Said::Checked(r) => {
                ex(c, "insert or replace into checks values (?, ?, ?, ?, ?)", &[json!(r.place), json!(hex(&r.device)), json!(r.time), json!(r.error), json!(r.report.to_string())])?;
                ex(c, "delete from checks where place = ? and time not in (select time from checks where place = ? order by time desc limit ?)", &[json!(r.place), json!(r.place), json!(CHECKS_KEPT)])
            }
        }
    };
    rt.write(&rt.caller()?, &Scope::Global, key::APP, DATABASE, SCHEMA, &mut change).await?;
    read(rt).await
}

/// Reads the rows as the network has them now into its state, telling whoever watches if they changed.
pub(crate) async fn read(rt: &Rt) -> Result<()> {
    let me = rt.caller()?;
    copy_once(rt, &me).await;
    let db = rt.open(&me, &Scope::Global, key::APP, DATABASE, SCHEMA).await?;
    let rows = {
        let db = db.lock().await;
        let mut rows = Rows::default();
        for r in db.query("select * from backups order by time", &[])? {
            let run = BackupRun {
                place: text(&r["place"]),
                device: device(&r["device"])?,
                time: n(&r["time"]),
                error: r["error"].as_str().map(str::to_string),
                trouble: r["trouble"].as_str().map(str::to_string),
                snapshots: stored(&r["snapshots"], "[]")?,
                missed: stored(&r["missed"], "[]")?,
            };
            if run.error.is_none() {
                for b in &run.snapshots {
                    let covered = rows.covered.entry((run.place.clone(), b.scope.clone())).or_default();
                    *covered = (covered.0.max(b.upto), covered.1.max(run.time));
                }
            }
            rows.backups.push(run);
        }
        for r in db.query("select * from alerts", &[])? {
            rows.alerted.insert(text(&r["place"]), n(&r["time"]));
        }
        for r in db.query("select * from checks order by time", &[])? {
            rows.checks.push(CheckRun { place: text(&r["place"]), device: device(&r["device"])?, time: n(&r["time"]), error: r["error"].as_str().map(str::to_string), report: stored(&r["report"], "null")? });
        }
        rows
    };
    let changed = {
        let mut kept = rt.state::<State>().rows.lock().unwrap();
        let changed = *kept != rows;
        *kept = rows;
        changed
    };
    if changed {
        rt.emit(Event::Changed { apps: Vec::new() });
    }
    Ok(())
}

/// Builds before these rows had a database of their own kept them in the app runtime's `main`: the first device on this build
/// copies them here, once, keeping any already here.
async fn copy_once(rt: &Rt, me: &Caller) {
    if rt.state::<State>().copied.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let copied = async {
        let ours = rt.open(me, &Scope::Global, key::APP, DATABASE, SCHEMA).await?;
        if !ours.lock().await.query("select 1 from backups union all select 1 from alerts union all select 1 from checks limit 1", &[])?.is_empty() {
            return Ok(());
        }
        let main = rt.open(me, &Scope::Global, key::APP, key::APP_DATABASE, &[]).await?;
        let old: Vec<(&str, Vec<serde_json::Map<String, Value>>)> = {
            let main = main.lock().await;
            ["backups", "alerts", "checks"].into_iter().map(|t| (t, main.query(&format!("select * from {t}"), &[]).unwrap_or_default())).collect()
        };
        if old.iter().all(|(_, rows)| rows.is_empty()) {
            return Ok(());
        }
        let mut change = |c: &rusqlite::Connection| -> Result<()> {
            for (table, rows) in &old {
                for r in rows {
                    let columns: Vec<&String> = r.keys().collect();
                    let sql = format!("insert or ignore into {table} ({}) values ({})", columns.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", "), vec!["?"; columns.len()].join(", "));
                    database::stored::execute(c, &sql, &r.values().cloned().collect::<Vec<_>>())?;
                }
            }
            Ok(())
        };
        rt.write(me, &Scope::Global, key::APP, DATABASE, SCHEMA, &mut change).await
    };
    if let Err(err) = copied.await {
        tracing::warn!("copying backups' rows from the core's main database: {err:#}");
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

fn device(v: &Value) -> Result<Key> {
    unhex(v.as_str().unwrap_or("")).context("a device in backups' rows isn't a key: they can't be read")
}

/// A JSON column of a row, `empty` when it's null.
fn stored<T: serde::de::DeserializeOwned>(v: &Value, empty: &str) -> Result<T> {
    serde_json::from_str(v.as_str().unwrap_or(empty)).context("a stored backups field isn't JSON: the rows can't be read")
}
