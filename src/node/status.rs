//! How things went, as devices say it: who used each app, its errors and its AI cost; schedules' runs; the build each
//! device runs; backups' runs. Rows each scope's node database changes in place, not steps of their own: written often, and
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
"];

pub const RUNS_KEPT: usize = 20;
/// How many backup runs to each place `global` keeps, besides each scope's last that backed it up.
pub const BACKUP_RUNS_KEPT: usize = 100;
pub const CHECKS_KEPT: usize = 20;

/// A scope's rows, as this device last read them.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// What each app's owners see of it here: who used it, its errors, its AI cost.
    pub insights: BTreeMap<String, Insights>,
    /// Each schedule's latest runs here, oldest first, by (app, schedule).
    pub runs: BTreeMap<(String, String), Vec<Run>>,
    /// When a schedule's runs first followed a different definition, by (app, schedule): (was, is, when).
    pub changed: BTreeMap<(String, String), (Value, Value, u64)>,
    /// In `global`: the build each device runs.
    pub builds: HashMap<Key, release::Running>,
    /// In `global`: the latest backup runs, oldest first.
    pub backups: Vec<BackupRun>,
    /// In `global`: by (place, scope), the newest step's time a snapshot there holds, and when it was written.
    pub covered: BTreeMap<(String, String), (u64, u64)>,
    /// In `global`: when admins were last told of a place's trouble, by place.
    pub alerted: BTreeMap<String, u64>,
    /// In `global`: the latest restore checks, oldest first.
    pub checks: Vec<CheckRun>,
}

/// An app's insights in one scope.
#[derive(Clone, Debug, Default)]
pub struct Insights {
    /// By (user, client): the days they used it, when last, and the agent last named.
    pub used: BTreeMap<(String, String), Used>,
    /// By (tool, error): how often, first and last, and by whom last.
    pub errors: BTreeMap<(String, String), Failed>,
    /// By model: dollars, tokens in and out, and calls.
    pub cost: BTreeMap<String, Cost>,
}

#[derive(Clone, Debug, Default)]
pub struct Used {
    pub days: u64,
    pub last: u64,
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Failed {
    pub count: u64,
    pub first: u64,
    pub last: u64,
    pub user: String,
}

#[derive(Clone, Debug, Default)]
pub struct Cost {
    pub usd: f64,
    pub input: u64,
    pub output: u64,
    pub calls: u64,
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
    /// A call of `app` by `user` from `client` (`agent` naming who asked through it), on `day`.
    Used { app: String, user: String, client: String, agent: Option<String>, day: u64 },
    Failed { app: String, tool: String, error: String, user: String },
    Cost { app: String, model: String, usd: f64, input: u64, output: u64 },
    Ran { app: String, schedule: String, run: Run },
    Build { device: Key, build: String, target: String },
    BackedUp(BackupRun),
    Alerted { place: String },
    Checked(CheckRun),
}

impl Node {
    /// Changes a scope's rows, then reads them back.
    pub(super) async fn say(&self, scope: &Scope, said: Said) -> Result<()> {
        let now = now_ms();
        let db = self.database(scope, NODE_DATABASE.0, NODE_DATABASE.1, Some(SCHEMA.iter().map(|s| s.to_string()).collect()), None).await?;
        // As every write is, so a step that says the log may be frozen is answered, not the end of every row here.
        self.write_in(scope, &mut *db.lock().await, &self.bucket()?, |c| {
            let ex = |sql: &str, params: &[Value]| crate::database::stored::execute(c, sql, params).map(|_| ());
            match &said {
                Said::Used { app, user, client, agent, day } => ex(
                    "insert into used values (?, ?, ?, 1, ?, ?, ?) on conflict (app, user, client) do update set
                       days = days + (excluded.day > day), day = max(day, excluded.day), last = max(last, excluded.last), agent = coalesce(excluded.agent, agent)",
                    &[json!(app), json!(user), json!(client), json!(day), json!(now), json!(agent)],
                ),
                Said::Failed { app, tool, error, user } => ex(
                    "insert into errors values (?, ?, ?, 1, ?, ?, ?) on conflict (app, tool, error) do update set
                       count = count + 1, first = min(first, excluded.first), last = max(last, excluded.last), user = excluded.user",
                    &[json!(app), json!(tool), json!(error), json!(now), json!(now), json!(user)],
                ),
                Said::Cost { app, model, usd, input, output } => ex(
                    "insert into cost values (?, ?, ?, ?, ?, 1) on conflict (app, model) do update set
                       usd = usd + excluded.usd, input = input + excluded.input, output = output + excluded.output, calls = calls + 1",
                    &[json!(app), json!(model), json!(usd.max(0.0)), json!(input), json!(output)],
                ),
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
                Said::Build { device, build, target } => ex("insert or replace into builds values (?, ?, ?, ?)", &[json!(hex(device)), json!(build), json!(target), json!(now)]),
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
        self.read_status(scope).await
    }

    pub(super) async fn read_status(&self, scope: &Scope) -> Result<()> {
        let db = self.database(scope, NODE_DATABASE.0, NODE_DATABASE.1, Some(SCHEMA.iter().map(|s| s.to_string()).collect()), None).await?;
        let mut db = db.lock().await;
        db.catch_up(&self.bucket()?).await?;
        let rows = |sql: &str| db.query(sql, &[]);
        let text = |v: &Value| v.as_str().unwrap_or("").to_string();
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let device = |v: &Value| unhex(v.as_str().unwrap_or("")).context("a device in this status isn't a key: the status can't be read");
        let mut s = Status::default();
        for r in rows("select * from used")? {
            let used = Used { days: n(&r["days"]), last: n(&r["last"]), agent: r["agent"].as_str().map(str::to_string) };
            s.insights.entry(text(&r["app"])).or_default().used.insert((text(&r["user"]), text(&r["client"])), used);
        }
        for r in rows("select * from errors")? {
            let failed = Failed { count: n(&r["count"]), first: n(&r["first"]), last: n(&r["last"]), user: text(&r["user"]) };
            s.insights.entry(text(&r["app"])).or_default().errors.insert((text(&r["tool"]), text(&r["error"])), failed);
        }
        for r in rows("select * from cost")? {
            let cost = Cost { usd: r["usd"].as_f64().unwrap_or(0.0), input: n(&r["input"]), output: n(&r["output"]), calls: n(&r["calls"]) };
            s.insights.entry(text(&r["app"])).or_default().cost.insert(text(&r["model"]), cost);
        }
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
        for r in rows("select * from builds")? {
            if let Ok(device) = unhex(r["device"].as_str().unwrap_or("")) {
                s.builds.insert(device, release::Running { commit: text(&r["build"]), target: text(&r["target"]) });
            }
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
            if *scope == Scope::Global {
                i.builds = s.builds.clone();
            }
            i.status.insert(scope.to_string(), s);
            Ok(())
        })
    }

    /// Reads every scope's rows this device holds again, for what other devices said: in the background.
    pub(super) fn read_all_status(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let scopes = match node.with(|i| Ok(i.r.db().scopes()?.into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>())) {
                Ok(scopes) => scopes,
                Err(err) => return tracing::warn!("reading every scope's status: {err:#}"),
            };
            for scope in scopes {
                if let Err(err) = node.read_status(&scope).await {
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
