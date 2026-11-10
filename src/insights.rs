//! Insights: how the network is doing, recorded by the network itself. A dumb pipeline: it records, stores, expires and
//! offers one read tool (`insights`), and has no opinion on what's a problem.
//!
//! It speaks OpenTelemetry's data model and field names (`service.name`, `error.type`, `exception.stacktrace`, W3C trace
//! and span ids) without its SDK. Each device keeps a batch per scope in memory and writes it to that scope's node
//! database every few minutes (`node.rs` here): three tables, of raw signals (a week), hourly rollups (90 days) and error
//! fingerprints (until their build is gone from the network). Failures, slow calls and one trace in 20 keep their spans.
//! Telemetry may be lost.
//!
//! A signal's attributes hold closed-set values only (`Attrs`): never a call's arguments or answer, a URL, a file, repo or
//! account's name, an email, an IP, a prompt or anything key-shaped, and never names a user or a group. The one free text is
//! `exception.message`, what a real failure said, cleaned first (`message`) and kept with the raw signal alone; a refusal's
//! is never kept.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::protocol::code;

pub mod message;
pub mod tools;
pub mod view;

#[cfg(feature = "node")]
pub mod cli;
#[cfg(feature = "node")]
pub(crate) mod store;
#[cfg(feature = "node")]
pub mod routes;
pub mod otlp;

/// Why this node's last process ended, kept in its home so the next one can say: what it said when it stopped. A crash can't
/// say (`panic = "abort"`), so none there, after a run that started, is `Abort`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Update,
    Watchdog,
    Signal,
    Abort,
    First,
}

const ENDED: &str = "ended";
const STARTED: &str = "started";

impl Ended {
    pub fn reason(self) -> &'static str {
        match self {
            Ended::Update => "update",
            Ended::Watchdog => "watchdog",
            Ended::Signal => "signal",
            Ended::Abort => "abort",
            Ended::First => "first",
        }
    }

    pub fn write(self, home: &std::path::Path) {
        let _ = std::fs::write(home.join(ENDED), self.reason());
    }

    /// What the last process said, and that this one is running now.
    pub fn read(home: &std::path::Path) -> Ended {
        let said = std::fs::read_to_string(home.join(ENDED)).unwrap_or_default();
        let ended = match said.as_str() {
            "update" => Ended::Update,
            "watchdog" => Ended::Watchdog,
            "signal" => Ended::Signal,
            _ if home.join(STARTED).exists() => Ended::Abort,
            _ => Ended::First,
        };
        let _ = std::fs::remove_file(home.join(ENDED));
        let _ = std::fs::write(home.join(STARTED), "");
        ended
    }
}

/// How long a call takes before it keeps a full span. ARCHITECTURE.md's targets state none for a whole call, and its
/// nearest is a write seen elsewhere within 1 s.
pub const SLOW_MS: u64 = 1_000;

/// One trace in this many that went well is kept whole, to see what normal looks like.
const SAMPLED_ONE_IN: u64 = 20;

/// Whether `trace` is one of those. By its id, so every span of a trace agrees, on any device.
pub fn sampled(trace: &str) -> bool {
    u64::from_str_radix(trace.get(trace.len().saturating_sub(8)..).unwrap_or(""), 16).is_ok_and(|n| n % SAMPLED_ONE_IN == 0)
}

pub const RAW_KEPT_MS: u64 = 7 * 86_400_000;
pub const ROLLUPS_KEPT_MS: u64 = 90 * 86_400_000;
/// The most raw signals a batch holds: past it, they only add to the rollups.
const MOST_SIGNALS: usize = 500;
/// The most stack frames a signal keeps.
pub const MOST_FRAMES: usize = 8;

/// OTEL's span status: left unset unless it failed, and never `Ok`, which is an operator's to set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Unset,
    Error,
}

impl Status {
    pub fn of(ok: bool) -> Status {
        if ok { Status::Unset } else { Status::Error }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Unset => "unset",
            Status::Error => "error",
        }
    }
}

/// How a call that didn't do what was asked ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The caller's to change, and nothing broke: OTEL's expected failure, its status unset and no `error.type`. Its code.
    Refused(String),
    /// Something broke: its `error.type`.
    Broke(String),
}

/// The codes a caller's request is refused with (`protocol::code`): everything else, and anything with no code, broke.
const REFUSALS: &[&str] = &[code::UPDATE, code::NOT_LET_IN, code::REFUSED, code::NOT_THIS_MACHINE, code::NOT_SIGNED_IN, code::NOT_ADDED, code::APP_DELETED, code::ALREADY, code::NOT_LISTED, code::PAIRING_USED, code::BAD_PAIRING_CODE, code::ALREADY_IN_NETWORK];

impl Failure {
    /// What ended a call with this `code`, if it carries one, or as `exception`, at `stage`.
    pub fn of(code: Option<&str>, exception: Option<&Exception>, stage: &str) -> Failure {
        match code {
            Some(c) if REFUSALS.contains(&c) => Failure::Refused(c.to_string()),
            _ => Failure::Broke(code.map(str::to_string).or(exception.map(|e| e.kind.clone())).unwrap_or_else(|| if stage == stage::LIMIT { TIMEOUT } else { OTHER }.to_string())),
        }
    }

    pub fn status(&self) -> Status {
        match self {
            Failure::Refused(_) => Status::Unset,
            Failure::Broke(_) => Status::Error,
        }
    }

    /// What it's listed as.
    pub fn kind(&self) -> &str {
        match self {
            Failure::Refused(kind) | Failure::Broke(kind) => kind,
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
    /// Going from device to device to find one that will run it.
    pub const ROUTE: &str = "route";
    pub const ALL: &[&str] = &[CHECK, GUARD, KEYS, LEASE, RUN, PREPARE, WRITE, LIMIT, ROUTE];
}

/// What a client names itself on a call. Anything else is `other`, since a caller says it.
pub const CLIENTS: &[&str] = &["cli", "mcp", "code", "mac", "phone", "android", "windows", "linux", "browser", "web", "schedule", "app", "node", "api"];

/// The JavaScript errors whose name is the engine's, not a program's: anything else is `Error`.
const EXCEPTIONS: &[&str] = &["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError", "URIError", "EvalError", "AggregateError"];

/// How a client's try of a node went (onet/src/lib.rs).
const TRY_RESULTS: &[&str] = &["answered", "elsewhere", "no connection", "not taking requests", "no answer", "unreachable", "lost", "failed"];

/// Why a process ended or an update wasn't taken.
const REASONS: &[&str] = &["renewed", "stopped", "failed", "update", "watchdog", "abort", "signal", "first", "download", "install", "verify", "app-bundle"];

/// A name of OTEL's for an error of no known kind.
pub const OTHER: &str = "_OTHER";
pub const TIMEOUT: &str = "timeout";
/// A device that couldn't be reached, and one that went away while it was being asked.
pub const UNREACHABLE: &str = "unreachable";
pub const LOST: &str = "lost";

/// `name` if it's one of the errors the engine names, else `Error`: an app can name its own anything.
pub fn exception_type(name: &str) -> &'static str {
    EXCEPTIONS.iter().find(|e| **e == name).copied().unwrap_or("Error")
}

/// An exception as a screen or the app runtime reports it: what threw it and where, and what it said, if it's been told.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exception {
    pub kind: String,
    pub frames: Vec<String>,
    /// As it came: `Attrs::message` cleans it.
    pub message: Option<String>,
}

impl Exception {
    /// From `{ type, message, frames: [{ file, line, col }] }`, whoever sent it: a frame that isn't only a location is left out.
    pub fn from_json(v: &Value) -> Option<Exception> {
        let kind = exception_type(v["type"].as_str()?).to_string();
        let frames = v["frames"].as_array().into_iter().flatten().filter_map(frame).take(MOST_FRAMES).collect();
        Some(Exception { kind, frames, message: v["message"].as_str().filter(|m| !m.is_empty()).map(str::to_string) })
    }

    /// The frame that says what threw it: the first of the app's own, past the runtime's that carried it there.
    pub fn top_frame(&self) -> Option<&String> {
        self.frames.iter().find(|f| !f.starts_with(RUNTIME_FRAME)).or(self.frames.first())
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

    pub fn schedule(mut self, schedule: &str) -> Attrs {
        self.token("one.schedule", schedule);
        self
    }

    /// Why a program ended or an update wasn't taken, a closed set: `exit-<code>`, `signal-<n>`, or one of `REASONS`.
    pub fn reason(mut self, reason: &str) -> Attrs {
        let numbered = |prefix: &str| reason.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()));
        if numbered("exit-") || numbered("signal-") || REASONS.contains(&reason) {
            self.0.insert("one.reason", json!(reason));
        }
        self
    }

    /// The device a signal is about, when that isn't the one saying it.
    pub fn subject(mut self, hex: &str) -> Attrs {
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.0.insert("one.node", json!(hex));
        }
        self
    }

    /// `gen_ai.request.model`.
    pub fn model(mut self, model: &str) -> Attrs {
        self.token("gen_ai.request.model", model);
        self
    }

    /// `gen_ai.token.type`.
    pub fn tokens(mut self, kind: &'static str) -> Attrs {
        self.0.insert("gen_ai.token.type", json!(kind));
        self
    }

    pub fn outcome(mut self, outcome: &'static str) -> Attrs {
        self.0.insert("one.outcome", json!(outcome));
        self
    }

    /// Who says a device is on or off: the relay, or this node.
    pub fn says(mut self, who: &'static str) -> Attrs {
        self.0.insert("one.says", json!(who));
        self
    }

    /// A part of an update: `node.<target>`, `app.<target>`, `service.<name>.<target>`, `web`, `workers`, `source`.
    pub fn part(mut self, part: &str) -> Attrs {
        self.token("one.part", &part.replace('/', "."));
        self
    }

    pub fn status_code(mut self, code: u64) -> Attrs {
        self.0.insert("http.response.status_code", json!(code));
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

    /// How a call that failed ended: `error.type` if something broke, or, as OTEL's expected failures do, a code of our own.
    pub fn failed(self, failure: &Failure) -> Attrs {
        match failure {
            Failure::Refused(code) => {
                let mut attrs = self;
                attrs.token("one.code", code);
                attrs
            }
            Failure::Broke(kind) => self.error(kind),
        }
    }

    /// `error.type`: a protocol code, an exception's type, a device that couldn't be asked, or OTEL's own `timeout` or `_OTHER`.
    pub fn error(mut self, kind: &str) -> Attrs {
        let known = code::ALL.contains(&kind) || EXCEPTIONS.contains(&kind) || [TIMEOUT, UNREACHABLE, LOST].contains(&kind);
        self.0.insert("error.type", json!(if known { kind } else { OTHER }));
        self
    }

    /// OTEL's `span.kind`.
    pub fn span_kind(mut self, kind: SpanKind) -> Attrs {
        self.0.insert("span.kind", json!(kind.as_str()));
        self
    }

    pub fn exception(mut self, e: &Exception) -> Attrs {
        self.0.insert("exception.type", json!(e.kind));
        if !e.frames.is_empty() {
            self.0.insert("exception.stacktrace", json!(e.frames.iter().map(|f| format!("at {f}")).collect::<Vec<_>>().join("\n")));
        }
        match &e.message {
            Some(said) => self.message(said),
            None => self,
        }
    }

    /// `exception.message`, what a real failure said, cleaned: only ever for one that broke, as a refusal keeps no text.
    pub fn message(mut self, said: &str) -> Attrs {
        self.0.insert("exception.message", json!(message::cleaned(said)));
        self
    }

    /// Which fingerprint this failure is, for a reader to find what a fingerprint's failures said: made from closed sets alone.
    pub fn fingerprint(mut self, id: &str) -> Attrs {
        self.0.insert("one.fingerprint", json!(id));
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

    /// The attributes a rollup groups by, as its key: those that are few.
    fn rollup_key(&self, kind: &str) -> String {
        let mut key: BTreeMap<&str, Value> = self.0.iter().filter(|(k, _)| ["one.client", "one.scope", "one.device", "one.build", "one.code", "gen_ai.request.model", "gen_ai.token.type", "one.outcome", "span.kind"].contains(k)).map(|(k, v)| (*k, v.clone())).collect();
        key.insert("kind", json!(kind));
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

/// OTEL's span kinds: what a span's rates mean. A `server` span is a request: it's what counts in a service's rates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Server,
    Client,
    Internal,
}

impl SpanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Server => "server",
            SpanKind::Client => "client",
            SpanKind::Internal => "internal",
        }
    }
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
    /// The app it's about, or none: the device's own, or a call of no such app.
    pub app: String,
    pub trace: String,
    pub span: String,
    pub parent: Option<String>,
    pub ms: u64,
    pub status: Status,
    pub attrs: Attrs,
}

/// An error grouped by what threw it: its type, its top code frame or the stage it failed at, the build it ran in and,
/// by its id alone, what it said once that's cleaned, so what differs only by a value is one line and the row holds no text.
#[derive(Clone, Debug)]
pub struct Print {
    pub app: String,
    pub tool: String,
    pub kind: String,
    pub frame: String,
    pub build: String,
    pub status: Status,
    /// `message::id` of what it said; empty for a refusal, which says nothing here.
    pub said: String,
}

impl Print {
    pub fn fingerprint(&self) -> String {
        match self.said.as_str() {
            "" => format!("{}|{}|{}", self.kind, self.frame, self.build),
            said => format!("{}|{}|{}|{said}", self.kind, self.frame, self.build),
        }
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

/// The most fingerprints a batch adds: past it what's new adds only to the rollups, as what a message varies by can be anything.
const MOST_PRINTS: usize = 500;

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty() && self.rollups.is_empty() && self.prints.is_empty()
    }

    /// Adds to a metric's rollup: for a metric, `ms` is the sum of its values and `count` how many were added.
    pub fn metric(&mut self, time: u64, name: &str, app: &str, attrs: &Attrs, value: u64) {
        self.rollups.entry((time / 3_600_000, name.to_string(), app.to_string(), attrs.rollup_key("metric"))).or_default().add(value, true);
    }

    /// Adds a finished signal: to the rollups always, and in full only if it's `kept`.
    pub fn add(&mut self, signal: Signal, kept: bool, print: Option<Print>) {
        let key = (signal.time / 3_600_000, signal.name.clone(), signal.app.clone(), signal.attrs.rollup_key(signal.kind.as_str()));
        self.rollups.entry(key).or_default().add(signal.ms, signal.status == Status::Unset);
        if let Some(print) = print {
            let at = signal.time;
            let id = (print.app.clone(), print.tool.clone(), print.fingerprint());
            if self.prints.len() < MOST_PRINTS || self.prints.contains_key(&id) {
                self.prints.entry(id).and_modify(|p| { p.count += 1; p.last = p.last.max(at); p.first = p.first.min(at) }).or_insert(Printed { print, count: 1, first: at, last: at });
            }
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
    pub trace: Option<String>,
    pub since: Option<u64>,
    /// `unset` or `error`.
    pub status: Option<String>,
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
        assert_eq!(attrs.json()["one.client"], "other");
        assert_eq!(attrs.json()["error.type"], OTHER);
        assert_eq!(attrs.json()["one.steps"], json!([{ "step": "device", "at": 1, "ms": 2 }]));
        let e = Exception::from_json(&json!({ "type": secret, "frames": [{ "file": "app.js", "line": 3, "col": 9 }, { "file": "a/b@c", "line": 1, "col": 1 }, { "file": "lib/a.js", "line": 2, "col": 1 }] })).unwrap();
        assert_eq!(e.kind, "Error");
        assert_eq!(e.frames, ["app.js:3:9", "lib/a.js:2:1"]);
        let cleaned = Attrs::default().exception(&Exception { message: Some(format!("no mail for {secret}")), ..e }).json();
        assert_eq!(cleaned["exception.message"], "no mail for <email>");
    }

    /// What's the caller's to change never counts as something that broke, and nothing else is taken for it.
    #[test]
    fn only_a_refusal_is_the_callers() {
        let thrown = Exception { kind: "TypeError".into(), frames: vec!["app.js:1:1".into()], message: None };
        assert_eq!(Failure::of(Some(code::REFUSED), None, stage::RUN), Failure::Refused("refused".into()));
        assert_eq!(Failure::of(Some(code::REFUSED), None, stage::RUN).status(), Status::Unset);
        assert_eq!(Failure::of(Some(code::RESTARTING), None, stage::RUN), Failure::Broke("restarting".into()));
        assert_eq!(Failure::of(None, None, stage::CHECK), Failure::Broke(OTHER.into()));
        assert_eq!(Failure::of(None, Some(&thrown), stage::RUN), Failure::Broke("TypeError".into()));
        assert_eq!(Failure::of(None, None, stage::LIMIT), Failure::Broke(TIMEOUT.into()));
        assert_eq!(Failure::of(Some("whatever"), None, stage::RUN).status(), Status::Error, "only the codes that say so are refusals");
    }
}
