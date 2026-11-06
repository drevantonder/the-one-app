//! Device services: native code a device runs beside its node, for what apps can't reach from workerd (a shell, local
//! models, WhatsApp's linked phone, a Mac's own apps). A service serves its device's users, and the node says so in its
//! hello to the other devices; its app, and only its app, calls it with `ctx.services`. Each is a program that
//! registers over the local API and long-polls for calls: an app's own (`apps/<name>/service/`, in Rust on the
//! `service` crate or in Go), or Swift in the Mac app. The core never names one. Each runs as an endpoint of its own
//! key, a part of its device's oned (`endpoint`), in its sandbox (`sandbox`).

pub mod endpoint;
pub mod sandbox;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::app::runtime::Me;

/// What a service's call gives back: its answer, and blobs the caller's scope keeps.
#[derive(Debug, Default)]
pub struct Answer {
    pub ok: Value,
    pub blobs: Vec<String>,
}

impl Answer {
    pub fn new(ok: Value) -> Self {
        Answer { ok, ..Default::default() }
    }
}

/// A service's answer that it can't serve this call here (a model it lacks), so the call goes on to the next device.
#[derive(Debug)]
pub struct Elsewhere(pub String);

impl std::fmt::Display for Elsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Elsewhere {}

/// A registered service that hasn't asked for calls in this long is gone.
const GONE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub id: u64,
    /// The function it runs: `op` on the wire, which older services read.
    #[serde(rename = "op")]
    pub function: String,
    pub input: Value,
    pub caller: Me,
    /// The app whose `ctx.services` made the call, if one did.
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
/// The services registered over the local API.
pub struct Services {
    by_app: Mutex<HashMap<String, std::sync::Arc<Registered>>>,
    /// Every service that asked to register lately, switched on here or not, by what it said it is: what this device
    /// could run (the Mac app's, which it offers while it runs).
    offered: Mutex<HashMap<String, (Value, Instant)>>,
    next: std::sync::atomic::AtomicU64,
}

impl Services {
    /// Registers `app`; whether that changes how it's listed: it wasn't here, had gone, or describes itself differently.
    pub fn register(&self, app: &str, mut describe: Value) -> bool {
        describe["name"] = json!(app);
        describe["runs"] = json!("here");
        let (calls, rx) = mpsc::unbounded_channel();
        let p = Registered { describe: describe.clone(), calls, waiting: tokio::sync::Mutex::new(rx), replies: Mutex::new(HashMap::new()), seen: Mutex::new(Instant::now()) };
        let before = self.by_app.lock().unwrap().insert(app.to_string(), std::sync::Arc::new(p));
        before.is_none_or(|p| p.describe != describe || p.seen.lock().unwrap().elapsed() >= GONE_AFTER)
    }

    /// Forgets `app`, registered here; whether it was. Calls waiting for its answer fail.
    pub fn forget(&self, app: &str) -> bool {
        self.by_app.lock().unwrap().remove(app).is_some()
    }

    /// Notes that `app` asked to register here, switched on or not.
    pub fn offer(&self, app: &str, describe: &Value) {
        self.offered.lock().unwrap().insert(app.to_string(), (describe.clone(), Instant::now()));
    }

    /// The services that asked to register here lately, switched on or not, each as it said it is.
    pub fn offered(&self) -> Vec<(String, Value)> {
        let offered = self.offered.lock().unwrap();
        offered.iter().filter(|(_, (_, at))| at.elapsed() < GONE_AFTER).map(|(n, (d, _))| (n.clone(), d.clone())).collect()
    }

    fn get(&self, app: &str) -> Option<std::sync::Arc<Registered>> {
        let p = self.by_app.lock().unwrap().get(app).cloned()?;
        let fresh = p.seen.lock().unwrap().elapsed() < GONE_AFTER;
        fresh.then_some(p)
    }

    /// A registered service's title, if it gives one.
    pub fn title(&self, app: &str) -> Option<String> {
        self.get(app)?.describe["title"].as_str().filter(|t| !t.trim().is_empty()).map(str::to_string)
    }

    pub fn has(&self, app: &str) -> bool {
        self.get(app).is_some()
    }

    /// Whether a registered service serves every user its device serves (`serves: "everyone"` in its description), not only
    /// its device's local user.
    pub fn serves_everyone(&self, app: &str) -> bool {
        self.get(app).is_some_and(|p| p.describe["serves"] == "everyone")
    }

    pub fn describe(&self) -> Vec<Value> {
        let names: Vec<String> = self.by_app.lock().unwrap().keys().cloned().collect();
        names.iter().filter_map(|n| self.get(n)).map(|p| p.describe.clone()).collect()
    }

    /// Hands a call to the service and waits for its reply: `{ ok, blobs? }` or `{ error }`. A blob it names, the
    /// caller's scope keeps, so the devices that hold the scope can fetch it.
    pub async fn call(&self, app: &str, function: &str, input: Value, caller: Me, from: Option<String>) -> Result<Answer> {
        // A node that just started, an update's, waits a moment for its services to register again with it.
        let mut p = self.get(app);
        let until = Instant::now() + Duration::from_secs(5);
        while p.is_none() && crate::handoff::since_start().is_some_and(|s| s < Duration::from_secs(15)) && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(50)).await;
            p = self.get(app);
        }
        let p = p.ok_or_else(|| crate::protocol::Coded::error(crate::protocol::code::RESTARTING, format!("{app} isn't running on this device")))?;
        let title = p.describe["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(app).to_string();
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        p.replies.lock().unwrap().insert(id, (tx, caller.clone()));
        p.calls.send(Call { id, function: function.into(), input, caller, from }).map_err(|_| anyhow!("{title} stopped"))?;
        let reply = tokio::time::timeout(Duration::from_secs(600), rx).await.map_err(|_| anyhow!("{title} didn't answer"))??;
        if let Some(why) = reply.get("elsewhere") {
            return Err(Elsewhere(why.as_str().unwrap_or("it can't run here").into()).into());
        }
        if let Some(err) = reply.get("error") {
            return Err(crate::protocol::Coded::or_plain(reply["code"].as_str().map(str::to_string), err.as_str().unwrap_or("failed")));
        }
        let blobs = reply["blobs"].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_string)).collect();
        Ok(Answer { ok: reply["ok"].clone(), blobs })
    }

    /// The next call for `app`, waiting up to 25 seconds; None if there was none.
    pub async fn next(&self, app: &str) -> Result<Option<Call>> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("register {app} first"))?;
        *p.seen.lock().unwrap() = Instant::now();
        let mut rx = p.waiting.lock().await;
        // Ended as the node hands over, so the service asks the new one.
        let call = tokio::select! {
            call = tokio::time::timeout(Duration::from_secs(25), rx.recv()) => call.ok().flatten(),
            _ = crate::handoff::stopping() => None,
        };
        *p.seen.lock().unwrap() = Instant::now();
        Ok(call)
    }

    pub fn reply(&self, app: &str, id: u64, reply: Value) -> Result<()> {
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("no service {app}"))?;
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

/// A call's input as `T`, where none is an empty one and a field that's null is one not given, as Go's services read them.
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
