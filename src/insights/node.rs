//! A node's part of insights: the batch per scope it keeps in memory, written every few minutes and when it stops, and
//! what it reads back for the tool.

use serde_json::Map;

use crate::insights::{self, Ask, Attrs, Batch, Exception, Fault, Kind, Print, Signal, stage};

use super::calls::record::Destination;
use super::*;

/// How often a device writes what it has to say. A write per call would cost far more than what it says is worth.
const FLUSH_EVERY: Duration = Duration::from_secs(300);
const MOST_SIGNALS_READ: usize = 500;
const MOST_ROLLUPS_READ: usize = 1000;
const MOST_PRINTS_READ: usize = 200;

/// What this device hasn't written yet.
#[derive(Default)]
pub(super) struct State(Mutex<HashMap<Scope, Batch>>);

/// A call as it runs here: its span, in the trace of the request that made it.
pub(super) struct Call {
    pub trace: trace::Trace,
    from: usize,
    started: Instant,
    time: u64,
    span: String,
    parent: Option<String>,
}

impl Call {
    /// Ends it with nothing said: it's a call for keys it wasn't handed, which the caller answers.
    pub(super) fn leave(self) {
        self.trace.leave(self.parent);
    }
}

impl Node {
    pub(super) fn begin_call(&self) -> Call {
        let trace = trace::current().unwrap_or_else(trace::Trace::quiet);
        let span = trace::ids::span();
        let parent = trace.enter(span.clone());
        let from = trace.len();
        trace.stage_is(stage::CHECK);
        Call { trace, from, started: Instant::now(), time: now_ms(), span, parent }
    }

    /// A call that ran here, or failed trying: one span and a place in the rollups, and an error's fingerprint. Only a
    /// failure or a slow call keeps its span whole. Elsewhere isn't a failure, and it says nothing.
    pub(super) async fn finish_call(&self, call: Call, req: &CallRequest, me: &Caller, reply: &Reply) {
        let Call { trace, from, started, time, span, parent } = call;
        trace.leave(parent.clone());
        let (ok, code) = match reply {
            Reply::Ok(_) => (true, None),
            Reply::Error(_, code) => (false, code.clone()),
            Reply::Elsewhere(_) | Reply::Needs(_) => return,
        };
        let ms = started.elapsed().as_millis() as u64;
        let kept = !ok || ms >= insights::SLOW_MS;
        let steps = trace.since(from);
        let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
        let at = trace.stage();
        let fault = (!ok).then(|| insights::fault(code.as_deref(), at, exception.as_ref()));
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let build = crate::config::BUILD;
        let bundle = self.0.bundles.lock().unwrap().get(&req.app).cloned();
        let (app, tool) = match destination.named {
            true => (req.app.as_str(), self.tool_named(req).await),
            false => ("", "".to_string()),
        };
        let mut attrs = Attrs::default().device(&hex(&self.0.id)).client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(build);
        if destination.named {
            attrs = attrs.service(app).tool(&tool);
            if let Some(bundle) = &bundle {
                attrs = attrs.version(bundle);
            }
        }
        if kept {
            if destination.named {
                attrs = attrs.steps(&steps);
            }
            attrs = attrs.bytes(req.input.to_string().len() as u64 + if let Reply::Ok(v) = reply { v.to_string().len() as u64 } else { 0 });
        }
        let print = (!ok).then(|| {
            let (frame, built) = match exception.as_ref().filter(|_| destination.named).and_then(|e| e.frames.first()) {
                Some(top) => (top.clone(), bundle.clone().unwrap_or_else(|| build.into())),
                None => (format!("stage:{at}"), build.to_string()),
            };
            let kind = exception_kind(code.as_deref(), exception.as_ref(), at);
            attrs = std::mem::take(&mut attrs).error(&kind).stage(at);
            if let Some(e) = exception.as_ref().filter(|_| destination.named) {
                attrs = std::mem::take(&mut attrs).exception(e);
            }
            Print { app: app.to_string(), tool: tool.clone(), kind, frame, build: built, fault: fault.unwrap_or(Fault::Internal) }
        });
        let name = if destination.named { format!("{app}.{tool}") } else { "call".into() };
        let signal = Signal { time, device: hex(&self.0.id), kind: Kind::Span, name, app: app.to_string(), trace: trace.trace_id(), span, parent, ms, ok, fault, attrs };
        self.0.insights.0.lock().unwrap().entry(destination.scope).or_default().add(signal, kept, print);
    }

    /// `req`'s tool if its app says it has it, else `_unknown`: what a caller typed is theirs to get wrong, and may be anything.
    async fn tool_named(&self, req: &CallRequest) -> String {
        let listed = self.0.apps.describe(&req.app).await.is_ok_and(|d| d["tools"].as_array().is_some_and(|t| t.iter().any(|t| t["name"] == req.tool.as_str())));
        if listed { req.tool.clone() } else { "_unknown".into() }
    }

    /// Writes what this device has to say, to each scope it holds, once it has anything: telemetry may be lost, so what
    /// doesn't go is dropped.
    pub async fn flush_insights(&self) {
        let batches = std::mem::take(&mut *self.0.insights.0.lock().unwrap());
        for (scope, batch) in batches.into_iter().filter(|(_, b)| !b.is_empty()) {
            let live = self.with(|i| {
                if i.builds.is_empty() {
                    return Ok(Vec::new());
                }
                let m = i.r.db();
                let mut live = Vec::new();
                for (key, running) in &i.builds {
                    if m.machine(key)?.is_some() {
                        live.push(running.commit.clone());
                    }
                }
                live.extend(m.apps()?.into_iter().filter_map(|a| a.bundle));
                Ok(live)
            });
            let holds = self.with_or(false, |i| Ok(i.r.holds(&scope)));
            match live {
                Ok(live) if holds => {
                    if let Err(err) = self.say(&scope, status::Said::Insights { batch, live }).await {
                        tracing::debug!("insights for {}: {err:#}", trace::kind(&scope));
                    }
                }
                _ => tracing::debug!("insights for {}: this device doesn't hold it now", trace::kind(&scope)),
            }
        }
    }

    pub(super) async fn insights_loop(self) {
        loop {
            tokio::time::sleep(FLUSH_EVERY).await;
            self.flush_insights().await;
        }
    }

    /// What `viewer` may see of how things went, by `ask`: in each scope they may use that this device holds, an app's
    /// rows only for someone who may deploy it, as `insights_of` shows them, and what's only a device's in `global` only
    /// for an admin. What this device hasn't written yet is written first.
    pub async fn insights(&self, viewer: &Caller, ask: &Ask) -> Result<Value> {
        self.flush_insights().await;
        let scopes = self.with(|i| {
            let m = i.r.db();
            Ok(m.scopes()?.into_iter().filter(|s| i.r.holds(s) && m.may(&viewer.user, "use", &scopes::object(s))).collect::<Vec<_>>())
        })?;
        let (mut signals, mut rollups, mut prints) = (Vec::new(), Vec::new(), Vec::new());
        let since = ask.since.unwrap_or(0);
        let limit = ask.limit.clamp(1, MOST_SIGNALS_READ);
        for scope in scopes {
            let db = self.node_database(&scope).await?;
            let mut db = db.lock().await;
            db.catch_up(&self.bucket()?).await?;
            let opt = |value: &Option<String>| value.as_ref().map_or(Value::Null, |v| json!(v));
            let (app, name, status, fault) = (opt(&ask.app), opt(&ask.name), opt(&ask.status), opt(&ask.fault));
            let may = |row: &Map<String, Value>| self.may_see(viewer, &scope, row["app"].as_str().unwrap_or(""));
            for row in db.query("select * from signals where time >= ?1 and (?2 is null or app = ?2) and (?3 is null or name = ?3) and (?4 is null or status = ?4) and (?5 is null or fault = ?5) order by time desc limit ?6", &[json!(since), app.clone(), name.clone(), status.clone(), fault.clone(), json!(limit)])?.into_iter().filter(may) {
                signals.push(self.signal_read(row));
            }
            let errors_only = i64::from(ask.status.as_deref() == Some("error"));
            for row in db.query("select * from rollups where hour >= ?1 and (?2 is null or app = ?2) and (?3 is null or name = ?3) and (?4 = 0 or errors > 0) and (?5 is null or json_extract(key, '$.fault') = ?5) order by hour desc limit ?6", &[json!(since / 3_600_000), app.clone(), name.clone(), json!(errors_only), fault.clone(), json!(MOST_ROLLUPS_READ)])?.into_iter().filter(may) {
                rollups.push(row_read(row, "key"));
            }
            for row in db.query("select * from fingerprints where last >= ?1 and (?2 is null or app = ?2) and (?3 is null or fault = ?3) order by last desc limit ?4", &[json!(since), app, fault, json!(MOST_PRINTS_READ)])?.into_iter().filter(may) {
                prints.push(Value::Object(row));
            }
        }
        let newest = |rows: &mut Vec<Value>, key: &str, most: usize| {
            rows.sort_by_key(|r| std::cmp::Reverse(r[key].as_u64()));
            rows.truncate(most);
        };
        newest(&mut signals, "time", limit);
        newest(&mut rollups, "hour", MOST_ROLLUPS_READ);
        newest(&mut prints, "last", MOST_PRINTS_READ);
        Ok(json!({ "signals": signals, "rollups": rollups, "fingerprints": prints }))
    }

    fn may_see(&self, viewer: &Caller, scope: &Scope, app: &str) -> bool {
        self.with_or(false, |i| {
            let m = i.r.db();
            Ok(match app {
                "" => *scope != Scope::Global || m.may(&viewer.user, "change-settings", scopes::NETWORK),
                app => m.may(&viewer.user, "deploy", &scopes::app(app)),
            })
        })
    }

    /// A stored signal as a reader gets it: the device by name, its attributes as JSON.
    fn signal_read(&self, row: Map<String, Value>) -> Value {
        let device = unhex(row["device"].as_str().unwrap_or("")).map_or_else(|_| "a device".to_string(), |k| self.name_of(&k));
        let mut read = row_read(row, "attrs");
        read["device"] = json!(device);
        read
    }
}

/// `error.type`: the code it carries, or the type of what was thrown, or that it ran out of time, or none known.
fn exception_kind(code: Option<&str>, exception: Option<&Exception>, stage: &str) -> String {
    code.map(str::to_string).or(exception.map(|e| e.kind.clone())).unwrap_or_else(|| if stage == insights::stage::LIMIT { insights::TIMEOUT } else { insights::OTHER }.to_string())
}

/// `row` with its JSON column `key` as JSON.
fn row_read(mut row: Map<String, Value>, key: &str) -> Value {
    let parsed = row.get(key).and_then(Value::as_str).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
    row.insert(key.to_string(), parsed);
    Value::Object(row)
}
