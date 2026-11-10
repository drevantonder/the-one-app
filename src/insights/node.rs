//! A node's part of insights: the batch per scope it keeps in memory, written every few minutes and when it stops, and
//! what it reads back for the tool.

use serde_json::Map;

use crate::insights::{self, Ask, Attrs, Batch, Exception, Fault, Kind, Print, Signal, SpanKind, stage};

use super::calls::record::Destination;
use super::*;

/// How often a device writes what it has to say. A write per call would cost far more than what it says is worth.
const FLUSH_EVERY: Duration = Duration::from_secs(300);
const MOST_SIGNALS_READ: usize = 2000;
const MOST_ROLLUPS_READ: usize = 20_000;
const MOST_PRINTS_READ: usize = 200;
/// The most reports a client's device may make a minute, and events a report may hold.
const REPORTS_A_MINUTE: u32 = 10;
const MOST_REPORTED: usize = 300;

/// What this device hasn't written yet.
#[derive(Default)]
pub(super) struct State(Mutex<HashMap<Scope, Batch>>, Mutex<HashMap<Key, (u64, u32)>>, Mutex<Option<std::collections::HashSet<Key>>>);

/// A span of a call as it runs here (its run, or trying the devices that might), in the trace of the request that made
/// it. What came before it is put back when it ends: another call's run nests in the same trace.
pub(super) struct Call {
    pub trace: trace::Trace,
    from: usize,
    at: u64,
    started: Instant,
    time: u64,
    span: String,
    parent: Option<String>,
    stage: &'static str,
}

impl Call {
    /// Ends it with nothing said: it's a call for keys it wasn't handed, which the caller answers.
    pub(super) fn leave(self) {
        self.trace.stage_is(self.stage);
        self.trace.leave(self.parent);
    }

    /// Its own steps, each timed from when it began.
    fn steps(&self) -> Vec<Value> {
        let mut steps = self.trace.since(self.from);
        for step in &mut steps {
            step["at"] = json!(step["at"].as_u64().unwrap_or(0).saturating_sub(self.at));
        }
        steps
    }
}

impl Node {
    pub(super) fn begin_call(&self) -> Call {
        let trace = trace::current().unwrap_or_else(trace::Trace::quiet);
        let span = trace::ids::span();
        let parent = trace.enter(span.clone());
        let stage = trace.stage();
        trace.stage_is(stage::CHECK);
        Call { from: trace.count(), at: trace.at(), trace, started: Instant::now(), time: now_ms(), span, parent, stage }
    }

    /// A call that ran here, or failed trying: one span and a place in the rollups, and an error's fingerprint. Only a
    /// failure or a slow call keeps its span whole. Elsewhere isn't a failure, and it says nothing.
    pub(super) async fn finish_call(&self, call: Call, req: &CallRequest, me: &Caller, reply: &Reply) {
        let (ok, code) = match reply {
            Reply::Ok(_) => (true, None),
            Reply::Error(_, code) => (false, code.clone()),
            Reply::Elsewhere(_) | Reply::Needs(_) => return call.leave(),
        };
        let steps = call.steps();
        let at = call.trace.stage();
        let Call { trace, started, time, span, parent, stage: was, .. } = call;
        trace.stage_is(was);
        trace.leave(parent.clone());
        let ms = started.elapsed().as_millis() as u64;
        let kept = !ok || ms >= insights::SLOW_MS || insights::sampled(&trace.trace_id());
        let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
        let fault = (!ok).then(|| insights::fault(code.as_deref(), at, exception.as_ref()));
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let build = crate::config::BUILD;
        let bundle = self.0.bundles.lock().unwrap().get(&req.app).cloned();
        let (app, tool) = match destination.named {
            true => (req.app.as_str(), self.tool_named(req).await),
            false => ("", "".to_string()),
        };
        let mut attrs = Attrs::default().span_kind(SpanKind::Server).device(&hex(&self.0.id)).client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(build);
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
            let (frame, built) = match exception.as_ref().filter(|_| destination.named).and_then(Exception::top_frame) {
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
        let name = if destination.named { tool.clone() } else { "call".into() };
        let bucket_of = |field: &str| steps.iter().filter(|s| s["step"] == "bucket").map(|s| s[field].as_u64().unwrap_or(0)).sum::<u64>();
        let (asks, took, behind) = (bucket_of("asks"), bucket_of("ms"), steps.iter().filter(|s| s["step"] == "data").map(|s| s["new"].as_u64().unwrap_or(0)).sum::<u64>());
        let about = Attrs::default().device(&hex(&self.0.id)).scope(destination.kind).build(build);
        let signal = Signal { time, device: hex(&self.0.id), kind: Kind::Span, name, app: app.to_string(), trace: trace.trace_id(), span, parent, ms, ok, fault, attrs };
        let mut batches = self.0.insights.0.lock().unwrap();
        let batch = batches.entry(destination.scope).or_default();
        batch.add(signal, kept, print);
        for (metric, value) in [("bucket.asks", asks), ("bucket.ms", took), ("steps.behind", behind)] {
            if value > 0 {
                batch.metric(time, metric, app, &about, value);
            }
        }
    }

    /// How trying the devices for a call went (`Node::route`): a span under its request, and, if no device's own run of it
    /// says how it ended (a service answered it, or none could) and nothing an app does is above it, the call's own span.
    pub(super) async fn finish_route(&self, call: Call, req: &CallRequest, me: &Caller, ended: &Result<Value>, run_says: bool) {
        if ended.as_ref().err().is_some_and(|e| e.is::<crate::keys::Needs>()) {
            return call.leave();
        }
        let tried = call.steps().into_iter().filter(|s| s["step"] == "device").collect::<Vec<_>>();
        let at = call.trace.stage();
        let Call { trace, started, time, span, parent, stage: was, .. } = call;
        trace.stage_is(was);
        trace.leave(parent.clone());
        let ok = ended.is_ok();
        let code = ended.as_ref().err().and_then(crate::protocol::Coded::of);
        let ms = started.elapsed().as_millis() as u64;
        let kept = !ok || ms >= insights::SLOW_MS || insights::sampled(&trace.trace_id());
        let answers = req.from.is_none() && !run_says;
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let build = crate::config::BUILD;
        let app = if destination.named { req.app.as_str() } else { "" };
        let name = match (answers, destination.named) {
            (true, true) => self.tool_named(req).await,
            (true, false) => "call".to_string(),
            (false, _) => "route".to_string(),
        };
        let fault = (!ok).then(|| insights::fault(code.as_deref(), at, None));
        let mut attrs = Attrs::default().span_kind(if answers { SpanKind::Server } else { SpanKind::Internal }).device(&hex(&self.0.id)).client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(build);
        if destination.named {
            attrs = attrs.service(app).tool(&name);
        }
        if kept {
            attrs = attrs.steps(&tried);
        }
        let print = (!ok && answers).then(|| {
            let kind = exception_kind(code.as_deref(), None, at);
            attrs = std::mem::take(&mut attrs).error(&kind).stage(at);
            Print { app: app.to_string(), tool: name.clone(), kind, frame: format!("stage:{at}"), build: build.to_string(), fault: fault.unwrap_or(Fault::Internal) }
        });
        if !ok && !answers {
            attrs = attrs.error(code.as_deref().unwrap_or(insights::OTHER)).stage(at);
        }
        let signal = Signal { time, device: hex(&self.0.id), kind: Kind::Span, name, app: app.to_string(), trace: trace.trace_id(), span, parent, ms, ok, fault, attrs };
        self.0.insights.0.lock().unwrap().entry(destination.scope).or_default().add(signal, kept, print);
    }

    /// What a call of `app` spent on a model, in tokens and time: OTEL's `gen_ai.client.*` metrics. Dollars stay in `cost`.
    /// Only a call that finished says what it spent, so its outcome is `ok`; one that failed is the call's own span.
    pub(super) fn record_ai(&self, app: &str, me: &Caller, model: &str, input: u64, output: u64, ms: Option<u64>) {
        let destination = self.destination(app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let named = if destination.named { app } else { "" };
        let time = now_ms();
        let about = || Attrs::default().device(&hex(&self.0.id)).scope(destination.kind).model(model).outcome("ok");
        let mut batches = self.0.insights.0.lock().unwrap();
        let batch = batches.entry(destination.scope.clone()).or_default();
        batch.metric(time, "gen_ai.client.token.usage", named, &about().tokens("input"), input);
        batch.metric(time, "gen_ai.client.token.usage", named, &about().tokens("output"), output);
        if let Some(ms) = ms {
            batch.metric(time, "gen_ai.client.operation.duration", named, &about(), ms);
        }
    }

    /// What's only this device's own, in `global`: named `name`, with a number if it has one.
    pub(super) fn device_signal(&self, name: &str, attrs: Attrs, ok: bool, fault: Option<Fault>) {
        let signal = Signal { time: now_ms(), device: hex(&self.0.id), kind: Kind::Event, name: name.into(), app: String::new(), trace: trace::ids::trace(), span: trace::ids::span(), parent: None, ms: 0, ok, fault, attrs: attrs.device(&hex(&self.0.id)).scope("everyone").build(crate::config::BUILD) };
        self.0.insights.0.lock().unwrap().entry(Scope::Global).or_default().add(signal, true, None);
    }

    /// Records a metric that's only this device's own, in `global`.
    fn device_metric(&self, name: &str, attrs: Attrs, value: u64) {
        let attrs = attrs.device(&hex(&self.0.id)).scope("everyone").build(crate::config::BUILD);
        self.0.insights.0.lock().unwrap().entry(Scope::Global).or_default().metric(now_ms(), name, "", &attrs, value);
    }

    /// How long a write another device told this one about took to get here. Its clock isn't this one's, so a big gap
    /// says more about the clocks than the network.
    pub(super) fn write_seen(&self, sent: u64) {
        let lag = now_ms().saturating_sub(sent);
        if lag < 600_000 {
            self.device_metric("write.seen", Attrs::default(), lag);
        }
    }

    /// Which devices the relay says are on, against who it said a moment ago.
    pub(super) fn relay_says(&self, was: &std::collections::HashSet<Key>, now: &std::collections::HashSet<Key>) {
        for key in was.symmetric_difference(now) {
            self.device_signal(if now.contains(key) { "device.online" } else { "device.offline" }, Attrs::default().says("relay").subject(&hex(key)), true, None);
        }
    }

    /// Whether this node has heard from each other device lately, which may disagree with the relay.
    pub(super) fn node_says(&self) {
        let heard: std::collections::HashSet<Key> = self.with_or(Default::default(), |i| Ok(i.r.db().computes()?.into_iter().map(|c| c.key).filter(|k| *k != self.0.id && i.heard(k)).collect()));
        let was = self.0.insights.2.lock().unwrap().replace(heard.clone());
        for key in was.iter().flat_map(|was| was.symmetric_difference(&heard)) {
            self.device_signal(if heard.contains(key) { "device.online" } else { "device.offline" }, Attrs::default().says("node").subject(&hex(key)), true, None);
        }
    }

    /// This node starting, and why the last one ended: what it said when it stopped, or `abort` if it didn't (panics abort, so
    /// a crash can't say), or `first` if there was none before it. An update's new node says so too.
    pub fn node_started(&self, last: crate::insights::Ended) {
        let reason = last.reason();
        self.device_signal("node.start", Attrs::default().reason(reason), true, None);
        if last == crate::insights::Ended::Update {
            self.device_signal("update.running", Attrs::default().part(&crate::latest::Thing::node_name(crate::config::TARGET)), true, None);
        }
    }

    /// What `req` runs, if its app says it has a tool of that name, else `_unknown`: what a caller typed is theirs to get
    /// wrong, and may be anything.
    async fn tool_named(&self, req: &CallRequest) -> String {
        let listed = self.0.apps.describe(&req.app).await.is_ok_and(|d| d["tools"].as_array().is_some_and(|t| t.iter().any(|t| t["name"] == req.runs())));
        if listed { req.runs().to_string() } else { "_unknown".into() }
    }

    /// Counts what the URL signer failed at since the last write.
    fn say_signer(&self) {
        let failures = crate::bucket::signed::take_failures();
        if failures > 0 {
            self.device_metric("signer.failures", Attrs::default(), failures);
        }
    }

    /// Writes what this device has to say, to each scope it holds, once it has anything: telemetry may be lost, so what
    /// doesn't go is dropped.
    pub async fn flush_insights(&self) {
        self.say_signer();
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

    /// What a client says went wrong on it (a screen that threw or never drew, its page's own errors) and what it did
    /// before that (`client.try`). What it says of itself is believed only as far as closed sets go: the app must exist and
    /// the screen be one of its, and a report more than `REPORTS_A_MINUTE` a minute is dropped.
    pub async fn report_insights(&self, me: &Caller, platform: &str, events: &[Value]) {
        let minute = now_ms() / 60_000;
        {
            let mut counts = self.0.insights.1.lock().unwrap();
            let count = counts.entry(me.key).or_insert((minute, 0));
            *count = if count.0 == minute { (minute, count.1 + 1) } else { (minute, 1) };
            if count.1 > REPORTS_A_MINUTE {
                return;
            }
        }
        for event in events.iter().take(MOST_REPORTED) {
            self.reported_event(me, platform, event).await;
        }
    }

    async fn reported_event(&self, me: &Caller, platform: &str, e: &Value) {
        let now = now_ms();
        let time = e["time"].as_u64().filter(|t| *t + 300_000 >= now && *t <= now + 60_000).unwrap_or(now);
        let name = e["name"].as_str().unwrap_or("");
        let ms = e["ms"].as_u64().unwrap_or(0);
        let attrs = Attrs::default().device(&hex(&me.key)).client(platform).build(crate::config::BUILD);
        let exception = e["exception"].is_object().then(|| Exception::from_json(&e["exception"])).flatten();
        let (destination, app, screen) = match name {
            "screen.exception" | "screen.undrawn" => {
                let Some(app) = e["app"].as_str() else { return };
                let Some(destination) = self.destination(app, me) else { return };
                let screen = self.screen_named(app, e["screen"].as_str().unwrap_or("")).await;
                (destination, app, Some(screen))
            }
            "page.exception" | "client.try" => (Destination { scope: Scope::Global, named: false, kind: "everyone" }, "", None),
            _ => return,
        };
        let (ok, result) = match name {
            "client.try" => (e["result"] == "answered", Some(e["result"].as_str().unwrap_or(""))),
            _ => (false, None),
        };
        let bundle = self.0.bundles.lock().unwrap().get(app).cloned();
        let app_named = if destination.named { app } else { "" };
        let mut attrs = attrs.scope(destination.kind);
        if destination.named && !app.is_empty() {
            attrs = attrs.service(app);
            if let Some(bundle) = &bundle {
                attrs = attrs.version(bundle);
            }
            if let Some(screen) = &screen {
                attrs = attrs.screen(screen);
            }
        }
        if let Some(result) = result {
            attrs = attrs.tried(e["node"].as_str().unwrap_or(""), result);
        }
        let exception = exception.filter(|_| destination.named || app.is_empty());
        let mut print = None;
        if !ok && result.is_none() {
            let (frame, build) = match exception.as_ref().and_then(|x| x.frames.first()) {
                Some(top) => (top.clone(), bundle.clone().unwrap_or_else(|| crate::config::BUILD.into())),
                None => (name.to_string(), crate::config::BUILD.to_string()),
            };
            attrs = attrs.error(exception.as_ref().map_or(insights::OTHER, |x| x.kind.as_str()));
            if let Some(x) = &exception {
                attrs = attrs.exception(x);
            }
            let tool = match (&screen, destination.named) {
                (Some(screen), true) => format!("screen:{screen}"),
                _ if name == "page.exception" => "page".into(),
                _ => "screen".into(),
            };
            print = Some(Print { app: app_named.to_string(), tool, kind: exception.as_ref().map_or_else(|| insights::OTHER.to_string(), |x| x.kind.clone()), frame, build, fault: Fault::Internal });
        }
        let signal = Signal { time, device: hex(&me.key), kind: Kind::Event, name: name.to_string(), app: app_named.to_string(), trace: trace::ids::trace(), span: trace::ids::span(), parent: None, ms, ok, fault: (!ok && result.is_none()).then_some(Fault::Internal), attrs };
        self.0.insights.0.lock().unwrap().entry(destination.scope).or_default().add(signal, true, print);
    }

    /// A service's program going through a hop's worth of a call: one child span, with its status and bytes. A hop that
    /// failed doesn't fail the call, which can go on to the next device: the call's own span says how that ended.
    pub(super) fn finish_hop(&self, req: &CallRequest, me: &Caller, device: Key, took: Duration, asked: &Result<Value>) {
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let ms = took.as_millis() as u64;
        let answer = asked.as_ref().ok();
        let ok = answer.is_some_and(|a| a.get("error").is_none());
        let code = answer.and_then(|a| a["code"].as_str());
        let (error, result) = match asked {
            Ok(_) if ok => (None, "answered"),
            Ok(_) => (Some(code.unwrap_or(insights::OTHER)), "failed"),
            Err(err) => hop_failed(err),
        };
        let fault = (!ok).then(|| insights::fault(code, stage::RUN, None));
        let (parent_trace, parent) = trace::parent_now();
        let trace = parent_trace.unwrap_or_else(trace::ids::trace);
        let kept = !ok || ms >= insights::SLOW_MS || insights::sampled(&trace);
        let mut attrs = Attrs::default().span_kind(SpanKind::Client).device(&hex(&self.0.id)).client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(crate::config::BUILD).tried(&hex(&device), result);
        if destination.named {
            attrs = attrs.service(&req.app);
        }
        if kept {
            attrs = attrs.bytes(req.input.to_string().len() as u64 + answer.map_or(0, |a| a.to_string().len() as u64));
        }
        if let Some(error) = error {
            attrs = attrs.error(error);
        }
        let app = if destination.named { req.app.clone() } else { String::new() };
        let signal = Signal { time: now_ms().saturating_sub(ms), device: hex(&self.0.id), kind: Kind::Span, name: "service.hop".into(), app, trace, span: trace::ids::span(), parent, ms, ok, fault, attrs };
        self.0.insights.0.lock().unwrap().entry(destination.scope).or_default().add(signal, kept, None);
    }

    /// A schedule's run: one span, with the call it made under it (`finish_call`). The run stays in `runs` for its own
    /// screens; this is for what reads every kind of failure the same.
    pub(super) fn finish_schedule(&self, app: &str, schedule: &str, user: &str, took: Duration, traced: &trace::Trace, err: Option<&anyhow::Error>) {
        let destination = self.destination(app, &Caller::own(self.0.id, user)).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let ms = took.as_millis() as u64;
        let ok = err.is_none();
        let code = err.and_then(crate::protocol::Coded::of);
        let steps = traced.since(0);
        let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
        let fault = (!ok).then(|| insights::fault(code.as_deref(), stage::RUN, None));
        let mut attrs = Attrs::default().span_kind(SpanKind::Internal).device(&hex(&self.0.id)).client("schedule").scope(destination.kind).build(crate::config::BUILD);
        if destination.named {
            attrs = attrs.service(app).schedule(schedule);
        }
        if !ok {
            attrs = attrs.error(&exception_kind(code.as_deref(), exception.as_ref(), traced.stage()));
        }
        let app = if destination.named { app.to_string() } else { String::new() };
        let signal = Signal { time: now_ms().saturating_sub(ms), device: hex(&self.0.id), kind: Kind::Span, name: "schedule.run".into(), app, trace: traced.trace_id(), span: trace::ids::span(), parent: None, ms, ok, fault, attrs };
        self.0.insights.0.lock().unwrap().entry(destination.scope).or_default().add(signal, !ok || ms >= insights::SLOW_MS || insights::sampled(&traced.trace_id()), None);
    }

    /// What a service's endpoint says for insights: its program starting and ending, and OTLP. Only one of the network's
    /// services, about itself: which app is what its key is listed as, never what it says.
    pub(super) async fn service_said(&self, remote: &Key, body: &Value) {
        let Ok(Some((app, user))) = self.with(|i| Ok(i.r.db().service(remote)?.map(|s| (s.service.clone().unwrap_or_default(), s.user.clone())))) else { return };
        let me = Caller::own(*remote, user);
        let destination = self.destination(&app, &me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let base = || Attrs::default().device(&hex(&self.0.id)).scope(destination.kind).build(crate::config::BUILD);
        let named_app = if destination.named { app.clone() } else { String::new() };
        let say = |signal: Signal, kept: bool| self.0.insights.0.lock().unwrap().entry(destination.scope.clone()).or_default().add(signal, kept, None);
        let signal = |name: &str, ms: u64, ok: bool, attrs: Attrs| Signal { time: now_ms().saturating_sub(ms), device: hex(&self.0.id), kind: Kind::Span, name: name.to_string(), app: named_app.clone(), trace: trace::ids::trace(), span: trace::ids::span(), parent: None, ms, ok, fault: (!ok).then_some(Fault::Internal), attrs };
        if let Some(name) = body["name"].as_str().filter(|n| ["service.start", "service.exit"].contains(n)) {
            let reason = body["reason"].as_str().unwrap_or("");
            let ok = name == "service.start" || ["exit-0", "renewed", "stopped"].contains(&reason);
            let mut attrs = base().version(body["build"].as_str().unwrap_or("")).reason(reason);
            if destination.named {
                attrs = attrs.service(&app);
            }
            let event = Signal { kind: Kind::Event, ..signal(name, 0, ok, attrs) };
            say(event, true);
        }
        for span in insights::otlp::spans(&body["otlp"]) {
            let mut attrs = base();
            if span.kind == Kind::Span {
                attrs = attrs.span_kind(span.span_kind);
            }
            if destination.named {
                attrs = attrs.service(&app);
            }
            if let Some(code) = span.status_code {
                attrs = attrs.status_code(code);
            }
            if !span.ok {
                attrs = attrs.error(&span.error);
            }
            let name = if destination.named { span.name.clone() } else { "span".into() };
            let mut built = signal(&name, span.ms, span.ok, attrs);
            built.kind = span.kind;
            if let Some(ids) = span.ids {
                (built.trace, built.span, built.parent) = ids;
            }
            let kept = !span.ok || span.ms >= insights::SLOW_MS || insights::sampled(&built.trace);
            say(built, kept);
        }
    }

    async fn screen_named(&self, app: &str, screen: &str) -> String {
        let listed = self.0.apps.describe(app).await.is_ok_and(|d| d["screens"].as_array().is_some_and(|s| s.iter().any(|s| s["name"] == screen)));
        if listed { screen.to_string() } else { "_unknown".into() }
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
                let mut rollup = row_read(row, "key");
                rollup["service"] = service_of(&rollup);
                self.name_devices(&mut rollup["key"]);
                rollups.push(rollup);
            }
            for row in db.query("select * from fingerprints where last >= ?1 and (?2 is null or app = ?2) and (?3 is null or fault = ?3) order by last desc limit ?4", &[json!(since), app, fault, json!(MOST_PRINTS_READ)])?.into_iter().filter(may) {
                let mut print = Value::Object(row);
                print["service"] = service_of(&print);
                prints.push(print);
            }
        }
        let newest = |rows: &mut Vec<Value>, key: &str, most: usize| {
            rows.sort_by_key(|r| std::cmp::Reverse(r[key].as_u64()));
            rows.truncate(most);
        };
        let more_signals = signals.len() > limit;
        let more_rollups = rollups.len() > MOST_ROLLUPS_READ;
        newest(&mut signals, "time", limit);
        newest(&mut rollups, "hour", MOST_ROLLUPS_READ);
        newest(&mut prints, "last", MOST_PRINTS_READ);
        Ok(json!({ "signals": signals.into_iter().flat_map(with_steps).collect::<Vec<_>>(), "rollups": rollups, "fingerprints": prints, "more_signals": more_signals, "more_rollups": more_rollups }))
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
        read["service"] = service_of(&read);
        self.name_devices(&mut read["attrs"]);
        read
    }

    /// A device's key among `attrs`, as a reader knows it: its name.
    fn name_devices(&self, attrs: &mut Value) {
        for key in ["one.device", "one.node"] {
            if let Some(device) = attrs[key].as_str().and_then(|k| unhex(k).ok()) {
                attrs[key] = json!(self.name_of(&device));
            }
        }
    }
}

/// Why a device couldn't be asked, as `error.type` and `one.result`.
fn hop_failed(err: &anyhow::Error) -> (Option<&'static str>, &'static str) {
    if err.is::<net::NotThere>() {
        (Some(insights::UNREACHABLE), "unreachable")
    } else if err.to_string() == "timed out" {
        (Some(insights::TIMEOUT), "no answer")
    } else {
        (Some(insights::LOST), "lost")
    }
}

/// `error.type`: the code it carries, or the type of what was thrown, or that it ran out of time, or none known.
fn exception_kind(code: Option<&str>, exception: Option<&Exception>, stage: &str) -> String {
    code.map(str::to_string).or(exception.map(|e| e.kind.clone())).unwrap_or_else(|| if stage == insights::stage::LIMIT { insights::TIMEOUT } else { insights::OTHER }.to_string())
}

/// OTEL's `service.name` of a row: its app, or the network's own program for what isn't an app's.
fn service_of(row: &Value) -> Value {
    if row["app"] == "" { json!(crate::config::CLI) } else { row["app"].clone() }
}

/// A span and, as OTEL's readers expect them, each step it went through as a child span.
fn with_steps(mut span: Value) -> Vec<Value> {
    let steps = span["attrs"].as_object_mut().and_then(|a| a.remove("one.steps")).and_then(|s| s.as_array().cloned()).unwrap_or_default();
    let mut all = Vec::with_capacity(steps.len() + 1);
    for (i, step) in steps.iter().enumerate() {
        let mut attrs = Map::from_iter([("span.kind".to_string(), json!(SpanKind::Internal.as_str()))]);
        for (key, value) in step.as_object().into_iter().flatten().filter(|(k, _)| !["step", "at", "ms"].contains(&k.as_str())) {
            attrs.insert(format!("one.{key}"), value.clone());
        }
        let mut child = span.clone();
        child["name"] = step["step"].clone();
        child["time"] = json!(span["time"].as_u64().unwrap_or(0) + step["at"].as_u64().unwrap_or(0));
        child["ms"] = step["ms"].clone();
        child["span"] = json!(format!("{}.{i}", span["span"].as_str().unwrap_or("")));
        child["parent"] = span["span"].clone();
        child["status"] = json!("ok");
        child["fault"] = Value::Null;
        child["attrs"] = Value::Object(attrs);
        all.push(child);
    }
    all.insert(0, span);
    all
}

/// `row` with its JSON column `key` as JSON.
fn row_read(mut row: Map<String, Value>, key: &str) -> Value {
    let parsed = row.get(key).and_then(Value::as_str).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
    row.insert(key.to_string(), parsed);
    Value::Object(row)
}
