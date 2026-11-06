//! Calls: where each runs, running it here or on another device, what it wrote, and what its app asks the node while it runs.
//!
//! A call runs once. It names the scopes it touches before it runs, so a device that can't run it says so before doing
//! anything; one that writes takes a lease in the bucket first, so failing over never repeats it. It writes nothing if
//! it fails, or is stopped at its limit, which happens only before it writes: what it keeps goes in the bucket first,
//! and once it writes, it's waited for. A node a client picked runs a call itself if it runs every scope the call
//! reaches; if it doesn't, it answers 421 with the nodes that do, and the client asks one itself, with keys for it,
//! rather than the node passing it on. A node asked by what doesn't pick its node (this machine's own programs, a
//! sign-in, the Cloudflare node's relay) passes a call on to the devices that run it. Calls go to the devices in run
//! priority's order (`src/rank.rs`).

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

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
/// How many app-to-app calls deep a call may be: a person's call of A, A's of B, B's of C, C's of D.
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

/// Loop guards: an app that calls other apps, or runs its schedules, more than `RATE_LIMIT` times a minute on this
/// device is stopped here for `STOPPED_FOR`, or until it's deployed again.
#[derive(Default)]
struct Guard {
    /// Each app's calls or runs since the minute it started counting (milliseconds): a minute from its first, not
    /// the clock's, so a loop that runs across the turn of a minute is still caught.
    counts: HashMap<(String, &'static str), (u64, u32)>,
    /// Each app stopped here: why, until when, and the bundle it was running.
    stopped: HashMap<String, (String, u64, Option<String>)>,
}

/// How long an app's prompt hook may take before it's left out: the agent's harness waits for them all.
const PROMPT_LIMIT: Duration = Duration::from_secs(2);
/// How many times a call that writes is tried with one id, each after the one before failed having written nothing.
const LEASE_TRIES: u32 = 10;
/// The most a call passed on to another device takes of the files put for it: under `net::MESSAGE_LIMIT` once in base64.
const CARRIED: usize = 40 << 20;

/// One of a call's statements: its SQL, its parameters, and what it fails with when its check finds nothing.
type Statement = (String, Vec<Value>, Option<String>);

/// A call's appends, checked, with the blobs they keep put in the bucket: what `write_prepared` writes.
pub(super) struct Prepared {
    statements: Vec<(Scope, Vec<Statement>, Vec<String>)>,
    writes: Vec<(Scope, Content)>,
    items: Vec<Append>,
    installs: Vec<Append>,
    /// `<scope> <hash>` of each blob put in the bucket.
    put: Vec<String>,
}

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
        let tried = |result: &str, why: Option<&str>| trace::note(json!({ "step": "device", "device": name, "app": req.app, "op": req.tool, "result": result, "here": true, "why": why }), started.elapsed());
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
                let mut step = json!({ "step": "device", "device": name, "app": req.app, "op": req.tool, "result": result });
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
                    let _ = self.with(|i| Ok(i.nearest.insert(device, started.elapsed())));
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
                    let _ = self.with(|i| Ok(i.nearest.insert(device, Duration::MAX)));
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
            Some(a) => format!("{title} isn't on any of your devices yet.{} apps_add {app} user adds it, on a node you choose (apps_devices {app} lists them).", a["needs"].as_str().map(|n| format!(" {n}.")).unwrap_or_default()),
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
        let added = info["installed"].as_array().is_some_and(|a| !a.is_empty());
        if !added || runs.iter().any(|r| r["name"].is_string() && r["online"] == true && r["problem"].is_null()) {
            return None;
        }
        if runs.is_empty() && info["nodes"].as_array().is_some_and(|n| n.is_empty()) {
            return Some(format!("{title} isn't on any of your devices yet.{}", info["needs"].as_str().map(|n| format!(" {n}.")).unwrap_or_default()));
        }
        let r = runs.first().unwrap_or(&Value::Null);
        let why = match (r["name"].as_str(), r["online"] == true, r["problem"].as_str()) {
            _ if runs.is_empty() => "it runs on none".to_string(),
            (None, ..) => "the node it ran on was removed".to_string(),
            (Some(node), false, _) => format!("{node} is off"),
            (Some(node), true, problem) => format!("on {node}, {}", problem.unwrap_or("it isn't running")),
        };
        Some(format!("{title} needs its service on a node that's on: {why}. Turn it on, or move it with apps_move {app} <node> (apps_devices {app} lists them)."))
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

    /// The devices that might run a call, in the order it tries them: those that are up with room first; then the
    /// near before the far (FAR); then by the network's order (src/rank.rs); this device before another as good; then
    /// the nearest. A service about its own machine runs here first.
    pub(super) fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        let passed = passed(me);
        if let Some(device) = &req.device {
            // A service's call goes to the service offering it on that device, if one serves the caller: their own
            // first (never for a call another node passed on), then the device itself, then one its person shares
            // with everyone.
            if req.service.is_some() {
                let own = (!passed).then(|| self.find_service(&req.app, device, &me.user, true)).flatten();
                let found = own.or_else(|| self.find_device(device).ok()).or_else(|| self.find_service(&req.app, device, &me.user, false));
                if let Some(found) = found {
                    return Ok(vec![found]);
                }
            }
            return Ok(vec![self.find_device(device)?]);
        }
        let here = self.runs_service_here(&req.app);
        // A function of a service that's about its own machine (`prefer: { run: "here" }`: Shell's run, Models' here) runs on
        // the caller's first.
        let about_here = |d: &Value| d["prefer"][req.tool.as_str()] == "here";
        // This device: its node, or a part of it (a service endpoint of oned's own).
        let ours = |i: &Network, k: &Key| -> Result<bool> { Ok(*k == self.0.id || i.r.db().machine(k)?.is_some_and(|m| m.device == Some(self.0.id))) };
        let here_first = req.service.is_some()
            && self.with(|i| {
                let mut part = false;
                for (k, p) in i.peers.iter() {
                    if ours(i, k)? && i.online(k) && p.services.iter().any(|d| d.0 == req.app && about_here(&d.2)) {
                        part = true;
                    }
                }
                Ok(part || (here && self.services_here(i)?.iter().any(|d| d.0 == req.app && about_here(&d.2))))
            })?;
        self.with(|i| {
            let m = i.r.db();
            let mut devices: Vec<Key> = if req.service.is_none() {
                // Not one whose person switched its app service off.
                let mut on = Vec::new();
                for s in m.runners(&me.user, &call_scopes(i, req, &me.user)?)? {
                    if !Self::off(m, &s.key, scopes::APPS_PART)? {
                        on.push(s.key);
                    }
                }
                on
            } else {
                let mut s: Vec<Key> = i.peers.iter().filter(|(k, _)| i.online(k)).filter(|(_, s)| s.services.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect();
                if here && ((me.user == i.user && !passed) || (self.0.services.serves_everyone(&req.app) && i.r.db().serves(&self.0.id, &me.user)?)) {
                    s.push(self.0.id);
                }
                s
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
                (!here && !i.online(k), here_first && !ours_now[k], load.is_some_and(full), far, rank::priority(&set, &hex(&at[k])), !here, i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)))
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
        // another device of its person passed here as itself, whose key the receiving device would refuse from this
        // connection), and a direct client's call only for a service — never one another node passed on: forwarding
        // another client's key, or app database keys, would make the receiving device refuse its own person's call.
        let as_own_client = match &me.credential {
            Credential::Own => true,
            Credential::Client(key) => req.service.is_some() && *key == me.key,
            _ => false,
        } && self.with(|i| Ok(i.r.db().client(&self.0.id)?.is_some_and(|c| c.user == me.user)))?;
        let key = if as_own_client { self.0.id } else { me.key };
        let mut request = json!({ "app": req.app, "service": req.service, "op": req.tool, "input": req.input, "scopes": req.scopes, "depth": req.depth, "from": req.from, "client": req.client, "me": { "key": hex(&key), "user": me.user, "agent": me.agent, "token": token } });
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
            self.append_as(&Scope::User(user.clone()), &user, Content { blobs: kept, ..content::node("keep", json!({})) }).await?;
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
            return self.read_logs(&req.tool, &input, me).map(Reply::Ok);
        }
        if req.service.is_some() {
            if !self.runs_service_here(&req.app) {
                return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
            }
            // A service that runs as this device's own person (Shell, say) never runs for a call another node passed
            // on, from whichever path it came: a hacked node gets only the calls it ran.
            if passed(me) && !self.0.services.serves_everyone(&req.app) {
                return Ok(Reply::error(format!("{} runs only for its person's own devices", req.app)));
            }
            return match self.run_service(&req.app, &req.tool, input, me, req.from.clone()).await {
                Err(err) if err.is::<crate::services::Elsewhere>() => Ok(Reply::Elsewhere(err.to_string())),
                other => other.map(Reply::Ok),
            };
        }
        if !self.runs_apps() {
            return Ok(Reply::Elsewhere(format!("{} doesn't run apps: its person switched that off", self.device_name())));
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
        if let Some(lab) = described["ops"].as_array().into_iter().flatten().find(|o| o["name"] == req.tool.as_str()).and_then(|o| o["lab"].as_str())
            && !self.with(|i| Ok(crate::labs::on(&i.r.db().settings()?, &crate::labs::app_key(&req.app, lab))))?
        {
            let lab_title = described["labs"][lab]["title"].as_str().unwrap_or(lab);
            return Ok(Reply::error(format!("{lab_title} is off. An admin can turn it on in Settings › Labs.")));
        }
        // A sign-in's call that lacks a key of a scope it names fails here, before it runs or writes anything: its
        // app's database's, and the Vault's there too if it says it uses items.
        let mut want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), req.app.as_str(), scopes::APP_DATABASE)).collect();
        if self.0.apps.describe(&req.app).await?["vault"].is_object() {
            want.extend(scopes.iter().map(|s| (s.clone(), scopes::APP, crate::vault::DATABASE)));
        }
        let checking = Instant::now();
        self.needs(me, &want).await?;
        trace::note(json!({ "step": "keys" }), checking.elapsed());
        // A call that writes, tried on another device before this one, ran there if it took the lease first.
        let mut lease = None;
        if let (Some(id), Some(scope), false) = (&req.id, scopes.first(), req.unleased) {
            let reads = self.0.apps.describe(&req.app).await?["ops"].as_array().into_iter().flatten().any(|o| o["name"] == req.tool.as_str() && o["reads"] == true);
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
            trace::note(json!({ "step": "run", "app": req.app, "op": req.tool }), running.elapsed());
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
        // Its person using it renews the app schedules that run it for them.
        if !matches!(me.credential, Credential::AppSchedules { .. }) {
            self.renew_app_schedules(me, &req.app);
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

    /// Records, for an app's owners, a call that ran here: the first each day by each user from each client
    /// (`used`), and every failure (`failed`), at most `FAILURES_KEPT` an hour per app on this device. They go in the
    /// owner's scope when the caller may use it and this device holds it, else in `global`; in the background.
    pub(super) fn record(&self, app: &str, req: &CallRequest, me: &Caller, error: Option<&str>) {
        let now = now_ms();
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let said = match error {
            None => {
                let day = now / 86_400_000;
                if !self.0.calls.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
                    return;
                }
                status::Said::Used { app: app.into(), user: me.user.clone(), client, agent: me.agent.clone(), day }
            }
            Some(error) => {
                let hour = now / 3_600_000;
                let mut failures = self.0.calls.failures.lock().unwrap();
                let n = failures.entry((app.to_string(), hour)).or_default();
                *n += 1;
                if *n > FAILURES_KEPT {
                    return;
                }
                let error: String = error.lines().next().unwrap_or("").chars().take(300).collect();
                status::Said::Failed { app: app.into(), tool: req.tool.clone(), error, user: me.user.clone() }
            }
        };
        self.write_insight(app, me, said);
    }

    /// Writes an insight about `app` for `me`'s call: in the app owner's scope when `me` may use it and this device
    /// holds it, else in `global`; in the background.
    pub(super) fn write_insight(&self, app: &str, me: &Caller, said: status::Said) {
        let Ok(scope) = self.with(|i| {
            let m = i.r.db();
            let owner = m.app(app)?.map(|a| a.owner).context("no such app")?;
            let own = match owner {
                Principal::User(u) => Scope::User(u),
                Principal::Group(g) => Scope::Group(g),
                _ => Scope::Global,
            };
            Ok(if m.may(&me.user, "use", &scopes::object(&own)) && i.r.holds(&own) { own } else { Scope::Global })
        }) else {
            return;
        };
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.say(&scope, said).await {
                tracing::debug!("insights: {err:#}");
            }
        });
    }

    /// What an app's owners see of it, from every scope `viewer` may use here: who used it, when and from what;
    /// its errors grouped, with counts; and what it spent on AI, by model. Only for someone acting for its owner.
    pub(super) fn insights_of(&self, viewer: &Caller, app: &str) -> Result<Option<Value>> {
        self.with(|i| {
            let m = i.r.db();
            let Some(a) = m.app(app)? else { return Ok(None) };
            if !m.may(&viewer.user, "deploy", &scopes::app(&a.name)) {
                return Ok(None);
            }
            let name = |u: &str| user_name(m, u);
            let mut people: BTreeMap<(String, String), (u64, u64, Option<String>)> = BTreeMap::new();
            let mut errors: BTreeMap<(String, String), (u64, u64, u64, String)> = BTreeMap::new();
            let mut cost: BTreeMap<String, (f64, u64, u64, u64)> = BTreeMap::new();
            for (log, status) in &i.status {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = status.insights.get(app).filter(|_| m.may(&viewer.user, "use", &scopes::object(&scope))) else { continue };
                for ((user, client), u) in &ins.used {
                    let p = people.entry((user.clone(), client.clone())).or_default();
                    p.0 += u.days;
                    p.1 = p.1.max(u.last);
                    if u.agent.is_some() {
                        p.2 = u.agent.clone();
                    }
                }
                for (key, e) in &ins.errors {
                    let x = errors.entry(key.clone()).or_insert((0, u64::MAX, 0, String::new()));
                    x.0 += e.count;
                    x.1 = x.1.min(e.first);
                    if e.last >= x.2 {
                        x.2 = e.last;
                        x.3 = name(&e.user)?;
                    }
                }
                for (model, c) in &ins.cost {
                    let x = cost.entry(model.clone()).or_default();
                    x.0 += c.usd;
                    x.1 += c.input;
                    x.2 += c.output;
                    x.3 += c.calls;
                }
            }
            let mut usage: Vec<Value> = Vec::new();
            for ((u, client), (days, last, agent)) in people {
                usage.push(json!({ "user": name(&u)?, "client": client, "agent": agent, "days": days, "last": last }));
            }
            usage.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let mut errors: Vec<Value> = errors.into_iter().map(|((tool, error), (count, first, last, user))| json!({ "op": tool, "error": error, "count": count, "first": first, "last": last, "user": user })).collect();
            errors.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let cost: Vec<Value> = cost.into_iter().map(|(model, (usd, input, output, calls))| json!({ "model": model, "usd": usd, "input": input, "output": output, "calls": calls })).collect();
            Ok(Some(json!({ "usage": usage, "errors": errors, "cost": cost })))
        })
    }

    pub(super) async fn write_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<()> {
        let prepared = self.prepare_appends(app, appends, scopes, me, from).await?;
        self.write_prepared(app, prepared, scopes, me).await
    }

    /// A call's appends, checked, before any is written: a call that fails writes nothing. The blobs its steps keep go
    /// in the bucket now, so what's slow and may fail comes before it writes.
    pub(super) async fn prepare_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<Prepared> {
        // What it spent on AI counts against the app that called it, if one did.
        let (costs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Cost { .. }));
        for cost in costs {
            if let Append::Cost { data } = cost {
                let to = from.unwrap_or(app);
                let said = status::Said::Cost {
                    app: to.into(),
                    model: data["model"].as_str().unwrap_or("?").into(),
                    usd: data["usd"].as_f64().unwrap_or(0.0),
                    input: data["input"].as_u64().unwrap_or(0),
                    output: data["output"].as_u64().unwrap_or(0),
                };
                self.write_insight(to, me, said);
            }
        }
        let (installs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Install { .. }));
        let (execs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Exec { .. }));
        let (items, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Vault { .. }));
        // Each scope's statements, in the order the call ran them, and the blobs they keep.
        let mut statements: Vec<(Scope, Vec<Statement>, Vec<String>)> = Vec::new();
        for exec in execs {
            let Append::Exec { scope, sql, params, blobs, check } = exec else { continue };
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blobs.iter().all(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())), "a blob is named by its hash");
            match statements.iter_mut().find(|(s, _, _)| *s == scope) {
                Some((_, list, kept)) => {
                    list.push((sql, params, check));
                    kept.extend(blobs);
                }
                None => statements.push((scope, vec![(sql, params, check)], blobs)),
            }
        }
        let mut writes = Vec::new();
        for append in appends {
            let title = match &append {
                Append::Switch { app, .. } | Append::Retire { app, .. } | Append::Source { app, .. } => self.title(app).await,
                _ => String::new(),
            };
            writes.push(self.with(|i| check_append(i, append, scopes, me, &title))?);
        }
        let mut kept: Vec<(Scope, String)> = statements.iter().flat_map(|(s, _, kept)| kept.iter().map(|b| (s.clone(), b.clone()))).collect();
        kept.extend(writes.iter().flat_map(|(s, c)| c.blobs.iter().map(|b| (s.clone(), b.clone()))));
        let mut put = Vec::new();
        for (scope, blob) in kept {
            self.share_blob(&scope, &blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
            put.push(format!("{scope} {blob}"));
        }
        Ok(Prepared { statements, writes, items, installs, put })
    }

    /// Writes a call's prepared appends as its caller. Installs come last, since installing a built-in app the network
    /// doesn't have yet adds it first.
    pub(super) async fn write_prepared(&self, app: &str, prepared: Prepared, scopes: &[Scope], me: &Caller) -> Result<()> {
        let Prepared { statements, mut writes, items, installs, put } = prepared;
        // The blobs it put in the bucket aren't put again by the steps that keep them.
        self.0.blobs.put_first.lock().unwrap().extend(put.iter().cloned());
        let wrote = async {
            let wrote = !statements.is_empty();
            for (scope, list, kept) in statements {
                let (db, bucket) = self.call_database(me, &scope, app, scopes::APP_DATABASE, None).await?;
                let mut db = db.lock().await;
                let at = db.at();
                self.write_in(&scope, &mut db, &bucket, |c| crate::database::stored::statements(c, &list)).await?;
                if db.at() > at {
                    self.ping(&scope, Some((app, scopes::APP_DATABASE, db.at())));
                }
                drop(db);
                if !kept.is_empty() {
                    writes.push((scope, Content { blobs: kept, ..content::node("keep", json!({})) }));
                }
            }
            if wrote {
                self.emit(Event::Changed { apps: vec![app.to_string()] });
            }
            for item in items {
                if let Append::Vault { id, scope, data, same, release } = item {
                    self.call_item(app, me, scopes, &id, scope, data, same, release).await?;
                }
            }
            for append in installs {
                if let Append::Install { app, scope, installed, node } = append {
                    let scope = Scope::parse_for(&scope, &me.user)?;
                    writes.extend(self.install_entries(me, &app, &scope, installed, node.as_deref()).await?);
                }
            }
            for (scope, content) in writes {
                self.append_as(&scope, &me.user, content).await?;
            }
            Ok(())
        }
        .await;
        let mut first = self.0.blobs.put_first.lock().unwrap();
        for p in &put {
            first.remove(p);
        }
        wrote
    }

    /// Keeps a call's grant after it returned, while the work it left running (`ctx.waitUntil`) goes on: its requests
    /// still go out as the call's, and it hands in its appends (`POST /appends`) until it says it's done, or for
    /// `LATER_LIMIT` at most.
    pub(super) fn keep_for_later(&self, granted: Granted) {
        let done = Arc::new(tokio::sync::Notify::new());
        self.0.calls.later.lock().unwrap().insert(granted.1.clone(), done.clone());
        let node = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(LATER_LIMIT, done.notified()).await;
            node.0.calls.later.lock().unwrap().remove(&granted.1);
            drop(granted);
        });
    }

    pub(super) async fn run_service(&self, app: &str, function: &str, input: Value, me: &Caller, from: Option<String>) -> Result<Value> {
        let everyone = self.0.services.serves_everyone(app) && self.with(|i| Ok(i.r.db().serves(&self.0.id, &me.user)?))?;
        let user = if everyone { me.user.clone() } else { self.user()? };
        ensure!(me.user == user, "{} runs only for {}'s own user", self.service_title(app), self.device_name());
        let caller = self.me_for(me)?;
        let mut input = input;
        let used = self.fill_for_service(app, &mut input, me, from.as_deref()).await?;
        let mut answer = self.0.services.call(app, function, input, caller.clone(), from).await?;
        crate::vault::blot(&mut answer.ok, &used);
        let scope = Scope::User(user.clone());
        if !answer.blobs.is_empty() {
            self.append_as(&scope, &user, Content { blobs: answer.blobs, ..content::node("keep", json!({})) }).await?;
        }
        Ok(answer.ok)
    }

    /// Fills in an app's call of `app`'s service (`from` the app that made it): the fields of the items it's switched on
    /// for, a secret one only for a service named among its item's hosts. → the secrets it filled, to blot out of the
    /// answer. The node that runs the call fills it, whether the service runs here or as a service endpoint of its own,
    /// which fills nothing itself.
    pub async fn fill_for_service(&self, app: &str, input: &mut Value, me: &Caller, from: Option<&str>) -> Result<Vec<String>> {
        let used = std::sync::Mutex::new(Vec::<String>::new());
        if let Some(from) = from.filter(|_| input.to_string().contains("{{")) {
            let scopes = self.with(|i| Ok(reachable(i, from, &me.user)?.into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>()))?;
            let lookup = self.lookup(me, from, &scopes).await?;
            lookup.refuse_shut(&input.to_string())?;
            crate::vault::fill_value(input, app, &|name| {
                let found = lookup.get(name);
                if let Some((hosts, value)) = &found
                    && !hosts.is_empty()
                {
                    used.lock().unwrap().push(value.clone());
                }
                found
            })?;
        }
        Ok(used.into_inner().unwrap())
    }

    /// Reads this device's log (or one of its services'), for a person over the network (`one logs`): the one tool of the
    /// node's own a call asks.
    pub(super) fn read_logs(&self, tool: &str, input: &Value, me: &Caller) -> Result<Value> {
        ensure!(tool == "logs", "the node has no tool {tool}");
        ensure!(self.may_read_logs(&me.user)?, "only {}'s person, or an admin of {}, reads its logs", self.device_name(), crate::config::YOURS);
        crate::logs::read(&self.0.dir, &crate::logs::Ask::from_json(input)?)
    }

    /// Whether `user` may read this device's logs: whoever may remove it from the network, its person and the network's
    /// admins (who may remove any device), since they could stop it anyway. Nothing in a log is a key or someone's data.
    pub fn may_read_logs(&self, user: &str) -> Result<bool> {
        self.with(|i| Ok(i.r.db().may(user, "remove", &scopes::device(&self.0.id))))
    }

    /// Lets the requests of a call, widget or hook of `app` through the node as `caller`'s, while it runs.
    pub(super) fn grant(&self, app: &str, caller: &Caller, ctx: &mut Context, depth: u32) -> Granted {
        let token = hex(&rand::random::<[u8; 32]>());
        let scopes = ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect();
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new(), caught_up: Vec::new(), depth, from: ctx.from.clone(), trace: trace::current() };
        self.0.grants.lock().unwrap().insert(token.clone(), grant);
        ctx.token = token.clone();
        Granted(self.clone(), token)
    }

    /// Counts one of `app`'s calls of other apps, or one run of its schedules (`kind`). Past `RATE_LIMIT` this minute,
    /// the app is stopped here: it's looping.
    pub(super) async fn count(&self, app: &str, kind: &'static str) -> Result<()> {
        let now = now_ms();
        let tripped = {
            let mut g = self.0.calls.guard.lock().unwrap();
            let (since, n) = g.counts.entry((app.to_string(), kind)).or_insert((now, 0));
            if now.saturating_sub(*since) >= 60_000 {
                (*since, *n) = (now, 0);
            }
            *n += 1;
            *n == RATE_LIMIT + 1
        };
        if tripped {
            let what = if kind == "call" { "called other apps" } else { "ran its schedules" };
            let why = format!("{} {what} more than {RATE_LIMIT} times in a minute on {}, so it was stopped there: is it looping? It runs again once it's deployed again, or in {} minutes.", self.title(app).await, self.device_name(), STOPPED_FOR / 60_000);
            let bundle = self.with_or(None, |i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())));
            self.0.calls.guard.lock().unwrap().stopped.insert(app.to_string(), (why.clone(), now_ms() + STOPPED_FOR, bundle));
            tracing::warn!("{why}");
        }
        match self.stopped(app) {
            Some(why) => bail!("{why}"),
            None => Ok(()),
        }
    }

    /// Why `app` is stopped here, if it is: it looped, and hasn't been deployed again or waited out its time since.
    pub(super) fn stopped(&self, app: &str) -> Option<String> {
        let latest = self.with_or(None, |i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())));
        let mut g = self.0.calls.guard.lock().unwrap();
        let (why, until, bundle) = g.stopped.get(app)?;
        if now_ms() < *until && *bundle == latest {
            return Some(why.clone());
        }
        g.stopped.remove(app);
        None
    }

    /// The ports this node serves on this machine: its local API, workerd, and apps' outbound requests.
    pub fn own_ports(&self) -> Vec<u16> {
        vec![crate::api::port(), self.0.apps.port(), self.0.outbound]
    }

    pub fn grant_of(&self, token: &str) -> Option<Grant> {
        self.0.grants.lock().unwrap().get(token).cloned()
    }

    /// What every app installed for this machine's person says with an agent's prompt here: each app's `prompt` hook,
    /// asked at once, their answers joined. The node remembers, per app and session, when it last asked, and tells
    /// the app as `since` (null on a session's first prompt). An app that fails, says nothing or takes longer than
    /// `PROMPT_LIMIT` (the harness waits for this) is left out.
    pub async fn prompt(&self, event: Value) -> Result<String> {
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let caller = Caller::local(self)?;
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let now = now_ms();
        let mut asked = tokio::task::JoinSet::new();
        for (app, ctx, granted) in self.page_contexts(&caller, None)? {
            let key = format!("{app}/{session}");
            let since = seen.get(&key).cloned().unwrap_or(Value::Null);
            let (node, event) = (self.clone(), event.clone());
            asked.spawn(async move {
                let _granted = granted;
                let has = node.0.apps.describe(&app).await.is_ok_and(|d| d["hooks"].as_array().is_some_and(|h| h.iter().any(|n| n == "prompt")));
                if !has {
                    return None;
                }
                let said = node.0.apps.hook(&app, "prompt", json!({ "event": event, "since": since }), &ctx);
                let text = match tokio::time::timeout(PROMPT_LIMIT, said).await {
                    Ok(Ok(Value::String(text))) => text.trim().to_string(),
                    Ok(Ok(_)) => String::new(),
                    Ok(Err(err)) => {
                        tracing::warn!("{app}'s prompt hook: {err:#}");
                        return None;
                    }
                    Err(_) => {
                        tracing::warn!("{app}'s prompt hook took longer than {}s; left out", PROMPT_LIMIT.as_secs());
                        return None;
                    }
                };
                Some((app, key, text))
            });
        }
        let mut said: Vec<(String, String, String)> = asked.join_all().await.into_iter().flatten().collect();
        said.sort();
        let mut out = Vec::new();
        for (_, key, text) in said {
            seen.insert(key, json!(now));
            if !text.is_empty() {
                out.push(text);
            }
        }
        let week_ago = now.saturating_sub(7 * 86_400_000);
        seen.retain(|_, t| t.as_u64().is_some_and(|t| t > week_ago));
        std::fs::write(&path, serde_json::to_vec(&seen)?)?;
        Ok(out.join("\n\n"))
    }

    /// `scope`'s database of the app a call runs, as the call reads it: caught up at its first read, so it reads one
    /// state of the scope throughout, and that read traced (where its data came from, and how long getting it took).
    async fn call_db(&self, token: &str, grant: &Grant, scope: &Scope) -> Result<tokio::sync::OwnedMutexGuard<crate::database::stored::Db>> {
        ensure!(grant.scopes.contains(scope), "the call didn't name {scope}");
        let started = Instant::now();
        let first = !grant.caught_up.contains(scope);
        let traced = grant.trace.clone().filter(|_| first);
        let (db, at) = trace::within(traced.clone(), async {
            let (db, bucket) = self.call_database(&grant.caller, scope, &grant.app, scopes::APP_DATABASE, None).await?;
            let mut db = db.lock_owned().await;
            let at = db.at();
            if first {
                self.read_ready(&(scope.clone(), format!("{}/{}", grant.app, scopes::APP_DATABASE)), &mut db, &bucket).await?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.caught_up.push(scope.clone());
                }
            }
            anyhow::Ok((db, at))
        })
        .await?;
        if let Some(t) = traced {
            t.note(json!({ "step": "data", "app": grant.app, "scope": trace::kind(scope), "steps": db.at(), "new": db.at() - at }), started.elapsed());
        }
        Ok(db)
    }

    /// What the shim asks the node for a running call (`src/app/outbound.rs`): `POST /call { app, op, input }` runs a
    /// tool of an app the calling app uses, or a function of its own service, as its caller; `POST /blob` keeps a blob, `GET
    /// /blob/<hash>` gets one the caller may read; `GET /catalog` lists the network's apps, `GET /devices` its devices.
    pub async fn internal(&self, token: &str, method: &str, path: &str, body: Vec<u8>) -> Result<axum::response::Response> {
        use axum::response::IntoResponse;
        let grant = &self.grant_of(token).context("only a running call asks the node")?;
        let ok = |v: Value| axum::Json(v).into_response();
        Ok(match (method, path) {
            ("POST", "/call") => {
                ensure!(!grant.read_only, "this call only reads: it can't run other apps' tools");
                let req: Value = serde_json::from_slice(&body)?;
                let (app, tool) = (req["app"].as_str().unwrap_or(""), req["op"].as_str().unwrap_or(""));
                // An app may say which device runs it, like the one with a local model.
                let device = req["device"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                // `ctx.services` says it's a service's; `ctx.apps`, an app's, unless it runs as a service anyway (on
                // every device, or with no deploy). A service is its app's alone: other apps call that app's tools.
                let service = req["service"] == true;
                let call = self.resolve(CallRequest { app: app.into(), service: service.then(|| app.into()), tool: tool.into(), input: req["input"].clone(), device, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() });
                let (apps, services) = self.uses(&grant.app).await;
                match call.service.is_some() {
                    true => {
                        ensure!(app == grant.app, "{} can't call the {app} service: a service is only its own app's, so call the {app} app's tools with ctx.apps", grant.app);
                        ensure!(services.iter().any(|u| u == app), "{app} doesn't publish a service: add it to uses.services");
                    }
                    false => ensure!(apps.iter().any(|u| u == app), "{} doesn't use {app}", grant.app),
                }
                ensure!(grant.depth < MAX_DEPTH, "{} called {app} {} calls deep, and apps call each other at most {MAX_DEPTH} deep: is it looping?", grant.app, grant.depth + 1);
                self.count(&grant.app, "call").await?;
                // Part of the call that asked, in its trace: where it went and what that took.
                ok(json!({ "ok": trace::within(grant.trace.clone(), Box::pin(self.route(call, grant.caller.clone()))).await? }))
            }
            ("POST", "/query") => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                let params = req["params"].as_array().cloned().unwrap_or_default();
                ok(json!({ "ok": self.call_db(token, grant, &scope).await?.query(req["sql"].as_str().unwrap_or(""), &params)? }))
            }
            // Whether the caller may do each permission to each object asked (`ctx.<scope>.may`), in order: the core's
            // rows, and the app's own types (its `types`) and rows (its database's `relations` in that scope).
            ("POST", "/may") => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                let asks: Vec<(String, String)> = serde_json::from_value(req["asks"].clone())?;
                let types = self.0.apps.describe(&grant.app).await?["types"].clone();
                let rows = match types.as_object().is_some_and(|t| !t.is_empty()) {
                    true => crate::database::scopes::app_relations(self.call_db(token, grant, &scope).await?.connection()),
                    false => Vec::new(),
                };
                ok(json!({ "ok": self.with(|i| Ok(i.r.db().app_may(&grant.caller.user, &asks, &types, &rows, now_ms())))? }))
            }
            ("GET", "/vault") => ok(json!(self.lookup(&grant.caller, &grant.app, &grant.scopes).await?.for_app())),
            ("POST", "/vault/match") => {
                // The item a sign-in would change rather than make: one of that type whose plain field has that value.
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse_for(req["scope"].as_str().unwrap_or("user"), &grant.caller.user)?;
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let (kind, field, value) = (req["type"].as_str().unwrap_or(""), req["field"].as_str().unwrap_or(""), req["value"].as_str());
                let items = self.items(&grant.caller, &scope).await?;
                let id = items.iter().filter(|(_, it)| it.kind == kind && it.fields.get(field).is_some_and(|f| !f.secret && Some(f.value.as_str()) == value)).min_by_key(|(_, it)| it.made).map(|(id, _)| id.clone());
                ok(json!({ "id": id }))
            }
            ("POST", "/blob") => {
                let hash = self.put_blob(&body)?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.puts.push(hash.clone());
                    // Someone not of this machine's own programs reads back what their call put, as what they put.
                    if g.caller.credential != Credential::Own {
                        self.0.blobs.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(g.caller.user.clone());
                    }
                }
                ok(json!({ "blob": hash }))
            }
            ("GET", p) if p.starts_with("/blob/") => {
                let hash = &p["/blob/".len()..];
                let built_in = self.0.built_in.values().any(|h| h == hash);
                // A call this machine's own programs asked may read what its local API would give them: a file put there to
                // attach. A sign-in's asks here too, but reads only what its person may.
                let local = self.this_machine(&grant.caller) && release::blob_path(&self.0.dir, hash).is_ok_and(|p| p.exists());
                ensure!(built_in || local || grant.puts.iter().any(|h| h == hash) || self.may_read_blob(&grant.caller.user, hash)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            ("POST", "/appends") => {
                ensure!(self.0.calls.later.lock().unwrap().contains_key(token), "only work a call left running after it returned hands in appends later");
                let req: Value = serde_json::from_slice(&body)?;
                let appends = crate::call::appends(&req["appends"])?;
                self.write_appends(&grant.app, appends, &grant.scopes, &grant.caller, grant.from.as_deref()).await?;
                ok(json!({}))
            }
            ("POST", "/done") => {
                let req: Value = serde_json::from_slice(&body).unwrap_or_default();
                if let Some(err) = req["error"].as_str() {
                    tracing::warn!("{}'s work after a call failed: {err}", grant.app);
                }
                if let Some(done) = self.0.calls.later.lock().unwrap().remove(token) {
                    done.notify_one();
                }
                ok(json!({}))
            }
            ("GET", "/catalog") => ok(json!(self.catalog(&grant.caller, true).await?)),
            ("POST", "/code") => {
                let req: Value = serde_json::from_slice(&body)?;
                ok(self.source(req["app"].as_str().unwrap_or_default(), None, req["path"].as_str()).await?)
            }
            ("GET", "/failing") => ok(json!(self.failing_schedules(&grant.caller).await?)),
            // A push to the caller's devices, from this app, through the network's push Worker.
            ("POST", "/notify") => {
                ensure!(!grant.read_only, "this call only reads: it can't push");
                let send: crate::push::Send = serde_json::from_slice(&body)?;
                ok(json!({ "ok": self.push_note(&grant.caller.user, &send.note(&grant.app)?).await? }))
            }
            ("GET", "/devices") => ok(self.state_for(&grant.caller)?["network"]["devices"].take()),
            // Run priority's order, as the person the call is for: only an admin changes it.
            ("GET", "/order") => {
                let network = self.state_for(&grant.caller)?["network"].take();
                let mut order: Vec<Value> = network["devices"].as_array().cloned().unwrap_or_default().into_iter().map(|s| json!({ "name": s["name"], "key": s["key"], "kind": s["kind"], "priority": s["priority"], "online": s["online"], "here": s["me"] })).collect();
                if let Some(priority) = network["cloudflare_node"].get("priority") {
                    order.push(json!({ "name": "Cloudflare", "key": rank::CLOUDFLARE, "kind": rank::CLOUDFLARE, "priority": priority, "online": true, "here": false }));
                }
                let set = self.with(|i| Ok(rank::set(&i.r.db().settings()?)))?;
                order.sort_by_cached_key(|s| (rank::priority(&set, s["key"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase()));
                ok(json!({ "order": order, "admin": network["role"] == "admin" }))
            }
            ("POST", "/priority") => {
                ensure!(!grant.read_only, "this call only reads: it can't change run priority");
                let req: Value = serde_json::from_slice(&body)?;
                let priority = rank::Priority::from_value(&req["priority"]).context("a run priority is a number from 1, or \"last\"")?;
                self.set_priority(&grant.caller, req["device"].as_str().unwrap_or(""), priority).await?;
                ok(json!({}))
            }
            _ => bail!("the node has no {method} {path}"),
        })
    }

    /// Runs a call another device, `remote`, asked for. A sign-in's runs only if its token is its caller's, with the
    /// keys handed with it, never this device's own: from a node, only in scopes that node runs. One with no token runs
    /// only from a client device of its caller, as that device's own: only then are this device's keys used, and only
    /// for its own person, whose device that is.
    pub(super) async fn accept_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept(remote, request).await {
            Ok((mut req, me)) => {
                req.puts = match self.take_carried(&me, request) {
                    Ok(puts) => puts,
                    Err(err) => return Reply::failed(&err),
                };
                let reply = self.run(&req, &me).await;
                for hash in &req.puts {
                    self.drop_put(hash);
                }
                reply
            }
            Err(err) => Reply::failed(&err),
        }
    }

    /// Runs a call one of the network's services asked for, as its person, wherever it runs: a service writes nothing
    /// itself, and asks for the calls the program it offers makes (`/v1/call` where it's hosted).
    pub(super) async fn accept_service_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept_service(remote, request) {
            // Its own service on another of its person's devices: sent there.
            Ok((req, me)) if req.device.is_some() => match self.route(req, me).await {
                Ok(ok) => Reply::Ok(ok),
                Err(err) => Reply::failed(&err),
            },
            Ok((req, me)) => self.run(&req, &me).await,
            Err(err) => Reply::failed(&err),
        }
    }

    /// What a service may have run, as `service::limited` says: only its own app, in its person's own scope, here on
    /// a node of that person's own, whose keys it is run with until grants land, calling no other app. A hacked
    /// service gets no more than its own app's tools for its own person.
    fn accept_service(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let me = self.check_caller(remote, request)?;
        ensure!(request["me"]["token"].is_null(), "a service asks with no sign-in");
        let (name, owner) = self.with(|i| Ok(i.r.db().service(&remote)?.map(|s| (s.service.clone().unwrap_or_default(), s.user.clone()))))?.context("not a service of this network")?;
        ensure!(me.key == remote && me.user == owner && owner == self.user()?, "a service's calls run only on its own person's node");
        let mut req: CallRequest = serde_json::from_value(crate::services::endpoint::limited(&name, request)?)?;
        let resolved = self.resolve(req.clone());
        ensure!(resolved.app == name, "a service asks only for its own app, {name}");
        (req.from, req.depth) = (None, MAX_DEPTH);
        Ok((req, me))
    }

    async fn accept(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let mut me = self.check_caller(remote, request)?;
        let passed = request["me"]["token"].is_null() && request["keys"].is_string();
        if let Some(token) = request["me"]["token"].as_str() {
            me.credential = Credential::SignIn(self.token_of(token, &me.user).await.context("the sign-in it came with isn't its caller's, or has expired")?);
            if let Some(keys) = request["keys"].as_str() {
                self.hand(&me.credential, &me.user, keys, u64::MAX)?;
            }
        } else if let Some(keys) = request["keys"].as_str() {
            // Passed on with no sign-in: run on the keys it was handed alone, never this device's own.
            me.credential = self.take_passed(&remote, &me.user, request, keys, true)?;
        }
        let mut req = call_in(request)?;
        let (own, serves) = self.with(|i| Ok((!passed && i.r.db().client(&remote)?.is_some_and(|c| c.user == me.user), i.r.db().serves(&remote, &me.user)?)))?;
        // Only a node that runs the user's calls says an app made a call (whose items fill it in), and how deep it is.
        if !serves {
            (req.from, req.depth) = (None, 0);
        }
        // A service that runs as this device's own person (Shell, say) runs only for a device of theirs, never for a
        // sign-in a node passes on.
        let resolved = self.resolve(req.clone());
        ensure!(own || resolved.service.is_none() || self.0.services.serves_everyone(&resolved.app), "{} runs only for its person's own devices", resolved.app);
        if !own && req.service.is_none() {
            let req = self.resolve(req.clone());
            let named = req.scopes.clone();
            self.with(|i| {
                let scopes: Vec<Scope> = match &named {
                    None => reachable(i, &req.app, &me.user)?,
                    Some(named) => named.iter().map(|s| Scope::parse_for(s, &me.user)).collect::<Result<_>>()?,
                };
                let m = i.r.db();
                ensure!(scopes.iter().all(|s| m.runs_for(&remote, s)), "{} doesn't run every scope that call reaches", short(&remote));
                Ok(())
            })?;
        }
        Ok((req, me))
    }

    /// The keys a call `from` another node passed on for `user` with no sign-in was handed (`sealed`): sealed here for
    /// `from`'s key, of the scopes the call names alone, every one of which `user` may use and, for a node on a machine
    /// (`machine`), `from` runs. → its credential.
    pub(crate) fn take_passed(&self, from: &Key, user: &str, call: &Value, sealed: &str, machine: bool) -> Result<Credential> {
        let service = self.resolve(call_in(call)?).service.is_some();
        let named = call["scopes"].as_array().context("a call passed on names its scopes")?;
        let scopes: Vec<Scope> = named.iter().map(|s| Scope::parse_for(s.as_str().context("a scope is its name")?, user)).collect::<Result<_>>()?;
        self.with(|i| {
            let m = i.r.db();
            ensure!(scopes.iter().all(|s| m.may(user, "use", &scopes::object(s))), "that call reaches a scope its person may not use");
            ensure!(!machine || scopes.iter().all(|s| m.runs_for(from, s)), "{} doesn't run every scope that call names", short(from));
            Ok(())
        })?;
        self.hand_passed(from, user, sealed, &scopes, service)
    }

    /// Who a call another device passed on is for: with a sign-in's token, someone that device's node serves or whose
    /// client it is; with none, only the person whose client device `remote` is, asking itself; a person's service asks as
    /// them, as itself.
    pub(super) fn check_caller(&self, remote: Key, request: &Value) -> Result<Caller> {
        let key = unhex(request["me"]["key"].as_str().context("who's asking?")?)?;
        let user = request["me"]["user"].as_str().context("who's asking?")?.to_string();
        let token = request["me"]["token"].as_str().is_some();
        // Passed on by another node with no sign-in, the keys it was handed with it, which `take_passed` checks.
        let passed = !token && request["keys"].is_string();
        self.with(|i| {
            let m = i.r.db();
            let client = m.client(&remote)?.is_some_and(|c| c.user == user);
            let service = m.service(&remote)?.is_some_and(|s| s.user == user);
            let allowed = match () {
                _ if token => client || m.serves(&remote, &user)?,
                _ if passed => remote == key && m.compute(&remote)?.is_some(),
                _ => (client || service) && remote == key,
            };
            ensure!(allowed, "{} may not call as that user", short(&remote));
            Ok(())
        })?;
        let agent = request["me"]["agent"].as_str().map(str::to_string);
        Ok(Caller { agent, ..Caller::own(key, user) })
    }
}

pub(super) fn address(m: &ScopeDb) -> Result<Option<String>> {
    Ok(m.setting("address")?.map(|a| a.trim_end_matches('/').to_string()))
}

/// Whether a call is one another node passed on: it runs only on what it was handed, never as this device's own person.
fn passed(me: &Caller) -> bool {
    matches!(me.credential, Credential::Passed { .. })
}

/// The scopes a call reaches for `user`: those it names, or every one its app reaches; a service's, those it names or
/// its person's own.
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
        bail!(crate::protocol::Coded::new(crate::protocol::code::NOT_INSTALLED, format!("{title} isn't on your pages: add it first")));
    }
    if let Some(s) = scopes.iter().find(|s| !i.r.holds(s)) {
        return Ok(Err(format!("it can't open {} yet", scope_name(i, &s.to_string())?)));
    }
    let ctx = Context { me: node_me(node, m, me)?, now: now_ms(), scopes: handles(m, &scopes, &me.user)?, read_only: false, token: String::new(), from: None, address: address(m)?, handles: m.handles()?, labs: crate::labs::on_keys(&m.settings()?) };
    Ok(Ok((ctx, scopes)))
}

/// The call in a request another node sent: what's left once who asks and the keys and files it brings are taken off.
fn call_in(request: &Value) -> Result<CallRequest> {
    let mut call = request.as_object().cloned().unwrap_or_default();
    for envelope in ["me", "keys", "blobs"] {
        call.remove(envelope);
    }
    Ok(serde_json::from_value(Value::Object(call))?)
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
