//! What a request did on this device, step by step with how long each took, for a client in developer mode: it asks
//! with `x-one-trace` and gets it back in the same header (`api::guard`), and another device asked for a call adds its
//! own, in its answer (`trace` beside `ok`). Steps hold timings, device, app and tool
//! names, never data or keys. Every call has one, asked for or not: it's the span insights keeps of a call (`src/insights`)
//! as well as what `--trace` shows.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

tokio::task_local! {
    static TRACE: Trace;
}

/// A request's trace while it runs: its steps, and when it started.
#[derive(Clone, Debug)]
pub struct Trace(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    start: Instant,
    steps: Mutex<Vec<Value>>,
    /// A client or another device asked for it, so it's given back and asked of the next device too.
    asked: bool,
    trace_id: String,
    /// The span running now, what a call started within it has as its parent.
    span: Mutex<Option<String>>,
    /// Where it's got to, which is where a failure failed (`insights::stage`).
    stage: Mutex<&'static str>,
}

impl Trace {
    pub fn new() -> Trace {
        Trace::make(true, None)
    }

    /// One no one asked for: only insights reads it.
    pub fn quiet() -> Trace {
        Trace::make(false, None)
    }

    /// One a device asked for this one's part of, continuing its `traceparent`, W3C's `00-<trace>-<span>-01`, if that's one.
    pub fn continuing(asked: bool, traceparent: Option<&str>) -> Trace {
        Trace::make(asked, traceparent.and_then(parent_of))
    }

    fn make(asked: bool, parent: Option<(String, String)>) -> Trace {
        let (trace_id, span) = parent.map_or_else(|| (ids::trace(), None), |(t, s)| (t, Some(s)));
        Trace(Arc::new(Inner { start: Instant::now(), steps: Mutex::new(Vec::new()), asked, trace_id, span: Mutex::new(span), stage: Mutex::new(crate::insights::stage::CHECK) }))
    }

    pub fn note(&self, mut step: Value, took: std::time::Duration) {
        let at = self.0.start.elapsed().saturating_sub(took);
        step["at"] = json!(at.as_millis() as u64);
        step["ms"] = json!(took.as_millis() as u64);
        self.0.steps.lock().unwrap().push(step);
    }

    /// What it did, as this device says it: `{ device, ms, steps }`.
    pub fn done(&self, device: &str) -> Value {
        json!({ "device": device, "ms": self.0.start.elapsed().as_millis() as u64, "steps": self.0.steps.lock().unwrap().clone() })
    }

    /// How many steps it has, where a call started now begins.
    pub fn count(&self) -> usize {
        self.0.steps.lock().unwrap().len()
    }

    /// The steps since the `from`th.
    pub fn since(&self, from: usize) -> Vec<Value> {
        self.0.steps.lock().unwrap()[from..].to_vec()
    }

    pub fn trace_id(&self) -> String {
        self.0.trace_id.clone()
    }

    /// Runs a new span from here on, saying which it replaced so the end of it can put it back.
    pub fn enter(&self, span: String) -> Option<String> {
        self.0.span.lock().unwrap().replace(span)
    }

    pub fn leave(&self, was: Option<String>) {
        *self.0.span.lock().unwrap() = was;
    }

    pub fn stage(&self) -> &'static str {
        *self.0.stage.lock().unwrap()
    }

    pub fn stage_is(&self, stage: &'static str) {
        *self.0.stage.lock().unwrap() = stage;
    }
}

/// `traceparent`'s trace and span, if it's W3C's and has them: what another device sent, so nothing else is kept.
fn parent_of(text: &str) -> Option<(String, String)> {
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let ["00", trace, span, _] = text.split('-').collect::<Vec<_>>()[..] else { return None };
    (hex(trace, 32) && hex(span, 16)).then(|| (trace.to_string(), span.to_string()))
}

/// W3C's ids: 16 random bytes for a trace and 8 for a span, as lowercase hex.
pub mod ids {
    pub fn trace() -> String {
        data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())
    }

    pub fn span() -> String {
        data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>())
    }
}

/// A scope as a trace names it: its kind alone, never whose.
pub fn kind(scope: &crate::database::scopes::Scope) -> &'static str {
    match scope {
        crate::database::scopes::Scope::Global => "everyone",
        crate::database::scopes::Scope::Group(_) => "group",
        crate::database::scopes::Scope::User(_) => "user",
    }
}

/// The trace running here, and the span within it: what a child span names as its trace and parent.
pub fn parent_now() -> (Option<String>, Option<String>) {
    TRACE.try_with(|t| (Some(t.0.trace_id.clone()), t.0.span.lock().unwrap().clone())).unwrap_or_default()
}

/// The trace of the request running here, if it has one: a call always does.
pub fn current() -> Option<Trace> {
    TRACE.try_with(Clone::clone).ok()
}

/// Whether the request running here asked for a trace.
pub fn on() -> bool {
    TRACE.try_with(|t| t.0.asked).unwrap_or(false)
}

/// The `traceparent` the next device continues, W3C's, if a call is running here.
pub fn traceparent() -> Option<String> {
    TRACE.try_with(|t| t.0.span.lock().unwrap().clone().map(|span| format!("00-{}-{span}-01", t.0.trace_id))).ok().flatten()
}

/// Says where the request running here has got to, which is where it failed, if it fails.
pub fn stage(stage: &'static str) {
    let _ = TRACE.try_with(|t| t.stage_is(stage));
}

/// Runs `f` as part of `trace`, if there is one.
pub async fn within<F: Future>(trace: Option<Trace>, f: F) -> F::Output {
    match trace {
        Some(trace) => TRACE.scope(trace, f).await,
        None => f.await,
    }
}

pub fn note(step: Value, took: std::time::Duration) {
    let _ = TRACE.try_with(|t| t.note(step, took));
}

/// Notes one ask of the bucket itself (not this device's copy), a `read` or a write (`create`, `replace`), joined to the
/// ask just before it if that was of the same kind: a database's steps are asked for one after another.
pub fn bucket(kind: &str, took: std::time::Duration, found: bool) {
    let _ = TRACE.try_with(|t| {
        let mut steps = t.0.steps.lock().unwrap();
        if let Some(last) = steps.last_mut().filter(|s| s["step"] == "bucket" && s["kind"] == kind) {
            last["asks"] = json!(last["asks"].as_u64().unwrap_or(0) + 1);
            last["found"] = json!(last["found"].as_u64().unwrap_or(0) + found as u64);
            last["ms"] = json!(last["ms"].as_u64().unwrap_or(0) + took.as_millis() as u64);
            return;
        }
        drop(steps);
        t.note(json!({ "step": "bucket", "kind": kind, "asks": 1, "found": found as u64 }), took);
    });
}

/// A call's trace as a few readable lines, in the Developer panel's words (src/screen/settings/devtools.jsx): what the
/// device that answered did (`node`, its `x-one-trace`), each wait with its milliseconds, and the total.
pub fn said(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    steps(node, node["device"].as_str().unwrap_or("the device that answered"), &mut out);
    out.push(format!("total {} ms", node["ms"].as_u64().unwrap_or(0)));
    out
}

/// What one device did, step by step, and the devices it passed the call to, in theirs.
fn steps(trace: &Value, lane: &str, out: &mut Vec<String>) {
    let mut listed = 0;
    for s in trace["steps"].as_array().into_iter().flatten() {
        let ms = s["ms"].as_u64().unwrap_or(0);
        let (app, runs) = (s["app"].as_str().unwrap_or(""), s["tool"].as_str().or(s["function"].as_str()).unwrap_or(""));
        match s["step"].as_str().unwrap_or("") {
            "device" => {
                let to = s["device"].as_str().unwrap_or("a device");
                let result = s["result"].as_str().unwrap_or("");
                // The device that is running it says what it did in its own steps.
                if s["here"] == true && result == "ran" {
                    continue;
                }
                listed += ms;
                match result {
                    r if s["here"] == true => out.push(format!("{app}.{runs} on {lane}: {r}{}: {ms} ms", s["why"].as_str().map(|w| format!(" ({w})")).unwrap_or_default())),
                    "ran" => out.push(format!("{lane} passed {app}.{runs} to {to}: {ms} ms")),
                    r => out.push(format!("{lane} tried {to}: {}{}: {ms} ms", r.replace("no answer", "didn't answer"), s["why"].as_str().map(|w| format!(" ({w})")).unwrap_or_default())),
                }
                if s["trace"].is_object() {
                    steps(&s["trace"], to, out);
                }
            }
            "bucket" => {
                listed += ms;
                match s["kind"].as_str() {
                    Some(kind @ ("create" | "replace")) => out.push(format!("{lane} {kind}d {} in the bucket: {ms} ms", s["asks"])),
                    _ => out.push(format!("{lane} fetched {} of {} from the bucket: {ms} ms", s["found"], s["asks"])),
                }
            }
            "online" if ms >= 5 => {
                listed += ms;
                out.push(format!("{lane} waited to be on the relay: {ms} ms"));
            }
            "connect" => {
                listed += ms;
                out.push(format!("{lane} connected to {}: {ms} ms", s["to"].as_str().unwrap_or("a device")));
            }
            "lease" => {
                listed += ms;
                out.push(format!("{lane} took the call's lease: {ms} ms"));
            }
            "prepare" if ms >= 5 => {
                listed += ms;
                out.push(format!("{lane} prepared what the call changed: {ms} ms"));
            }
            "write" => {
                listed += ms;
                out.push(format!("{lane} wrote what the call changed: {ms} ms"));
            }
            "keys" => {
                listed += ms;
                out.push(format!("{lane} checked the call's keys: {ms} ms"));
            }
            "behind" => {
                listed += ms;
                out.push(format!("{lane} asked the bucket first: {}: {ms} ms", s["result"].as_str().unwrap_or("")));
            }
            "data" => {
                listed += ms;
                let new = s["new"].as_u64().filter(|n| *n > 0).map(|n| format!(", {n} new steps")).unwrap_or_default();
                out.push(format!("{lane} read {app} data for {} from its copy{new}: {ms} ms", s["scope"].as_str().unwrap_or("")));
            }
            "open" => {
                listed += ms;
                out.push(format!("{lane} opened {app} data: {ms} ms"));
            }
            "logs" if ms >= 5 => {
                listed += ms;
                out.push(format!("{lane} read who's who: {ms} ms"));
            }
            "run" => {
                listed += ms;
                out.push(format!("{app}.{runs} ran on {lane}: {ms} ms"));
            }
            _ => {}
        }
    }
    let rest = trace["ms"].as_u64().unwrap_or(0).saturating_sub(listed);
    if rest >= 1 && trace["steps"].as_array().is_some_and(|s| !s.is_empty()) {
        out.push(format!("other work on {lane}: {rest} ms"));
    }
}
