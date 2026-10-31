//! Providers: device features written outside the node, in whatever language the platform needs (Swift for
//! Apple's Reminders, Go for WhatsApp). A provider registers an app with its own device's node, then
//! long-polls for calls and answers them. To the space it's a native app that runs on that device.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::apps::Me;
use crate::entry::{Content, Entry, Key};
use crate::space::Members;

/// A provider that hasn't asked for calls in this long is gone.
const GONE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize)]
pub struct Call {
    pub id: u64,
    pub op: String,
    pub input: Value,
    pub caller: Me,
}

struct Provider {
    describe: Value,
    calls: mpsc::UnboundedSender<Call>,
    waiting: tokio::sync::Mutex<mpsc::UnboundedReceiver<Call>>,
    replies: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    seen: Mutex<Instant>,
}

#[derive(Default)]
pub struct Providers {
    by_app: Mutex<HashMap<String, std::sync::Arc<Provider>>>,
    next: std::sync::atomic::AtomicU64,
}

impl Providers {
    /// Registers `app` and returns its description as the space sees it.
    pub fn register(&self, app: &str, mut describe: Value) -> Value {
        describe["name"] = json!(app);
        describe["runs"] = json!("here");
        let (calls, rx) = mpsc::unbounded_channel();
        let p = Provider { describe: describe.clone(), calls, waiting: tokio::sync::Mutex::new(rx), replies: Mutex::new(HashMap::new()), seen: Mutex::new(Instant::now()) };
        self.by_app.lock().unwrap().insert(app.to_string(), std::sync::Arc::new(p));
        describe
    }

    fn get(&self, app: &str) -> Option<std::sync::Arc<Provider>> {
        let p = self.by_app.lock().unwrap().get(app).cloned()?;
        let fresh = p.seen.lock().unwrap().elapsed() < GONE_AFTER;
        fresh.then_some(p)
    }

    pub fn has(&self, app: &str) -> bool {
        self.get(app).is_some()
    }

    pub fn describe(&self) -> Vec<Value> {
        let names: Vec<String> = self.by_app.lock().unwrap().keys().cloned().collect();
        names.iter().filter_map(|n| self.get(n)).map(|p| p.describe.clone()).collect()
    }

    /// Hands a call to the provider and waits for its reply: `{ ok, entries? }` or `{ error }`.
    pub async fn call(&self, app: &str, op: &str, input: Value, caller: Me) -> Result<(Value, Vec<Content>)> {
        let p = self.get(app).ok_or_else(|| anyhow!("{app} isn't running on this device"))?;
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        p.replies.lock().unwrap().insert(id, tx);
        p.calls.send(Call { id, op: op.into(), input, caller }).map_err(|_| anyhow!("{app} stopped"))?;
        let reply = tokio::time::timeout(Duration::from_secs(600), rx).await.map_err(|_| anyhow!("{app} didn't answer"))??;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("failed"));
        }
        let entries = reply["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| Content { app: app.into(), kind: e["kind"].as_str().unwrap_or("").into(), data: e["data"].clone() })
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
        let p = self.by_app.lock().unwrap().get(app).cloned().ok_or_else(|| anyhow!("no provider {app}"))?;
        let tx = p.replies.lock().unwrap().remove(&id).ok_or_else(|| anyhow!("no call {id} waiting"))?;
        let _ = tx.send(reply);
        Ok(())
    }
}

/// A device announces each provider it runs, so its user's other devices list it too (SPEC.md, "Providers").
pub fn announcement(describe: &Value) -> Content {
    Content { app: crate::release::APP.into(), kind: "provides".into(), data: describe.clone() }
}

/// The newest description each of `user`'s devices announced for each app, in replay order.
pub fn announced(members: &Members, entries: &[Entry], user: &str) -> Vec<(Key, Value)> {
    let mut newest: HashMap<(Key, String), Value> = HashMap::new();
    for e in entries.iter().filter(|e| e.content.app == crate::release::APP && e.content.kind == "provides") {
        let Some(name) = e.content.data["name"].as_str() else { continue };
        if members.active(&e.author(), e.order()).is_some_and(|m| m.user == user) {
            newest.insert((e.author(), name.to_string()), e.content.data.clone());
        }
    }
    let mut out: Vec<(Key, Value)> = newest.into_iter().map(|((device, _), d)| (device, d)).collect();
    out.sort_by(|a, b| (a.0, a.1["name"].as_str()).cmp(&(b.0, b.1["name"].as_str())));
    out
}
