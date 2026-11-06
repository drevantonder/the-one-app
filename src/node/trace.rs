//! What a request did on this device, step by step with how long each took, for a client in developer mode: it asks
//! with `x-one-trace` and gets it back in the same header (`api::traced`), and another device asked for a call adds its
//! own, in its answer (`trace` beside `ok`). Steps hold timings, device, app and tool
//! names, never data or keys.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

/// The header a client asks for a trace with, and gets it back in, as base64 of its JSON.
pub const HEADER: &str = "x-one-trace";

tokio::task_local! {
    static TRACE: Trace;
}

/// A request's trace while it runs: its steps, and when it started.
#[derive(Clone, Debug)]
pub struct Trace(Arc<(Instant, Mutex<Vec<Value>>)>);

impl Trace {
    pub fn new() -> Trace {
        Trace(Arc::new((Instant::now(), Mutex::new(Vec::new()))))
    }

    pub fn note(&self, mut step: Value, took: std::time::Duration) {
        let (start, steps) = &*self.0;
        let at = start.elapsed().saturating_sub(took);
        step["at"] = json!(at.as_millis() as u64);
        step["ms"] = json!(took.as_millis() as u64);
        steps.lock().unwrap().push(step);
    }

    /// What it did, as this device says it: `{ device, ms, steps }`.
    pub fn done(&self, device: &str) -> Value {
        let (start, steps) = &*self.0;
        json!({ "device": device, "ms": start.elapsed().as_millis() as u64, "steps": steps.lock().unwrap().clone() })
    }
}

impl Default for Trace {
    fn default() -> Self {
        Trace::new()
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

/// The trace of the request running here, if it asked for one.
pub fn current() -> Option<Trace> {
    TRACE.try_with(Clone::clone).ok()
}

/// Whether the request running here asked for a trace.
pub fn on() -> bool {
    TRACE.try_with(|_| ()).is_ok()
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
        let mut steps = t.0.1.lock().unwrap();
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

/// A call's trace as a few readable lines, in the Developer panel's words (src/screen/settings/devtools.jsx): the devices
/// the client tried (`tries`, from the client library), what the one that answered did (`node`, its `x-one-trace`),
/// each wait with its milliseconds, and the total. `answered` names the device when its node sent no trace.
pub fn said(tries: Option<&Value>, node: Option<&Value>, answered: &str) -> Vec<String> {
    let mut out = Vec::new();
    let names = node.map(|n| n["names"].clone()).unwrap_or_default();
    let name = |key: &str| names[key].as_str().map(str::to_string).unwrap_or_else(|| format!("a device ({})", key.get(..6).unwrap_or(key)));
    let mut total = node.and_then(|n| n["ms"].as_u64()).unwrap_or(0);
    for t in tries.and_then(Value::as_array).into_iter().flatten() {
        let (who, ms) = (name(t["node"].as_str().unwrap_or("")), t["ms"].as_u64().unwrap_or(0));
        if let Some(connect) = t["connect"].as_u64() {
            out.push(format!("connected to {who}: {connect} ms"));
        }
        out.push(match t["result"].as_str().unwrap_or("") {
            "answered" => format!("asked {who}, which answered: {ms} ms there and back"),
            "elsewhere" => format!("asked {who}, which said to ask elsewhere: {ms} ms"),
            r => format!("asked {who}: {}, gave up after {ms} ms", r.replace("no connection", "couldn't connect").replace("no answer", "didn't answer")),
        });
        total = total.max(t["at"].as_u64().unwrap_or(0) + ms);
    }
    match node {
        Some(node) => steps(node, node["device"].as_str().unwrap_or(answered), &mut out),
        None => out.push(format!("no trace from {answered}")),
    }
    out.push(format!("total {total} ms"));
    out
}

/// What one device did, step by step, and the devices it passed the call to, in theirs.
fn steps(trace: &Value, lane: &str, out: &mut Vec<String>) {
    let mut listed = 0;
    for s in trace["steps"].as_array().into_iter().flatten() {
        let ms = s["ms"].as_u64().unwrap_or(0);
        let (app, op) = (s["app"].as_str().unwrap_or(""), s["op"].as_str().unwrap_or(""));
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
                    r if s["here"] == true => out.push(format!("{app}.{op} on {lane}: {r}{}: {ms} ms", s["why"].as_str().map(|w| format!(" ({w})")).unwrap_or_default())),
                    "ran" => out.push(format!("{lane} passed {app}.{op} to {to}: {ms} ms")),
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
                out.push(format!("{app}.{op} ran on {lane}: {ms} ms"));
            }
            _ => {}
        }
    }
    let rest = trace["ms"].as_u64().unwrap_or(0).saturating_sub(listed);
    if rest >= 1 && trace["steps"].as_array().is_some_and(|s| !s.is_empty()) {
        out.push(format!("other work on {lane}: {rest} ms"));
    }
}
