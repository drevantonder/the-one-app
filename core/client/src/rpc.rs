//! What a client asks a server, in MCP's JSON-RPC: what each request says of who it's from, the answer, and what a
//! server's answer says of the client (it's nobody's, or behind).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Client, BUILD, PROTOCOL, telemetry, resolve, state::{State, Connection}, transport::RequestError};

/// The `progressToken` a request carries, with which MCP asks for progress notifications while it runs.
pub fn progress_token(message: &[u8]) -> Option<Value> {
    #[derive(Deserialize)]
    struct Request {
        params: Option<Params>,
    }
    #[derive(Deserialize)]
    struct Params {
        #[serde(rename = "_meta")]
        meta: Option<Meta>,
    }
    #[derive(Deserialize)]
    struct Meta {
        #[serde(rename = "progressToken")]
        progress_token: Option<Value>,
    }
    let token = serde_json::from_slice::<Request>(message).ok()?.params?.meta?.progress_token?;
    (token.is_string() || token.is_number()).then_some(token)
}

/// An id for a request of this run's own, which no other request of it has, so one is cancelled by it alone.
pub fn request_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(1);
    IDS.fetch_add(1, Ordering::Relaxed)
}

/// MCP's progress notification for the request whose `token` it is, `progress` being how far it is and `message`
/// what it's doing, if it says, on a line of its own as it's sent ahead of the response (`Reached::ask`).
pub fn progress(token: &Value, progress: u64, message: Option<&str>) -> Vec<u8> {
    let mut params = json!({"progressToken": token, "progress": progress});
    if let Some(message) = message {
        params["message"] = json!(message);
    }
    let mut line =
        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": params}).to_string().into_bytes();
    line.push(b'\n');
    line
}

/// What a progress notification says its request is doing, if it says.
pub fn progress_message(line: &[u8]) -> Option<String> {
    let notification: Value = serde_json::from_slice(line).ok()?;
    (notification["method"] == "notifications/progress")
        .then(|| notification["params"]["message"].as_str().map(str::to_owned))?
}

/// Whether `message` asks what a server answers whatever version asks it, as it's how a client catches up: the list
/// of tools, which builds there are, downloading one, updating, and following an update's task (`talks` in
/// `OneCore.Mcp`).
fn updates(message: &[u8]) -> bool {
    let Ok(request) = serde_json::from_slice::<Value>(message) else { return false };
    let tool = request["params"]["name"].as_str().unwrap_or_default();
    ["tools/list", "tasks/get"].contains(&request["method"].as_str().unwrap_or_default())
        || (request["method"] == "tools/call" && ["builds", "download", "update"].contains(&tool))
}

/// Why a server refused this client, where it's for this client speaking only protocols older than the server's: a
/// server answers that -32022, with the protocols it speaks, as MCP refuses a protocol version it doesn't speak. One
/// behind this client, which is asked again once it's updated, isn't this client's to fix.
fn left_behind(response: &[u8]) -> Option<String> {
    if response.len() > 16 << 10 {
        return None;
    }
    let response: Value = serde_json::from_slice(response).ok()?;
    let error = &response["error"];
    let oldest = error["data"]["supported"]["oldest"].as_u64()?;
    (error["code"] == -32022 && PROTOCOL["newest"].as_u64()? < oldest)
        .then(|| error["message"].as_str().unwrap_or("This app needs updating").to_owned())
}

/// Why a server refused what it was asked, as the person asking is told, if it did: a JSON-RPC error, or a tool's.
/// A refusal is short, and a long answer isn't read twice to find it isn't one.
fn refused(response: &[u8]) -> Option<String> {
    if response.len() > 16 << 10 {
        return None;
    }
    let response: Value = serde_json::from_slice(response).ok()?;
    if let Some(why) = response["error"]["message"].as_str() {
        return Some(why.to_owned());
    }
    let result = &response["result"];
    (result["isError"] == true).then(|| result["content"][0]["text"].as_str().unwrap_or("The tool failed").to_owned())
}

/// What a person is told when no server answered: what went wrong, and what to do about it.
pub(crate) const UNANSWERED: &str = "No server is answering. Check this device is online and a server is on, then try again.";

/// The JSON-RPC error a server answers a device that's nobody's (`OneCore.User`).
pub(crate) const NOBODY: i64 = -32003;

impl Client {
    /// Asks a server one JSON-RPC message and gives its answer, which for a notification is nothing. A server that
    /// has gone was asked nothing, so whichever answers now is asked instead.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>> {
        self.ask_notified(message, |_| {}).await
    }

    /// Asks as `ask` does, handing `notified` each notification the server sends about the request while it runs,
    /// as MCP's progress notifications. A request is asked for them with a `progressToken` of its own where it
    /// carries none, since they're what keeps a long one from being taken for a server that went away.
    pub async fn ask_notified(&self, message: &[u8], notified: impl Fn(&[u8])) -> Result<Vec<u8>> {
        // Only a device that's in asks its servers, and behind them, nothing they'd refuse it for that. One that's
        // opening is asked once it's in or isn't, unless its network doesn't answer, which leaves it unable to say
        // which.
        let mut state = self.state.subscribe();
        loop {
            match &*state.borrow_and_update() {
                State::SignedIn { connection: Connection::Behind(why) } if !updates(message) => bail!("{why}"),
                State::SignedIn { .. } => break,
                State::Opening { connection: Connection::Offline } => bail!("{}", Connection::Offline),
                State::Opening { .. } => {}
                out => bail!("{out}"),
            }
            state.changed().await?;
        }
        // A request is read once on its way out: for what this device fills in of it, what it says of itself, and
        // the span it is of a trace (`span`).
        let (message, lists, reads, span, said): (std::borrow::Cow<[u8]>, bool, bool, Option<opentelemetry_sdk::trace::Span>, Option<Value>) =
            match serde_json::from_slice::<Value>(message) {
                Ok(mut request) if request["method"].is_string() => {
                    if request.get("id").is_some() && request["params"]["_meta"]["progressToken"].is_null() {
                        static TOKENS: AtomicU64 = AtomicU64::new(0);
                        request["params"]["_meta"]["progressToken"] = json!(TOKENS.fetch_add(1, Ordering::Relaxed));
                    }
                    self.resolve(&mut request).await?;
                    let reads = self.reads(&request).await?;
                    let span = request.get("id").is_some().then(|| self.span(&mut request));
                    self.say(&mut request);
                    (request.to_string().into_bytes().into(), request["method"] == "tools/list", reads, span, Some(request))
                }
                _ => (message.into(), false, false, None, None),
            };
        let reached = std::sync::Mutex::new("none");
        // A server that went away after it was asked is asked again elsewhere only for what only reads: anything
        // else it may have done.
        let request = async {
            let mut stopped = false;
            for _ in 0..2 {
                let open = self.reached().await?;
                *reached.lock().expect("no ask panics") = open.path();
                // It says the attestation of the run the connection it goes over is of, which a run made again as it
                // went (`relink`) changed: a server found the owner's phone nobody's as it started (20 Nov).
                let sending: std::borrow::Cow<[u8]> = match &said {
                    Some(request) if request["params"]["_meta"]["attestation"].is_string() && request["params"]["_meta"]["attestation"] != open.attestation.as_str() && !open.attestation.is_empty() => {
                        let mut request = request.clone();
                        request["params"]["_meta"]["attestation"] = json!(open.attestation);
                        request.to_string().into_bytes().into()
                    }
                    _ => message.clone(),
                };
                match open.ask(&sending, &notified).await {
                    Ok(response) => return Ok(response),
                    Err(RequestError::Unreachable) => {}
                    Err(RequestError::Stopped) if reads => stopped = true,
                    Err(RequestError::Stopped) => bail!(
                        "The server went away before it answered, as one does while it restarts, so this may or may \
                         not have been done. Look before trying again."
                    ),
                }
            }
            match stopped {
                true => bail!("The server went away before it answered, as one does while it restarts. Try again in a moment."),
                false => bail!(UNANSWERED),
            }
        };
        let response = request.await;
        // Refused for being behind, it's behind; answered what only a server it talks with answers, it isn't.
        self.moved(|now| {
            now.with(match &response {
                Err(_) => Connection::Offline,
                Ok(response) if !updates(&message) => left_behind(response).map_or(Connection::Online, Connection::Behind),
                Ok(_) => match now.connection() {
                    Connection::Behind(why) => Connection::Behind(why),
                    _ => Connection::Online,
                },
            })
        });
        if let Some(mut span) = span {
            use opentelemetry::trace::{Span, Status};
            let refused = match &response {
                Err(why) => Some(why.to_string()),
                Ok(response) => refused(response),
            };
            span.set_attribute(opentelemetry::KeyValue::new("one.reached", *reached.lock().expect("no ask panics")));
            match refused {
                Some(_) if response.is_err() && self.offline() => {
                    span.set_attribute(opentelemetry::KeyValue::new("one.outcome", "offline"))
                }
                Some(refused) => span.set_status(Status::error(refused)),
                None => {}
            }
            span.end();
        }
        if response.is_ok() {
            self.send_soon();
        }
        match response {
            Ok(response) => {
                // What a device fills in of each tool's inputs is what the latest list of them says.
                if lists
                    && let Ok(listed) = serde_json::from_slice::<Value>(&response)
                    && listed["result"]["tools"].is_array()
                {
                    let known = resolve::listed(&listed["result"], &self.formats());
                    *self.listed.lock().expect("no ask panics") = Some(Arc::new(known));
                }
                Ok(self.taken_out(response).await)
            }
            refused => refused,
        }
    }

    /// What a server answered, unless it says this device is nobody's: one that was taken out of its network since
    /// it got its token. A server alone may be wrong, so its network's address is asked (`admit`), and only if it
    /// says so too is the device out, and the answer says where its person lets it in again.
    async fn taken_out(&self, bytes: Vec<u8>) -> Vec<u8> {
        let Ok(mut response) = serde_json::from_slice::<Value>(&bytes) else { return bytes };
        if response["error"]["code"] != NOBODY || self.address.is_none() {
            return bytes;
        }
        let now = self.state.borrow().clone();
        if !matches!(now, State::SignedIn { .. }) {
            return bytes;
        }
        self.admit(&now, None).await;
        let now = self.state.borrow().clone();
        if !matches!(now, State::SignIn { .. }) {
            return bytes;
        }
        response["error"]["message"] = json!(now.to_string());
        response.to_string().into_bytes()
    }

    /// Has `request` say who it's from, in its `_meta`: the device this run is of, with its attestation for the run's key,
    /// and what the environment says, unless the request says otherwise, as one of a device's own does. It says the
    /// protocols it speaks too, which a server refuses, saying why, only where it speaks none of them (`OneCore.Mcp`),
    /// and this build, and which client this is, as MCP's 2026-07-28 has each request say (`one-cli`, `one-app`), unless it's an MCP
    /// client's own request that says which that is.
    pub(crate) fn say(&self, request: &mut Value) {
        for (says, value) in [("device", self.key.public().to_string()), ("attestation", self.link().attestation)] {
            request["params"]["_meta"][says] = json!(value);
        }
        request["params"]["_meta"]["protocol"] = PROTOCOL.clone();
        request["params"]["_meta"]["version"] = json!(BUILD);
        let info = &mut request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"];
        if info.is_null() {
            let service = self.resource.read().expect("no report panics")["service.name"].clone();
            *info = json!({"name": format!("one-{}", service.as_str().unwrap_or("cli")), "version": BUILD});
        }
        for (says, value) in &self.says {
            if request["params"]["_meta"][says].is_null() {
                request["params"]["_meta"][says] = json!(value);
            }
        }
    }

    /// Asks the server for `method`, and gives its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_notified(request_id(), method, params, |_| {}).await
    }

    /// Asks as `request` does, as the request `id`, which cancels it (`notifications/cancelled`), handing `notified`
    /// each notification the server sends about it while it runs, as MCP's progress notifications.
    pub async fn request_notified(
        &self,
        id: u64,
        method: &str,
        params: Value,
        notified: impl Fn(&[u8]),
    ) -> Result<Value> {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = self.ask_notified(message.to_string().as_bytes(), notified).await?;
        let mut response: Value = serde_json::from_slice(&response).context("the server's answer isn't JSON")?;
        if let Some(error) = response.get("error") {
            bail!("{}", error["message"].as_str().unwrap_or("the server refused"))
        }
        let mut result = response["result"].take();
        self.keep_files(&mut result).await?;
        Ok(result)
    }

    /// Has `request` carry the span it is, as W3C's `traceparent` in its `_meta`, which a server's span of it is under,
    /// so a trace runs from this client on, by OpenTelemetry's propagator. Gives the span, which is kept should it fail
    /// or be slow. An MCP client says which it is, for what it records: in `initialize`, or in each request's `_meta`
    /// from MCP's 2026-07-28 on.
    fn span(&self, request: &mut Value) -> opentelemetry_sdk::trace::Span {
        use opentelemetry::propagation::TextMapPropagator;
        use opentelemetry::trace::{Span, TraceContextExt};
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let client = match method.as_str() {
            "initialize" => &request["params"]["clientInfo"]["name"],
            _ => &request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
        };
        if client.is_string() {
            let mut resource = self.resource.write().expect("no report panics");
            resource.insert("mcp.client.name".into(), client.clone());
        }
        let client = client.as_str().map(str::to_owned);
        let tool = request["params"]["name"].as_str().filter(|_| method == "tools/call").map(str::to_owned);
        let mut span = telemetry::request_span(self.telemetry(), &method, tool.as_deref());
        if let Some(client) = client {
            span.set_attribute(opentelemetry::KeyValue::new("mcp.client.name", client));
        }
        let meta = &mut request["params"]["_meta"];
        if meta["traceparent"].is_null() {
            let mut carrier = std::collections::HashMap::new();
            let cx = opentelemetry::Context::new().with_remote_span_context(span.span_context().clone());
            opentelemetry_sdk::propagation::TraceContextPropagator::new().inject_context(&cx, &mut carrier);
            for (name, value) in carrier {
                meta[name] = json!(value);
            }
        }
        span
    }
}
