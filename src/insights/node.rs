//! A node's part of insights: the batch it keeps in memory, sent every minute or so and when it stops, and what it reads
//! back for the tool.

use crate::insights::{self, Ask, Attrs, Batch, Exception, Failure, Log, Severity, Span, SpanKind, Status, stage};

use super::calls::record::Destination;
use super::*;

/// How often a device sends what it has to say.
const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// How far back an app's own screens look for what broke.
const ERRORS_SHOWN: u64 = 7 * 86_400_000;
/// The most reports a client's device may make a minute, and events a report may hold.
const REPORTS_A_MINUTE: u32 = 10;
const MOST_REPORTED: usize = 300;

/// What this device hasn't sent yet.
#[derive(Default)]
pub(super) struct State(Mutex<Batch>, Mutex<HashMap<Key, (u64, u32)>>, Mutex<Option<std::collections::HashSet<Key>>>);

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

    /// A call that ran here, or failed trying: one server span, and metrics of what it waited for. Only a failure, a refusal
    /// or a slow call keeps its steps as child spans. Elsewhere isn't a failure, and it says nothing.
    pub(super) async fn finish_call(&self, call: Call, req: &CallRequest, me: &Caller, reply: &Reply) {
        let (code, said) = match reply {
            Reply::Ok(_) => (None, None),
            Reply::Error(said, code) => (Some(code.clone()), Some(said.as_str())),
            Reply::Elsewhere(_) | Reply::Needs(_) => return call.leave(),
        };
        let steps = call.steps();
        let at = call.trace.stage();
        let Call { trace, started, time, span, parent, stage: was, .. } = call;
        trace.stage_is(was);
        trace.leave(parent.clone());
        let ms = started.elapsed().as_millis() as u64;
        let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
        let failure = code.map(|code| Failure::of(code.as_deref(), exception.as_ref(), at));
        let status = failure.as_ref().map_or(Status::Unset, Failure::status);
        let kept = failure.is_some() || ms >= insights::SLOW_MS || insights::sampled(&trace.trace_id());
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let build = crate::config::BUILD;
        let bundle = self.0.bundles.lock().unwrap().get(&req.app).cloned().filter(|_| destination.named);
        let (service, tool) = match destination.named {
            true => (req.app.as_str(), self.tool_named(req).await),
            false => (insights::own(), "call".to_string()),
        };
        let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(build);
        if destination.named {
            attrs = attrs.rpc(service, &tool);
        }
        if kept {
            attrs = attrs.bytes(req.input.to_string().len() as u64 + if let Reply::Ok(v) = reply { v.to_string().len() as u64 } else { 0 });
        }
        let mut message = None;
        if let Some(failure) = &failure {
            let thrown = exception.as_ref().filter(|_| destination.named);
            let (frame, built) = match thrown.and_then(Exception::top_frame) {
                Some(top) => (top.clone(), bundle.clone().unwrap_or_else(|| build.into())),
                None => (format!("stage:{at}"), build.to_string()),
            };
            attrs = attrs.failed(failure).stage(at).fingerprint(failure.kind(), &frame, &built);
            if let (Failure::Broke(_), Some(e)) = (failure, thrown) {
                attrs = attrs.exception(e);
            }
            message = broke(failure, said).as_deref().map(insights::message::cleaned);
        }
        let bucket_of = |field: &str| steps.iter().filter(|s| s["step"] == "bucket").map(|s| s[field].as_u64().unwrap_or(0)).sum::<u64>();
        let (asks, took, behind) = (bucket_of("asks"), bucket_of("ms"), steps.iter().filter(|s| s["step"] == "data").map(|s| s["new"].as_u64().unwrap_or(0)).sum::<u64>());
        let about = Attrs::default().scope(destination.kind).build(build);
        let instance = hex(&self.0.id);
        let traced = Span { time, trace: trace.trace_id(), span, parent, name: tool, service: service.to_string(), version: bundle, instance: instance.clone(), kind: SpanKind::Server, status, duration: ms, message, attrs };
        let mut batch = self.0.insights.0.lock().unwrap();
        batch.span(traced, kept, if destination.named { &steps } else { &[] });
        for (metric, value) in [(insights::BUCKET_ASKS, asks), (insights::BUCKET_DURATION, took), (insights::STEPS_BEHIND, behind)] {
            if value > 0 {
                batch.metric(time, metric, service, &instance, &about, value);
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
        let code = ended.as_ref().err().map(crate::protocol::Coded::of);
        let failure = code.map(|code| Failure::of(code.as_deref(), None, at));
        let status = failure.as_ref().map_or(Status::Unset, Failure::status);
        let ms = started.elapsed().as_millis() as u64;
        let kept = failure.is_some() || ms >= insights::SLOW_MS || insights::sampled(&trace.trace_id());
        let answers = req.from.is_none() && !run_says;
        let destination = self.destination(&req.app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let build = crate::config::BUILD;
        let service = if destination.named { req.app.as_str() } else { insights::own() };
        let name = match (answers, destination.named) {
            (true, true) => self.tool_named(req).await,
            (true, false) => "call".to_string(),
            (false, _) => "route".to_string(),
        };
        let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(build);
        if destination.named {
            attrs = attrs.rpc(service, &name);
        }
        let mut message = None;
        if let Some(failure) = &failure {
            attrs = attrs.failed(failure).stage(at);
            if answers {
                attrs = attrs.fingerprint(failure.kind(), &format!("stage:{at}"), build);
                let said = ended.as_ref().err().map(|err| format!("{err:#}"));
                message = broke(failure, said.as_deref()).as_deref().map(insights::message::cleaned);
            }
        }
        let traced = Span { time, trace: trace.trace_id(), span, parent, name, service: service.to_string(), version: None, instance: hex(&self.0.id), kind: if answers { SpanKind::Server } else { SpanKind::Internal }, status, duration: ms, message, attrs };
        self.0.insights.0.lock().unwrap().span(traced, kept, &tried);
    }

    /// What a call of `app` spent on a model, in tokens and time: OTEL's `gen_ai.client.*` metrics. Dollars stay in `cost`.
    /// Only a call that finished says what it spent, so its outcome is `ok`; one that failed is the call's own span.
    pub(super) fn record_ai(&self, app: &str, me: &Caller, model: &str, input: u64, output: u64, ms: Option<u64>) {
        let destination = self.destination(app, me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let service = if destination.named { app } else { insights::own() };
        let time = now_ms();
        let about = || Attrs::default().scope(destination.kind).model(model).outcome("ok");
        let instance = hex(&self.0.id);
        let mut batch = self.0.insights.0.lock().unwrap();
        batch.metric(time, insights::TOKEN_USAGE, service, &instance, &about().tokens("input"), input);
        batch.metric(time, insights::TOKEN_USAGE, service, &instance, &about().tokens("output"), output);
        if let Some(ms) = ms {
            batch.metric(time, insights::OPERATION_DURATION, service, &instance, &about(), ms);
        }
    }

    /// A log record that's only this device's own, named `name`: an error if it says so.
    pub(super) fn device_signal(&self, name: &str, attrs: Attrs, status: Status) {
        let severity = if status == Status::Error { Severity::Error } else { Severity::Info };
        let log = Log { time: now_ms(), trace: None, span: None, severity, body: name.into(), service: insights::own().into(), version: None, instance: hex(&self.0.id), attrs: attrs.scope("everyone").build(crate::config::BUILD) };
        self.0.insights.0.lock().unwrap().log(log);
    }

    /// Records a metric that's only this device's own.
    fn device_metric(&self, metric: insights::Metric, attrs: Attrs, value: u64) {
        let attrs = attrs.scope("everyone").build(crate::config::BUILD);
        self.0.insights.0.lock().unwrap().metric(now_ms(), metric, insights::own(), &hex(&self.0.id), &attrs, value);
    }

    /// How long a write another device told this one about took to get here. Its clock isn't this one's, so a big gap
    /// says more about the clocks than the network.
    pub(super) fn write_seen(&self, sent: u64) {
        let lag = now_ms().saturating_sub(sent);
        if lag < 600_000 {
            self.device_metric(insights::WRITE_SEEN, Attrs::default(), lag);
        }
    }

    /// Which devices the relay says are on, against who it said a moment ago.
    pub(super) fn relay_says(&self, was: &std::collections::HashSet<Key>, now: &std::collections::HashSet<Key>) {
        for key in was.symmetric_difference(now) {
            self.device_signal(if now.contains(key) { "device.online" } else { "device.offline" }, Attrs::default().says("relay").subject(&hex(key)), Status::Unset);
        }
    }

    /// Whether this node has heard from each other device lately, which may disagree with the relay.
    pub(super) fn node_says(&self) {
        let heard: std::collections::HashSet<Key> = self.with_or(Default::default(), |i| Ok(i.r.db().computes()?.into_iter().map(|c| c.key).filter(|k| *k != self.0.id && i.heard(k)).collect()));
        let was = self.0.insights.2.lock().unwrap().replace(heard.clone());
        for key in was.iter().flat_map(|was| was.symmetric_difference(&heard)) {
            self.device_signal(if heard.contains(key) { "device.online" } else { "device.offline" }, Attrs::default().says("node").subject(&hex(key)), Status::Unset);
        }
    }

    /// This node starting, and why the last one ended: what it said when it stopped, or `abort` if it didn't (panics abort, so
    /// a crash can't say), or `first` if there was none before it. An update's new node says so too.
    pub fn node_started(&self, last: crate::insights::Ended) {
        let reason = last.reason();
        self.device_signal("node.start", Attrs::default().reason(reason), Status::Unset);
        if last == crate::insights::Ended::Update {
            self.device_signal("update.running", Attrs::default().part(&crate::latest::Thing::node_name(crate::config::TARGET)), Status::Unset);
        }
    }

    /// What `req` runs, if its app says it has a tool of that name, else `_unknown`: what a caller typed is theirs to get
    /// wrong, and may be anything.
    async fn tool_named(&self, req: &CallRequest) -> String {
        let listed = self.0.apps.describe(&req.app).await.is_ok_and(|d| d["tools"].as_array().is_some_and(|t| t.iter().any(|t| t["name"] == req.runs())));
        if listed && insights::token(req.runs()) { req.runs().to_string() } else { "_unknown".into() }
    }

    /// Counts what the URL signer failed at since the last send.
    fn say_signer(&self) {
        let failures = crate::bucket::signed::take_failures();
        if failures > 0 {
            self.device_metric(insights::SIGNER_FAILURES, Attrs::default(), failures);
        }
    }

    /// What this device hasn't sent yet, handed over: the batch starts again.
    pub fn take_insights(&self) -> Batch {
        std::mem::take(&mut *self.0.insights.0.lock().unwrap())
    }

    /// Sends what this device has to say to the telemetry Worker. Telemetry may be lost, so what doesn't go is dropped.
    pub async fn flush_insights(&self) {
        self.say_signer();
        let batch = self.take_insights();
        if batch.is_empty() {
            return;
        }
        let Ok(address) = self.address() else { return tracing::debug!("insights: {YOURS} has no address to send them to") };
        let key = self.0.key.clone();
        match tokio::task::spawn_blocking(move || insights::worker::write(&address, &key, &batch)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::debug!("insights weren't sent: {err:#}"),
            Err(err) => tracing::debug!("insights weren't sent: {err}"),
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
        let instance = hex(&me.key);
        let attrs = Attrs::default().client(platform).build(crate::config::BUILD);
        if name == "client.try" {
            let result = e["result"].as_str().unwrap_or("");
            let severity = if result == "answered" { Severity::Info } else { Severity::Warn };
            let attrs = attrs.scope("everyone").tried(e["node"].as_str().unwrap_or(""), result).duration(ms);
            return self.0.insights.0.lock().unwrap().log(Log { time, trace: None, span: None, severity, body: name.into(), service: insights::own().into(), version: None, instance, attrs });
        }
        let exception = e["exception"].is_object().then(|| Exception::from_json(&e["exception"])).flatten();
        let (destination, app, screen) = match name {
            "screen.exception" | "screen.undrawn" => {
                let Some(app) = e["app"].as_str() else { return };
                let Some(destination) = self.destination(app, me) else { return };
                let screen = self.screen_named(app, e["screen"].as_str().unwrap_or("")).await;
                (destination, app, Some(screen))
            }
            "page.exception" => (Destination { scope: Scope::Global, named: false, kind: "everyone" }, "", None),
            _ => return,
        };
        let bundle = self.0.bundles.lock().unwrap().get(app).cloned().filter(|_| destination.named);
        let mut attrs = attrs.scope(destination.kind);
        if let (true, Some(screen)) = (destination.named, &screen) {
            attrs = attrs.screen(screen);
        }
        let exception = exception.filter(|_| destination.named || app.is_empty());
        let (frame, build) = match exception.as_ref().and_then(|x| x.frames.first()) {
            Some(top) => (top.clone(), bundle.clone().unwrap_or_else(|| crate::config::BUILD.into())),
            None => (name.to_string(), crate::config::BUILD.to_string()),
        };
        let kind = exception.as_ref().map_or(insights::OTHER, |x| x.kind.as_str());
        attrs = attrs.error(kind).fingerprint(kind, &frame, &build);
        if let Some(x) = &exception {
            attrs = attrs.exception(x);
        }
        let span_name = match (&screen, destination.named) {
            (Some(screen), true) => format!("screen:{screen}"),
            _ if name == "page.exception" => "page".to_string(),
            _ => "screen".to_string(),
        };
        let service = if destination.named { app.to_string() } else { insights::own().to_string() };
        let message = exception.as_ref().and_then(|x| x.message.as_deref()).map(insights::message::cleaned);
        let span = Span { time, trace: trace::ids::trace(), span: trace::ids::span(), parent: None, name: span_name, service, version: bundle, instance, kind: SpanKind::Internal, status: Status::Error, duration: ms, message, attrs };
        self.0.insights.0.lock().unwrap().span(span, true, &[]);
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
        let (parent_trace, parent) = trace::parent_now();
        let trace = parent_trace.unwrap_or_else(trace::ids::trace);
        let kept = !ok || ms >= insights::SLOW_MS || insights::sampled(&trace);
        let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).scope(destination.kind).build(crate::config::BUILD).tried(&hex(&device), result);
        if kept {
            attrs = attrs.bytes(req.input.to_string().len() as u64 + answer.map_or(0, |a| a.to_string().len() as u64));
        }
        if let Some(error) = error {
            attrs = attrs.error(error);
        }
        let service = if destination.named { req.app.clone() } else { insights::own().to_string() };
        let span = Span { time: now_ms().saturating_sub(ms), trace, span: trace::ids::span(), parent, name: "service.hop".into(), service, version: None, instance: hex(&self.0.id), kind: SpanKind::Client, status: Status::of(ok), duration: ms, message: None, attrs };
        self.0.insights.0.lock().unwrap().span(span, kept, &[]);
    }

    /// A schedule's run: one span, with the call it made under it (`finish_call`). The run stays in `runs` for its own
    /// screens; this is for what reads every kind of failure the same.
    pub(super) fn finish_schedule(&self, app: &str, schedule: &str, user: &str, took: Duration, traced: &trace::Trace, err: Option<&anyhow::Error>) {
        let destination = self.destination(app, &Caller::own(self.0.id, user)).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let ms = took.as_millis() as u64;
        let steps = traced.since(0);
        let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
        let failure = err.map(|err| Failure::of(crate::protocol::Coded::of(err).as_deref(), exception.as_ref(), traced.stage()));
        let status = failure.as_ref().map_or(Status::Unset, Failure::status);
        let mut attrs = Attrs::default().client("schedule").scope(destination.kind).build(crate::config::BUILD);
        if destination.named {
            attrs = attrs.schedule(schedule);
        }
        let mut message = None;
        if let Some(failure) = &failure {
            attrs = attrs.failed(failure);
            if let (Failure::Broke(_), Some(err)) = (failure, err) {
                message = Some(insights::message::cleaned(&format!("{err:#}")));
            }
        }
        let service = if destination.named { app.to_string() } else { insights::own().to_string() };
        let kept = failure.is_some() || ms >= insights::SLOW_MS || insights::sampled(&traced.trace_id());
        let span = Span { time: now_ms().saturating_sub(ms), trace: traced.trace_id(), span: trace::ids::span(), parent: None, name: "schedule.run".into(), service, version: None, instance: hex(&self.0.id), kind: SpanKind::Internal, status, duration: ms, message, attrs };
        self.0.insights.0.lock().unwrap().span(span, kept, &[]);
    }

    /// What a service's endpoint says for insights: its program starting and ending, and OTLP. Only one of the network's
    /// services, about itself: which app is what its key is listed as, never what it says.
    pub(super) async fn service_said(&self, remote: &Key, body: &Value) {
        let Ok(Some((app, user))) = self.with(|i| Ok(i.r.db().service(remote)?.map(|s| (s.service.clone().unwrap_or_default(), s.user.clone())))) else { return };
        let me = Caller::own(*remote, user);
        let destination = self.destination(&app, &me).unwrap_or(Destination { scope: Scope::Global, named: false, kind: "everyone" });
        let base = || Attrs::default().scope(destination.kind).build(crate::config::BUILD);
        let service = if destination.named { app.clone() } else { insights::own().to_string() };
        let instance = hex(&self.0.id);
        let version = body["build"].as_str().filter(|b| insights::token(b)).map(str::to_string);
        if let Some(name) = body["name"].as_str().filter(|n| ["service.start", "service.exit"].contains(n)) {
            let reason = body["reason"].as_str().unwrap_or("");
            let ok = name == "service.start" || ["exit-0", "renewed", "stopped"].contains(&reason);
            let log = Log { time: now_ms(), trace: None, span: None, severity: if ok { Severity::Info } else { Severity::Error }, body: name.to_string(), service: service.clone(), version: version.clone(), instance: instance.clone(), attrs: base().reason(reason) };
            self.0.insights.0.lock().unwrap().log(log);
        }
        for item in insights::otlp::items(&body["otlp"]) {
            let mut attrs = base();
            if let Some(code) = item.status_code {
                attrs = attrs.status_code(code);
            }
            if !item.ok {
                attrs = attrs.error(&item.error);
            }
            let name = if destination.named { item.name.clone() } else { "span".into() };
            let time = now_ms().saturating_sub(item.ms);
            let (trace, span, parent) = item.ids.clone().unwrap_or_else(|| (trace::ids::trace(), trace::ids::span(), None));
            let mut batch = self.0.insights.0.lock().unwrap();
            match item.span_kind {
                Some(kind) => {
                    let kept = !item.ok || item.ms >= insights::SLOW_MS || insights::sampled(&trace);
                    batch.span(Span { time, trace, span, parent, name, service: service.clone(), version: version.clone(), instance: instance.clone(), kind, status: Status::of(item.ok), duration: item.ms, message: None, attrs }, kept, &[]);
                }
                None => batch.log(Log { time, trace: Some(trace), span: Some(span), severity: if item.ok { Severity::Info } else { Severity::Error }, body: name, service: service.clone(), version: version.clone(), instance: instance.clone(), attrs }),
            }
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

    /// What `viewer` may see of how things went, by `ask`, from the telemetry Worker: an app's only for someone who may
    /// deploy it, and what's the network's own program's only for an admin. What this device hasn't sent yet is sent first.
    pub async fn insights(&self, viewer: &Caller, ask: &Ask) -> Result<Value> {
        self.flush_insights().await;
        let services = self.with(|i| {
            let m = i.r.db();
            let mut seen: Vec<String> = m.apps()?.into_iter().map(|a| a.name).filter(|app| app != insights::own() && m.may(&viewer.user, "deploy", &scopes::app(app))).collect();
            if m.may(&viewer.user, "change-settings", scopes::NETWORK) {
                seen.push(insights::own().to_string());
            }
            Ok(seen)
        })?;
        let (address, key) = (self.address()?, self.0.key.clone());
        let fields = insights::worker::asked(ask, services);
        let mut rows = tokio::task::spawn_blocking(move || insights::worker::read(&address, &key, fields)).await??;
        for table in ["spans", "logs", "metrics"] {
            for row in rows[table].as_array_mut().into_iter().flatten() {
                self.read_row(row);
            }
        }
        for row in rows["errors"].as_array_mut().into_iter().flatten() {
            read_error(row);
        }
        Ok(rows)
    }

    /// What's broken most lately in `app`, for someone who may deploy it, as an app's screens show it: empty if it can't be read.
    pub(super) async fn errors_of(&self, viewer: &Caller, app: &str) -> Vec<Value> {
        let ask = Ask { queries: vec!["errors"], service: Some(app.to_string()), status: Some("error".into()), since: Some(now_ms().saturating_sub(ERRORS_SHOWN)), limit: 50, ..Default::default() };
        let rows = match self.insights(viewer, &ask).await {
            Ok(rows) => rows,
            Err(err) => {
                tracing::debug!("{app}'s errors couldn't be read: {err:#}");
                return Vec::new();
            }
        };
        rows["errors"].as_array().into_iter().flatten().map(|e| json!({ "tool": e["name"], "error": format!("{} at {}", e["type"].as_str().unwrap_or(""), e["frame"].as_str().unwrap_or("")), "build": e["build"], "count": e["count"], "first": e["first"], "last": e["last"] })).collect()
    }

    /// A row as a reader gets it: its attributes as JSON, and its device and the devices it names by name.
    fn read_row(&self, row: &mut Value) {
        row["attrs"] = row["attrs"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(json!({}));
        if let Some(device) = row["instance"].as_str().and_then(|k| unhex(k).ok()) {
            row["device"] = json!(self.name_of(&device));
        }
        if let Some(node) = row["attrs"]["one.node"].as_str().and_then(|k| unhex(k).ok()) {
            row["attrs"]["one.node"] = json!(self.name_of(&node));
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

/// An error as a reader gets it: its fingerprint's type, frame and build each by name, and when.
fn read_error(row: &mut Value) {
    let print = row["fingerprint"].as_str().unwrap_or("").to_string();
    let mut parts = print.splitn(3, '|');
    row["type"] = json!(parts.next().unwrap_or(""));
    row["frame"] = json!(parts.next().unwrap_or(""));
    row["build"] = json!(parts.next().unwrap_or(""));
    row["first"] = row["first_seen"].take();
    row["last"] = row["last_seen"].take();
    if let Some(row) = row.as_object_mut() {
        row.remove("first_seen");
        row.remove("last_seen");
    }
}

/// What a call said, if it broke: a refusal's is the caller's own to read, and kept nowhere.
fn broke(failure: &Failure, said: Option<&str>) -> Option<String> {
    matches!(failure, Failure::Broke(_)).then(|| said.map(str::to_string)).flatten()
}
