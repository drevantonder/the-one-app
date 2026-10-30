//! Apps: JavaScript modules run in QuickJS, each folding entries into its own SQLite views.
//! One thread owns the JS engine and every view database; the node talks to it by message.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow, bail};
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Promise, Runtime};
use rusqlite::{Connection, types::Value as Sql};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::entry::{Content, Key, hex};


const PRELUDE: &str = r#"
const RealDate = Date;
globalThis.Date = function (...args) {
  if (args.length === 0) throw new Error("apps read the time with ctx.now()");
  return new RealDate(...args);
};
Date.UTC = RealDate.UTC; Date.parse = RealDate.parse; Date.prototype = RealDate.prototype;
Date.now = () => { throw new Error("apps read the time with ctx.now()"); };
Math.random = () => { throw new Error("apps have no randomness: folds must give the same result everywhere"); };
const json = (v) => JSON.stringify(v === undefined ? null : v);
globalThis.__apps = {};
const db = {
  exec: (sql, params) => __exec(sql, json(params || [])),
  query: (sql, params) => JSON.parse(__query(sql, json(params || []))),
};
const ctx = (me) => ({
  me,
  append: (kind, data) => __append(kind, json(data)),
  query: db.query,
  now: () => __now(),
});
globalThis.__run = (app, op, input, me) => {
  const o = __apps[app].ops?.[op];
  if (!o) throw new Error(`${app} has no operation ${op}`);
  return json(o.run(ctx(JSON.parse(me)), JSON.parse(input)));
};
globalThis.__fold = (app, kind, entry) => {
  const f = __apps[app].fold?.[kind];
  if (f) f(db, JSON.parse(entry));
};
globalThis.__hook = (app, name, input, me) => {
  const h = __apps[app].hooks?.[name];
  if (!h) throw new Error(`${app} has no hook ${name}`);
  return json(h(ctx(JSON.parse(me)), JSON.parse(input)));
};
globalThis.__widget = (app, name, me) => json(__apps[app].widgets[name].render(ctx(JSON.parse(me))));
globalThis.__describe = (app) => {
  const a = __apps[app];
  return json({
    name: a.name, title: a.title, icon: a.icon, instructions: a.instructions || "",
    ops: Object.entries(a.ops || {}).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {} })),
    widgets: Object.entries(a.widgets || {}).map(([name, w]) => ({ name, title: w.title || name })),
  });
};
globalThis.__schema = (app) => __apps[app].schema || "";
"#;

#[derive(Clone, Debug, Serialize)]
pub struct Me {
    pub space: String,
    pub device: String,
    pub person: String,
    pub name: String,
}

/// An entry as a fold handler sees it.
#[derive(Clone, Debug, Serialize)]
pub struct Folded {
    pub app: String,
    pub kind: String,
    pub author: String,
    pub person: String,
    pub name: String,
    pub time: u64,
    pub id: String,
    pub data: Value,
}

pub struct Outcome {
    pub result: Value,
    pub appends: Vec<Content>,
}

/// One app's part of a fold: its new entries, or all of them into a fresh view.
pub struct FoldJob {
    pub app: String,
    pub fresh: bool,
    pub entries: Vec<Folded>,
}

enum Job {
    Fold { space: Key, jobs: Vec<FoldJob>, reply: oneshot::Sender<Result<()>> },
    Call { space: Key, app: String, op: String, input: Value, me: Me, now: u64, reply: oneshot::Sender<Result<Outcome>> },
    Widget { space: Key, app: String, name: String, me: Me, now: u64, reply: oneshot::Sender<Result<Value>> },
    Describe { reply: oneshot::Sender<Vec<Value>> },
    Hook { space: Key, app: String, name: String, input: Value, me: Me, now: u64, reply: oneshot::Sender<Result<Value>> },
}

#[derive(Clone)]
pub struct Apps {
    jobs: mpsc::UnboundedSender<Job>,
}

impl Apps {
    /// Starts the engine with these apps: (name, JavaScript source).
    pub fn start(dir: PathBuf, apps: Vec<(String, String)>) -> Result<Self> {
        let (jobs, rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("apps".into()).spawn(move || {
            match Engine::new(dir, apps) {
                Ok(engine) => {
                    let _ = ready_tx.send(Ok(()));
                    engine.serve(rx);
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                }
            }
        })?;
        ready_rx.recv()??;
        Ok(Apps { jobs })
    }

    pub async fn fold(&self, space: Key, jobs: Vec<FoldJob>) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.jobs.send(Job::Fold { space, jobs, reply }).map_err(|_| anyhow!("apps stopped"))?;
        rx.await?
    }

    pub async fn call(&self, space: Key, app: &str, op: &str, input: Value, me: Me, now: u64) -> Result<Outcome> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Call { space, app: app.into(), op: op.into(), input, me, now, reply };
        self.jobs.send(job).map_err(|_| anyhow!("apps stopped"))?;
        rx.await?
    }

    pub async fn widget(&self, space: Key, app: &str, name: &str, me: Me, now: u64) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Widget { space, app: app.into(), name: name.into(), me, now, reply };
        self.jobs.send(job).map_err(|_| anyhow!("apps stopped"))?;
        rx.await?
    }

    pub async fn hook(&self, space: Key, app: &str, name: &str, input: Value, me: Me, now: u64) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        let job = Job::Hook { space, app: app.into(), name: name.into(), input, me, now, reply };
        self.jobs.send(job).map_err(|_| anyhow!("apps stopped"))?;
        rx.await?
    }

    pub async fn describe(&self) -> Result<Vec<Value>> {
        let (reply, rx) = oneshot::channel();
        self.jobs.send(Job::Describe { reply }).map_err(|_| anyhow!("apps stopped"))?;
        Ok(rx.await?)
    }
}

/// What the host functions act on during one call into JavaScript.
struct Current {
    db: Connection,
    mode: Mode,
    appends: Vec<Content>,
    app: String,
}

#[derive(PartialEq)]
enum Mode {
    Fold,
    Op(u64),
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// How long one call into an app may run before it's stopped.
const TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

struct Engine {
    _runtime: Runtime,
    deadline: std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
    context: Context,
    dir: PathBuf,
    names: Vec<String>,
    views: HashMap<(Key, String), Connection>,
}

impl Engine {
    fn new(dir: PathBuf, apps: Vec<(String, String)>) -> Result<Self> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(256 << 20);
        let deadline = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now() + TIME_LIMIT));
        let watch = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || std::time::Instant::now() > *watch.lock().unwrap())));
        let context = Context::full(&runtime)?;
        let mut names = Vec::new();
        context.with(|ctx| -> Result<()> {
            install_host(&ctx)?;
            ctx.eval::<(), _>(PRELUDE).catch(&ctx).map_err(|e| anyhow!("prelude: {e}"))?;
            for (name, source) in &apps {
                load(&ctx, name, source)?;
                names.push(name.clone());
            }
            Ok(())
        })?;
        Ok(Engine { _runtime: runtime, deadline, context, dir, names, views: HashMap::new() })
    }

    fn serve(mut self, mut rx: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = rx.blocking_recv() {
            match job {
                Job::Fold { space, jobs, reply } => {
                    let _ = reply.send(self.fold(space, jobs));
                }
                Job::Call { space, app, op, input, me, now, reply } => {
                    let _ = reply.send(self.call(space, &app, &op, &input, &me, now));
                }
                Job::Widget { space, app, name, me, now, reply } => {
                    let _ = reply.send(self.widget(space, &app, &name, &me, now));
                }
                Job::Hook { space, app, name, input, me, now, reply } => {
                    let _ = reply.send(self.hook(space, &app, &name, &input, &me, now));
                }
                Job::Describe { reply } => {
                    let _ = reply.send(self.describe());
                }
            }
        }
    }

    /// Takes an app's view database out to work on; `put` returns it.
    fn take(&mut self, space: Key, app: &str, fresh: bool) -> Result<Connection> {
        let key = (space, app.to_string());
        let path = self.path(space, app);
        if fresh {
            self.views.remove(&key);
            let _ = std::fs::remove_file(&path);
        }
        if let Some(db) = self.views.remove(&key) {
            return Ok(db);
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        let existed = path.exists();
        let db = Connection::open(&path)?;
        db.execute_batch("pragma journal_mode = wal; pragma synchronous = off;")?;
        // An app reaches only its own views: no attaching another app's database, or the log.
        db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0)?;
        if !existed {
            let schema: String = self.context.with(|ctx| -> Result<String> {
                let f: Function = ctx.globals().get("__schema")?;
                f.call((app,)).catch(&ctx).map_err(|e| anyhow!("{e}"))
            })?;
            db.execute_batch(&schema).with_context(|| format!("{app} schema"))?;
        }
        Ok(db)
    }

    fn put(&mut self, space: Key, app: &str, db: Connection) {
        self.views.insert((space, app.to_string()), db);
    }

    fn path(&self, space: Key, app: &str) -> PathBuf {
        self.dir.join(hex(&space)).join(format!("{app}.db"))
    }

    fn fold(&mut self, space: Key, jobs: Vec<FoldJob>) -> Result<()> {
        for job in jobs {
            if !self.names.contains(&job.app) || (job.entries.is_empty() && !job.fresh) {
                continue;
            }
            let mut db = self.take(space, &job.app, job.fresh)?;
            db.execute_batch("begin")?;
            for entry in &job.entries {
                let entry_json = serde_json::to_string(entry)?;
                let (result, current) = self.with_current(Current { db, mode: Mode::Fold, appends: Vec::new(), app: job.app.clone() }, |ctx| {
                    let f: Function = ctx.globals().get("__fold")?;
                    f.call::<_, ()>((job.app.as_str(), entry.kind.as_str(), entry_json.as_str())).catch(ctx).map_err(|e| anyhow!("{e}"))
                });
                db = current.db;
                if let Err(err) = result {
                    tracing::warn!("{} fold of {} failed: {err:#}", job.app, entry.kind);
                }
            }
            db.execute_batch("commit")?;
            self.put(space, &job.app, db);
        }
        Ok(())
    }

    /// Runs one of an app's functions (`__run`, `__widget`, `__hook`) with its views to read.
    fn op(&mut self, space: Key, app: &str, now: u64, f: impl FnOnce(&Ctx) -> Result<Value>) -> Result<Outcome> {
        if !self.names.iter().any(|n| n == app) {
            bail!("no app named {app}");
        }
        let db = self.take(space, app, false)?;
        let (result, current) = self.with_current(Current { db, mode: Mode::Op(now), appends: Vec::new(), app: app.into() }, f);
        self.put(space, app, current.db);
        Ok(Outcome { result: result?, appends: current.appends })
    }

    fn call(&mut self, space: Key, app: &str, op: &str, input: &Value, me: &Me, now: u64) -> Result<Outcome> {
        let (input, me) = (serde_json::to_string(input)?, serde_json::to_string(me)?);
        self.op(space, app, now, |ctx| {
            let f: Function = ctx.globals().get("__run")?;
            let out: String = f.call((app, op, input.as_str(), me.as_str())).catch(ctx).map_err(|e| anyhow!("{e}"))?;
            Ok(serde_json::from_str(&out)?)
        })
    }

    fn widget(&mut self, space: Key, app: &str, name: &str, me: &Me, now: u64) -> Result<Value> {
        let me = serde_json::to_string(me)?;
        let outcome = self.op(space, app, now, |ctx| {
            let f: Function = ctx.globals().get("__widget")?;
            let out: String = f.call((app, name, me.as_str())).catch(ctx).map_err(|e| anyhow!("{e}"))?;
            Ok(serde_json::from_str(&out)?)
        })?;
        Ok(outcome.result)
    }

    fn hook(&mut self, space: Key, app: &str, name: &str, input: &Value, me: &Me, now: u64) -> Result<Value> {
        let (input, me) = (serde_json::to_string(input)?, serde_json::to_string(me)?);
        let outcome = self.op(space, app, now, |ctx| {
            let f: Function = ctx.globals().get("__hook")?;
            let out: String = f.call((app, name, input.as_str(), me.as_str())).catch(ctx).map_err(|e| anyhow!("{e}"))?;
            Ok(serde_json::from_str(&out)?)
        })?;
        Ok(outcome.result)
    }

    fn describe(&self) -> Vec<Value> {
        self.context.with(|ctx| {
            self.names
                .iter()
                .filter_map(|name| {
                    let f: Function = ctx.globals().get("__describe").ok()?;
                    let out: String = f.call((name.as_str(),)).ok()?;
                    serde_json::from_str(&out).ok()
                })
                .collect()
        })
    }

    /// Runs `f` with `current` where the host functions can reach it, and hands it back.
    fn with_current<T>(&self, current: Current, f: impl FnOnce(&Ctx) -> Result<T>) -> (Result<T>, Current) {
        *self.deadline.lock().unwrap() = std::time::Instant::now() + TIME_LIMIT;
        CURRENT.with(|c| *c.borrow_mut() = Some(current));
        let result = self.context.with(|ctx| f(&ctx));
        let current = CURRENT.with(|c| c.borrow_mut().take()).expect("current is set for the call");
        (result, current)
    }
}

fn load(ctx: &Ctx, name: &str, source: &str) -> Result<()> {
    let (module, promise) = Module::declare(ctx.clone(), name, source).catch(ctx).map_err(|e| anyhow!("{name}: {e}"))?.eval().catch(ctx).map_err(|e| anyhow!("{name}: {e}"))?;
    let _: () = Promise::finish(&promise).catch(ctx).map_err(|e| anyhow!("{name}: {e}"))?;
    let app: rquickjs::Object = module.get("default").catch(ctx).map_err(|e| anyhow!("{name} has no default export: {e}"))?;
    let apps: rquickjs::Object = ctx.globals().get("__apps")?;
    apps.set(name, app)?;
    Ok(())
}

fn install_host(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();
    globals.set(
        "__query",
        Function::new(ctx.clone(), |sql: String, params: String| -> rquickjs::Result<String> { host(|c| query(c, &sql, &params)) }),
    )?;
    globals.set(
        "__exec",
        Function::new(ctx.clone(), |sql: String, params: String| -> rquickjs::Result<()> {
            host(|c| {
                if c.mode != Mode::Fold {
                    bail!("only fold handlers write views; operations append entries");
                }
                c.db.execute(&sql, rusqlite::params_from_iter(to_sql(&params)?))?;
                Ok(())
            })
        }),
    )?;
    globals.set(
        "__append",
        Function::new(ctx.clone(), |kind: String, data: String| -> rquickjs::Result<()> {
            host(|c| {
                if c.mode == Mode::Fold {
                    bail!("fold handlers can't append");
                }
                c.appends.push(Content { app: c.app.clone(), kind, data: serde_json::from_str(&data)? });
                Ok(())
            })
        }),
    )?;
    globals.set(
        "__now",
        Function::new(ctx.clone(), || -> rquickjs::Result<f64> {
            host(|c| match c.mode {
                Mode::Op(now) => Ok(now as f64),
                Mode::Fold => bail!("fold handlers can't read the clock; use entry.time"),
            })
        }),
    )?;
    Ok(())
}

fn host<T>(f: impl FnOnce(&mut Current) -> Result<T>) -> rquickjs::Result<T> {
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        let current = c.as_mut().ok_or_else(|| rquickjs::Error::new_from_js_message("host", "call", "not inside an app call"))?;
        f(current).map_err(|e| rquickjs::Error::new_from_js_message("host", "call", format!("{e:#}")))
    })
}

fn query(c: &Current, sql: &str, params: &str) -> Result<String> {
    let mut stmt = c.db.prepare(sql)?;
    if !stmt.readonly() {
        bail!("query only reads; fold handlers write with db.exec");
    }
    let names: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(to_sql(params)?), |r| {
        let mut row = Map::new();
        for (i, name) in names.iter().enumerate() {
            let v = match r.get_ref(i)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => json!(n),
                rusqlite::types::ValueRef::Real(f) => json!(f),
                rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                rusqlite::types::ValueRef::Blob(b) => json!(data_encoding::BASE64.encode(b)),
            };
            row.insert(name.clone(), v);
        }
        Ok(Value::Object(row))
    })?;
    let rows: Vec<Value> = rows.collect::<Result<_, _>>()?;
    Ok(serde_json::to_string(&rows)?)
}

fn to_sql(params: &str) -> Result<Vec<Sql>> {
    let params: Vec<Value> = serde_json::from_str(params)?;
    Ok(params
        .into_iter()
        .map(|v| match v {
            Value::Null => Sql::Null,
            Value::Bool(b) => Sql::Integer(b as i64),
            Value::Number(n) => n.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => Sql::Text(s),
            other => Sql::Text(other.to_string()),
        })
        .collect())
}
