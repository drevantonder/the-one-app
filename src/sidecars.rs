//! Device sidecars: native code a device runs beside its node, for what apps can't reach from workerd (a shell, local
//! models, WhatsApp's linked phone, a Mac's own apps). A sidecar serves its device's users, and the node says so in its
//! hello to the other devices; an app over it calls it with `ctx.sidecars`. Some are built into the node (Shell, AGENTS.md, Models: `built_in`);
//! the rest are programs that register over the local API and long-poll for calls: this program itself, hosting one
//! (`Hosted`), or another in whatever language the platform needs (Go for WhatsApp, Swift in the Mac app). The core
//! never names one: only `built_in` and `hosted` do, for the binary.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::apps::Me;

pub type Reply = Pin<Box<dyn Future<Output = Result<Answer>> + Send>>;

/// What a sidecar's call gives back: its answer; for a built-in one, records it keeps, each its kind and data; and
/// blobs the caller's scope keeps.
#[derive(Debug, Default)]
pub struct Answer {
    pub ok: Value,
    pub records: Vec<(String, Value)>,
    pub blobs: Vec<String>,
}

impl Answer {
    pub fn new(ok: Value) -> Self {
        Answer { ok, ..Default::default() }
    }
}

/// What a built-in sidecar kept, in its own database of its device's local user: one thing it did or was told.
#[derive(Clone, Debug)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub data: Value,
    pub time: u64,
    /// The person's name, and their device's.
    pub by: String,
    pub device: String,
}

/// The schema of a built-in sidecar's database.
pub const RECORDS: &str = "create table records (n integer primary key, kind text not null, data text not null, time integer not null, by text not null, device text not null)";

/// A sidecar's answer that it can't serve this call here (a model it lacks), so the call goes on to the next device.
#[derive(Debug)]
pub struct Elsewhere(pub String);

impl std::fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Elsewhere {}

/// A sidecar built into the node.
pub struct BuiltIn {
    pub name: &'static str,
    pub describe: fn() -> Value,
    /// Whether `call` needs its records (reading them all costs a pass over its database).
    pub records: bool,
    /// Runs an operation, given every record it kept, oldest first.
    pub call: fn(op: String, input: Value, caller: Me, records: Vec<Record>) -> Reply,
    /// Brings this machine in line with its records, after they change and when the node starts.
    pub sync: Option<fn(records: &[Record])>,
}

/// The sidecars built into this binary: they run on every device, for its local user.
pub fn built_in() -> Vec<BuiltIn> {
    vec![
        BuiltIn { name: "shell", describe: crate::shell::describe, records: false, call: crate::shell::call, sync: None },
        BuiltIn { name: crate::agents::NAME, describe: crate::agents::describe, records: true, call: crate::agents::call, sync: Some(crate::agents::sync) },
        BuiltIn { name: "models", describe: crate::models::describe, records: false, call: crate::models::call, sync: None },
    ]
}

/// A registered sidecar that hasn't asked for calls in this long is gone.
const GONE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub id: u64,
    pub op: String,
    pub input: Value,
    pub caller: Me,
    /// The app whose `ctx.sidecars` made the call, if one did: a push says which app sent it.
    pub from: Option<String>,
}

struct Registered {
    describe: Value,
    calls: mpsc::UnboundedSender<Call>,
    waiting: tokio::sync::Mutex<mpsc::UnboundedReceiver<Call>>,
    /// The calls waiting for its answer, with whom each is for.
    replies: Mutex<HashMap<u64, (oneshot::Sender<Value>, Me)>>,
    seen: Mutex<Instant>,
}

#[derive(Default)]
/// The sidecars registered over the local API.
pub struct Sidecars {
    by_app: Mutex<HashMap<String, std::sync::Arc<Registered>>>,
    next: std::sync::atomic::AtomicU64,
}

impl Sidecars {
    /// Registers `app`; whether that changes how it's listed: it wasn't here, had gone, or describes itself differently.
    pub fn register(&self, app: &str, mut describe: Value) -> bool {
        describe["name"] = json!(app);
        describe["runs"] = json!("here");
        let (calls, rx) = mpsc::unbounded_channel();
        let p = Registered { describe: describe.clone(), calls, waiting: tokio::sync::Mutex::new(rx), replies: Mutex::new(HashMap::new()), seen: Mutex::new(Instant::now()) };
        let before = self.by_app.lock().unwrap().insert(app.to_string(), std::sync::Arc::new(p));
        before.is_none_or(|p| p.describe != describe || p.seen.lock().unwrap().elapsed() >= GONE_AFTER)
    }

    fn get(&self, app: &str) -> Option<std::sync::Arc<Registered>> {
        let p = self.by_app.lock().unwrap().get(app).cloned()?;
        let fresh = p.seen.lock().unwrap().elapsed() < GONE_AFTER;
        fresh.then_some(p)
    }

    /// A registered sidecar's title, if it gives one.
    pub fn title(&self, app: &str) -> Option<String> {
        self.get(app)?.describe["title"].as_str().filter(|t| !t.trim().is_empty()).map(str::to_string)
    }

    pub fn has(&self, app: &str) -> bool {
        self.get(app).is_some()
    }

    /// Whether a registered sidecar serves every user its device serves (`serves: "everyone"` in its description), not only
    /// its device's local user: a push sender on an admin's device, say.
    pub fn serves_everyone(&self, app: &str) -> bool {
        self.get(app).is_some_and(|p| p.describe["serves"] == "everyone")
    }

    pub fn describe(&self) -> Vec<Value> {
        let names: Vec<String> = self.by_app.lock().unwrap().keys().cloned().collect();
        names.iter().filter_map(|n| self.get(n)).map(|p| p.describe.clone()).collect()
    }

    /// Hands a call to the sidecar and waits for its reply: `{ ok, blobs? }` or `{ error }`. A blob it names, the
    /// caller's scope keeps, so the devices that hold the scope can fetch it.
    pub async fn call(&self, app: &str, op: &str, input: Value, caller: Me, from: Option<String>) -> Result<Answer> {
        // A node that just started, an update's, waits a moment for its sidecars to register again with it.
        let mut p = self.get(app);
        let until = Instant::now() + Duration::from_secs(5);
        while p.is_none() && crate::handoff::since_start().is_some_and(|s| s < Duration::from_secs(15)) && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
            p = self.get(app);
        }
        let p = p.ok_or_else(|| anyhow!("{app} isn't running on this device"))?;
        let title = p.describe["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(app).to_string();
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        p.replies.lock().unwrap().insert(id, (tx, caller.clone()));
        p.calls.send(Call { id, op: op.into(), input, caller, from }).map_err(|_| anyhow!("{title} stopped"))?;
        let reply = tokio::time::timeout(Duration::from_secs(600), rx).await.map_err(|_| anyhow!("{title} didn't answer"))??;
        if let Some(why) = reply.get("elsewhere") {
            return Err(Elsewhere(why.as_str().unwrap_or("it can't run here").into()).into());
        }
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("failed"));
        }
        let blobs = reply["blobs"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
        Ok(Answer { ok: reply["ok"].clone(), records: Vec::new(), blobs })
    }

    /// The next call for `app`, waiting up to 25 seconds; None if there was none.
    pub async fn next(&self, app: &str) -> Result<Option<Call>> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("register {app} first"))?;
        *p.seen.lock().unwrap() = Instant::now();
        let mut rx = p.waiting.lock().await;
        // Ended as the node hands over, so the sidecar asks the new one.
        let call = tokio::select! {
            call = tokio::time::timeout(Duration::from_secs(25), rx.recv()) => call.ok().flatten(),
            _ = crate::handoff::stopping() => None,
        };
        *p.seen.lock().unwrap() = Instant::now();
        Ok(call)
    }

    pub fn reply(&self, app: &str, id: u64, reply: Value) -> Result<()> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("no sidecar {app}"))?;
        let (tx, _) = p.replies.lock().unwrap().remove(&id).ok_or_else(|| anyhow!("no call {id} waiting"))?;
        let _ = tx.send(reply);
        Ok(())
    }

    /// Whom a call of `app`'s that waits for its answer is for.
    pub fn caller_of(&self, app: &str, id: u64) -> Option<Me> {
        let p = self.get(app)?;
        let replies = p.replies.lock().unwrap();
        replies.get(&id).map(|(_, me)| me.clone())
    }
}

/// A sidecar this program hosts as a service endpoint of its own, `<cli> sidecars serve <name>` (src/service.rs), which
/// `sidecars/install.sh` installs on a device whose owner wants it there, with or without a node. It registers with
/// its endpoint over a loopback as any hosted sidecar does (`answer_hosted` is the one place it meets its callers),
/// keeps what it keeps in its folder, and runs only where it's installed, unlike those `built_in`.
pub struct Hosted {
    pub name: &'static str,
    pub describe: fn() -> Value,
    /// Runs one call, on a thread of its own.
    pub call: fn(host: &Host, call: &Call) -> Result<Value>,
}

/// The sidecars this program hosts.
pub fn hosted() -> Vec<Hosted> {
    vec![
        Hosted { name: crate::push::NAME, describe: crate::push::describe, call: crate::push::call },
        Hosted { name: crate::messenger::NAME, describe: crate::messenger::describe, call: crate::messenger::call },
        Hosted { name: crate::ai_usage::NAME, describe: crate::ai_usage::describe, call: crate::ai_usage::call },
    ]
}

/// What a hosted sidecar has: its folder, and its machine's node.
#[derive(Clone)]
pub struct Host {
    pub name: &'static str,
    pub dir: std::path::PathBuf,
    node: String,
    agent: ureq::Agent,
}

impl Host {
    pub fn get(&self, path: &str) -> Result<Value> {
        Ok(answer(self.agent.get(format!("{}{path}", self.node)).call())?.unwrap_or_default())
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        Ok(answer(self.agent.post(format!("{}{path}", self.node)).send_json(body))?.unwrap_or_default())
    }

    /// `post`, given up on after `limit`.
    pub fn post_within(&self, path: &str, body: &Value, limit: Duration) -> Result<Value> {
        Ok(answer(self.agent.post(format!("{}{path}", self.node)).config().timeout_global(Some(limit)).build().send_json(body))?.unwrap_or_default())
    }

    /// `text` with the Vault's fields (`{{name}}`) filled in by the node for call `id`, which only `host` may be sent.
    pub fn fill(&self, id: u64, text: &str, host: &str) -> Result<String> {
        let out = self.post(&format!("/v1/sidecars/{}/calls/{id}/vault", self.name), &json!({ "text": text, "host": host })).map_err(|err| anyhow!("the Vault: {err}"))?;
        Ok(out["text"].as_str().unwrap_or_default().to_string())
    }
}

/// A call's input as `T`, where none is an empty one and a field that's null is one not given, as Go's sidecars read them.
pub fn input<T: serde::de::DeserializeOwned + Default>(input: &Value) -> Result<T> {
    fn given(v: &Value) -> Value {
        match v {
            Value::Object(o) => Value::Object(o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), given(v))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(given).collect()),
            v => v.clone(),
        }
    }
    Ok(if input.is_null() { T::default() } else { serde_json::from_value(given(input)).map_err(|err| anyhow!("bad input: {err}"))? })
}

/// The node's answer: None for no content.
fn answer(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Option<Value>> {
    let mut resp = resp?;
    if resp.status() == 204 {
        return Ok(None);
    }
    let body: Value = resp.body_mut().read_json().unwrap_or_default();
    if let Some(err) = body["error"].as_str().filter(|e| !e.is_empty()) {
        bail!("{err}");
    }
    if !resp.status().is_success() {
        bail!("{}", resp.status());
    }
    Ok(Some(body))
}

/// Answers the calls for the hosted sidecar `name`, from the service endpoint at `at` (src/service.rs), on the
/// loopback its local token lets this program in at, until `replaced` says this program was: registers and answers its
/// calls, registering again whenever it's forgotten (sidecars are kept in memory).
pub fn answer_hosted(name: &str, dir: std::path::PathBuf, at: String, token: String, replaced: impl Fn() -> bool) -> Result<()> {
    let hosted = hosted().into_iter().find(|h| h.name == name).ok_or_else(|| anyhow!("{name} isn't a sidecar this program hosts"))?;
    let as_this_program = move |mut req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext| {
        if let Ok(value) = token.parse() {
            req.headers_mut().insert(crate::api::LOCAL_TOKEN_HEADER, value);
        }
        next.handle(req)
    };
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).middleware(as_this_program.clone()).build().into();
    let poll: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(40))).middleware(as_this_program).build().into();
    let host = std::sync::Arc::new(Host { name: hosted.name, dir, node: format!("http://{at}"), agent });
    let mut describe = (hosted.describe)();
    describe["version"] = json!(crate::config::BUILD);
    while !replaced() {
        if let Err(err) = host.post(&format!("/v1/sidecars/{name}"), &describe) {
            eprintln!("registering: {err:#}");
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        while !replaced() {
            let call = match answer(poll.get(format!("{}/v1/sidecars/{name}/calls", host.node)).call()) {
                Ok(Some(call)) => match serde_json::from_value::<Call>(call) {
                    Ok(call) => call,
                    Err(err) => {
                        eprintln!("a call it can't read: {err}");
                        continue;
                    }
                },
                Ok(None) => continue,
                Err(err) => {
                    eprintln!("waiting for calls: {err:#}");
                    std::thread::sleep(Duration::from_secs(1));
                    break;
                }
            };
            let (host, run) = (host.clone(), hosted.call);
            std::thread::spawn(move || {
                let reply = match run(&host, &call) {
                    Ok(ok) => json!({ "ok": ok }),
                    Err(err) => json!({ "error": format!("{err:#}") }),
                };
                if let Err(err) = host.post(&format!("/v1/sidecars/{}/calls/{}", host.name, call.id), &reply) {
                    eprintln!("answering {}: {err:#}", call.op);
                }
            });
        }
    }
    Ok(())
}
