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

use crate::state::Me;

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
    /// The function it runs.
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
    next: std::sync::atomic::AtomicU64,
    /// Woken when a service registers, for a call waiting for one that is starting.
    registered: tokio::sync::Notify,
}

impl Services {
    /// Registers `app`; whether that changes how it's listed: it wasn't here, had gone, or describes itself differently.
    pub fn register(&self, app: &str, mut describe: Value) -> bool {
        describe["name"] = json!(app);
        describe["runs"] = json!("here");
        let (calls, rx) = mpsc::unbounded_channel();
        let p = Registered { describe: describe.clone(), calls, waiting: tokio::sync::Mutex::new(rx), replies: Mutex::new(HashMap::new()), seen: Mutex::new(Instant::now()) };
        let before = self.by_app.lock().unwrap().insert(app.to_string(), std::sync::Arc::new(p));
        self.registered.notify_waiters();
        before.is_none_or(|p| p.describe != describe || p.seen.lock().unwrap().elapsed() >= GONE_AFTER)
    }

    fn get(&self, app: &str) -> Option<std::sync::Arc<Registered>> {
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

    /// Waits until `app` is registered.
    pub async fn registered(&self, app: &str) {
        loop {
            let registered = self.registered.notified();
            tokio::pin!(registered);
            registered.as_mut().enable();
            if self.get(app).is_some() {
                return;
            }
            registered.await;
        }
    }

    /// Hands a call to the service and waits for its reply: `{ ok, blobs? }` or `{ error }`. A blob it names, the
    /// caller's scope keeps, so the devices that hold the scope can fetch it.
    pub async fn call(&self, app: &str, function: &str, input: Value, caller: Me, from: Option<String>) -> Result<Answer> {
        // A node that just started, an update's, waits a moment for its services to register again with it.
        if crate::handoff::since_start().is_some_and(|s| s < Duration::from_secs(15)) {
            let _ = tokio::time::timeout(Duration::from_secs(5), self.registered(app)).await;
        }
        let p = self.get(app);
        let p = p.ok_or_else(|| crate::protocol::Coded::error(crate::protocol::code::RESTARTING, format!("{app} isn't running on this device")))?;
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        p.replies.lock().unwrap().insert(id, (tx, caller.clone()));
        p.calls.send(Call { id, function: function.into(), input, caller, from }).map_err(|_| anyhow!("{app} stopped"))?;
        let reply = tokio::time::timeout(Duration::from_secs(600), rx).await.map_err(|_| anyhow!("{app} didn't answer"))??;
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

/// The environment a service's program starts with (`wire.json`'s `env`): where it registers, its folder, which holds
/// the token it sends (`local-token`), the CLI it may run, and where its data goes.
pub fn program_env(name: &str, at: std::net::SocketAddr, home: &std::path::Path, data: &std::path::Path) -> Vec<(String, std::ffi::OsString)> {
    let token = crate::api::local_token(home).unwrap_or_default();
    vec![
        ("ONE_ADDR".into(), at.to_string().into()),
        ("ONE_HOME".into(), home.into()),
        // Where its program may send standard OTLP/JSON: the loopback that also takes `ONE_ADDR`'s requests.
        ("OTEL_EXPORTER_OTLP_ENDPOINT".into(), format!("http://{at}").into()),
        ("OTEL_EXPORTER_OTLP_PROTOCOL".into(), "http/json".into()),
        ("OTEL_EXPORTER_OTLP_HEADERS".into(), format!("{}={token}", crate::protocol::header::LOCAL_TOKEN).into()),
        ("ONE_CLI".into(), crate::config::CLI.into()),
        (format!("{}_DATA", name.to_uppercase().replace('-', "_")), data.into()),
    ]
}

/// What a service's program registers with and asks for its calls (`wire.json`), on its endpoint's loopback: each
/// request with the token in its folder, for the service of its name only.
pub struct Registry {
    name: String,
    token: String,
    services: Services,
}

impl Registry {
    pub fn new(name: &str, token: &str) -> std::sync::Arc<Registry> {
        std::sync::Arc::new(Registry { name: name.into(), token: token.into(), services: Services::default() })
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether a request is the program's on this machine: it sends the token in the service's folder.
    pub fn ours(&self, headers: &axum::http::HeaderMap) -> std::result::Result<(), axum::response::Response> {
        match headers.get(crate::protocol::header::LOCAL_TOKEN).and_then(|v| v.to_str().ok()) {
            Some(t) if crate::keys::same_secret(t.as_bytes(), self.token.as_bytes()) => Ok(()),
            _ => Err(refused(axum::http::StatusCode::UNAUTHORIZED, "send the local token in the service's folder")),
        }
    }

    fn this_one(&self, headers: &axum::http::HeaderMap, app: &str) -> std::result::Result<(), axum::response::Response> {
        self.ours(headers)?;
        match app == self.name {
            true => Ok(()),
            false => Err(refused(axum::http::StatusCode::NOT_FOUND, format!("this is {}'s service", self.name))),
        }
    }

    pub fn routes(self: &std::sync::Arc<Self>) -> axum::Router {
        use axum::extract::{Path, State};
        use axum::http::{HeaderMap, StatusCode};
        use axum::response::{IntoResponse, Response};
        use axum::{Json, routing};
        type Shared = State<std::sync::Arc<Registry>>;

        async fn register(State(r): Shared, headers: HeaderMap, Path(app): Path<String>, Json(describe): Json<Value>) -> Response {
            if let Err(refused) = r.this_one(&headers, &app) {
                return refused;
            }
            r.services.register(&app, describe);
            Json(json!({})).into_response()
        }

        async fn next_call(State(r): Shared, headers: HeaderMap, Path(app): Path<String>) -> Response {
            if let Err(refused) = r.this_one(&headers, &app) {
                return refused;
            }
            match r.services.next(&app).await {
                Ok(Some(call)) => Json(json!(call)).into_response(),
                Ok(None) => StatusCode::NO_CONTENT.into_response(),
                Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
            }
        }

        async fn reply(State(r): Shared, headers: HeaderMap, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Response {
            if let Err(refused) = r.this_one(&headers, &app) {
                return refused;
            }
            match r.services.reply(&app, id, body) {
                Ok(()) => Json(json!({})).into_response(),
                Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
            }
        }

        axum::Router::new()
            .route("/v1/services/{app}", routing::post(register))
            .route("/v1/services/{app}/calls", routing::get(next_call))
            .route("/v1/services/{app}/calls/{id}", routing::post(reply))
            .with_state(self.clone())
    }
}

pub(crate) fn refused(code: axum::http::StatusCode, why: impl std::fmt::Display) -> axum::response::Response {
    use axum::response::IntoResponse;
    (code, axum::Json(json!({ "error": why.to_string() }))).into_response()
}
