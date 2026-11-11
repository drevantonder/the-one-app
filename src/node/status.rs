//! How things went, as devices say it: schedules' runs; backups' runs. Rows each scope's node database changes in place, not steps of their own: written often, and
//! needed only as they are now. Each device reads them back into `Status`, which the rest of the node looks at.

use super::*;

const NODE_DATABASE: (&str, &str) = (scopes::APP, scopes::APP_DATABASE);

/// Its schema, as its changes, oldest first.
const SCHEMA: &[&str] = &["
create table used (app text, user text, client text, days integer not null, day integer not null, last integer not null, agent text,
                   primary key (app, user, client));
create table errors (app text, op text, error text, count integer not null, first integer not null, last integer not null,
                     user text not null, primary key (app, op, error));
create table cost (app text, model text, usd real not null, input integer not null, output integer not null, calls integer not null,
                   primary key (app, model));
create table runs (app text, schedule text, slot integer, time integer not null, server text not null, user text not null,
                   ok integer not null, error text, ms integer not null, spec text, primary key (app, schedule, slot));
create table builds (server text primary key, build text not null, target text not null, time integer not null);
create table backups (place text, server text, time integer, error text, trouble text, snapshots text not null,
                      primary key (place, server, time));
create table alerts (place text primary key, time integer not null);
", "
alter table runs rename column server to device;
alter table builds rename column server to device;
alter table backups rename column server to device;
", "
create table checks (place text, device text, time integer, error text, report text not null, primary key (place, device, time));
", "
create table reads (device text primary key, reads integer not null, build text not null, time integer not null);
", "
alter table errors rename column op to tool;
", "
update runs set spec = json_remove(json_set(spec, '$.tool', json_extract(spec, '$.op')), '$.op') where json_valid(spec) and json_extract(spec, '$.op') is not null;
", "
create table signals (time integer not null, device text not null, kind text not null, name text not null, app text not null, trace text not null,
                      span text not null, parent text, ms integer not null, status text not null, fault text, attrs text not null);
create index signals_time on signals (time);
create table rollups (hour integer not null, name text not null, app text not null, key text not null, count integer not null, errors integer not null,
                      ms integer not null, under_10 integer not null, under_100 integer not null, under_1000 integer not null, under_10000 integer not null,
                      over integer not null, primary key (hour, name, app, key));
create table fingerprints (app text not null, tool text not null, fingerprint text not null, type text not null, frame text not null, build text not null,
                           fault text, count integer not null, first integer not null, last integer not null, primary key (app, tool, fingerprint));
insert into fingerprints select app, tool, 'Error|migrated|', 'Error', 'migrated', '', null, sum(count), min(first), max(last) from errors group by app, tool;
drop table errors;
", "
update rollups set key = json_set(key, '$.kind', case
    when name in ('bucket.asks', 'bucket.ms', 'steps.behind', 'write.seen', 'signer.failures', 'gen_ai.client.token.usage', 'gen_ai.client.operation.duration') then 'metric'
    when name in ('device.online', 'device.offline', 'node.start', 'service.start', 'service.exit', 'screen.exception', 'screen.undrawn', 'page.exception', 'client.try') or name like 'update.%' then 'event'
    else 'span' end);
update or replace rollups set name = substr(name, length(app) + 2) where app != '' and substr(name, 1, length(app) + 1) = app || '.';
update signals set name = substr(name, length(app) + 2) where app != '' and substr(name, 1, length(app) + 1) = app || '.';
", "
update rollups set key = json_set(key, '$.\"span.kind\"', case
    when name = 'service.hop' then 'client'
    when name = 'schedule.run' or json_extract(key, '$.\"one.client\"') is null then 'internal'
    else 'server' end) where json_extract(key, '$.kind') = 'span';
update signals set attrs = json_set(attrs, '$.\"span.kind\"', case
    when name = 'service.hop' then 'client'
    when name = 'schedule.run' or json_extract(attrs, '$.\"one.client\"') is null then 'internal'
    else 'server' end) where kind = 'span';
", "
delete from fingerprints where frame = 'migrated';
alter table fingerprints rename column fault to status;
update fingerprints set status = case status when 'caller' then 'unset' else 'error' end;
update signals set attrs = json_remove(json_set(attrs, '$.\"one.code\"', case when json_extract(attrs, '$.\"error.type\"') in ('Error', '_OTHER') then 'refused' else json_extract(attrs, '$.\"error.type\"') end), '$.\"error.type\"'), status = 'unset' where fault = 'caller';
update signals set status = 'unset' where status = 'ok';
alter table signals drop column fault;
update rollups set errors = 0 where json_extract(key, '$.fault') = 'caller';
create table rollups_merged (hour integer not null, name text not null, app text not null, key text not null, count integer not null, errors integer not null,
                             ms integer not null, under_10 integer not null, under_100 integer not null, under_1000 integer not null, under_10000 integer not null,
                             over integer not null, primary key (hour, name, app, key));
insert into rollups_merged select hour, name, app, json_remove(key, '$.fault'), sum(count), sum(errors), sum(ms), sum(under_10), sum(under_100), sum(under_1000), sum(under_10000), sum(over) from rollups group by 1, 2, 3, 4;
drop table rollups;
alter table rollups_merged rename to rollups;
", "
drop table signals;
drop table rollups;
drop table fingerprints;
", "
drop table used;
drop table cost;
", "
drop table builds;
"];

pub const RUNS_KEPT: usize = 20;
/// How many backup runs to each place `global` keeps, besides each scope's last that backed it up.
pub const BACKUP_RUNS_KEPT: usize = 100;
pub const CHECKS_KEPT: usize = 20;

/// A scope's rows, as this device last read them.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Each schedule's latest runs here, oldest first, by (app, schedule).
    pub runs: BTreeMap<(String, String), Vec<Run>>,
    /// When a schedule's runs first followed a different definition, by (app, schedule): (was, is, when).
    pub changed: BTreeMap<(String, String), (Value, Value, u64)>,
    /// In `global`: the latest backup runs, oldest first.
    pub backups: Vec<BackupRun>,
    /// In `global`: by (place, scope), the newest step's time a snapshot there holds, and when it was written.
    pub covered: BTreeMap<(String, String), (u64, u64)>,
    /// In `global`: when admins were last told of a place's trouble, by place.
    pub alerted: BTreeMap<String, u64>,
    /// In `global`: the latest restore checks, oldest first.
    pub checks: Vec<CheckRun>,
}

/// One run of a schedule: the slot it ran for, when, on which device, as whom, how it went, and the definition it
/// followed.
#[derive(Clone, Debug)]
pub struct Run {
    pub slot: u64,
    pub time: u64,
    pub device: Key,
    pub user: String,
    pub ok: bool,
    pub error: Option<String>,
    pub ms: u64,
    pub spec: Value,
}

/// One device's backup run to one place: the snapshots it wrote, or what went wrong.
#[derive(Clone, Debug)]
pub struct BackupRun {
    /// The place's Vault item.
    pub place: String,
    pub device: Key,
    pub time: u64,
    pub error: Option<String>,
    /// What kind of trouble the error is, for its fix: `crate::bucket::Trouble`'s name.
    pub trouble: Option<String>,
    pub snapshots: Vec<BackedUp>,
}

/// A scope's snapshot in a run: when it holds the scope as of, and what it added.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackedUp {
    pub scope: String,
    pub upto: u64,
    pub blobs: u64,
    pub bytes: u64,
}

/// One device's restore check of one place: what came back (`backups::check`'s report, in short), or what's wrong.
#[derive(Clone, Debug)]
pub struct CheckRun {
    pub place: String,
    pub device: Key,
    pub time: u64,
    pub error: Option<String>,
    pub report: Value,
}

pub enum Said {
    Ran { app: String, schedule: String, run: Run },
    BackedUp(BackupRun),
    Alerted { place: String },
    Checked(CheckRun),
}

impl Node {
    /// Changes a scope's rows, then reads them back.
    pub(super) async fn say(&self, scope: &Scope, said: Said) -> Result<()> {
        let now = now_ms();
        let db = self.node_database(scope).await?;
        let mut db = db.lock().await;
        let before = db.at();
        // As every write is, so a step that says the log may be frozen is answered, not the end of every row here.
        self.write_in(scope, &mut db, &self.bucket()?, |c| {
            let ex = |sql: &str, params: &[Value]| crate::database::stored::execute(c, sql, params).map(|_| ());
            match &said {
                Said::Ran { app, schedule, run } => {
                    ex(
                        "insert or replace into runs values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        &[json!(app), json!(schedule), json!(run.slot), json!(run.time), json!(hex(&run.device)), json!(run.user), json!(run.ok), json!(run.error), json!(run.ms), json!(run.spec.to_string())],
                    )?;
                    ex(
                        "delete from runs where app = ? and schedule = ? and slot not in (select slot from runs where app = ? and schedule = ? order by slot desc limit ?)",
                        &[json!(app), json!(schedule), json!(app), json!(schedule), json!(RUNS_KEPT)],
                    )
                }
                Said::BackedUp(r) => {
                    ex(
                        "insert or replace into backups values (?, ?, ?, ?, ?, ?)",
                        &[json!(r.place), json!(hex(&r.device)), json!(r.time), json!(r.error), json!(r.trouble), json!(serde_json::to_string(&r.snapshots)?)],
                    )?;
                    // Beyond the newest, each scope's last run that backed it up stays, as what's covered is read from
                    // them: a scope that hasn't changed would otherwise look a day late once busier ones fill these.
                    ex(
                        "delete from backups where place = ? and time not in (select time from backups where place = ? order by time desc limit ?) \
                         and time not in (select max(b.time) from backups b, json_each(b.snapshots) j where b.place = ? and b.error is null group by json_extract(j.value, '$.scope'))",
                        &[json!(r.place), json!(r.place), json!(BACKUP_RUNS_KEPT), json!(r.place)],
                    )
                }
                Said::Alerted { place } => ex("insert or replace into alerts values (?, ?)", &[json!(place), json!(now)]),
                Said::Checked(r) => {
                    ex("insert or replace into checks values (?, ?, ?, ?, ?)", &[json!(r.place), json!(hex(&r.device)), json!(r.time), json!(r.error), json!(r.report.to_string())])?;
                    ex("delete from checks where place = ? and time not in (select time from checks where place = ? order by time desc limit ?)", &[json!(r.place), json!(r.place), json!(CHECKS_KEPT)])
                }
            }
        })
        .await?;
        let at = db.at();
        drop(db);
        // The devices that hold the scope read these rows from their copies, which this fills.
        self.ping(scope, Some((NODE_DATABASE.0, NODE_DATABASE.1, at)));
        self.read_status(scope, &self.bucket()?, Some(before)).await
    }

    /// A scope's node database, brought to its schema.
    pub(super) async fn node_database(&self, scope: &Scope) -> Result<Arc<tokio::sync::Mutex<Db>>> {
        self.database(scope, NODE_DATABASE.0, NODE_DATABASE.1, Some(SCHEMA.iter().map(|s| s.to_string()).collect()), None).await
    }

    /// Reads a scope's rows from `bucket` (or only this device's copy of it) after what its database here has, and says
    /// they changed if there were any since `since` (by default, where it was), or this device hadn't read them yet.
    pub(super) async fn read_status(&self, scope: &Scope, bucket: &Copied, since: Option<u64>) -> Result<()> {
        let db = self.node_database(scope).await?;
        let mut db = db.lock().await;
        let at = since.unwrap_or(db.at());
        db.catch_up(bucket).await?;
        if db.at() == at && self.with(|i| Ok(i.status.contains_key(&scope.to_string())))? {
            return Ok(());
        }
        let rows = |sql: &str| db.query(sql, &[]);
        let text = |v: &Value| v.as_str().unwrap_or("").to_string();
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let device = |v: &Value| unhex(v.as_str().unwrap_or("")).context("a device in this status isn't a key: the status can't be read");
        let mut s = Status::default();
        for r in rows("select * from runs order by slot")? {
            let key = (text(&r["app"]), text(&r["schedule"]));
            let run = Run {
                slot: n(&r["slot"]),
                time: n(&r["time"]),
                device: device(&r["device"])?,
                user: text(&r["user"]),
                ok: r["ok"] == 1,
                error: r["error"].as_str().map(str::to_string),
                ms: n(&r["ms"]),
                spec: stored(&r["spec"], "null")?,
            };
            let runs = s.runs.entry(key.clone()).or_default();
            if let Some(last) = runs.last().filter(|l| !l.spec.is_null() && !run.spec.is_null() && l.spec != run.spec) {
                s.changed.insert(key, (last.spec.clone(), run.spec.clone(), run.time));
            }
            runs.push(run);
        }
        for r in rows("select * from backups order by time")? {
            let snapshots: Vec<BackedUp> = stored(&r["snapshots"], "[]")?;
            let run = BackupRun {
                place: text(&r["place"]),
                device: device(&r["device"])?,
                time: n(&r["time"]),
                error: r["error"].as_str().map(str::to_string),
                trouble: r["trouble"].as_str().map(str::to_string),
                snapshots,
            };
            if run.error.is_none() {
                for b in &run.snapshots {
                    let covered = s.covered.entry((run.place.clone(), b.scope.clone())).or_default();
                    *covered = (covered.0.max(b.upto), covered.1.max(run.time));
                }
            }
            s.backups.push(run);
        }
        for r in rows("select * from alerts")? {
            s.alerted.insert(text(&r["place"]), n(&r["time"]));
        }
        for r in rows("select * from checks order by time")? {
            s.checks.push(CheckRun {
                place: text(&r["place"]),
                device: device(&r["device"])?,
                time: n(&r["time"]),
                error: r["error"].as_str().map(str::to_string),
                report: stored(&r["report"], "null")?,
            });
        }
        // While it holds the database, so a read that started earlier can't land after this one.
        self.with(|i| {
            i.status.insert(scope.to_string(), s);
            Ok(())
        })?;
        self.emit(super::Event::Changed { apps: Vec::new() });
        Ok(())
    }

    /// Reads every scope's rows this device holds again, for what other devices said, in the background: from the
    /// bucket, or with `here`, only as far as this device's copy, which their notices fill.
    pub(super) fn read_all_status(&self, here: bool) {
        let node = self.clone();
        tokio::spawn(async move {
            let (scopes, bucket) = match node.with(|i| Ok(i.r.db().scopes()?.into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>())).and_then(|s| Ok((s, node.bucket()?))) {
                Ok(read) => read,
                Err(err) => return tracing::warn!("reading every scope's status: {err:#}"),
            };
            let bucket = if here { bucket.here_only() } else { bucket };
            for scope in scopes {
                if let Err(err) = node.read_status(&scope, &bucket, None).await {
                    tracing::warn!("reading {scope}'s status: {err:#}");
                }
            }
        });
    }
}

/// A JSON column of a status row, `empty` when it's null.
fn stored<T: serde::de::DeserializeOwned>(v: &Value, empty: &str) -> Result<T> {
    serde_json::from_str(v.as_str().unwrap_or(empty)).context("a stored status field isn't JSON: the status can't be read")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rollup kept before kinds says what it's of, and a span's name no longer repeats its service, and none is lost.
    #[test]
    fn rollups_gain_their_kind_and_lose_their_service_in_the_name() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = SCHEMA.iter().position(|s| s.contains("substr(name, length(app) + 2)")).unwrap();
        db.execute_batch(&SCHEMA[..before].concat()).unwrap();
        db.execute_batch("insert into rollups values (1, 'repos.list', 'repos', '{}', 3, 0, 9, 3, 0, 0, 0, 0), (1, 'bucket.ms', '', '{}', 2, 0, 9, 2, 0, 0, 0, 0), (1, 'node.start', '', '{}', 1, 0, 0, 1, 0, 0, 0, 0);").unwrap();
        db.execute_batch(SCHEMA[before]).unwrap();
        let of = |name: &str| -> (String, i64) { db.query_row("select json_extract(key, '$.kind'), count from rollups where name = ?", [name], |r| Ok((r.get(0)?, r.get(1)?))).unwrap() };
        assert_eq!(of("list"), ("span".to_string(), 3));
        assert_eq!(of("bucket.ms"), ("metric".to_string(), 2));
        assert_eq!(of("node.start"), ("event".to_string(), 1));
    }

    /// A span kept before kinds says what kind it is, so only requests count in a service's rates, and none is lost.
    #[test]
    fn spans_kept_before_span_kinds_gain_theirs() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = SCHEMA.iter().position(|s| s.contains("when name = 'service.hop'")).unwrap();
        db.execute_batch(&SCHEMA[..before].concat()).unwrap();
        let span = r#"{"kind":"span","one.client":"cli"}"#;
        db.execute_batch(&format!("insert into rollups values (1, 'list', 'repos', '{span}', 3, 0, 9, 3, 0, 0, 0, 0), (1, 'service.hop', 'shell', '{span}', 2, 1, 9, 2, 0, 0, 0, 0), (1, 'schedule.run', 'a', '{span}', 1, 0, 9, 1, 0, 0, 0, 0), (1, 'ping', 'a', '{{\"kind\":\"span\"}}', 1, 0, 2, 1, 0, 0, 0, 0), (1, 'node.start', '', '{{\"kind\":\"event\"}}', 1, 0, 0, 1, 0, 0, 0, 0);")).unwrap();
        db.execute_batch(SCHEMA[before]).unwrap();
        let of = |name: &str| -> (Option<String>, i64) { db.query_row(r#"select json_extract(key, '$."span.kind"'), count from rollups where name = ?"#, [name], |r| Ok((r.get(0)?, r.get(1)?))).unwrap() };
        assert_eq!(of("list"), (Some("server".to_string()), 3));
        assert_eq!(of("service.hop"), (Some("client".to_string()), 2));
        assert_eq!(of("schedule.run"), (Some("internal".to_string()), 1));
        assert_eq!(of("ping"), (Some("internal".to_string()), 1));
        assert_eq!(of("node.start"), (None, 1));
    }

    /// What was refused stops being counted as broken, without losing how often it happened, and what was migrated from
    /// before fingerprints goes.
    #[test]
    fn what_was_refused_stops_being_an_error() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = SCHEMA.iter().position(|s| s.contains("rollups_merged")).unwrap();
        db.execute_batch(&SCHEMA[..before].concat()).unwrap();
        let fault = |fault: &str| format!(r#"{{"kind":"span","fault":"{fault}"}}"#);
        db.execute_batch(&format!(
            "insert into rollups values (1, 'get', 'repos', '{}', 3, 3, 9, 3, 0, 0, 0, 0), (1, 'get', 'repos', '{}', 2, 2, 9, 2, 0, 0, 0, 0);
             insert into signals values (5, 'd', 'span', 'get', 'repos', 't', 's', null, 1, 'error', 'caller', '{{\"error.type\":\"refused\"}}'), (6, 'd', 'span', 'get', 'repos', 't', 'u', null, 1, 'error', 'internal', '{{\"error.type\":\"TypeError\"}}'), (7, 'd', 'span', 'get', 'repos', 't', 'v', null, 1, 'ok', null, '{{}}');
             insert into fingerprints values ('repos', 'get', 'a', 'refused', 'stage:run', 'b1', 'caller', 3, 1, 2), ('repos', 'get', 'b', 'TypeError', 'app.js:1:1', 'b1', 'internal', 2, 1, 2), ('repos', 'get', 'Error|migrated|', 'Error', 'migrated', '', null, 9, 1, 2);",
            fault("caller"), fault("internal"),
        )).unwrap();
        db.execute_batch(SCHEMA[before]).unwrap();
        let (count, errors): (i64, i64) = db.query_row("select sum(count), sum(errors) from rollups", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((count, errors), (5, 2), "both counted, only what broke an error");
        let rows: i64 = db.query_row("select count(*) from rollups", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "the same row once what told them apart is gone");
        let of = |span: &str| -> (String, String) { db.query_row("select status, attrs from signals where span = ?", [span], |r| Ok((r.get(0)?, r.get(1)?))).unwrap() };
        assert_eq!(of("s"), ("unset".to_string(), r#"{"one.code":"refused"}"#.to_string()));
        assert_eq!(of("u"), ("error".to_string(), r#"{"error.type":"TypeError"}"#.to_string()));
        assert_eq!(of("v").0, "unset");
        let statuses: Vec<String> = db.prepare("select status from fingerprints order by fingerprint").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(statuses, ["unset", "error"], "and the migrated row is gone");
    }

    /// Who used each app and its AI cost go, and a schedule's runs stay.
    #[test]
    fn who_used_an_app_goes_and_runs_stay() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = SCHEMA.iter().position(|s| s.contains("drop table used")).unwrap();
        db.execute_batch(&SCHEMA[..before].concat()).unwrap();
        db.execute_batch("insert into used values ('a', 'u1', 'cli', 1, 1, 1, null); insert into cost values ('a', 'm', 1.0, 1, 1, 1); insert into runs values ('a', 's', 1, 5, 'd', 'u1', 1, null, 3, null);").unwrap();
        db.execute_batch(SCHEMA[before]).unwrap();
        assert!(db.prepare("select * from used").is_err() && db.prepare("select * from cost").is_err());
        let runs: i64 = db.query_row("select count(*) from runs", [], |r| r.get(0)).unwrap();
        assert_eq!(runs, 1);
    }

    /// A failure kept before fingerprints keeps its app, tool, count and times, and never what it said.
    #[test]
    fn errors_move_to_fingerprints_without_their_text() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let before = SCHEMA.iter().position(|s| s.contains("drop table errors")).unwrap();
        db.execute_batch(&SCHEMA[..before].concat()).unwrap();
        db.execute_batch("insert into errors values ('repos', 'get', 'no repo anna/secret', 2, 5, 9, 'u1'), ('repos', 'get', 'other', 1, 3, 4, 'u2');").unwrap();
        db.execute_batch(SCHEMA[before]).unwrap();
        let (kept, first, last): (i64, i64, i64) = db.query_row("select count, first, last from fingerprints where app = 'repos' and tool = 'get'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((kept, first, last), (3, 3, 9));
        assert!(db.prepare("select * from errors").is_err(), "the table with the text in it is gone");
    }
}
