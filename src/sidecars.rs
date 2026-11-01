//! Server sidecars: native code a server runs beside its node, for what apps can't reach from workerd (a shell, local
//! models, WhatsApp's linked phone, a Mac's own apps). A sidecar serves its server's users, and the node advertises it in
//! sync; an app over it calls it with `ctx.sidecars`. Some are built into the node (Shell, Agents, Models: `built_in`);
//! the rest are programs that register over the local API, in whatever language the platform needs (Go for WhatsApp,
//! Swift in the Mac app), and long-poll for calls. The core never names one: only `built_in` does, for the binary.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::apps::{AppEntry, Me};
use crate::entry::Content;

pub type Reply = Pin<Box<dyn Future<Output = Result<(Value, Vec<Content>)>> + Send>>;

/// A sidecar's answer that it can't serve this call here (a model it lacks), so the call goes on to the next server.
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
    /// Whether `call` needs the app's entries (reading them all costs a pass over the log).
    pub entries: bool,
    /// Runs an operation, given every entry the app has in the local user's scope, oldest first.
    pub call: fn(op: String, input: Value, caller: Me, entries: Vec<AppEntry>) -> Reply,
    /// Brings this machine in line with the app's entries, after they change and when the node starts.
    pub sync: Option<fn(entries: &[AppEntry])>,
}

/// The sidecars built into this binary: they run on every server, for its local user.
pub fn built_in() -> Vec<BuiltIn> {
    vec![
        BuiltIn { name: "shell", describe: crate::shell::describe, entries: false, call: crate::shell::call, sync: None },
        BuiltIn { name: "agents", describe: crate::agents::describe, entries: true, call: crate::agents::call, sync: Some(crate::agents::sync) },
        BuiltIn { name: "models", describe: crate::models::describe, entries: false, call: crate::models::call, sync: None },
    ]
}

/// A registered sidecar that hasn't asked for calls in this long is gone.
const GONE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize)]
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
    replies: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    seen: Mutex<Instant>,
}

#[derive(Default)]
/// The sidecars registered over the local API.
pub struct Sidecars {
    by_app: Mutex<HashMap<String, std::sync::Arc<Registered>>>,
    next: std::sync::atomic::AtomicU64,
}

impl Sidecars {
    /// Registers `app` and returns its description as the space sees it.
    pub fn register(&self, app: &str, mut describe: Value) -> Value {
        describe["name"] = json!(app);
        describe["runs"] = json!("here");
        let (calls, rx) = mpsc::unbounded_channel();
        let p = Registered { describe: describe.clone(), calls, waiting: tokio::sync::Mutex::new(rx), replies: Mutex::new(HashMap::new()), seen: Mutex::new(Instant::now()) };
        self.by_app.lock().unwrap().insert(app.to_string(), std::sync::Arc::new(p));
        describe
    }

    fn get(&self, app: &str) -> Option<std::sync::Arc<Registered>> {
        let p = self.by_app.lock().unwrap().get(app).cloned()?;
        let fresh = p.seen.lock().unwrap().elapsed() < GONE_AFTER;
        fresh.then_some(p)
    }

    pub fn has(&self, app: &str) -> bool {
        self.get(app).is_some()
    }

    /// Whether a registered sidecar serves every user its server serves (`serves: "everyone"` in its description), not only
    /// its server's local user: a push sender on a privileged server, say.
    pub fn serves_everyone(&self, app: &str) -> bool {
        self.get(app).is_some_and(|p| p.describe["serves"] == "everyone")
    }

    pub fn describe(&self) -> Vec<Value> {
        let names: Vec<String> = self.by_app.lock().unwrap().keys().cloned().collect();
        names.iter().filter_map(|n| self.get(n)).map(|p| p.describe.clone()).collect()
    }

    /// Hands a call to the sidecar and waits for its reply: `{ ok, entries? }` or `{ error }`. Each entry is
    /// `{ kind, data, blobs? }`; naming a blob lets the servers that hold the entry fetch it.
    pub async fn call(&self, app: &str, op: &str, input: Value, caller: Me, from: Option<String>) -> Result<(Value, Vec<Content>)> {
        let p = self.get(app).ok_or_else(|| anyhow!("{app} isn't running on this device"))?;
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        p.replies.lock().unwrap().insert(id, tx);
        p.calls.send(Call { id, op: op.into(), input, caller, from }).map_err(|_| anyhow!("{app} stopped"))?;
        let reply = tokio::time::timeout(Duration::from_secs(600), rx).await.map_err(|_| anyhow!("{app} didn't answer"))??;
        if let Some(why) = reply.get("elsewhere") {
            return Err(Elsewhere(why.as_str().unwrap_or("it can't run here").into()).into());
        }
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("failed"));
        }
        let entries = reply["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| Content { blobs: serde_json::from_value(e["blobs"].clone()).unwrap_or_default(), ..Content::new(app, e["kind"].as_str().unwrap_or(""), e["data"].clone()) })
            .collect();
        Ok((reply["ok"].clone(), entries))
    }

    /// The next call for `app`, waiting up to 25 seconds; None if there was none.
    pub async fn next(&self, app: &str) -> Result<Option<Call>> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("register {app} first"))?;
        *p.seen.lock().unwrap() = Instant::now();
        let mut rx = p.waiting.lock().await;
        let call = tokio::time::timeout(Duration::from_secs(25), rx.recv()).await.ok().flatten();
        *p.seen.lock().unwrap() = Instant::now();
        Ok(call)
    }

    pub fn reply(&self, app: &str, id: u64, reply: Value) -> Result<()> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("no sidecar {app}"))?;
        let tx = p.replies.lock().unwrap().remove(&id).ok_or_else(|| anyhow!("no call {id} waiting"))?;
        let _ = tx.send(reply);
        Ok(())
    }
}
