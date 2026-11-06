//! What a request did on this device, step by step with how long each took, for a client in developer mode: it asks
//! with `x-one-trace` and gets it back in the same header (`api::traced`), and another device asked for a call adds its
//! own, in its answer (`trace` beside `ok`). Older devices ignore both. Steps hold timings, device, app and tool
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

    /// Notes a step that took `took`, ending now.
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

/// Notes a step that took `took`, ending now.
pub fn note(step: Value, took: std::time::Duration) {
    let _ = TRACE.try_with(|t| t.note(step, took));
}

/// Notes one ask of the bucket itself (not this device's copy), joined to the ask just before it: a database's steps
/// are asked for one after another.
pub fn bucket(took: std::time::Duration, found: bool) {
    let _ = TRACE.try_with(|t| {
        let mut steps = t.0.1.lock().unwrap();
        if let Some(last) = steps.last_mut().filter(|s| s["step"] == "bucket") {
            last["asks"] = json!(last["asks"].as_u64().unwrap_or(0) + 1);
            last["found"] = json!(last["found"].as_u64().unwrap_or(0) + found as u64);
            last["ms"] = json!(last["ms"].as_u64().unwrap_or(0) + took.as_millis() as u64);
            return;
        }
        drop(steps);
        t.note(json!({ "step": "bucket", "asks": 1, "found": found as u64 }), took);
    });
}
