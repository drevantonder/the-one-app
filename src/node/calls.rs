//! Calls: where each runs, running it here or on another device, what it wrote, and what its app asks the node while it runs.
//!
//! A call runs once. It names the scopes it touches before it runs, so a device that can't run it says so before doing
//! anything; one that writes takes a lease in the bucket first, so failing over never repeats it. It writes nothing if
//! it fails, or is stopped at its limit, which happens only before it writes: what it keeps goes in the bucket first,
//! and once it writes, it's waited for. A node a client picked runs a call itself if it runs every scope the call
//! reaches; if it doesn't, it answers 421 with the nodes that do, and the client asks one itself, with keys for it,
//! rather than the node passing it on. A node asked by what doesn't pick its node (this machine's own programs, a
//! auth, the Cloudflare node's relay) passes a call on to the devices that run it. Calls go to the devices in run
//! priority's order (`src/rank.rs`).

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// A call to run: an app's tool, or a service's function.
#[derive(Clone, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallRequest {
    /// Names this call wherever it's tried, so a call that writes runs on one device only: the first to take its lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The app, or for a service's call, the service.
    #[serde(default)]
    pub app: String,
    /// The service whose function this is, when it is one: an app and the service it wraps may share a name.
    #[serde(default)]
    pub service: Option<String>,
    /// The tool it runs, for an app's call.
    #[serde(default)]
    pub tool: String,
    /// The function it runs, for a service's call.
    #[serde(default)]
    pub function: String,
    #[serde(default)]
    pub input: Value,
    /// The scopes the call touches, named before it runs; by default every scope it may reach.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// The device to run it on, by name or key prefix; by default the nearest that can.
    #[serde(default)]
    pub device: Option<String>,
    /// The agent that asked (an MCP client's name), credited on the call.
    #[serde(default)]
    pub agent: Option<String>,
    /// How many app-to-app calls deep it is: 0 when a user, an agent or a schedule made it.
    #[serde(default)]
    #[schemars(skip)]
    pub depth: u32,
    /// The app whose `ctx.apps` made it, if one did.
    #[serde(default)]
    #[schemars(skip)]
    pub from: Option<String>,
    /// What made it, for its app's insights: `cli`, `mcp`, `mac`, `phone`, `browser`, `schedule` or `app`.
    #[serde(default)]
    pub client: Option<String>,
    /// Files put on this device for this call alone (an app's `_http` body), which go with it to the device that runs
    /// it (`Node::carried`): never a client's to name.
    #[serde(skip)]
    pub puts: Vec<String>,
    /// This node named the call (`Node::identified`), no client did.
    #[serde(skip)]
    pub named_here: bool,
    /// It runs where it was named, first, so no other device has it or the id: it takes no lease.
    #[serde(skip)]
    pub unleased: bool,
}

impl CallRequest {
    /// What it runs: its service's function, or its app's tool.
    pub fn runs(&self) -> &str {
        if self.service.is_some() { &self.function } else { &self.tool }
    }

    /// `fields` of the call with its app and what it runs, under the name its kind has: how a call goes between devices
    /// and into the trace.
    pub(super) fn named(&self, mut fields: Value) -> Value {
        fields["app"] = json!(self.app);
        fields[if self.service.is_some() { "function" } else { "tool" }] = json!(self.runs());
        fields
    }
}

pub(super) enum Reply {
    Ok(Value),
    /// It failed, saying why and, for the kinds callers act on, which kind (`protocol::code`).
    Error(String, Option<String>),
    /// It can't run here; try another device.
    Elsewhere(String),
    /// It lacks keys its caller hands: said as the API says it (`{ error, need, to, seal }`), so the device that passed
    /// it on can ask for them in turn (428).
    Needs(Needs),
}

impl Reply {
    pub(super) fn error(why: impl Into<String>) -> Reply {
        Reply::Error(why.into(), None)
    }

    pub(super) fn failed(err: &anyhow::Error) -> Reply {
        Reply::Error(format!("{err:#}"), crate::protocol::Coded::of(err))
    }

    pub(super) fn json(self) -> Value {
        match self {
            Reply::Ok(v) => json!({ "ok": v }),
            Reply::Error(e, None) => json!({ "error": e }),
            Reply::Error(e, Some(code)) => json!({ "error": e, "code": code }),
            Reply::Elsewhere(w) => json!({ "elsewhere": w }),
            Reply::Needs(n) => n.json(),
        }
    }

    pub(super) fn from_json(v: Value) -> Reply {
        if let Some(w) = v.get("elsewhere") {
            return Reply::Elsewhere(w.as_str().unwrap_or("").into());
        }
        if let Some(needs) = Needs::from_json(&v) {
            return Reply::Needs(needs);
        }
        match v.get("error") {
            Some(e) => Reply::Error(e.as_str().unwrap_or("failed").into(), v["code"].as_str().map(str::to_string)),
            None => Reply::Ok(v["ok"].clone()),
        }
    }
}

/// What a running call may do through the node: whose it is, which app's, and which scopes it reaches.
#[derive(Clone, Debug)]
pub struct Grant {
    pub app: String,
    pub caller: Caller,
    pub scopes: Vec<Scope>,
    pub read_only: bool,
    /// The blobs it kept, which it may read back before any entry names them.
    pub puts: Vec<String>,
    /// The scopes whose databases it has brought up to date, which it reads as they are from then on.
    pub caught_up: Vec<Scope>,
    pub depth: u32,
    /// The app whose `ctx.apps` made the call, if one did.
    pub from: Option<String>,
    /// The trace of the request that made the call, if it asked for one: what its app's reads add to.
    pub trace: Option<trace::Trace>,
}

/// Holds a grant for as long as its call runs.
pub(super) struct Granted(Node, String);

impl Drop for Granted {
    fn drop(&mut self) {
        self.0.0.grants.lock().unwrap().remove(&self.1);
    }
}

mod accept;
mod record;
mod running;
mod writes;


use record::Guard;

/// A service's call may take longer, as a browser or a desktop starts: as long as a service is waited for.
const SERVICE_LIMIT: Duration = Duration::from_secs(600);

/// Whether a device has no room for more: its memory, or its CPU of late, over 90% used.
fn full(load: &crate::load::Load) -> bool {
    let busy = load.cpu.iter().rev().take(3).map(|c| *c as u32).sum::<u32>() / 3;
    load.memory.is_some_and(|m| m.total > 0 && m.used * 10 > m.total * 9) || busy > 90
}

/// How much longer a call may take for each MiB put for it (a git push's pack), which it reads and may keep in the bucket.
const PER_MIB: Duration = Duration::from_secs(1);
/// How long an app that looped stays stopped on a device, unless it's deployed again sooner.
const STOPPED_FOR: u64 = 15 * 60_000;
/// How long a call's work may carry on after it returns (`ctx.waitUntil`).
const LATER_LIMIT: Duration = Duration::from_secs(600);
/// How many of an app's failures a device records an hour; the rest are only counted on the next hour's.
const FAILURES_KEPT: u32 = 30;
/// How many app-to-app calls deep a call may be: a user's call of A, A's of B, B's of C, C's of D.
pub const MAX_DEPTH: u32 = 3;
/// How many calls an app may make of other apps, or runs of its schedules, a minute on one device.
pub const RATE_LIMIT: u32 = 600;

#[derive(Default)]
pub(super) struct State {
    /// What each app did in its last minute, and the apps stopped here for looping.
    guard: Mutex<Guard>,
    /// The calls that returned with work still running (`ctx.waitUntil`), by token: told when it's done.
    later: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Who used which app today from which client, as recorded here: (app, user, client, day).
    seen: Mutex<std::collections::HashSet<(String, String, String, u64)>>,
    /// How many failures of each app were recorded here each hour.
    failures: Mutex<HashMap<(String, u64), u32>>,
}

/// How long an app's prompt hook may take before it's left out: the agent's harness waits for them all.
const PROMPT_LIMIT: Duration = Duration::from_secs(2);
/// How many times a call that writes is tried with one id, each after the one before failed having written nothing.
const LEASE_TRIES: u32 = 10;
/// The most a call passed on to another device takes of the files put for it: under `net::MESSAGE_LIMIT` once in base64.
const CARRIED: usize = 40 << 20;

fn stopped(limit: Duration) -> String {
    format!("it took longer than {}s and was stopped, having written nothing", limit.as_secs())
}

/// Runs `work` until `deadline`, when it's stopped (None) unless it's writing by then, which it's left to finish.
async fn unless_writing<T>(deadline: tokio::time::Instant, writing: &AtomicBool, work: impl Future<Output = T>) -> Option<T> {
    let mut work = std::pin::pin!(work);
    tokio::select! {
        done = &mut work => return Some(done),
        _ = tokio::time::sleep_until(deadline) => {}
    }
    if writing.load(Ordering::Relaxed) { Some(work.await) } else { None }
}

/// A call a client device picked this node for that this node can't run: the API answers 421 with the nodes that can,
/// in the order to ask them (`{ error, elsewhere }`), which the client asks itself, handing each its own keys.
#[derive(Debug)]
pub struct Elsewhere {
    pub why: String,
    pub nodes: Vec<String>,
}

impl std::fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.why)
    }
}

impl std::error::Error for Elsewhere {}

impl Node {
    /// Runs a tool for this device's local user: here, or on the nearest device that can.
    pub async fn call(&self, req: CallRequest) -> Result<Value> {
        self.call_as(req, Caller::local(self)?).await
    }

    /// Runs a tool for `me`, whom this device acts for: here, or on the nearest device that can.
    pub async fn call_as(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let me = Caller { agent: req.agent.clone().or(me.agent), ..me };
        self.route(req, me).await
    }

    /// Runs a call for a client device that picks the node it asks itself (`x-one-elsewhere`), as `me`: here if this
    /// node runs every scope it reaches, whatever the order things run in would pick, as the client raced its way here.
    /// One it can't run (it doesn't run them all, or isn't ready for that app) is `Elsewhere`, naming those that can,
    /// which the client asks itself with keys for each, rather than this node passing it on. A service's call, which
    /// runs where its service is, and one for a named device, go where `route` sends them.
    pub async fn call_picked(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let me = Caller { agent: req.agent.clone().or(me.agent), ..me };
        let req = self.identified(req)?;
        // The Cloudflare node answers a device of the caller's that asks it itself, as no node does: this client is told so.
        if let Some(cloudflare) = self.names_cloudflare(&req) {
            let why = self.cloudflare_is_asked_directly();
            trace::note(req.named(json!({ "step": "device", "device": self.device_name(), "result": "elsewhere", "here": true, "why": why })), Duration::ZERO);
            return Err(Elsewhere { why, nodes: vec![hex(&cloudflare)] }.into());
        }
        if req.service.is_some() || req.device.is_some() {
            return self.route(req, me).await;
        }
        let req = self.reaching(req, &me)?;
        let devices = self.candidates(&req, &me)?;
        if devices.is_empty() {
            bail!(self.no_device(&req.app, &me).await);
        }
        let others: Vec<String> = devices.iter().filter(|k| **k != self.0.id).map(hex).collect();
        let started = Instant::now();
        let name = self.device_name();
        let tried = |result: &str, why: Option<&str>| trace::note(req.named(json!({ "step": "device", "device": name, "result": result, "here": true, "why": why })), started.elapsed());
        if !devices.contains(&self.0.id) {
            let why = format!("{name} doesn't run everything {} reaches for you", req.app);
            tried("elsewhere", Some(&why));
            return Err(Elsewhere { why, nodes: others }.into());
        }
        match self.run_or_need(&req, &me).await {
            Ok(Reply::Ok(v)) => {
                tried("ran", None);
                Ok(v)
            }
            Ok(Reply::Error(e, code)) => {
                tried("failed", None);
                return Err(crate::protocol::Coded::or_plain(code, e))
            }
            Ok(Reply::Elsewhere(why)) if !others.is_empty() => {
                tried("elsewhere", Some(&why));
                Err(Elsewhere { why, nodes: others }.into())
            }
            Ok(Reply::Elsewhere(why)) => bail!("None of your devices can do this now. Try again soon. ({name}: {why})"),
            Ok(Reply::Needs(need)) | Err(need) => {
                tried("needs keys", None);
                Err(need.into())
            }
        }
    }

    /// A call that names no scopes reaches its caller's own too, when its app is on everyone's or a group's page: if no
    /// device runs their own (they haven't picked one) but one runs the rest, it runs on the rest, with no ctx.user.
    fn reaching(&self, mut req: CallRequest, me: &Caller) -> Result<CallRequest> {
        if req.scopes.is_none() && req.service.is_none() {
            req.scopes = self.with(|i| {
                let all = call_scopes(i, &req, &me.user)?;
                let own = Scope::User(me.user.clone());
                let rest: Vec<Scope> = all.iter().filter(|s| **s != own).cloned().collect();
                let m = i.r.db();
                let without = rest.len() < all.len() && !rest.is_empty() && m.runners(&me.user, &all)?.is_empty() && !m.runners(&me.user, &rest)?.is_empty();
                Ok(without.then(|| rest.iter().map(ToString::to_string).collect()))
            })?;
        }
        Ok(req)
    }

    /// A call as it runs: its app's or service's (`resolve`), with its id, the client's own (so a call it sends again
    /// runs once, as it names its lease in the bucket), or a new one.
    fn identified(&self, req: CallRequest) -> Result<CallRequest> {
        let mut req = self.resolve(req);
        ensure!(if req.service.is_some() { req.tool.is_empty() } else { req.function.is_empty() }, "a call to an app names a tool, and one to a service a function");
        req.named_here = req.id.is_none();
        let id = req.id.get_or_insert_with(new_id);
        ensure!(id.len() <= 64 && !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "a call's id is up to 64 letters, digits, - and _");
        Ok(req)
    }

    /// Tries the devices that might run a call, nearest first, one at a time, the next only if one can't be reached
    /// or says to go elsewhere.
    pub(super) async fn route(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let req = self.identified(req)?;
        if req.device.as_deref() == Some("all") {
            return self.everywhere(req, me).await;
        }
        let req = self.reaching(req, &me)?;
        let mut why = Vec::new();
        for device in self.candidates(&req, &me)? {
            let started = Instant::now();
            let name = self.name_of(&device);
            // Each device tried, for a trace: how long it took, and why the next was tried.
            let tried = |result: &str, more: Value| {
                let mut step = req.named(json!({ "step": "device", "device": name, "result": result }));
                if let (Value::Object(step), Value::Object(more)) = (&mut step, more) {
                    step.extend(more);
                }
                trace::note(step, started.elapsed());
            };
            let reply = match device == self.0.id {
                // Keys it lacks are its caller's to hand this device, which they were sealed to: no other runs it.
                // A call this node named, that no device has tried before, has no other to take its lease first: none is.
                true => match self.run_or_need(&CallRequest { unleased: req.named_here && why.is_empty(), ..req.clone() }, &me).await {
                    Ok(reply) => Ok((reply, None)),
                    Err(need) => {
                        tried("needs keys", json!({ "here": true }));
                        return Err(need.into());
                    }
                },
                false => self.ask_device(device, &req, &me).await,
            };
            let here = device == self.0.id;
            match reply {
                Ok((Reply::Ok(v), theirs)) => {
                    tried("ran", json!({ "here": here, "trace": theirs }));
                    self.with_or(None, |i| Ok(i.nearest.insert(device, started.elapsed())));
                    return Ok(v);
                }
                Ok((Reply::Error(e, code), theirs)) => {
                    tried("failed", json!({ "here": here, "trace": theirs }));
                    return Err(crate::protocol::Coded::or_plain(code, e))
                }
                Ok((Reply::Elsewhere(w), theirs)) => {
                    tried("elsewhere", json!({ "here": here, "why": w, "trace": theirs }));
                    why.push(format!("{name}: {w}"))
                }
                // The keys it lacks are its caller's to hand this device, which passes on to it those of the scopes
                // it runs: asked for as this device's own need (428), as if it ran it.
                Ok((Reply::Needs(need), theirs)) => {
                    tried("needs keys", json!({ "here": here, "trace": theirs }));
                    return Err(self.need(need.need).into());
                }
                Err(err) => {
                    tried(if err.is::<net::NotThere>() { "unreachable" } else if err.to_string() == "timed out" { "no answer" } else { "failed" }, json!({}));
                    self.with_or(None, |i| Ok(i.nearest.insert(device, Duration::MAX)));
                    why.push(format!("{name}: {err:#}"));
                }
            }
        }
        if req.service.is_some()
            && let Some(problem) = self.service_problem(&req.app, &me).await
        {
            bail!("{problem}");
        }
        if why.is_empty() {
            bail!(self.no_device(&req.app, &me).await);
        }
        bail!("None of your devices can do this now. Try again soon. ({})", why.join("; "))
    }

    /// No device of `me`'s runs `app`, in words: not on any yet, with what it needs and how to add it, or on one that's
    /// starting (`restarting`: asking again in a moment answers).
    async fn no_device(&self, app: &str, me: &Caller) -> crate::protocol::Coded {
        let title = self.title(app).await;
        let info = self.catalog(me, false).await.ok().and_then(|c| c.into_iter().find(|a| a["name"] == app));
        let starting = info.as_ref().is_some_and(|a| a["runs_on"].as_array().is_some_and(|r| r.iter().any(|r| r["online"] == true)));
        let text = match &info {
            _ if starting => format!("{title} is starting: try again in a moment."),
            Some(a) => format!("{title} isn't on any of your devices yet.{} add {app} adds it, on a node you choose (apps_devices {app} lists them).", a["needs"].as_str().map(|n| format!(" {n}.")).unwrap_or_default()),
            None => format!("{title} isn't on any of your devices yet."),
        };
        crate::protocol::Coded::new(crate::protocol::code::RESTARTING, text)
    }

    /// Why `app`'s service runs nowhere for `me` though it was put on a node, as its problem box in the App Store says
    /// it: that node is off, was removed, or said what went wrong.
    async fn service_problem(&self, app: &str, me: &Caller) -> Option<String> {
        let title = self.title(app).await;
        let info = self.catalog(me, false).await.ok()?.into_iter().find(|a| a["name"] == app)?;
        let runs: Vec<Value> = info["runs_on"].as_array().cloned().unwrap_or_default();
        let added = info["added"].as_array().is_some_and(|a| !a.is_empty());
        if !added || runs.iter().any(|r| r["name"].is_string() && r["online"] == true && r["problem"].is_null()) {
            return None;
        }
        if runs.is_empty() && info["nodes"].as_array().is_some_and(|n| n.is_empty()) {
            return Some(format!("{title} isn't on any of your devices yet.{}", info["needs"].as_str().map(|n| format!(" {n}.")).unwrap_or_default()));
        }
        let r = runs.first().unwrap_or(&Value::Null);
        let why = match (r["name"].as_str(), r["online"] == true, r["problem"].as_str()) {
            _ if runs.is_empty() => "it runs on none".to_string(),
            (None, ..) => "its device was removed".to_string(),
            (Some(node), false, _) => format!("{node} is off"),
            (Some(node), true, problem) => format!("on {node}, {}", problem.unwrap_or("it isn't running")),
        };
        Some(format!("{title} needs a device that's on: {why}. Turn it on, or move it with apps_move {app} <node> (apps_devices {app} lists them)."))
    }

    /// Runs a service's call on every device that runs it for the caller, one at a time: `{ <device>: answer }`, or
    /// `{ error }` for one that failed. Only for services: an app's call runs once.
    pub(super) async fn everywhere(&self, req: CallRequest, me: Caller) -> Result<Value> {
        ensure!(req.service.is_some(), "{} is an app: its calls run once, on one device", req.app);
        let req = CallRequest { device: None, ..req };
        let mut out = serde_json::Map::new();
        for device in self.candidates(&req, &me)? {
            let reply = match device == self.0.id {
                true => Ok(self.run(&req, &me).await),
                false => self.ask_device(device, &req, &me).await.map(|(reply, _)| reply),
            };
            let answer = match reply {
                Ok(Reply::Ok(v)) => v,
                Ok(Reply::Error(e, _) | Reply::Elsewhere(e)) => json!({ "error": e }),
                Ok(Reply::Needs(n)) => json!({ "error": n.to_string() }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            };
            out.insert(self.name_of(&device), answer);
        }
        if out.is_empty() {
            bail!(self.no_device(&req.app, &me).await);
        }
        Ok(Value::Object(out))
    }

    /// The Cloudflare node's key, if the call names it (`--device cloudflare`).
    fn names_cloudflare(&self, req: &CallRequest) -> Option<Key> {
        let found = self.with(|i| i.r.db().find_runner(req.device.as_deref().unwrap_or_default())).ok()?;
        self.is_cloudflare_node(&found).then_some(found)
    }

    fn cloudflare_is_asked_directly(&self) -> String {
        format!("{} is asked by a device of yours itself, not through {}, which runs a node: from the web app, a phone, or the command line of a computer that runs no node of its own (one <command> --device {}). From this one, leave --device off and the nearest device that can runs it", crate::rank::CLOUDFLARE, self.device_name(), crate::rank::CLOUDFLARE)
    }

    /// The devices that might run a call, in the order it tries them: those that are up with room first; then the
    /// near before the far (FAR); then by the network's order (src/rank.rs); this device before another as good; then
    /// the nearest. A service about its own machine runs here first.
    pub(super) fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        let passed = passed(me);
        if let Some(device) = &req.device {
            // A service's call goes to the service offering it on that device, if one serves the caller: their own
            // first (never for a call another node passed on), then the device itself, then one its user shares
            // with everyone.
            if req.service.is_some() {
                let own = (!passed).then(|| self.find_service(&req.app, device, &me.user, true)).flatten();
                let found = own.or_else(|| self.find_device(device).ok()).or_else(|| self.find_service(&req.app, device, &me.user, false));
                if let Some(found) = found {
                    return Ok(vec![found]);
                }
            }
            // A client's log is read through a node it's connected to, which only that node knows: each is tried.
            if req.app == scopes::APP && self.with(|i| Ok(i.r.db().find_machine(device).and_then(|k| Ok(i.r.db().client(&k)?.is_some())).unwrap_or(false)))? {
                let mut nodes = self.with(|i| Ok(i.r.db().computes()?.into_iter().map(|m| m.key).filter(|k| *k == self.0.id || i.heard(k)).collect::<Vec<_>>()))?;
                nodes.sort_by_key(|k| *k != self.0.id);
                return Ok(nodes);
            }
            ensure!(self.names_cloudflare(req).is_none(), "{}", self.cloudflare_is_asked_directly());
            return Ok(vec![self.with(|i| i.r.db().find_runner(device))?]);
        }
        // A function of a service that's about its own machine (`prefer: { run: "here" }`: Shell's run, Models' here) runs on
        // the caller's first.
        let about_here = |d: &Value| d["prefer"][req.runs()] == "here";
        // This device: its node, or a part of it (a service endpoint of oned's own).
        let ours = |i: &Network, k: &Key| -> Result<bool> { Ok(*k == self.0.id || i.r.db().machine(k)?.is_some_and(|m| m.device == Some(self.0.id))) };
        let here_first = req.service.is_some()
            && self.with(|i| {
                let mut part = false;
                for (k, p) in i.peers.iter() {
                    if ours(i, k)? && i.heard(k) && p.services.iter().any(|d| d.0 == req.app && about_here(&d.2)) {
                        part = true;
                    }
                }
                Ok(part)
            })?;
        self.with(|i| {
            let m = i.r.db();
            let mut devices: Vec<Key> = if req.service.is_none() {
                // Not one whose user switched its app service off.
                let mut on = Vec::new();
                for s in m.runners(&me.user, &call_scopes(i, req, &me.user)?)? {
                    if !Self::off(m, &s.key, scopes::APPS_PART)? {
                        on.push(s.key);
                    }
                }
                on
            } else {
                i.peers.iter().filter(|(k, _)| i.heard(k)).filter(|(_, s)| s.services.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect()
            };
            let set = rank::set(&i.r.db().settings()?);
            let here_load = crate::load::current();
            // A service with an endpoint of its own is where its device is.
            let mut at = BTreeMap::new();
            for k in &devices {
                at.insert(*k, i.r.db().service(k)?.and_then(|s| s.device).unwrap_or(*k));
            }
            let mut ours_now = BTreeMap::new();
            for k in &devices {
                ours_now.insert(*k, ours(i, k)?);
            }
            let rank = |k: &Key| {
                let here = *k == self.0.id;
                let load = if here { Some(&here_load) } else { i.peers.get(k).and_then(|p| p.load.as_ref()) };
                let far = !here && i.far.get(k).is_some_and(|d| *d > FAR);
                (!here && !i.heard(k), here_first && !ours_now[k], load.is_some_and(full), far, rank::priority(&set, &hex(&at[k])), !here, i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)))
            };
            devices.sort_by_cached_key(rank);
            Ok(devices)
        })
    }

    /// A call as the node runs it: one that names a service is that service's, whatever app shares its name; so is one
    /// that names an app with no deploy, or that runs on every device.
    pub(super) fn resolve(&self, mut req: CallRequest) -> CallRequest {
        if let Some(service) = &req.service {
            req.app = service.clone();
        } else if req.app != scopes::APP && (!self.deployed(&req.app) || req.device.as_deref() == Some("all")) {
            req.service = Some(req.app.clone());
        }
        if req.service.is_some() && req.function.is_empty() {
            req.function = std::mem::take(&mut req.tool);
        }
        req
    }

    /// Asks another device to run a call: a sign-in's with the keys its caller handed this device, sealed to that one.
    /// → its reply, and what it did, if this request is being traced.
    pub(super) async fn ask_device(&self, device: Key, req: &CallRequest, me: &Caller) -> Result<(Reply, Option<Value>)> {
        // A sign-in's token goes along, with the keys it was handed: to a service only if it serves everyone, which
        // checks it itself, and never keys. A call handed keys with no sign-in passes on its keys alone (below), and
        // app schedules' calls pass on none.
        let service = self.is_service(&device);
        let shared = self.with(|i| Ok(i.r.db().service(&device)?.is_some_and(|s| s.shared)))?;
        let token = match &me.credential {
            Credential::SignIn(token) if !service || shared => Some(token),
            _ => None,
        };
        // A node listed as the caller's own client asks as itself: always for a call it runs with its own keys (one
        // another device of its user passed here as itself, whose key the receiving device would refuse from this
        // connection), and a direct client's call only for a service — never one another node passed on: forwarding
        // another client's key, or app database keys, would make the receiving device refuse its own user's call.
        let as_own_client = match &me.credential {
            Credential::Own => true,
            Credential::Client(key) => req.service.is_some() && *key == me.key,
            _ => false,
        } && self.with(|i| Ok(i.r.db().client(&self.0.id)?.is_some_and(|c| c.user == me.user)))?;
        let key = if as_own_client { self.0.id } else { me.key };
        let mut request = req.named(json!({ "service": req.service, "device": req.device, "input": req.input, "scopes": req.scopes, "depth": req.depth, "from": req.from, "client": req.client, "me": { "key": hex(&key), "user": me.user, "agent": me.agent, "token": token } }));
        // Keys handed here with no sign-in (a client device's, or with a call passed on) go on to another node sealed
        // to it for this node's key, of the scopes the call reaches alone, which it runs on them alone: only those the
        // device runs, and only the call's app's.
        let resolved = self.resolve(req.clone());
        if matches!(me.credential, Credential::Client(_) | Credential::Passed { .. })
            && !service
            && !as_own_client
        {
            let scopes = self.with(|i| call_scopes(i, &resolved, &me.user))?;
            let runs = self.with(|i| Ok(scopes.iter().filter(|s| i.r.db().runs_for(&device, s)).cloned().collect::<Vec<_>>()))?;
            let (keys, until) = self.passing_on(&me.credential, &runs, &resolved.app);
            let seal = self.with(|i| Ok(i.r.db().seal_key(&device)?))?.context("that device has no key to seal to")?;
            let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
            request = crate::keys::forwarded(&request, &me.user, req.client.as_deref().unwrap_or(""), &self.0.id, &seal, until, keys, &named)?["body"]["call"].take();
            request["me"]["key"] = json!(hex(&self.0.id));
            request["me"]["agent"] = json!(me.agent);
        }
        // A sign-in's keys go on only of the scopes the call reaches that the device runs.
        if token.is_some()
            && !service
            && let Some(seal) = self.with(|i| Ok(i.r.db().seal_key(&device)?))?
            && let Some(keys) = self.with(|i| Ok(call_scopes(i, &resolved, &me.user)?.into_iter().filter(|s| i.r.db().runs_for(&device, s)).collect::<Vec<_>>())).map(|scopes| self.hand_on(&me.credential, &seal, &scopes, &resolved.app))?
        {
            request["keys"] = json!(keys);
        }
        let carried = self.carried(req)?;
        if !carried.is_empty() {
            request["blobs"] = Value::Object(carried);
        }
        // A service endpoint fills in nothing itself: an app's call to it is filled here, as one run here is.
        let used = match service {
            true => self.fill_for_service(&resolved.app, &mut request["input"], me, req.from.as_deref()).await?,
            false => Vec::new(),
        };
        let mut reply = self.ask(device, request, self.limit(req).await + Duration::from_secs(5)).await?;
        if let Some(ok) = reply.get_mut("ok") {
            crate::vault::blot(ok, &used);
        }
        // What a service endpoint kept for the call (on its device's oned), its caller's own scope keeps, as for one
        // run here.
        let kept: Vec<String> = match service {
            true => reply.get_mut("blobs").map(Value::take).and_then(|b| serde_json::from_value(b).ok()).unwrap_or_default(),
            false => Vec::new(),
        };
        if !kept.is_empty() && reply.get("ok").is_some() {
            let user = me.user.clone();
            self.append_as(&Scope::User(user.clone()), &user, content::keep(kept)).await?;
        }
        let theirs = reply.get_mut("trace").map(Value::take).filter(Value::is_object);
        Ok((Reply::from_json(reply), theirs))
    }

    /// The files put here for a call that go with it to the device that runs it (`CallRequest::puts`), as
    /// `{ <hash>: <base64> }`: up to `CARRIED` in all, since they go in the one message that asks it.
    fn carried(&self, req: &CallRequest) -> Result<serde_json::Map<String, Value>> {
        let mut out = serde_json::Map::new();
        let mut size = 0;
        for hash in &req.puts {
            let bytes = std::fs::read(release::blob_path(&self.0.dir, hash)?).with_context(|| format!("the file put for that call isn't on {}", self.device_name()))?;
            size += bytes.len();
            ensure!(size <= CARRIED, "that's {} MiB, and a call another device runs takes at most {} MiB with it: send it from a device that runs it first in run priority", size >> 20, CARRIED >> 20);
            out.insert(hash.clone(), json!(data_encoding::BASE64.encode(&bytes)));
        }
        Ok(out)
    }

    /// Keeps the files a call another device passed on came with (`carried`), as its caller's: they read them back
    /// while it runs, and they're deleted once it's answered unless something keeps them. → their hashes.
    fn take_carried(&self, me: &Caller, request: &Value) -> Result<Vec<String>> {
        let Some(blobs) = request["blobs"].as_object() else { return Ok(Vec::new()) };
        let mut puts = Vec::new();
        for (hash, data) in blobs {
            let bytes = data_encoding::BASE64.decode(data.as_str().context("a file a call came with is base64")?.as_bytes())?;
            ensure!(self.put_blob(&bytes)? == *hash, "a file a call came with isn't the one it names");
            self.0.blobs.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(me.user.clone());
            puts.push(hash.clone());
        }
        Ok(puts)
    }

    /// Runs a call here, for `me`, or says why it can't, or what keys it lacks.
    pub(super) async fn run(&self, req: &CallRequest, me: &Caller) -> Reply {
        self.run_or_need(req, me).await.unwrap_or_else(Reply::Needs)
    }

    /// `run`, but a call that lacks keys its caller can hand fails with what it needs. A call is stopped at its limit
    /// only before it writes: one that's writing is waited for, and answers as it went.
    pub(super) async fn run_or_need(&self, req: &CallRequest, me: &Caller) -> Result<Reply, sessions::Needs> {
        let req = &self.resolve(req.clone());
        let limit = self.limit(req).await;
        let writing = AtomicBool::new(false);
        let result = unless_writing(tokio::time::Instant::now() + limit, &writing, self.run_here(req, me, limit, &writing)).await;
        Ok(match result {
            Some(Ok(reply)) => reply,
            Some(Err(err)) => match err.downcast::<sessions::Needs>() {
                Ok(need) => return Err(need),
                Err(err) => Reply::failed(&err),
            },
            None => Reply::error(stopped(limit)),
        })
    }

    /// How long a call may run: an app's, CALL_LIMIT; a service's, and that of an app that uses services or other apps
    /// (which wait on theirs, and may be calls of services), SERVICE_LIMIT; and a second more for each MiB put for it
    /// (`PER_MIB`), which it reads and may keep.
    pub(super) async fn limit(&self, req: &CallRequest) -> Duration {
        let (apps, services) = self.uses_here(&req.app).await;
        let limit = if req.service.is_some() || !apps.is_empty() || !services.is_empty() { SERVICE_LIMIT } else { CALL_LIMIT };
        let put: u64 = req.puts.iter().filter_map(|h| std::fs::metadata(release::blob_path(&self.0.dir, h).ok()?).ok()).map(|m| m.len()).sum();
        limit + PER_MIB * put.div_ceil(1 << 20) as u32
    }

    pub(super) async fn run_here(&self, req: &CallRequest, me: &Caller, limit: Duration, writing: &AtomicBool) -> Result<Reply> {
        // Just under the call's own limit, so a call that's stopped is recorded first.
        let deadline = tokio::time::Instant::now() + limit.saturating_sub(Duration::from_secs(1));
        let input = if req.input.is_null() { json!({}) } else { req.input.clone() };
        let reading = Instant::now();
        self.read_callers_scopes(me).await?;
        if reading.elapsed() >= Duration::from_millis(1) {
            trace::note(json!({ "step": "logs" }), reading.elapsed());
        }
        if req.app == scopes::APP {
            return self.read_logs(req, &input, me).await;
        }
        if req.service.is_some() {
            return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
        }
        if !self.runs_apps() {
            return Ok(Reply::Elsewhere(format!("{} doesn't run apps: its user switched that off", self.device_name())));
        }
        // Before it's read every scope it holds (just taken over from an update's node, say), what's added where reads as
        // nothing: another device runs it.
        if !self.ready() {
            return Ok(Reply::Elsewhere("it isn't ready yet".into()));
        }
        let title = self.title(&req.app).await;
        let prepared = self.with(|i| prepare(self, i, req, me, &title));
        let (ctx, scopes) = match prepared? {
            Ok(p) => p,
            Err(why) => return Ok(Reply::Elsewhere(why)),
        };
        let deployed_here = self.0.bundles.lock().unwrap().get(&req.app).cloned();
        let latest = self.with(|i| Ok(i.r.db().app(&req.app)?.and_then(|a| a.bundle.clone())))?;
        if deployed_here.is_none() || deployed_here != latest {
            return Ok(Reply::Elsewhere(format!("it doesn't have {}'s latest version yet", req.app)));
        }
        if self.deleted(&req.app) {
            return Ok(Reply::error(format!("{title} was deleted: its owners can restore it from Recently Deleted in the App Store")));
        }
        if let Some(why) = self.stopped(&req.app) {
            self.record(&req.app, req, me, Some(&why));
            return Ok(Reply::error(why));
        }
        // A tool of a lab that's off isn't there.
        let described = self.0.apps.describe(&req.app).await?;
        if let Some(lab) = described["tools"].as_array().into_iter().flatten().find(|o| o["name"] == req.tool.as_str()).and_then(|o| o["lab"].as_str())
            && !self.with(|i| Ok(crate::labs::on(&i.r.db().settings()?, &crate::labs::app_key(&req.app, lab))))?
        {
            let lab_title = described["labs"][lab]["title"].as_str().unwrap_or(lab);
            return Ok(Reply::error(format!("{lab_title} is off. An admin can turn it on in Settings › Labs.")));
        }
        // A sign-in's call that lacks a key of a scope it names fails here, before it runs or writes anything: its
        // app's database's, and the Vault's there too if it says it uses items.
        let mut want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), req.app.as_str(), scopes::APP_DATABASE)).collect();
        if self.0.apps.describe(&req.app).await?["vault"].is_object() {
            want.extend(scopes.iter().map(|s| (s.clone(), scopes::APP, scopes::CoreDb::Vault.name())));
        }
        let checking = Instant::now();
        self.needs(me, &want).await?;
        trace::note(json!({ "step": "keys" }), checking.elapsed());
        // A call that writes, tried on another device before this one, ran there if it took the lease first.
        let mut lease = None;
        if let (Some(id), Some(scope), false) = (&req.id, scopes.first(), req.unleased) {
            let reads = self.0.apps.describe(&req.app).await?["tools"].as_array().into_iter().flatten().any(|o| o["name"] == req.tool.as_str() && o["reads"] == true);
            if !reads {
                let base = format!("{}/{}/calls/{id}", crate::bucket::folder(&scope.to_string()), req.app);
                let leasing = Instant::now();
                let taken = self.take_lease(&base).await?;
                trace::note(json!({ "step": "lease" }), leasing.elapsed());
                let Some(taken) = taken else {
                    return Ok(Reply::error("This may already be done: it ran on another device, whose answer was lost. Check before trying again."));
                };
                lease = Some(taken);
            }
        }
        let mut ctx = ctx;
        ctx.from = req.from.clone();
        let granted = self.grant(&req.app, me, &mut ctx, req.depth);
        // What a call writes comes with its answer, and what it keeps is put in the bucket first, all while it may be
        // stopped; then its steps are written whatever happens to whoever's waiting.
        let running = Instant::now();
        let answered = AtomicBool::new(false);
        let ran = unless_writing(deadline, writing, async {
            let ran = self.0.apps.call(&req.app, &req.tool, input, &ctx, deadline.saturating_duration_since(tokio::time::Instant::now())).await;
            answered.store(true, Ordering::Relaxed);
            let mut outcome = ran?;
            trace::note(req.named(json!({ "step": "run" })), running.elapsed());
            let preparing = Instant::now();
            let prepared = self.prepare_appends(&req.app, std::mem::take(&mut outcome.appends), &scopes, me, req.from.as_deref()).await?;
            trace::note(json!({ "step": "prepare" }), preparing.elapsed());
            writing.store(true, Ordering::Relaxed);
            let writing_at = Instant::now();
            let (node, app, scopes, me) = (self.clone(), req.app.clone(), scopes.clone(), me.clone());
            let traced = trace::current();
            tokio::spawn(trace::within(traced, async move { node.write_prepared(&app, prepared, &scopes, &me).await })).await??;
            trace::note(json!({ "step": "write" }), writing_at.elapsed());
            anyhow::Ok(outcome)
        })
        .await
        .unwrap_or_else(|| Err(anyhow!(stopped(limit))));
        let outcome = match ran {
            Ok(outcome) => outcome,
            Err(err) => {
                // The app answered and the call wrote nothing (a call's writes are one transaction): its next try may
                // run. Not when the app was stopped, as it may still be running, nor when it failed writing.
                if let Some(lease) = lease.filter(|_| answered.load(Ordering::Relaxed) && !writing.load(Ordering::Relaxed))
                    && let Err(err) = crate::bucket::Bucket::create(&self.bucket()?, &format!("{lease}.failed"), b"").await
                {
                    tracing::warn!("saying a call failed, so it may run again: {err:#}");
                }
                self.record(&req.app, req, me, Some(&format!("{err:#}")));
                return Err(err);
            }
        };
        self.record(&req.app, req, me, None);
        // Its user using it here renews the app schedules that run it for them, as their other devices do themselves.
        if self.this_machine(me) {
            self.renew_app_schedules(&req.app);
        }
        if outcome.later {
            self.keep_for_later(granted);
        }
        Ok(Reply::Ok(outcome.result))
    }

    /// Takes a write call's lease, the first try's at `base` and each next one's at `base.<n>`, the next only once the
    /// one before says it failed (`<lease>.failed`), having written nothing: → its name, or None when one is taken and
    /// didn't fail, so the call may have run.
    async fn take_lease(&self, base: &str) -> Result<Option<String>> {
        let bucket = self.bucket()?;
        for n in 1..=LEASE_TRIES {
            let lease = if n == 1 { base.to_string() } else { format!("{base}.{n}") };
            if crate::bucket::Bucket::create(&bucket, &lease, hex(&self.0.id).as_bytes()).await? {
                return Ok(Some(lease));
            }
            if crate::bucket::Bucket::get(&bucket, &format!("{lease}.failed")).await?.is_none() {
                return Ok(None);
            }
        }
        Ok(None)
    }

}

pub(super) fn address(m: &ScopeDb) -> Result<Option<String>> {
    Ok(m.setting("address")?.map(|a| a.trim_end_matches('/').to_string()))
}

/// Whether a call is one another node passed on: it runs only on what it was handed, never as this device's own user.
fn passed(me: &Caller) -> bool {
    matches!(me.credential, Credential::Passed { .. })
}

/// The scopes a call reaches for `user`: those it names, or every one its app reaches; a service's, those it names or
/// its user's own.
fn call_scopes(i: &Network, req: &CallRequest, user: &str) -> Result<Vec<Scope>> {
    let named = |s: &String| Scope::parse_for(s, user);
    match (&req.scopes, &req.service) {
        (Some(named_), _) => named_.iter().map(named).collect(),
        (None, Some(_)) => Ok(vec![Scope::User(user.to_string())]),
        (None, None) => reachable(i, &req.app, user),
    }
}

/// Checks a call may run here, and makes its context; or says why it should run elsewhere.
fn prepare(node: &Node, i: &Network, req: &CallRequest, me: &Caller, title: &str) -> Result<std::result::Result<(Context, Vec<Scope>), String>> {
    let m = i.r.db();
    let here = node.0.id;
    if !m.serves(&here, &me.user)? {
        return Ok(Err(format!("{} doesn't serve you", node.device_name())));
    }
    let reach = reachable(i, &req.app, &me.user)?;
    let scopes: Vec<Scope> = match &req.scopes {
        None => reach.clone(),
        Some(named) => {
            let mut out = Vec::new();
            for s in named {
                let s = Scope::parse_for(s, &me.user)?;
                ensure!(reach.contains(&s), "{title} can't reach {} for you", scope_name(i, &s.to_string())?);
                out.push(s);
            }
            out
        }
    };
    if reach.is_empty() {
        bail!(crate::protocol::Coded::new(crate::protocol::code::NOT_ADDED, format!("{title} isn't on your pages: add it first")));
    }
    if let Some(s) = scopes.iter().find(|s| !i.r.holds(s)) {
        return Ok(Err(format!("it can't open {} yet", scope_name(i, &s.to_string())?)));
    }
    let ctx = Context { me: node_me(node, m, me)?, now: now_ms(), scopes: handles(m, &scopes, &me.user)?, read_only: false, token: String::new(), from: None, address: address(m)?, handles: m.handles()?, labs: crate::labs::on_keys(&m.settings()?) };
    Ok(Ok((ctx, scopes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_call_is_stopped_at_its_limit_only_before_it_writes() {
        let ran = |writing: &'static AtomicBool, writes_at: u64| async move {
            tokio::time::sleep(Duration::from_secs(writes_at)).await;
            writing.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(30)).await;
            "wrote"
        };
        let deadline = || tokio::time::Instant::now() + Duration::from_secs(30);
        let before: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        assert_eq!(unless_writing(deadline(), before, ran(before, 40)).await, None);
        assert!(!before.load(Ordering::Relaxed), "it was stopped before it wrote");
        let writing: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        assert_eq!(unless_writing(deadline(), writing, ran(writing, 20)).await, Some("wrote"));
    }
}
