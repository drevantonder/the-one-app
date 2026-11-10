//! Insights: how the network is doing, recorded by the network itself. A dumb pipeline: it records, stores, expires and
//! offers one read tool (`insights`), and has no opinion on what's a problem.
//!
//! It speaks OpenTelemetry's data model and field names (`service.name`, `error.type`, `exception.stacktrace`, W3C trace
//! and span ids) without its SDK. Each device keeps a batch per scope in memory and writes it to that scope's node
//! database every few minutes (`node.rs` here): three tables, of raw signals (a week), hourly rollups (90 days) and error
//! fingerprints (until their build is gone from the network). Telemetry may be lost.
//!
//! A signal holds closed-set values only (`Attrs`): never a call's arguments or answer, an app's error text, a URL, a file,
//! repo or account's name, an email, an IP, a prompt or anything key-shaped. `global` holds only device, build and Worker
//! facts, with a scope's kind, never a user, a group or the name of an app in someone's private scope.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::protocol::code;

pub mod tools;

#[cfg(feature = "node")]
pub mod cli;
#[cfg(feature = "node")]
pub(crate) mod store;
#[cfg(feature = "node")]
pub mod routes;

/// How long a call takes before it keeps a full span. ARCHITECTURE.md's targets state none for a whole call, and its
/// nearest is a write seen elsewhere within 1 s.
pub const SLOW_MS: u64 = 1_000;

pub const RAW_KEPT_MS: u64 = 7 * 86_400_000;
pub const ROLLUPS_KEPT_MS: u64 = 90 * 86_400_000;
/// The most raw signals a batch holds: past it, they only add to the rollups.
const MOST_SIGNALS: usize = 500;
/// The most stack frames a signal keeps.
pub const MOST_FRAMES: usize = 8;

/// Whose a failure is, which only where it happened knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fault {
    /// Refused, invalid input, not allowed, no such thing.
    Caller,
    Internal,
}

impl Fault {
    pub fn as_str(self) -> &'static str {
        match self {
            Fault::Caller => "caller",
            Fault::Internal => "internal",
        }
    }
}

/// Where a call failed: the order it goes through them.
pub mod stage {
    /// Whether it can run at all: its scopes, whether its app's deleted, a lab that's off.
    pub const CHECK: &str = "check";
    /// The loop guard that stopped its app.
    pub const GUARD: &str = "guard";
    pub const KEYS: &str = "keys";
    pub const LEASE: &str = "lease";
    pub const RUN: &str = "run";
    pub const PREPARE: &str = "prepare";
    pub const WRITE: &str = "write";
    /// It took longer than it may.
    pub const LIMIT: &str = "limit";
    pub const ALL: &[&str] = &[CHECK, GUARD, KEYS, LEASE, RUN, PREPARE, WRITE, LIMIT];
}

/// What a client names itself on a call. Anything else is `other`, since a caller says it.
pub const CLIENTS: &[&str] = &["cli", "mcp", "code", "mac", "phone", "android", "windows", "linux", "browser", "web", "schedule", "app", "node", "api"];

/// The JavaScript errors whose name is the engine's, not a program's: anything else is `Error`.
const EXCEPTIONS: &[&str] = &["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError", "URIError", "EvalError", "AggregateError"];

/// How a client's try of a node went (client/src/lib.rs).
const TRY_RESULTS: &[&str] = &["answered", "elsewhere", "no connection", "not taking requests", "no answer", "unreachable", "lost", "failed"];

/// A name of OTEL's for an error of no known kind.
pub const OTHER: &str = "_OTHER";
pub const TIMEOUT: &str = "timeout";

/// `name` if it's one of the errors the engine names, else `Error`: an app can name its own anything.
pub fn exception_type(name: &str) -> &'static str {
    EXCEPTIONS.iter().find(|e| **e == name).copied().unwrap_or("Error")
}

/// An exception as a screen or the app runtime reports it: what threw it and where, never what it said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exception {
    pub kind: String,
    pub frames: Vec<String>,
}

impl Exception {
    /// From `{ type, frames: [{ file, line, col }] }`, whoever sent it: a frame that isn't only a location is left out.
    pub fn from_json(v: &Value) -> Option<Exception> {
        let kind = exception_type(v["type"].as_str()?).to_string();
        let frames = v["frames"].as_array().into_iter().flatten().filter_map(frame).take(MOST_FRAMES).collect();
        Some(Exception { kind, frames })
    }

    /// Whether the app's own code threw it, as `throw new Error("…")`: a choice to refuse, unlike the engine's errors
    /// and what the app runtime throws for it.
    pub fn thrown_by_app(&self) -> bool {
        self.kind == "Error" && self.frames.first().is_some_and(|f| !f.starts_with(RUNTIME_FRAME))
    }
}

/// What the app runtime's own frames start with (src/app/runtime/shim.js).
const RUNTIME_FRAME: &str = "shim.js:";

/// A frame as `file:line:col`, if it's only that: a file's name is the app's code's, in a bundle, and nothing else may be
/// where one goes.
fn frame(v: &Value) -> Option<String> {
    let file = v["file"].as_str()?;
    let name_ok = !file.is_empty() && file.len() <= 96 && file.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b));
    name_ok.then(|| format!("{file}:{}:{}", v["line"].as_u64().unwrap_or(0), v["col"].as_u64().unwrap_or(0)))
}

/// Whose failure it is: said by its code if it has one, else by where it failed.
pub fn fault(code: Option<&str>, stage: &str, exception: Option<&Exception>) -> Fault {
    let caller = |is: bool| if is { Fault::Caller } else { Fault::Internal };
    match code {
        Some(c) => caller([code::UPDATE, code::NOT_LET_IN, code::REFUSED, code::NOT_THIS_MACHINE, code::NOT_SIGNED_IN, code::NOT_ADDED, code::APP_DELETED, code::ALREADY, code::NOT_LISTED, code::PAIRING_USED, code::BAD_PAIRING_CODE, code::ALREADY_IN_NETWORK].contains(&c)),
        None => caller(stage == stage::CHECK || (stage == stage::RUN && exception.is_some_and(Exception::thrown_by_app))),
    }
}

/// A signal's attributes: what one may name, and only values that can say nothing but what the key says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs(BTreeMap<&'static str, Value>);

impl Attrs {
    /// The app, or a service's own name.
    pub fn service(mut self, name: &str) -> Attrs {
        self.token("service.name", name);
        self
    }

    /// The app's bundle, or a service's build: its hash.
    pub fn version(mut self, hash: &str) -> Attrs {
        self.token("service.version", hash);
        self
    }

    pub fn tool(mut self, tool: &str) -> Attrs {
        self.token("one.tool", tool);
        self
    }

    pub fn device(mut self, hex: &str) -> Attrs {
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.0.insert("one.device", json!(hex));
        }
        self
    }

    pub fn client(mut self, kind: &str) -> Attrs {
        self.0.insert("one.client", json!(if CLIENTS.contains(&kind) { kind } else { "other" }));
        self
    }

    /// A scope's kind (`everyone`, `group`, `user`), never whose.
    pub fn scope(mut self, kind: &'static str) -> Attrs {
        self.0.insert("one.scope", json!(kind));
        self
    }

    pub fn build(mut self, build: &str) -> Attrs {
        self.token("one.build", build);
        self
    }

    /// A screen of an app: one its app says it has, or `_unknown`, as what a page says isn't believed.
    pub fn screen(mut self, screen: &str) -> Attrs {
        self.token("one.screen", screen);
        self
    }

    /// What a client tried: which node, and how it went.
    pub fn tried(mut self, node: &str, result: &str) -> Attrs {
        if node.len() == 64 && node.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.0.insert("one.node", json!(node));
        }
        self.0.insert("one.result", json!(if TRY_RESULTS.contains(&result) { result } else { "failed" }));
        self
    }

    pub fn stage(mut self, stage: &str) -> Attrs {
        if stage::ALL.contains(&stage) {
            self.0.insert("one.stage", json!(stage));
        }
        self
    }

    pub fn bytes(mut self, n: u64) -> Attrs {
        self.0.insert("one.bytes", json!(n));
        self
    }

    /// `error.type`: a protocol code, an exception's type, or OTEL's own `timeout` or `_OTHER`.
    pub fn error(mut self, kind: &str) -> Attrs {
        let known = code::ALL.contains(&kind) || EXCEPTIONS.contains(&kind) || kind == TIMEOUT;
        self.0.insert("error.type", json!(if known { kind } else { OTHER }));
        self
    }

    pub fn exception(mut self, e: &Exception) -> Attrs {
        self.0.insert("exception.type", json!(e.kind));
        if !e.frames.is_empty() {
            self.0.insert("exception.stacktrace", json!(e.frames.iter().map(|f| format!("at {f}")).collect::<Vec<_>>().join("\n")));
        }
        self
    }

    /// What a span did, step by step, without anything a step says that's more than its name and time.
    pub fn steps(mut self, steps: &[Value]) -> Attrs {
        let kept: Vec<Value> = steps.iter().filter_map(stored_step).collect();
        if !kept.is_empty() {
            self.0.insert("one.steps", Value::Array(kept));
        }
        self
    }

    fn token(&mut self, key: &'static str, value: &str) {
        if token(value) {
            self.0.insert(key, json!(value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// The attributes a rollup groups by, as its key: those that are few.
    fn rollup_key(&self, fault: Option<Fault>) -> String {
        let mut key: BTreeMap<&str, Value> = self.0.iter().filter(|(k, _)| ["one.client", "one.scope", "one.device", "one.build"].contains(k)).map(|(k, v)| (*k, v.clone())).collect();
        if let Some(fault) = fault {
            key.insert("fault", json!(fault.as_str()));
        }
        serde_json::to_string(&key).unwrap_or_default()
    }

    pub fn json(&self) -> Value {
        json!(self.0)
    }
}

/// Whether a value is a plain name or hash: letters, digits and `_ . - +`, so no address, path, URL or sentence.
pub fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 72 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-+".contains(&b))
}

/// The steps a trace may name, and the fields of each that are only a name or a number.
const STEPS: &[&str] = &["device", "bucket", "online", "connect", "lease", "prepare", "write", "keys", "behind", "data", "open", "logs", "run"];
const RESULTS: &[&str] = &["ran", "failed", "elsewhere", "needs keys", "unreachable", "no answer", "caught up", "behind", "asked"];

fn stored_step(step: &Value) -> Option<Value> {
    let name = step["step"].as_str().filter(|s| STEPS.contains(s))?;
    let mut out = json!({ "step": name, "at": step["at"], "ms": step["ms"] });
    for key in ["asks", "found", "new"] {
        if step[key].is_number() {
            out[key] = step[key].clone();
        }
    }
    if let Some(kind) = step["kind"].as_str().filter(|k| ["read", "create", "replace"].contains(k)) {
        out["kind"] = json!(kind);
    }
    if let Some(result) = step["result"].as_str().filter(|r| RESULTS.contains(r)) {
        out["result"] = json!(result);
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Span,
    Event,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Span => "span",
            Kind::Event => "event",
        }
    }
}

/// One thing that happened, as a device says it.
#[derive(Clone, Debug)]
pub struct Signal {
    pub time: u64,
    pub device: String,
    pub kind: Kind,
    pub name: String,
    /// The app it's about, or none: the device's own, or a private app's named in `global`, where it can't be.
    pub app: String,
    pub trace: String,
    pub span: String,
    pub parent: Option<String>,
    pub ms: u64,
    pub ok: bool,
    pub fault: Option<Fault>,
    pub attrs: Attrs,
}

/// An error grouped by what threw it: its type, its top code frame or the stage it failed at, and the build it ran in.
#[derive(Clone, Debug)]
pub struct Print {
    pub app: String,
    pub tool: String,
    pub kind: String,
    pub frame: String,
    pub build: String,
    pub fault: Fault,
}

impl Print {
    pub fn fingerprint(&self) -> String {
        format!("{}|{}|{}", self.kind, self.frame, self.build)
    }
}

/// A row of a rollup, as it adds up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roll {
    pub count: u64,
    pub errors: u64,
    pub ms: u64,
    /// How many took under 10 ms, 100 ms, 1 s and 10 s, and longer.
    pub under: [u64; 5],
}

impl Roll {
    fn add(&mut self, ms: u64, ok: bool) {
        self.count += 1;
        self.errors += u64::from(!ok);
        self.ms += ms;
        self.under[match ms {
            0..10 => 0,
            10..100 => 1,
            100..1_000 => 2,
            1_000..10_000 => 3,
            _ => 4,
        }] += 1;
    }
}

/// How a fingerprint adds up.
#[derive(Clone, Debug)]
pub struct Printed {
    pub print: Print,
    pub count: u64,
    pub first: u64,
    pub last: u64,
}

/// What a device has to say about one scope, since it last said it.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub signals: Vec<Signal>,
    /// By (hour since 1970, name, app, key).
    pub rollups: BTreeMap<(u64, String, String, String), Roll>,
    /// By (app, tool, fingerprint).
    pub prints: BTreeMap<(String, String, String), Printed>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty() && self.rollups.is_empty() && self.prints.is_empty()
    }

    /// Adds a finished signal: to the rollups always, and in full only if it's `kept`.
    pub fn add(&mut self, signal: Signal, kept: bool, print: Option<Print>) {
        let key = (signal.time / 3_600_000, signal.name.clone(), signal.app.clone(), signal.attrs.rollup_key(signal.fault));
        self.rollups.entry(key).or_default().add(signal.ms, signal.ok);
        if let Some(print) = print {
            let at = signal.time;
            let id = (print.app.clone(), print.tool.clone(), print.fingerprint());
            self.prints.entry(id).and_modify(|p| { p.count += 1; p.last = p.last.max(at); p.first = p.first.min(at) }).or_insert(Printed { print, count: 1, first: at, last: at });
        }
        if kept && self.signals.len() < MOST_SIGNALS {
            self.signals.push(signal);
        }
    }
}

/// What a reader asks of `insights`.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub app: Option<String>,
    pub name: Option<String>,
    pub since: Option<u64>,
    /// `ok` or `error`.
    pub status: Option<String>,
    pub fault: Option<String>,
    pub limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_set_allows_is_kept() {
        let secret = "anna@example.org";
        let attrs = Attrs::default().tool(secret).service("https://x.example/a?b=c").version("a b").client(secret).device(secret).build(secret).stage(secret).error(secret).steps(&[json!({ "step": "device", "at": 1, "ms": 2, "why": secret, "device": secret, "result": secret, "trace": { "secret": secret } }), json!({ "step": secret })]);
        let stored = attrs.json().to_string();
        assert!(!stored.contains("anna") && !stored.contains("example"), "{stored}");
        assert_eq!(attrs.get("one.client"), Some(&json!("other")));
        assert_eq!(attrs.get("error.type"), Some(&json!(OTHER)));
        assert_eq!(attrs.get("one.steps"), Some(&json!([{ "step": "device", "at": 1, "ms": 2 }])));
        let e = Exception::from_json(&json!({ "type": secret, "frames": [{ "file": "app.js", "line": 3, "col": 9 }, { "file": "a/b@c", "line": 1, "col": 1 }, { "file": "lib/a.js", "line": 2, "col": 1 }] })).unwrap();
        assert_eq!(e.kind, "Error");
        assert_eq!(e.frames, ["app.js:3:9", "lib/a.js:2:1"]);
    }

    #[test]
    fn a_caller_is_told_apart_from_a_fault() {
        let thrown = |kind: &str, file: &str| Exception { kind: kind.into(), frames: vec![format!("{file}:1:1")] };
        assert_eq!(fault(Some(code::REFUSED), stage::RUN, None), Fault::Caller);
        assert_eq!(fault(Some(code::RESTARTING), stage::RUN, None), Fault::Internal);
        assert_eq!(fault(None, stage::CHECK, None), Fault::Caller);
        assert_eq!(fault(None, stage::WRITE, None), Fault::Internal);
        assert_eq!(fault(None, stage::RUN, Some(&thrown("Error", "app.js"))), Fault::Caller);
        assert_eq!(fault(None, stage::RUN, Some(&thrown("TypeError", "app.js"))), Fault::Internal);
        assert_eq!(fault(None, stage::RUN, Some(&thrown("Error", "shim.js"))), Fault::Internal);
    }
}
