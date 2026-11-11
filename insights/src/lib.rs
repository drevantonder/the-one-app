//! Insights: how the network is doing, recorded by the network itself. A dumb pipeline: it records, sends and offers one
//! read tool (`insights`), and has no opinion on what's a problem.
//!
//! It speaks OpenTelemetry's data model and field names (`service.name`, `error.type`, W3C trace and span ids) without its
//! SDK. Each device keeps one batch in memory and sends it to the telemetry Worker every minute or so (`worker.rs` here),
//! which keeps it in Cloudflare Basin (`worker/`): spans, logs and metrics. Every server span is sent, since
//! a service's rates are read from them; a call's steps and its hops only for failures, slow calls and one trace in 20.
//! Telemetry may be lost.
//!
//! A signal's attributes hold closed-set values only (`Attrs`): never a call's arguments or answer, a URL, a file, repo or
//! account's name, an email, an IP, a prompt or anything key-shaped, and never names a group. A call's user is said by their
//! id alone (`enduser.id`), which is operational. The one free text is `exception.message`, what a real failure said, cleaned
//! first (`message`); a refusal's is never kept.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use ::custom_build::protocol::code;
use ::api::traceparent::{TIMEOUT, UNREACHABLE, stage};

pub mod cost;
pub mod message;
pub mod otlp;
pub mod says;
pub mod source_map;
pub mod tools;
pub mod view;

#[cfg(not(target_family = "wasm"))]
pub mod cli;

/// Why this app runtime's last process ended, kept in its home so the next one can say: what it said when it stopped. A crash can't
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


/// The most a batch holds of each, past which what's new is dropped: as many as the telemetry Worker takes at once.
const MOST_ROWS: usize = 5000;
/// The Workers read as `worker.<name>` services (`worker_service`): all but oned on Cloudflare, which reports as an app runtime.
pub const WORKERS: [&str; 7] = ["gateway", "auth", "relay", "url-signer", "notifications", "onet", "telemetry"];
/// Those whose traces and logs Cloudflare exports, as the telemetry Worker's `TRACED` says: not the telemetry Worker itself,
/// as an export of what it did would be the next thing it was handed.
pub const TRACED: [&str; 6] = ["gateway", "auth", "relay", "url-signer", "notifications", "onet"];

/// One of `WORKERS`, as a service.
pub fn worker_service(worker: &str) -> String {
    format!("worker.{worker}")
}

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
const REFUSALS: &[&str] = &[code::UPDATE, code::NOT_LET_IN, code::REFUSED, code::NOT_THIS_MACHINE, code::NOT_SIGNED_IN, code::NOT_ADDED, code::APP_DELETED, code::ALREADY, code::NOT_LISTED, code::PAIRING_USED, code::BAD_PAIRING_CODE, code::ALREADY_IN_NETWORK, code::NO_KEY, code::RETRY_CHANGED];

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


/// What a client names itself on a call. Anything else is `other`, since a caller says it.
pub const CLIENTS: &[&str] = &["cli", "mcp", "code", "mac", "phone", "android", "windows", "linux", "browser", "web", "schedule", "app", "node", "api", "onet", "remote-mcp", "cloudflare-node"];

/// The JavaScript errors whose name is the engine's, not a program's: anything else is `Error`.
const EXCEPTIONS: &[&str] = &["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError", "URIError", "EvalError", "AggregateError"];

/// How a client's try of an app runtime went (onet/src/lib.rs).
const TRY_RESULTS: &[&str] = &["answered", "elsewhere", "no connection", "not taking requests", "no answer", "unreachable", "lost", "failed"];

/// Why a process ended or an update wasn't taken.
const REASONS: &[&str] = &["renewed", "stopped", "failed", "update", "watchdog", "abort", "signal", "first", "download", "install", "verify", "app-bundle"];

/// What a refusal is listed as when what was asked isn't something its route takes (`api::Invalid`).
pub const INVALID: &str = "invalid";
/// A name of OTEL's for an error of no known kind.
pub const OTHER: &str = "_OTHER";
pub const LOST: &str = "lost";
/// Every device asked said to ask another, so none would run it.
pub const UNAVAILABLE: &str = "unavailable";

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
    /// What it had asked the app runtime (`call::Ask`) when the app runtime's answer was the failure: the runtime threw it, not the app.
    pub ask: Option<&'static str>,
}

impl Exception {
    /// From `{ type, message, frames: [{ file, line, col }] }`, whoever sent it: a frame that isn't only a location is left out.
    pub fn from_json(v: &Value) -> Option<Exception> {
        let kind = exception_type(v["type"].as_str()?).to_string();
        let frames = v["frames"].as_array().into_iter().flatten().filter_map(frame).take(MOST_FRAMES).collect();
        let ask = v["ask"].as_str().and_then(::api::call::Ask::parse).map(::api::call::Ask::name);
        Some(Exception { kind, frames, message: v["message"].as_str().filter(|m| !m.is_empty()).map(str::to_string), ask })
    }

    /// What says where it failed: the first frame of the app's or screen's own, past the runtime's that carried it there;
    /// with none of its own, what it had asked the app runtime, as `ask:<name>`, since the runtime's frames are the same for every app.
    pub fn top_frame(&self) -> Option<String> {
        let own = self.frames.iter().find(|f| !RUNTIME_FRAMES.iter().any(|r| f.starts_with(r)));
        own.cloned().or(self.ask.map(|ask| format!("ask:{ask}"))).or(self.frames.first().cloned())
    }

    /// Each frame where it was written, for those of a built file `map_of` knows the source map of: the rest stay as they are.
    pub fn located(mut self, map_of: impl Fn(&str) -> Option<String>) -> Exception {
        let mut maps = BTreeMap::new();
        for at in &mut self.frames {
            let mut parts = at.rsplitn(3, ':');
            let (col, line, file) = (parts.next().and_then(|c| c.parse().ok()), parts.next().and_then(|l| l.parse().ok()), parts.next());
            let (Some(col), Some(line), Some(file)) = (col, line, file) else { continue };
            let map = maps.entry(file.to_string()).or_insert_with(|| map_of(file));
            let Some((source, line, col)) = map.as_deref().and_then(|map| source_map::located(map, line, col)) else { continue };
            if file_named(&source) {
                *at = format!("{source}:{line}:{col}");
            }
        }
        self
    }
}

/// What the runtime's own frames start with: an app's (app-runtime/src/app/runtime/shim.js), and a screen's kit (@one/ui).
const RUNTIME_FRAMES: [&str; 2] = ["shim.js:", "ui.js:"];

/// A frame as `file:line:col`, if it's only that: a file's name is the app's code's, in a bundle, and nothing else may be
/// where one goes.
fn frame(v: &Value) -> Option<String> {
    let file = v["file"].as_str()?;
    file_named(file).then(|| format!("{file}:{}:{}", v["line"].as_u64().unwrap_or(0), v["col"].as_u64().unwrap_or(0)))
}

fn file_named(file: &str) -> bool {
    !file.is_empty() && file.len() <= 96 && file.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
}

/// A signal's attributes: what one may name, and only values that can say nothing but what the key says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs(BTreeMap<&'static str, Value>);

impl Attrs {
    /// A call as OTEL's RPC conventions name it: `rpc.system.name`, and `rpc.method` as `<app>/<tool>`.
    pub fn rpc(mut self, app: &str, tool: &str) -> Attrs {
        self.0.insert("rpc.system.name", json!("one"));
        if token(app) && token(tool) {
            self.0.insert("rpc.method", json!(format!("{app}/{tool}")));
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

    /// What a client tried: which app runtime, and how it went.
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

    /// Who made a call, as OTEL's `enduser.id`: a user's id, never their name.
    pub fn user(mut self, id: &str) -> Attrs {
        if scope::scopes::is_id(id) {
            self.0.insert("enduser.id", json!(id));
        }
        self
    }

    pub fn outcome(mut self, outcome: &'static str) -> Attrs {
        self.0.insert("one.outcome", json!(outcome));
        self
    }

    /// What a request reached: `bucket`, or a Worker at the network's address, from a closed set.
    pub fn reached(mut self, what: &'static str) -> Attrs {
        self.0.insert("one.reached", json!(what));
        self
    }

    /// Who says a device is on or off: the relay, or this app runtime.
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

    /// How long something a log record says took, as that record has no duration of its own.
    pub fn duration(mut self, ms: u64) -> Attrs {
        self.0.insert("one.duration", json!(ms));
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

    /// `error.type`: a protocol code, an exception's type, how the devices asked couldn't run it, or OTEL's own `timeout` or `_OTHER`.
    pub fn error(mut self, kind: &str) -> Attrs {
        self.0.insert("error.type", json!(error_kind(kind)));
        self
    }

    /// An exception as OTEL's span event for one, among what the span says. What it said is the span's `message`.
    pub fn exception(mut self, e: &Exception) -> Attrs {
        let mut said = serde_json::Map::from_iter([("exception.type".to_string(), json!(e.kind))]);
        if !e.frames.is_empty() {
            said.insert("exception.stacktrace".into(), json!(e.frames.iter().map(|f| format!("at {f}")).collect::<Vec<_>>().join("\n")));
        }
        self.0.insert("events", json!([{ "name": "exception", "attributes": said }]));
        self
    }

    /// What the Errors list groups by: the failure's type, what threw it and the build it ran in. Made from closed sets alone.
    pub fn fingerprint(mut self, kind: &str, frame: &str, build: &str) -> Attrs {
        self.0.insert("one.fingerprint", json!(format!("{}|{frame}|{build}", error_kind(kind))));
        self
    }

    /// What a step of a call says besides its name and time, each a number or from a closed set.
    fn step(mut self, step: &Value) -> Attrs {
        for (field, key) in [("asks", "one.asks"), ("found", "one.found"), ("new", "one.new")] {
            if step[field].is_number() {
                self.0.insert(key, step[field].clone());
            }
        }
        if let Some(kind) = step["kind"].as_str().filter(|k| ["read", "create", "replace"].contains(k)) {
            self.0.insert("one.kind", json!(kind));
        }
        if let Some(result) = step["result"].as_str().filter(|r| RESULTS.contains(r)) {
            self.0.insert("one.result", json!(result));
        }
        self
    }

    /// What an app says of a span of its own (`ctx.span`): only numbers and true or false, so it can name nothing.
    fn app(mut self, said: &Value) -> Attrs {
        let kept: serde_json::Map<String, Value> = said.as_object().into_iter().flatten().filter(|(key, value)| token(key) && (value.is_number() || value.is_boolean())).take(MOST_APP_ATTRS).map(|(key, value)| (key.clone(), value.clone())).collect();
        if !kept.is_empty() {
            self.0.insert("app", Value::Object(kept));
        }
        self
    }

    /// An AI Gateway call's attributes (`ai_gateway::span_attrs`): only those it lists, and of them only numbers, true or
    /// false, plain names and model ids, so no prompt or answer can be one.
    pub fn gen_ai(mut self, attrs: &[(&'static str, Value)]) -> Attrs {
        let plain = |v: &str| !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-+/@:".contains(&b));
        for (key, value) in attrs {
            let kept = match value {
                Value::Number(_) | Value::Bool(_) => true,
                Value::String(s) => plain(s),
                Value::Array(all) => all.len() <= 8 && all.iter().all(|v| v.as_str().is_some_and(plain)),
                _ => false,
            };
            if kept && ai_gateway::SPAN_ATTRS.contains(key) {
                self.0.insert(key, value.clone());
            }
        }
        self
    }

    fn token(&mut self, key: &'static str, value: &str) {
        if token(value) {
            self.0.insert(key, json!(value));
        }
    }

    pub fn json(&self) -> Value {
        json!(self.0)
    }
}

/// `kind` if it's a name `error.type` may hold, else `_OTHER`.
fn error_kind(kind: &str) -> &str {
    if code::ALL.contains(&kind) || EXCEPTIONS.contains(&kind) || [TIMEOUT, UNREACHABLE, LOST, UNAVAILABLE, INVALID].contains(&kind) { kind } else { OTHER }
}

/// Whether a value is a plain name or hash: letters, digits and `_ . - +`, so no address, path, URL or sentence.
pub fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 72 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-+".contains(&b))
}

/// The steps a trace may name, and the fields of each that are only a name or a number.
const STEPS: &[&str] = &["device", "bucket", "online", "connect", "lease", "prepare", "write", "keys", "behind", "data", "open", "logs", "run"];
const RESULTS: &[&str] = &["ran", "failed", "elsewhere", "needs keys", "unreachable", "no answer", "lost", "caught up", "behind", "asked"];
/// Those of them that say a step didn't do what it tried.
const FAILED_RESULTS: &[&str] = &["failed", "unreachable", "no answer", "lost"];


/// The most spans of its own a call may add (`ctx.span`), and attributes on each.
pub const MOST_APP_SPANS: usize = 32;
const MOST_APP_ATTRS: usize = 16;

/// A span's steps, each as a real child of it. Only the steps `STEPS` names are kept, and an app's own (`ctx.span`) by a
/// plain name, with only its numbers and true or false.
pub fn step_spans(parent: &Span, steps: &[Value]) -> Vec<Span> {
    let app_named = |s: &Value| s["step"] == "span" && s["name"].as_str().is_some_and(token);
    steps.iter().filter(|s| s["step"].as_str().is_some_and(|name| STEPS.contains(&name)) || app_named(s)).map(|step| {
        let app = step["step"] == "span";
        Span {
            time: parent.time + step[if app { "began" } else { "at" }].as_u64().unwrap_or(0),
            trace: parent.trace.clone(),
            span: ::api::traceparent::ids::span(),
            parent: Some(parent.span.clone()),
            name: step[if app { "name" } else { "step" }].as_str().unwrap_or("").to_string(),
            service: parent.service.clone(),
            version: None,
            instance: parent.instance.clone(),
            kind: SpanKind::Internal,
            status: Status::of(step["failed"] != true && !step["result"].as_str().is_some_and(|r| FAILED_RESULTS.contains(&r))),
            duration: step["ms"].as_u64().unwrap_or(0),
            message: None,
            attrs: if app { Attrs::default().app(&step["attrs"]) } else { Attrs::default().step(step) },
        }
    }).collect()
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

/// OTEL's severities, those it's used at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn number(self) -> u64 {
        match self {
            Severity::Info => 9,
            Severity::Warn => 13,
            Severity::Error => 17,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warn => "WARN",
            Severity::Error => "ERROR",
        }
    }
}

/// OTEL's default duration boundaries, in ms. The same as the telemetry Worker's and the screen's, as tests/contracts.rs holds them.
pub const BOUNDS: [u64; 15] = [0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000];

/// A metric: what it counts, whether it's a histogram, and its unit. A name doesn't carry its unit.
#[derive(Clone, Copy, Debug)]
pub struct Metric {
    pub name: &'static str,
    pub histogram: bool,
    pub unit: &'static str,
}

pub const BUCKET_ASKS: Metric = Metric { name: "bucket.asks", histogram: false, unit: "{ask}" };
/// What a device read of the bucket, by kind of object (`one.reached`) and whether it was there (`one.outcome`).
pub const BUCKET_READS: Metric = Metric { name: "bucket.reads", histogram: false, unit: "{read}" };
pub const BUCKET_DURATION: Metric = Metric { name: "bucket.duration", histogram: true, unit: "ms" };
pub const STEPS_BEHIND: Metric = Metric { name: "steps.behind", histogram: false, unit: "{step}" };
pub const WRITE_SEEN: Metric = Metric { name: "write.seen", histogram: true, unit: "ms" };
/// How a client's first screen opened, from its page's start: its window drawn, the screen showing (what it kept of it,
/// or itself), and what it asked of the user's devices answered.
pub const OPEN_WINDOW: Metric = Metric { name: "client.open.window", histogram: true, unit: "ms" };
pub const OPEN_SHOWN: Metric = Metric { name: "client.open.shown", histogram: true, unit: "ms" };
pub const OPEN_FRESH: Metric = Metric { name: "client.open.fresh", histogram: true, unit: "ms" };
/// Requests to Cloudflare that passed their learned limit with no first byte (`hedge`), by what they reached and how each ended.
pub const LATE: Metric = Metric { name: "http.client.late", histogram: false, unit: "{request}" };
pub const SIGNER_FAILURES: Metric = Metric { name: "signer.failures", histogram: false, unit: "{failure}" };
pub const CANARY: Metric = Metric { name: "telemetry.canary", histogram: false, unit: "{check}" };
/// What AI Gateway's calls used and took, as OTEL's GenAI conventions name them (but in ms, as every duration here is).
pub const GEN_AI_TOKENS: Metric = Metric { name: "gen_ai.client.token.usage", histogram: true, unit: "{token}" };
pub const GEN_AI_DURATION: Metric = Metric { name: "gen_ai.client.operation.duration", histogram: true, unit: "ms" };

/// One span, as a device says it. `service` is an app, or the network's own program (`own`).
#[derive(Clone, Debug)]
pub struct Span {
    pub time: u64,
    pub trace: String,
    pub span: String,
    pub parent: Option<String>,
    pub name: String,
    pub service: String,
    pub version: Option<String>,
    pub instance: String,
    pub kind: SpanKind,
    pub status: Status,
    pub duration: u64,
    /// `exception.message`, cleaned.
    pub message: Option<String>,
    pub attrs: Attrs,
}

impl Span {
    /// As the telemetry Worker takes it and gives it back: its table's columns.
    pub fn row(&self) -> Value {
        json!({ "time": self.time, "trace": self.trace, "span": self.span, "parent": self.parent, "name": self.name, "service": self.service, "version": self.version, "instance": self.instance, "kind": self.kind.as_str(), "status": self.status.as_str(), "duration": self.duration, "message": self.message, "attrs": self.attrs.json().to_string() })
    }
}

/// One log record: what used to be called an event, and where warnings live.
#[derive(Clone, Debug)]
pub struct Log {
    pub time: u64,
    pub trace: Option<String>,
    pub span: Option<String>,
    pub severity: Severity,
    pub body: String,
    pub service: String,
    pub version: Option<String>,
    pub instance: String,
    pub attrs: Attrs,
}

/// A metric as it adds up over a minute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sum {
    pub count: u64,
    pub sum: u64,
    /// Bucket `i` is what's at most `BOUNDS[i]`, past the one before; the last what's over them all.
    pub buckets: [u64; BOUNDS.len() + 1],
}

impl Sum {
    fn add(&mut self, value: u64) {
        self.count += 1;
        self.sum += value;
        self.buckets[BOUNDS.iter().position(|b| value <= *b).unwrap_or(BOUNDS.len())] += 1;
    }
}


/// The network's own program, the app runtime, as a service: what Insights names what it does.
pub fn own() -> &'static str {
    "app-runtime"
}

/// What a device has to say, since it last said it.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub spans: Vec<Span>,
    pub logs: Vec<Log>,
    /// By (minute, name, service, instance, attributes as JSON): its metric and how it adds up.
    pub metrics: BTreeMap<(u64, &'static str, String, String, String), (Metric, Sum)>,
    /// What programs said in OTLP (`otlp::metrics`), by (minute, name, service, instance), added up.
    pub programs: BTreeMap<(u64, String, String, String), otlp::Point>,
}

impl Batch {
    /// An AI Gateway call ended at `now`: one GenAI client span, a child of `parent` in `trace` (a trace of its own if
    /// none), always kept, and what it used and took as OTEL's two GenAI metrics. `service` is the app that asked, or the
    /// network's own; `user` who asked.
    pub fn gen_ai(&mut self, now: u64, trace: Option<(String, Option<String>)>, service: &str, instance: &str, user: &str, call: ai_gateway::Span) {
        let failure = call.failed.as_ref().map(|(code, _)| Failure::of(*code, None, stage::RUN));
        let mut attrs = Attrs::default().gen_ai(&call.attrs);
        let mut message = None;
        if let (Some(failure), Some((_, said))) = (&failure, &call.failed) {
            attrs = attrs.failed(failure);
            if matches!(failure, Failure::Broke(_)) {
                message = Some(message::cleaned(said));
            }
        }
        let time = now.saturating_sub(call.ms);
        let (trace, parent) = trace.unwrap_or_else(|| (::api::traceparent::ids::trace(), None));
        let status = failure.as_ref().map_or(Status::Unset, Failure::status);
        let span = Span { time, trace, span: ::api::traceparent::ids::span(), parent, name: call.name, service: service.to_string(), version: None, instance: instance.to_string(), kind: SpanKind::Client, status, duration: call.ms, message, attrs };
        // The metrics' attributes: what they're read by (operation, company, model, payer, tier), never more.
        let by: Vec<(&'static str, Value)> = call.attrs.iter().filter(|(k, _)| matches!(*k, "gen_ai.operation.name" | "gen_ai.provider.name" | "gen_ai.request.model" | "one.paid" | "one.tier")).cloned().collect();
        let about = Attrs::default().gen_ai(&by).user(user);
        self.span(span, true, &[]);
        self.metric(time, GEN_AI_DURATION, service, instance, &about, call.ms);
        if let Some(used) = &call.used {
            for (kind, n) in [("input", used.input), ("output", used.output)] {
                let mut typed = by.clone();
                typed.push(("gen_ai.token.type", json!(kind)));
                self.metric(time, GEN_AI_TOKENS, service, instance, &Attrs::default().gen_ai(&typed).user(user), n);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty() && self.logs.is_empty() && self.metrics.is_empty() && self.programs.is_empty()
    }

    /// Adds a finished span, and its children if it's kept. A server span always goes: rates are read from them.
    pub fn span(&mut self, span: Span, kept: bool, steps: &[Value]) {
        if kept {
            self.spans.extend(step_spans(&span, steps));
        }
        if kept || span.kind == SpanKind::Server {
            self.spans.push(span);
        }
        self.spans.truncate(MOST_ROWS);
    }

    pub fn log(&mut self, log: Log) {
        if self.logs.len() < MOST_ROWS {
            self.logs.push(log);
        }
    }

    pub fn metric(&mut self, time: u64, metric: Metric, service: &str, instance: &str, attrs: &Attrs, value: u64) {
        let key = (time / 60_000 * 60_000, metric.name, service.to_string(), instance.to_string(), attrs.json().to_string());
        if self.metrics.len() < MOST_ROWS || self.metrics.contains_key(&key) {
            self.metrics.entry(key).or_insert_with(|| (metric, Sum::default())).1.add(value);
        }
    }

    /// Adds what a program said of a metric to this minute's.
    pub fn program(&mut self, time: u64, point: otlp::Point, service: &str, instance: &str) {
        let key = (time / 60_000 * 60_000, point.name.clone(), service.to_string(), instance.to_string());
        if let Some(had) = self.programs.get_mut(&key) {
            had.add(&point);
        } else if self.programs.len() < MOST_ROWS {
            self.programs.insert(key, point);
        }
    }

    /// As the telemetry Worker takes it, each row with its table's columns.
    pub fn rows(&self) -> Value {
        let spans: Vec<Value> = self.spans.iter().map(Span::row).collect();
        let logs: Vec<Value> = self.logs.iter().map(|l| json!({ "time": l.time, "trace": l.trace, "span": l.span, "severity_number": l.severity.number(), "severity_text": l.severity.text(), "body": l.body, "service": l.service, "version": l.version, "instance": l.instance, "attrs": l.attrs.json().to_string() })).collect();
        let metrics: Vec<Value> = self.metrics.iter().map(|((time, name, service, instance, attrs), (metric, sum))| json!({ "time": time, "name": name, "type": if metric.histogram { "histogram" } else { "counter" }, "unit": metric.unit, "service": service, "instance": instance, "count": sum.count, "sum": sum.sum, "buckets": metric.histogram.then(|| json!(sum.buckets).to_string()), "attrs": attrs })).collect();
        let programs = self.programs.iter().map(|((time, name, service, instance), p)| json!({ "time": time, "name": name, "type": p.kind, "unit": p.unit, "service": service, "instance": instance, "count": p.count, "sum": p.sum, "buckets": p.buckets.as_ref().map(|b| json!(b).to_string()), "attrs": "{}" }));
        json!({ "spans": spans, "logs": logs, "metrics": metrics.iter().cloned().chain(programs).collect::<Vec<Value>>() })
    }
}

/// What a reader asks of `insights`.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    /// Which of the telemetry Worker's reads: all of them if none's named.
    pub queries: Vec<&'static str>,
    pub service: Option<String>,
    pub name: Option<String>,
    pub trace: Option<String>,
    pub since: Option<u64>,
    /// `unset` or `error`.
    pub status: Option<String>,
    /// Only traces with a span that took at least this long, in ms.
    pub least: Option<u64>,
    pub limit: usize,
}

/// What `client` read of the bucket (`reads`), the device `instance`'s (its key, hex), as the telemetry Worker writes it (`/telemetry/write`'s
/// fields), once it's been a minute since it last said.
pub fn bucket_reads(reads: &bucket::Reads, now: u64, instance: &str, client: &str) -> Option<serde_json::Value> {
    let read = reads.due(now)?;
    let mut batch = crate::Batch::default();
    for (kind, found, n) in read {
        let attrs = crate::Attrs::default().reached(kind).outcome(if found { "found" } else { "missing" }).client(client).scope("everyone").build(::custom_build::config::BUILD);
        batch.metric(now, crate::BUCKET_READS, crate::own(), instance, &attrs, n);
    }
    (!batch.is_empty()).then(|| batch.rows())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_set_allows_is_kept() {
        let secret = "anna@example.org";
        let parent = Span { time: 5, trace: "0".repeat(32), span: "1".repeat(16), parent: None, name: "x".into(), service: "s".into(), version: None, instance: "2".repeat(64), kind: SpanKind::Server, status: Status::Unset, duration: 3, message: None, attrs: Attrs::default() };
        let steps = step_spans(&parent, &[json!({ "step": "device", "at": 1, "ms": 2, "why": secret, "device": secret, "result": secret, "trace": { "secret": secret } }), json!({ "step": secret })]);
        assert_eq!(steps.len(), 1, "a step of no known name isn't one");
        assert_eq!((steps[0].time, steps[0].duration, steps[0].parent.as_deref()), (6, 2, Some(parent.span.as_str())));
        let attrs = Attrs::default().rpc(secret, "t").client(secret).user(secret).build(secret).stage(secret).error(secret).fingerprint(secret, "stage:run", "b1");
        let stored = format!("{} {}", attrs.json(), steps[0].attrs.json());
        assert!(!stored.contains("anna") && !stored.contains("example"), "{stored}");
        assert_eq!(attrs.json()["one.client"], "other");
        assert_eq!(attrs.json().get("enduser.id"), None, "only a user's id says who");
        assert_eq!(attrs.json()["error.type"], OTHER);
        assert_eq!(attrs.json()["one.fingerprint"], format!("{OTHER}|stage:run|b1"));
        let e = Exception::from_json(&json!({ "type": secret, "frames": [{ "file": "app.js", "line": 3, "col": 9 }, { "file": "a/b@c", "line": 1, "col": 1 }, { "file": "lib/a.js", "line": 2, "col": 1 }] })).unwrap();
        assert_eq!(e.kind, "Error");
        assert_eq!(e.frames, ["app.js:3:9", "lib/a.js:2:1"]);
        let events = Attrs::default().exception(&e).json();
        assert_eq!(events["events"][0]["attributes"]["exception.stacktrace"], "at app.js:3:9\nat lib/a.js:2:1");
        let asked = Exception::from_json(&json!({ "type": "Error", "ask": "call", "frames": [{ "file": "shim.js", "line": 76, "col": 34 }] })).unwrap();
        assert_eq!(asked.top_frame().as_deref(), Some("ask:call"), "the runtime's frames name no app's code");
        assert_eq!(Exception::from_json(&json!({ "type": "Error", "ask": secret, "frames": [] })).unwrap().ask, None);
    }

    /// What's the caller's to change never counts as something that broke, and nothing else is taken for it.
    #[test]
    fn only_a_refusal_is_the_callers() {
        let thrown = Exception { kind: "TypeError".into(), frames: vec!["app.js:1:1".into()], message: None, ask: None };
        assert_eq!(Failure::of(Some(code::REFUSED), None, stage::RUN), Failure::Refused("refused".into()));
        assert_eq!(Failure::of(Some(code::REFUSED), None, stage::RUN).status(), Status::Unset);
        assert_eq!(Failure::of(Some(code::RESTARTING), None, stage::RUN), Failure::Broke("restarting".into()));
        assert_eq!(Failure::of(None, None, stage::CHECK), Failure::Broke(OTHER.into()));
        assert_eq!(Failure::of(None, Some(&thrown), stage::RUN), Failure::Broke("TypeError".into()));
        assert_eq!(Failure::of(None, None, stage::LIMIT), Failure::Broke(TIMEOUT.into()));
        assert_eq!(Failure::of(Some("whatever"), None, stage::RUN).status(), Status::Error, "only the codes that say so are refusals");
    }
}
