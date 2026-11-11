//! Insights, as an app runtime runs it: the batch this device keeps in memory, sent every minute or so (`flush`) and
//! when it stops, telemetry checking its own path, what clients and services report, and reading it back for the tool.
//! The runtime records each call, route, hop and schedule run here as it ends (`begin_call`, `finish_*`).

mod routes;
mod tools;
mod worker;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use api::call::CallRequest;
use api::runtime::Runs;
use api::runtime::prelude::*;
use api::trace;
use api::traceparent::stage;
use api::Caller;
use custom_build::config::YOURS;
use custom_build::now_ms;
use insights::{Ask, Attrs, Batch, Exception, Failure, Log, Severity, Span, SpanKind, Status};
use key::{Key, hex, short, unhex};
use scope::scopes::{self, Principal};
use serde_json::{Value, json};

pub fn runs() -> Runs {
    Runs {
        tools: tools::served(),
        routes: routes::routes(),
        state: Box::new(State::default()),
        every: Some((FLUSH_EVERY, |rt| Box::pin(flush_loop(rt)))),
        // Telemetry's own check failing, for an admin: the network's own program is theirs.
        problem: Some(|rt, me| {
            let admin = rt.read_or(false, |r| Ok(r.db().may(&me.user, "change-settings", scopes::NETWORK)));
            Ok(telemetry_problem(rt).filter(|_| admin))
        }),
        ..Runs::named("insights")
    }
}

/// How often a device sends what it has to say.
const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// How often a device sends a known signal down telemetry's own path, and reads back the last one it sent. Longer than what
/// a row takes to be read (a minute or two), so a check only ever asks about what ought to have arrived.
const CANARY_EVERY: Duration = Duration::from_secs(15 * 60);
/// How many checks in a row have to fail before that's a problem: telemetry may be lost, so one isn't.
const CANARY_FAILS: u32 = 2;
/// What the known signal is named, in each of the three tables.
const CANARY: &str = "telemetry.canary";
/// The most reports a client's device may make a minute, and events a report may hold.
const REPORTS_A_MINUTE: u32 = 10;
const MOST_REPORTED: usize = 300;
/// How long ago what a client reports may have happened: a client keeps what no node took while none answers, and
/// sends it once one does.
const KEPT_UNSENT: u64 = 86_400_000;
/// How long a span this device sent stays with it too, and the most that do: a row takes a minute or two to be readable,
/// and a failure names its trace to be read at once.
const ARRIVING_FOR: u64 = 180_000;
const MOST_ARRIVING: usize = 2000;

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// What this device hasn't sent yet.
    batch: Mutex<Batch>,
    /// How many reports each client's device made this minute.
    reports: Mutex<HashMap<Key, (u64, u32)>>,
    /// How its check of telemetry's own path is going.
    canary: Mutex<Canary>,
    /// Each service's program's cumulative readings, as last said.
    programs: Mutex<HashMap<(String, String), insights::otlp::Point>>,
    /// The spans it sent lately (`ARRIVING_FOR`), each with when.
    arriving: Mutex<std::collections::VecDeque<(u64, Span)>>,
}

/// Telemetry checking itself: what it last sent, and how many checks in a row failed, how and since when; and when it
/// last checked.
#[derive(Default)]
struct Canary {
    sent: Option<u64>,
    failed: u32,
    failing: Option<(&'static str, u64)>,
    checked: Option<Instant>,
}

/// Where a call's signals go: whether there's an app to name (every member can list every app's name, so only a call of
/// no such app has none), and the kind of the app's owner's scope.
struct Destination {
    named: bool,
    kind: &'static str,
}

const EVERYONE: Destination = Destination { named: false, kind: "everyone" };

/// What a call's telemetry says of `app`: that it's an app, and its owner's kind of scope. `None` for no such app, as
/// what a caller typed is theirs and may be anything.
fn destination(rt: &Rt, app: &str) -> Option<Destination> {
    rt.read(|r| {
        let Some(owner) = r.db().app(app)?.map(|a| a.owner) else { return Ok(None) };
        let kind = match owner {
            Principal::User(_) => "user",
            Principal::Group(_) => "group",
            _ => "everyone",
        };
        Ok(Some(Destination { named: true, kind }))
    })
    .ok()
    .flatten()
}

/// A device's name, as a reader of a row gets it.
fn name_of(rt: &Rt, key: &Key) -> String {
    rt.read_or(short(key), |r| Ok(r.name_of(key)))
}


/// A span of a call as it runs here (its run, or trying the devices that might), in the trace of the request that made
/// it. What came before it is put back when it ends: another call's run nests in the same trace.
pub struct Call {
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
    pub fn leave(self) {
        self.trace.stage_is(self.stage);
        self.trace.leave(self.parent);
    }

    /// Its own steps, each timed from when it began.
    fn steps(&self) -> Vec<Value> {
        let mut steps = self.trace.within(self.from, &self.span);
        for step in &mut steps {
            step["at"] = json!(step["at"].as_u64().unwrap_or(0).saturating_sub(self.at));
        }
        steps
    }
}

pub fn begin_call() -> Call {
    let trace = trace::current().unwrap_or_else(trace::Trace::quiet);
    trace.call_begins();
    let span = api::traceparent::ids::span();
    let parent = trace.enter(span.clone());
    let stage = trace.stage();
    trace.stage_is(stage::CHECK);
    Call { from: trace.count(), at: trace.at(), trace, started: Instant::now(), time: now_ms(), span, parent, stage }
}

/// How a call that ran here ended: its answer, or what it said as it failed and the kind of failure callers act on
/// (`protocol::code`).
pub enum Ran<'a> {
    Answered(&'a Value),
    Failed(&'a str, Option<String>),
}

/// A call that ran here, or failed trying: one server span, and metrics of what it waited for. Only a failure, a refusal
/// or a slow call keeps its steps as child spans. One that goes elsewhere says nothing: `Call::leave`.
pub async fn finish_call(rt: &Rt, call: Call, req: &CallRequest, me: &Caller, ran: Ran<'_>) {
    let (code, said) = match ran {
        Ran::Answered(_) => (None, None),
        Ran::Failed(said, ref code) => (Some(code.clone()), Some(said)),
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
    let kept = failure.is_some() || ms >= custom_build::protocol::SLOW_MS || api::traceparent::sampled(&trace.trace_id());
    let destination = destination(rt, &req.app).unwrap_or(EVERYONE);
    let build = custom_build::config::BUILD;
    let bundle = rt.bundle_here(&req.app).filter(|_| destination.named);
    let exception = located(rt, exception, bundle.clone()).await;
    let (service, tool) = match destination.named {
        true => (req.app.as_str(), tool_named(rt, req).await),
        false => (insights::own(), "call".to_string()),
    };
    let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).user(&me.user).scope(destination.kind).build(build);
    if destination.named {
        attrs = attrs.rpc(service, &tool);
    }
    if kept {
        attrs = attrs.bytes(req.input.to_string().len() as u64 + if let Ran::Answered(v) = ran { v.to_string().len() as u64 } else { 0 });
    }
    let mut message = None;
    if let Some(failure) = &failure {
        let thrown = exception.as_ref().filter(|_| destination.named);
        let (frame, built) = match thrown.and_then(Exception::top_frame) {
            Some(top) => (top, bundle.clone().unwrap_or_else(|| build.into())),
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
    let instance = hex(&rt.id());
    let traced = Span { time, trace: trace.trace_id(), span, parent, name: tool, service: service.to_string(), version: bundle, instance: instance.clone(), kind: SpanKind::Server, status, duration: ms, message, attrs };
    let mut batch = rt.state::<State>().batch.lock().unwrap();
    batch.span(traced, kept, if destination.named { &steps } else { &[] });
    for (metric, value) in [(insights::BUCKET_ASKS, asks), (insights::BUCKET_DURATION, took), (insights::STEPS_BEHIND, behind)] {
        if value > 0 {
            batch.metric(time, metric, service, &instance, &about, value);
        }
    }
}

/// An AI Gateway call (ai-gateway/runtime), a child of the call that asked (`Batch::gen_ai`).
pub fn record_gen_ai(rt: &Rt, trace: Option<&trace::Trace>, service: &str, user: &str, call: ::ai_gateway::Span) {
    let trace = trace.map(|t| (t.trace_id(), t.span()));
    rt.state::<State>().batch.lock().unwrap().gen_ai(now_ms(), trace, service, &hex(&rt.id()), user, call);
}

/// A request to one of the API's routes that failed with no call to say so, so its trace has a span to find: named by
/// its route, never its path.
pub fn finish_route_request(rt: &Rt, traced: &trace::Trace, route: &str, took: Duration, failed: &::api::RouteFailed) {
    if traced.called() {
        return;
    }
    let failure = if failed.invalid { Failure::Refused(insights::INVALID.into()) } else { Failure::of(failed.code.as_deref(), None, stage::CHECK) };
    let name = route.replace(['{', '}', '*'], "");
    let build = custom_build::config::BUILD;
    let attrs = Attrs::default().client("api").build(build).failed(&failure).stage(stage::CHECK).fingerprint(failure.kind(), &name, build);
    let message = matches!(failure, Failure::Broke(_)).then(|| insights::message::cleaned(&failed.said));
    let span = Span { time: now_ms().saturating_sub(took.as_millis() as u64), trace: traced.trace_id(), span: api::traceparent::ids::span(), parent: traced.span(), name, service: insights::own().to_string(), version: None, instance: hex(&rt.id()), kind: SpanKind::Server, status: failure.status(), duration: took.as_millis() as u64, message, attrs };
    rt.state::<State>().batch.lock().unwrap().span(span, true, &[]);
}

/// How trying the devices for a call went (`Node::route`): a span under its request, and, if no device's own run of it
/// says how it ended (a service answered it, or none could) and nothing an app does is above it, the call's own span.
pub async fn finish_route(rt: &Rt, call: Call, req: &CallRequest, me: &Caller, ended: &Result<Value>, run_says: bool) {
    if ended.as_ref().err().is_some_and(|e| e.is::<::api::handed::Needs>()) {
        return call.leave();
    }
    let tried = call.steps().into_iter().filter(|s| s["step"] == "device").collect::<Vec<_>>();
    let at = call.trace.stage();
    let Call { trace, started, time, span, parent, stage: was, .. } = call;
    trace.stage_is(was);
    trace.leave(parent.clone());
    // A device that ran it says how that went in its own span: trying the devices only failed if none did.
    let failure = ended.as_ref().err().filter(|_| !run_says).map(|err| match err.downcast_ref::<::api::runtime::NoneCould>() {
        Some(none) => Failure::Broke(none.kind.into()),
        None => Failure::of(custom_build::protocol::Coded::of(err).as_deref(), None, at),
    });
    let status = failure.as_ref().map_or(Status::Unset, Failure::status);
    let ms = started.elapsed().as_millis() as u64;
    let kept = ended.is_err() || ms >= custom_build::protocol::SLOW_MS || api::traceparent::sampled(&trace.trace_id());
    let answers = req.from.is_none() && !run_says;
    let destination = destination(rt, &req.app).unwrap_or(EVERYONE);
    let build = custom_build::config::BUILD;
    let service = if destination.named { req.app.as_str() } else { insights::own() };
    let name = match (answers, destination.named) {
        (true, true) => tool_named(rt, req).await,
        (true, false) => "call".to_string(),
        (false, _) => "route".to_string(),
    };
    let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).user(&me.user).scope(destination.kind).build(build);
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
    let traced = Span { time, trace: trace.trace_id(), span, parent, name, service: service.to_string(), version: None, instance: hex(&rt.id()), kind: if answers { SpanKind::Server } else { SpanKind::Internal }, status, duration: ms, message, attrs };
    rt.state::<State>().batch.lock().unwrap().span(traced, kept, &tried);
}

/// A log record that's only this device's own, named `name`: an error if it says so.
pub fn device_signal(rt: &Rt, name: &str, attrs: Attrs, status: Status) {
    let severity = if status == Status::Error { Severity::Error } else { Severity::Info };
    let log = Log { time: now_ms(), trace: None, span: None, severity, body: name.into(), service: insights::own().into(), version: None, instance: hex(&rt.id()), attrs: attrs.scope("everyone").build(custom_build::config::BUILD) };
    rt.state::<State>().batch.lock().unwrap().log(log);
}

/// Records a metric that's only this device's own.
fn device_metric(rt: &Rt, metric: insights::Metric, attrs: Attrs, value: u64) {
    let attrs = attrs.scope("everyone").build(custom_build::config::BUILD);
    rt.state::<State>().batch.lock().unwrap().metric(now_ms(), metric, insights::own(), &hex(&rt.id()), &attrs, value);
}

/// How long a write another device told this one about took to get here. Its clock isn't this one's, so a big gap
/// says more about the clocks than the network.
pub fn write_seen(rt: &Rt, sent: u64) {
    let lag = now_ms().saturating_sub(sent);
    if lag < 600_000 {
        device_metric(rt, insights::WRITE_SEEN, Attrs::default(), lag);
    }
}

/// Which devices the relay says are on, against who it said a moment ago.
pub fn relay_says(rt: &Rt, was: &std::collections::HashSet<Key>, now: &std::collections::HashSet<Key>) {
    for key in was.symmetric_difference(now) {
        device_signal(rt, if now.contains(key) { "device.online" } else { "device.offline" }, Attrs::default().says("relay").subject(&hex(key)), Status::Unset);
    }
}

/// What `req` runs, if its app says it has a tool of that name: what a caller typed is theirs to get wrong, and may be
/// anything. Else `_unknown`, or for a function of its service, which nothing lists, `_function`.
async fn tool_named(rt: &Rt, req: &CallRequest) -> String {
    let listed = rt.describe(&req.app).await.is_ok_and(|d| d["tools"].as_array().is_some_and(|t| t.iter().any(|t| t["name"] == req.runs())));
    match (listed && insights::token(req.runs()), &req.service) {
        (true, _) => req.runs().to_string(),
        (false, Some(_)) => "_function".into(),
        (false, None) => "_unknown".into(),
    }
}

/// Counts what the URL signer failed at since the last send, the requests to Cloudflare that passed their limit, and
/// what this device read of the bucket.
fn say_requests(rt: &Rt) {
    for (kind, found, n) in bucket::READS.take() {
        device_metric(rt, insights::BUCKET_READS, Attrs::default().reached(kind).outcome(if found { "found" } else { "missing" }).client("node"), n);
    }
    let failures = bucket::signed::take_failures();
    if failures > 0 {
        device_metric(rt, insights::SIGNER_FAILURES, Attrs::default(), failures);
    }
    for (reached, ended, n) in platform::hedge::take_late() {
        device_metric(rt, insights::LATE, Attrs::default().reached(reached).outcome(ended), n);
    }
}

/// What this device hasn't sent yet, handed over: the batch starts again.
pub fn take(rt: &Rt) -> Batch {
    std::mem::take(&mut *rt.state::<State>().batch.lock().unwrap())
}

/// `exception` with each frame where it was written, by what was deployed beside what ran (`bundle`, or the client's own
/// screens for none): only ever a location, as a frame is. Read from disk, so never on the thread answering calls.
async fn located(rt: &Rt, exception: Option<Exception>, bundle: Option<String>) -> Option<Exception> {
    let exception = exception?;
    let (rt, ran) = (rt.clone(), exception.clone());
    Some(tokio::task::spawn_blocking(move || {
        let files = bundle.and_then(|bundle| deploy::bundle::unpack(&std::fs::read(database::blob_path(rt.home(), &bundle).ok()?).ok()?).ok()).unwrap_or_default();
        exception.located(|file| map_of(&rt, file, &files))
    }).await.unwrap_or(ran))
}

/// The map of the file a frame is in: an app's module's from its bundle's `files`, or a screen's, an app's own or, with no
/// bundle, the client's.
fn map_of(rt: &Rt, file: &str, files: &[(String, Vec<u8>)]) -> Option<String> {
    let in_bundle = |name: &str| files.iter().find(|(path, _)| path == name).and_then(|(_, bytes)| String::from_utf8(bytes.clone()).ok());
    if file == "app.js" {
        return in_bundle(deploy::bundle::BUILT);
    }
    let screen = file.strip_suffix(".screen.js")?;
    if files.is_empty() {
        return rt.client_map(screen).map(str::to_string);
    }
    in_bundle(&format!("src/dist/{screen}.html.map"))
}

/// Sends what this device has to say to the telemetry Worker, and says whether it went. Telemetry may be lost, so what
/// doesn't go is dropped.
pub async fn flush(rt: &Rt) -> bool {
    say_requests(rt);
    let batch = take(rt);
    if batch.is_empty() {
        return true;
    }
    {
        let now = now_ms();
        let mut arriving = rt.state::<State>().arriving.lock().unwrap();
        arriving.extend(batch.spans.iter().cloned().map(|span| (now, span)));
        while arriving.len() > MOST_ARRIVING || arriving.front().is_some_and(|(sent, _)| now.saturating_sub(*sent) > ARRIVING_FOR) {
            arriving.pop_front();
        }
    }
    let Ok(address) = rt.address() else {
        tracing::debug!("insights: {YOURS} has no address to send them to");
        return false;
    };
    let key = rt.signing_key();
    match tokio::task::spawn_blocking(move || worker::write(&address, &key, &batch)).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            tracing::debug!("insights weren't sent: {err:#}");
            false
        }
        Err(err) => {
            tracing::debug!("insights weren't sent: {err}");
            false
        }
    }
}

/// Telemetry checking itself, a dumb test of the whole path and nothing else: did what this device sent last time come
/// back through a read, and can it send another? A failure is the path's, not this device's to explain.
pub async fn check_telemetry(rt: &Rt) {
    if rt.address().is_err() {
        return;
    }
    let sent = rt.state::<State>().canary.lock().unwrap().sent;
    let back = match sent {
        Some(sent) => canary_back(rt, sent).await.unwrap_or_else(|err| {
            tracing::debug!("the canary couldn't be read: {err:#}");
            Some("read")
        }),
        None => None,
    };
    let now = now_ms();
    send_canary(rt, now);
    let wrote = flush(rt).await;
    let mut canary = rt.state::<State>().canary.lock().unwrap();
    canary.sent = wrote.then_some(now);
    let failure = (!wrote).then_some("write").or(back);
    match failure {
        None => *canary = Canary { sent: canary.sent, failed: 0, failing: None, checked: canary.checked },
        Some(kind) => {
            canary.failed += 1;
            canary.failing = Some((kind, canary.failing.map_or(now, |(_, since)| since)));
        }
    }
}

/// One of each kind of signal, as any signal goes: the batch carries them.
fn send_canary(rt: &Rt, now: u64) {
    let instance = hex(&rt.id());
    let attrs = Attrs::default().scope("everyone").build(custom_build::config::BUILD);
    device_signal(rt, CANARY, Attrs::default(), Status::Unset);
    let mut batch = rt.state::<State>().batch.lock().unwrap();
    batch.metric(now, insights::CANARY, insights::own(), &instance, &attrs, 1);
    batch.span(Span { time: now, trace: api::traceparent::ids::trace(), span: api::traceparent::ids::span(), parent: None, name: CANARY.into(), service: insights::own().into(), version: None, instance, kind: SpanKind::Internal, status: Status::Unset, duration: 0, message: None, attrs }, true, &[]);
}

/// Whether what was `sent` came back: none if it did, or what's wrong.
async fn canary_back(rt: &Rt, sent: u64) -> Result<Option<&'static str>> {
    let (address, key) = (rt.address()?, rt.signing_key());
    // A metric's time is the start of its minute.
    let ask = Ask { queries: vec!["spans", "logs", "metrics"], service: Some(insights::own().to_string()), name: Some(CANARY.into()), since: Some(sent.saturating_sub(60_000)), limit: 200, ..Default::default() };
    let fields = worker::asked(&ask, vec![insights::own().to_string()]);
    let rows = tokio::task::spawn_blocking(move || worker::read(&address, &key, fields)).await??;
    let me = hex(&rt.id());
    let arrived = |table: &str| rows[table].as_array().is_some_and(|got| got.iter().any(|row| row["instance"] == me));
    Ok((!["spans", "logs", "metrics"].iter().all(|t| arrived(t))).then_some("arriving"))
}

/// What's wrong with telemetry's own path, once it's more than one check: how it failed and since when. Only an admin's
/// to see (the network's own program is theirs).
pub fn telemetry_problem(rt: &Rt) -> Option<Value> {
    let canary = rt.state::<State>().canary.lock().unwrap();
    canary.failing.filter(|_| canary.failed >= CANARY_FAILS).map(|(kind, since)| json!({ "kind": kind, "since": since }))
}

/// What a client says went wrong on it (a screen that threw or never drew, its page's own errors) and what it did
/// before that (`client.try`), and how its first screen opened (`client.open`). What it says of itself is believed
/// only as far as closed sets go: the app must exist and the screen be one of its, and a report more than
/// `REPORTS_A_MINUTE` a minute is dropped.
pub async fn report_insights(rt: &Rt, me: &Caller, platform: &str, events: &[Value]) {
    let minute = now_ms() / 60_000;
    {
        let mut counts = rt.state::<State>().reports.lock().unwrap();
        let count = counts.entry(me.key).or_insert((minute, 0));
        *count = if count.0 == minute { (minute, count.1 + 1) } else { (minute, 1) };
        if count.1 > REPORTS_A_MINUTE {
            return;
        }
    }
    for event in events.iter().take(MOST_REPORTED) {
        reported_event(rt, me, platform, event).await;
    }
}

async fn reported_event(rt: &Rt, me: &Caller, platform: &str, e: &Value) {
    let now = now_ms();
    let time = e["time"].as_u64().filter(|t| *t + KEPT_UNSENT >= now && *t <= now + 60_000).unwrap_or(now);
    let name = e["name"].as_str().unwrap_or("");
    let ms = e["ms"].as_u64().unwrap_or(0);
    let instance = hex(&me.key);
    let attrs = Attrs::default().client(platform).build(custom_build::config::BUILD);
    if name == "client.try" {
        let result = e["result"].as_str().unwrap_or("");
        let severity = if result == "answered" { Severity::Info } else { Severity::Warn };
        let attrs = attrs.scope("everyone").tried(e["node"].as_str().unwrap_or(""), result).duration(ms);
        return rt.state::<State>().batch.lock().unwrap().log(Log { time, trace: None, span: None, severity, body: name.into(), service: insights::own().into(), version: None, instance, attrs });
    }
    if name == "client.open" {
        return opened(rt, me, e, time, attrs).await;
    }
    if name == "client.call" {
        return client_called(rt, me, e, time, ms, attrs).await;
    }
    let exception = e["exception"].is_object().then(|| Exception::from_json(&e["exception"])).flatten();
    let (destination, app, screen) = match name {
        "screen.exception" | "screen.undrawn" => {
            let Some(app) = e["app"].as_str() else { return };
            let Some(destination) = destination(rt, app) else { return };
            let screen = screen_named(rt, app, e["screen"].as_str().unwrap_or("")).await;
            (destination, app, Some(screen))
        }
        "page.exception" => (EVERYONE, "", None),
        _ => return,
    };
    let bundle = rt.bundle_here(app).filter(|_| destination.named);
    let mut attrs = attrs.scope(destination.kind);
    if let (true, Some(screen)) = (destination.named, &screen) {
        attrs = attrs.screen(screen);
    }
    let exception = located(rt, exception.filter(|_| destination.named || app.is_empty()), bundle.clone().filter(|_| destination.named)).await;
    let (frame, build) = match exception.as_ref().and_then(Exception::top_frame) {
        Some(top) => (top, bundle.clone().unwrap_or_else(|| custom_build::config::BUILD.into())),
        None => (name.to_string(), custom_build::config::BUILD.to_string()),
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
    let span = Span { time, trace: api::traceparent::ids::trace(), span: api::traceparent::ids::span(), parent: None, name: span_name, service, version: bundle, instance, kind: SpanKind::Internal, status: Status::Error, duration: ms, message, attrs };
    rt.state::<State>().batch.lock().unwrap().span(span, true, &[]);
}

/// A service's program going through a hop's worth of a call: one child span, with its status and bytes. A hop that
/// failed doesn't fail the call, which can go on to the next device: the call's own span says how that ended.
pub fn finish_hop(rt: &Rt, req: &CallRequest, me: &Caller, device: Key, took: Duration, asked: &Result<Value>) {
    let destination = destination(rt, &req.app).unwrap_or(EVERYONE);
    let ms = took.as_millis() as u64;
    let answer = asked.as_ref().ok();
    let ok = answer.is_some_and(|a| a.get("error").is_none());
    let code = answer.and_then(|a| a["code"].as_str());
    let (error, result) = match asked {
        Ok(_) if ok => (None, "answered"),
        Ok(_) => (Some(code.unwrap_or(insights::OTHER)), "failed"),
        Err(err) => {
            let (kind, result) = asking_failed(err);
            (Some(kind), result)
        }
    };
    let (parent_trace, parent) = trace::parent_now();
    let trace = parent_trace.unwrap_or_else(api::traceparent::ids::trace);
    let kept = !ok || ms >= custom_build::protocol::SLOW_MS || api::traceparent::sampled(&trace);
    let mut attrs = Attrs::default().client(req.client.as_deref().unwrap_or("api")).user(&me.user).scope(destination.kind).build(custom_build::config::BUILD).tried(&hex(&device), result);
    if kept {
        attrs = attrs.bytes(req.input.to_string().len() as u64 + answer.map_or(0, |a| a.to_string().len() as u64));
    }
    if let Some(error) = error {
        attrs = attrs.error(error);
    }
    let service = if destination.named { req.app.clone() } else { insights::own().to_string() };
    let span = Span { time: now_ms().saturating_sub(ms), trace, span: api::traceparent::ids::span(), parent, name: "service.hop".into(), service, version: None, instance: hex(&rt.id()), kind: SpanKind::Client, status: Status::of(ok), duration: ms, message: None, attrs };
    rt.state::<State>().batch.lock().unwrap().span(span, kept, &[]);
}

/// A schedule's run: one span, with the call it made under it (`finish_call`). The run stays in `runs` for its own
/// screens; this is for what reads every kind of failure the same.
pub fn finish_schedule(rt: &Rt, app: &str, schedule: &str, user: &str, took: Duration, traced: &trace::Trace, err: Option<&anyhow::Error>) {
    let destination = destination(rt, app).unwrap_or(EVERYONE);
    let ms = took.as_millis() as u64;
    let steps = traced.since(0);
    let exception = steps.iter().rev().find(|s| s["step"] == "exception").and_then(|s| Exception::from_json(&s["exception"]));
    let failure = err.map(|err| Failure::of(custom_build::protocol::Coded::of(err).as_deref(), exception.as_ref(), traced.stage()));
    let status = failure.as_ref().map_or(Status::Unset, Failure::status);
    let mut attrs = Attrs::default().client("schedule").user(user).scope(destination.kind).build(custom_build::config::BUILD);
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
    let kept = failure.is_some() || ms >= custom_build::protocol::SLOW_MS || api::traceparent::sampled(&traced.trace_id());
    let span = Span { time: now_ms().saturating_sub(ms), trace: traced.trace_id(), span: api::traceparent::ids::span(), parent: None, name: "schedule.run".into(), service, version: None, instance: hex(&rt.id()), kind: SpanKind::Internal, status, duration: ms, message, attrs };
    rt.state::<State>().batch.lock().unwrap().span(span, kept, &[]);
}

/// What a service says for insights: its program starting and ending, and OTLP. Only one of the network's
/// services, about itself: which app is what its key is listed as, never what it says.
pub async fn service_said(rt: &Rt, remote: &Key, body: &Value) {
    let Ok(Some(app)) = rt.read(|r| Ok(r.db().service(remote)?.map(|s| s.service.clone().unwrap_or_default()))) else { return };
    let destination = destination(rt, &app).unwrap_or(EVERYONE);
    let base = || Attrs::default().scope(destination.kind).build(custom_build::config::BUILD);
    let service = if destination.named { app.clone() } else { insights::own().to_string() };
    let instance = hex(&rt.id());
    let version = body["build"].as_str().filter(|b| insights::token(b)).map(str::to_string);
    if let Some(name) = body["name"].as_str().filter(|n| ["service.start", "service.exit"].contains(n)) {
        let reason = body["reason"].as_str().unwrap_or("");
        let ok = name == "service.start" || ["exit-0", "renewed", "stopped"].contains(&reason);
        let log = Log { time: now_ms(), trace: None, span: None, severity: if ok { Severity::Info } else { Severity::Error }, body: name.to_string(), service: service.clone(), version: version.clone(), instance: instance.clone(), attrs: base().reason(reason) };
        rt.state::<State>().batch.lock().unwrap().log(log);
    }
    for point in insights::otlp::metrics(&body["otlp"]) {
        let new = match point.cumulative {
            true => {
                let key = (service.clone(), point.name.clone());
                rt.state::<State>().programs.lock().unwrap().insert(key, point.clone()).map(|was| point.since(&was))
            }
            false => Some(point),
        };
        if let Some(new) = new.filter(|p| p.count > 0) {
            rt.state::<State>().batch.lock().unwrap().program(now_ms(), new, &service, &instance);
        }
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
        let (trace, span, parent) = item.ids.clone().unwrap_or_else(|| (api::traceparent::ids::trace(), api::traceparent::ids::span(), None));
        let mut batch = rt.state::<State>().batch.lock().unwrap();
        match item.span_kind {
            Some(kind) => {
                let kept = !item.ok || item.ms >= custom_build::protocol::SLOW_MS || api::traceparent::sampled(&trace);
                batch.span(Span { time, trace, span, parent, name, service: service.clone(), version: version.clone(), instance: instance.clone(), kind, status: Status::of(item.ok), duration: item.ms, message: None, attrs }, kept, &[]);
            }
            None => batch.log(Log { time, trace: Some(trace), span: Some(span), severity: if item.ok { Severity::Info } else { Severity::Error }, body: name, service: service.clone(), version: version.clone(), instance: instance.clone(), attrs }),
        }
    }
}

/// A request a client made of its nodes (`client.call`), as OTEL's client span, the parent of what the node that took
/// it recorded: it sent the trace and span in its `traceparent`. A call by its app's tool, as the node names its own;
/// anything else by its route. The client keeps only what failed, was slow or is sampled, and says nothing else of it:
/// which node answered and whether what it answered failed, or why none did (`error`). One no node answered has no
/// node's span to be found by, so it's in the Errors list itself.
async fn client_called(rt: &Rt, me: &Caller, e: &Value, time: u64, ms: u64, attrs: Attrs) {
    let Some((trace, span)) = e["traceparent"].as_str().and_then(api::traceparent::parent_of) else { return };
    let attrs = attrs.user(&me.user);
    let (service, name, mut attrs) = match (e["app"].as_str(), e["tool"].as_str(), e["path"].as_str().and_then(|p| rt.route_of(p))) {
        (Some(app), Some(tool), _) => {
            let destination = destination(rt, app).unwrap_or(EVERYONE);
            let req = CallRequest { app: app.into(), tool: tool.into(), ..Default::default() };
            match destination.named {
                true => {
                    let tool = tool_named(rt, &req).await;
                    (app.to_string(), tool.clone(), attrs.scope(destination.kind).rpc(app, &tool))
                }
                false => (insights::own().to_string(), "call".to_string(), attrs.scope(destination.kind)),
            }
        }
        (_, _, Some(route)) => (insights::own().to_string(), route.replace(['{', '}', '*'], ""), attrs.scope("everyone")),
        _ => return,
    };
    let node = e["node"].as_str().unwrap_or("");
    if !node.is_empty() {
        attrs = attrs.tried(node, "answered");
    }
    let unanswered = e["error"].as_str();
    let failure = match (e["failed"] == true, unanswered) {
        (true, _) => Some(Failure::of(e["code"].as_str(), None, stage::ROUTE)),
        (false, Some(why)) => Some(Failure::Broke(why.to_string())),
        _ => None,
    };
    let mut message = None;
    if let Some(failure) = &failure {
        attrs = attrs.failed(failure).stage(stage::ROUTE);
        if unanswered.is_some() {
            attrs = attrs.fingerprint(failure.kind(), &format!("stage:{}", stage::ROUTE), custom_build::config::BUILD);
        }
        message = broke(failure, e["message"].as_str()).as_deref().map(insights::message::cleaned);
    }
    let status = failure.as_ref().map_or(Status::Unset, Failure::status);
    let span = Span { time, trace, span, parent: None, name, service, version: None, instance: hex(&me.key), kind: SpanKind::Client, status, duration: ms, message, attrs };
    rt.state::<State>().batch.lock().unwrap().span(span, true, &[]);
}

/// How a client's first screen opened (`client.open`): an app's under its app, and the client's own under this
/// program, each by a screen it has.
async fn opened(rt: &Rt, me: &Caller, e: &Value, time: u64, attrs: Attrs) {
    let (service, attrs) = match e["app"].as_str() {
        Some(app) => {
            let Some(destination) = destination(rt, app) else { return };
            (app.to_string(), attrs.scope(destination.kind).screen(&screen_named(rt, app, e["screen"].as_str().unwrap_or("")).await))
        }
        None => {
            let screen = e["screen"].as_str().filter(|s| rt.client_screens().into_iter().any(|c| c == *s)).unwrap_or("_unknown");
            (insights::own().to_string(), attrs.scope("everyone").screen(screen))
        }
    };
    let mut batch = rt.state::<State>().batch.lock().unwrap();
    for (metric, key) in [(insights::OPEN_WINDOW, "window"), (insights::OPEN_SHOWN, "shown"), (insights::OPEN_FRESH, "fresh")] {
        if let Some(ms) = e[key].as_u64().filter(|ms| *ms < 600_000) {
            batch.metric(time, metric, &service, &hex(&me.key), &attrs, ms);
        }
    }
}

async fn screen_named(rt: &Rt, app: &str, screen: &str) -> String {
    let listed = rt.describe(app).await.is_ok_and(|d| d["screens"].as_array().is_some_and(|s| s.iter().any(|s| s["name"] == screen)));
    if listed { screen.to_string() } else { "_unknown".into() }
}

/// Sends what it has every minute or so (`FLUSH_EVERY`), and checks telemetry's own path every `CANARY_EVERY`.
async fn flush_loop(rt: Rt) -> Result<()> {
    flush(&rt).await;
    let due = {
        let mut canary = rt.state::<State>().canary.lock().unwrap();
        let checked = *canary.checked.get_or_insert_with(Instant::now);
        let due = checked.elapsed() >= CANARY_EVERY;
        if due {
            canary.checked = Some(Instant::now());
        }
        due
    };
    if due {
        check_telemetry(&rt).await;
    }
    Ok(())
}

/// What `viewer` may see of how things went, by `ask`, from the telemetry Worker: an app's only for someone who may
/// deploy it, and what's the network's own program's only for an admin. What this device hasn't sent yet goes alongside
/// rather than before: a row takes a minute or two to be readable, so waiting for it says nothing about what's read.
pub async fn insights(rt: &Rt, viewer: &Caller, ask: &Ask) -> Result<Value> {
    let sending = rt.clone();
    tokio::spawn(async move {
        flush(&sending).await;
    });
    let (services, admin) = rt.read(|r| {
        let m = r.db();
        let mut seen: Vec<String> = m.apps()?.into_iter().map(|a| a.name).filter(|app| app != insights::own() && m.may(&viewer.user, "deploy", &scopes::app(app))).collect();
        let admin = m.may(&viewer.user, "change-settings", scopes::NETWORK);
        if admin {
            seen.push(insights::own().to_string());
            seen.extend(insights::WORKERS.iter().map(|w| insights::worker_service(w)));
            seen.push(insights::cost::CLOUDFLARE.to_string());
        }
        Ok((seen, admin))
    })?;
    let (address, key) = (rt.address()?, rt.signing_key());
    let fields = worker::asked(ask, services.clone());
    let mut rows = tokio::task::spawn_blocking(move || worker::read(&address, &key, fields)).await??;
    if let (Some(trace), Some(read)) = (&ask.trace, rows.get_mut("spans").and_then(Value::as_array_mut)) {
        add_arriving(rt, read, trace, &services);
    }
    for table in ["spans", "logs", "metrics", "traces"] {
        for row in rows.get_mut(table).and_then(Value::as_array_mut).into_iter().flatten() {
            read_row(rt, row);
        }
    }
    for table in ["errors", "build_errors"] {
        for row in rows.get_mut(table).and_then(Value::as_array_mut).into_iter().flatten() {
            read_error(row);
            name_device(rt, row);
        }
    }
    if let Some(problem) = telemetry_problem(rt).filter(|_| admin) {
        rows["problem"] = problem;
    }
    Ok(rows)
}

/// Adds to `read` this device's own spans of `trace` that can't be read back yet (unsent, or sent within
/// `ARRIVING_FOR`), of `services`, each marked `arriving`: what other devices recorded of it is still on its way.
fn add_arriving(rt: &Rt, read: &mut Vec<Value>, trace: &str, services: &[String]) {
    let mut mine: Vec<Span> = rt.state::<State>().batch.lock().unwrap().spans.iter().filter(|s| s.trace == trace).cloned().collect();
    mine.extend(rt.state::<State>().arriving.lock().unwrap().iter().filter(|(_, s)| s.trace == trace).map(|(_, s)| s.clone()));
    let have: std::collections::HashSet<String> = read.iter().filter_map(|row| row["span"].as_str().map(str::to_string)).collect();
    for span in mine.iter().filter(|s| services.contains(&s.service) && !have.contains(&s.span)) {
        let mut row = span.row();
        row["arriving"] = json!(true);
        read.push(row);
    }
}

/// A row's device by name. A Worker's own span says no device, as it's none of them.
fn name_device(rt: &Rt, row: &mut Value) {
    if let Some(device) = row["instance"].as_str().and_then(|k| unhex(k).ok()).filter(|k| k.iter().any(|b| *b != 0)) {
        row["device"] = json!(name_of(rt, &device));
    }
}

/// A row as a reader gets it: its attributes as JSON, and its device and the devices it names by name.
/// A user's name, by their id, for what Insights says: blank for one it never knew.
pub fn user_named(rt: &Rt, id: &str) -> String {
    rt.read(|r| r.user_name(id)).unwrap_or_default()
}

fn read_row(rt: &Rt, row: &mut Value) {
    row["attrs"] = row["attrs"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(json!({}));
    name_device(rt, row);
    if let Some(node) = row["attrs"]["one.node"].as_str().and_then(|k| unhex(k).ok()) {
        row["attrs"]["one.node"] = json!(name_of(rt, &node));
    }
}

/// Why a device couldn't be asked, as `error.type` and as a try's result.
pub fn asking_failed(err: &anyhow::Error) -> (&'static str, &'static str) {
    if err.is::<::api::runtime::NotThere>() {
        (api::traceparent::UNREACHABLE, "unreachable")
    } else if err.to_string() == "timed out" {
        (api::traceparent::TIMEOUT, "no answer")
    } else {
        (insights::LOST, "lost")
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
