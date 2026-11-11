//! How a client asks its node's local API: the CLI and its MCP server over HTTP, as one of this machine's own programs
//! (cli/src/main.rs), and the node's own MCP in the node itself (`InProcess`), with the proof of who asks that the request it
//! answers came with.

use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::node::Node;

/// What the node answered: its status, its trace when asked for one (`node::trace::HEADER`, as sent), and its body as
/// it comes.
pub struct Answer {
    pub status: u16,
    pub trace: Option<String>,
    pub body: Box<dyn Read + Send>,
}

/// How a client's requests reach its node.
pub trait Via: Send + Sync {
    /// Asks `method` `path`: an error only when no node answered. `wait` bounds the whole answer, for one that streams.
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<Duration>) -> Result<Answer>;

    /// The home it reads the token it shows the node from, for the refusal that says it was the wrong one.
    fn home(&self) -> std::path::PathBuf {
        platform::home()
    }
}

/// A client of the node: what the CLI's commands and the MCP server ask it with, one for each command or MCP message.
pub struct Client {
    via: std::sync::Arc<dyn Via>,
    elsewhere: bool,
    /// What it is, as a call names its client (`Reader::client`).
    client: &'static str,
    needed: Mutex<Option<Value>>,
    apps: Mutex<Option<Vec<Value>>>,
    sets: Mutex<Option<Vec<Value>>>,
}

/// A call's answer, with what it did as lines when its trace was asked for (`node::trace::said`).
pub struct Called {
    pub ok: Result<Value>,
    pub trace: Option<Vec<String>>,
}

/// A request's client span while it's answered: `client.call`, as `cloudflare/web/insights.js` makes one.
struct Asked {
    call: Value,
    began: std::time::Instant,
}

impl Client {
    /// `elsewhere`: it asks for someone who isn't this machine's own user, a device of theirs, whose MCP
    /// request the node answers. `client`: what it is, as a call names it (`Reader::client`).
    pub fn new(via: impl Via + 'static, elsewhere: bool, client: &'static str) -> Client {
        Client { via: std::sync::Arc::new(via), elsewhere, client, needed: Mutex::new(None), apps: Mutex::new(None), sets: Mutex::new(None) }
    }

    /// Another, asking the same way, for the next MCP message of a session that serves many.
    pub fn again(&self) -> Client {
        Client { via: self.via.clone(), elsewhere: self.elsewhere, client: self.client, needed: Mutex::new(None), apps: Mutex::new(None), sets: Mutex::new(None) }
    }

    /// Every app, the node's own among them, as `GET /api/apps` gives them: asked once, the first time.
    pub fn apps(&self) -> Result<Vec<Value>> {
        let mut kept = self.apps.lock().unwrap();
        if kept.is_none() {
            *kept = Some(self.get("/api/apps")?.as_array().cloned().unwrap_or_default());
        }
        Ok(kept.clone().unwrap_or_default())
    }

    /// Every set of tools, the core's and each app's, as the API's document lists them for this caller
    /// (`openapi::sets`): asked once, the first time.
    pub fn sets(&self) -> Result<Vec<Value>> {
        let mut kept = self.sets.lock().unwrap();
        if kept.is_none() {
            *kept = Some(crate::openapi::sets(&self.get(::api::path::OPENAPI)?));
        }
        Ok(kept.clone().unwrap_or_default())
    }

    pub fn elsewhere(&self) -> bool {
        self.elsewhere
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.answered(self.send("GET", path, &[], None, None, None)?)
    }

    /// The network as this node says it (`GET /api/state`).
    pub fn state(&self) -> Result<scope::state::State> {
        serde_json::from_value(self.get("/api/state")?).context("the node's answer isn't a state: it may be a different build than this one")
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.answered(self.send("POST", path, &[("content-type", "application/json")], Some(serde_json::to_vec(&body)?), None, None)?)
    }

    /// `post`, crediting `agent` (`header::AGENT`).
    pub fn post_for(&self, agent: Option<&str>, path: &str, body: Value) -> Result<Value> {
        let named = agent.map(|a| data_encoding::BASE64URL_NOPAD.encode(a.as_bytes()));
        let mut headers = vec![("content-type", "application/json")];
        headers.extend(named.as_deref().map(|n| (custom_build::protocol::header::AGENT, n)));
        self.answered(self.send("POST", path, &headers, Some(serde_json::to_vec(&body)?), None, None)?)
    }

    /// Keeps `bytes` as a blob on the node: → its hash.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        Ok(self.answered(self.send("POST", "/api/blobs", &[], Some(bytes.to_vec()), None, None)?)?["blob"].as_str().unwrap_or("").to_string())
    }

    /// What `path` answers, as it comes, whatever its status: a screen's page, a blob, the node's events.
    pub fn open(&self, path: &str, wait: Option<Duration>) -> Result<Answer> {
        let (answer, asked) = self.send("GET", path, &[], None, wait, None)?;
        self.ended(asked, if answer.status < 400 { json!({}) } else { json!({ "failed": true }) });
        Ok(answer)
    }

    /// `post_for`, with what it did when `trace` asks, as a call says it.
    pub fn route(&self, agent: Option<&str>, path: &str, body: &Value, trace: bool) -> Called {
        let named = agent.map(|a| data_encoding::BASE64URL_NOPAD.encode(a.as_bytes()));
        let mut headers = vec![("content-type", "application/json")];
        headers.extend(named.as_deref().map(|n| (custom_build::protocol::header::AGENT, n)));
        self.traced(path, headers, body, trace, None, |r| r)
    }

    /// Runs a call (`/api/call`'s body), with what it did when `trace` asks: after its answer, or the reason it failed.
    pub fn call(&self, body: &Value, trace: bool) -> Called {
        self.traced("/api/call", vec![("content-type", "application/json")], body, trace, Some(json!({ "app": body["app"], "tool": body["tool"] })), |mut r| r["ok"].take())
    }

    fn traced(&self, path: &str, mut headers: Vec<(&str, &str)>, body: &Value, trace: bool, what: Option<Value>, answered: impl FnOnce(Value) -> Value) -> Called {
        if trace {
            headers.push((custom_build::protocol::header::TRACE, "1"));
        }
        let (ok, node) = match self.send("POST", path, &headers, Some(serde_json::to_vec(body).unwrap_or_default()), None, what) {
            Ok((mut answer, asked)) => {
                let node = answer.trace.take().and_then(|h| data_encoding::BASE64.decode(h.as_bytes()).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                (self.answered((answer, asked)), node)
            }
            Err(err) => (Err(err), None),
        };
        let trace = trace.then(|| match &node {
            Some(node) => crate::node::trace::said(node),
            None => vec!["no trace came back".to_string()],
        });
        Called { ok: ok.map(answered), trace }
    }

    /// Asks the node as the client span of a trace of its own (W3C's `traceparent`), named by `what` it asked (a call's
    /// app and tool), or else its path: → its answer, and the span, to end once the answer's read. One no node answered
    /// has ended here.
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, wait: Option<Duration>, what: Option<Value>) -> Result<(Answer, Asked)> {
        let traceparent = format!("00-{}-{}-01", crate::insights::new_trace(), crate::insights::new_span());
        let mut call = what.unwrap_or_else(|| json!({ "path": path.split('?').next().unwrap_or(path) }));
        call["traceparent"] = json!(traceparent);
        let asked = Asked { call, began: std::time::Instant::now() };
        let mut headers = headers.to_vec();
        headers.push((custom_build::protocol::header::TRACEPARENT, &traceparent));
        match self.via.send(method, path, &headers, body, wait) {
            Ok(answer) => Ok((answer, asked)),
            Err(err) => {
                self.ended(asked, json!({ "error": unreached(&err), "message": format!("{err:#}") }));
                Err(err)
            }
        }
    }

    /// What the node answered, read, and the span that asked it ended.
    fn answered(&self, (answer, asked): (Answer, Asked)) -> Result<Value> {
        let read = self.json(answer);
        // A call that needs keys it wasn't handed is answered so, and asked again: it hasn't failed.
        if self.needed.lock().unwrap().is_none() {
            self.ended(asked, read.as_ref().err().map_or(json!({}), |err| json!({ "failed": true, "code": code_of(err) })));
        }
        read
    }

    /// Ends a request's span with how it went, and records it if it's kept: only what failed, was slow or is one trace in
    /// 20 (`insights::sampled`). It's sent to the node at once, with what wasn't sent before, or kept for the next to send
    /// once a node answers, for as long as a node takes them back (a day).
    fn ended(&self, Asked { mut call, began }: Asked, how: Value) {
        let ms = began.elapsed().as_millis() as u64;
        call.as_object_mut().unwrap().extend(how.as_object().cloned().unwrap_or_default().into_iter().filter(|(_, v)| !v.is_null()));
        (call["name"], call["time"], call["ms"]) = (json!("client.call"), json!(now_ms().saturating_sub(ms)), json!(ms));
        let kept = call["failed"] == true || call["error"].is_string() || ms >= custom_build::protocol::SLOW_MS || call["traceparent"].as_str().and_then(crate::insights::parent_of).is_some_and(|(trace, _)| crate::insights::sampled(&trace));
        let unsent = platform::cache().join(format!("unsent-{}.json", self.client));
        let mut events: Vec<Value> = std::fs::read(&unsent).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if kept {
            events.push(call);
        }
        if events.is_empty() {
            return;
        }
        let from = now_ms().saturating_sub(UNSENT_FOR);
        events.retain(|e| e["time"].as_u64() >= Some(from));
        let report = serde_json::to_vec(&json!({ "platform": self.client, "events": events })).unwrap_or_default();
        let sent = self.via.send("POST", "/api/insights/report", &[("content-type", "application/json")], Some(report), Some(REPORT_WAIT)).is_ok_and(|a| a.status < 300);
        if sent {
            let _ = std::fs::remove_file(&unsent);
        } else {
            let keep = &events[events.len().saturating_sub(MOST_UNSENT)..];
            let _ = std::fs::create_dir_all(platform::cache()).and_then(|_| std::fs::write(&unsent, serde_json::to_vec(keep).unwrap_or_default()));
        }
    }

    /// What a call it made needed and wasn't handed, since this was last asked (`{ error, need, to, seal }`): the node's
    /// MCP answers its caller 428 with it, to hand them and ask again.
    pub fn needed(&self) -> Option<Value> {
        self.needed.lock().unwrap().take()
    }

    fn json(&self, answer: Answer) -> Result<Value> {
        let body: Value = serde_json::from_reader(answer.body)?;
        if body.get("error").is_some() && body.get("need").is_some() {
            *self.needed.lock().unwrap() = Some(body.clone());
        }
        answered(answer.status, body, &self.via.home())
    }
}

/// How long what a client recorded is kept while no node takes it, as long as a node takes it back; the most kept; and
/// how long sending it may hold up the command that made it.
const UNSENT_FOR: u64 = 86_400_000;
const MOST_UNSENT: usize = 300;
const REPORT_WAIT: Duration = Duration::from_secs(2);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Why no node answered a request, as insights names it: it timed out, or none could be reached.
fn unreached(err: &anyhow::Error) -> &'static str {
    let said = format!("{err:#}").to_lowercase();
    if said.contains("timeout") || said.contains("timed out") { crate::insights::TIMEOUT } else { crate::insights::UNREACHABLE }
}

/// The code of what a node answered failed, as `answered` read it: a refusal's is `refused`.
fn code_of(err: &anyhow::Error) -> Option<String> {
    let refused = matches!(err.downcast_ref::<crate::cli::Failed>(), Some(crate::cli::Failed::Refused(_)));
    custom_build::protocol::Coded::of(err).or_else(|| refused.then(|| custom_build::protocol::code::REFUSED.to_string()))
}

/// What the node answered, or what it says failed: refused (401, 403 or `code::REFUSED`) as `cli::Failed::Refused`, the
/// rest with its code, if it has one (`protocol::Coded`). `home` is where the asker read its token from.
pub fn answered(status: u16, body: Value, home: &std::path::Path) -> Result<Value> {
    let Some(err) = body.get("error") else { return Ok(body) };
    let code = body.get("code").and_then(Value::as_str).map(str::to_string);
    let err = match code.as_deref() {
        Some(custom_build::protocol::code::NOT_THIS_MACHINE) => super::token_refused(home),
        Some(custom_build::protocol::code::UPDATE) => format!("{} Run {} update.", err.as_str().unwrap_or("failed"), custom_build::config::CLI),
        _ => err.as_str().unwrap_or("failed").to_string(),
    };
    let err = match body["trace"].as_str() {
        Some(trace) => format!("{err} (trace-id {trace})"),
        None => err,
    };
    Err(match matches!(status, 401 | 403) || code.as_deref() == Some(custom_build::protocol::code::REFUSED) {
        true => crate::cli::Failed::Refused(err).into(),
        false => custom_build::protocol::Coded::or_plain(code, err),
    })
}

/// The node's own routes, asked in the node itself as the request its MCP answers asked them: with that request's proof
/// of who asks (the client device its connection proved, or this machine's own program), which
/// `only_this_machine` checks for each, as it would over HTTP.
pub struct InProcess {
    router: axum::Router,
    client: Option<super::FromClient>,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    headers: axum::http::HeaderMap,
    runtime: tokio::runtime::Handle,
}

impl InProcess {
    /// As a request that came with `client` (over iroh), from `peer` (over TCP), and `headers`. Called in the runtime.
    pub fn new(node: Node, client: Option<super::FromClient>, peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>, headers: &axum::http::HeaderMap) -> InProcess {
        let proof = [axum::http::header::AUTHORIZATION.as_str(), custom_build::protocol::header::LOCAL_TOKEN, custom_build::protocol::header::READS];
        let headers = headers.iter().filter(|(n, _)| proof.contains(&n.as_str())).map(|(n, v)| (n.clone(), v.clone())).collect();
        InProcess { router: super::guarded(super::routes(), node), client, peer, headers, runtime: tokio::runtime::Handle::current() }
    }
}

impl Via for InProcess {
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>, _wait: Option<Duration>) -> Result<Answer> {
        use hyper::service::Service;
        let mut req = axum::http::Request::builder().method(method).uri(path).body(axum::body::Body::from(body.unwrap_or_default()))?;
        req.headers_mut().extend(self.headers.clone());
        for (name, value) in headers {
            req.headers_mut().insert(axum::http::HeaderName::try_from(*name)?, value.parse()?);
        }
        if let Some(client) = self.client {
            req.extensions_mut().insert(client);
        }
        if let Some(peer) = self.peer {
            req.extensions_mut().insert(peer);
        }
        let service = hyper_util::service::TowerToHyperService::new(self.router.clone());
        let (parts, body) = self.runtime.block_on(service.call(req))?.into_parts();
        // Its body as it comes: the node's events stream until whoever reads them stops.
        let (sending, chunks) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            use tokio_stream::StreamExt;
            let mut body = body.into_data_stream();
            while let Some(Ok(chunk)) = body.next().await {
                if sending.send(chunk).is_err() {
                    break;
                }
            }
        });
        let trace = parts.headers.get(custom_build::protocol::header::TRACE).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(Answer { status: parts.status.as_u16(), trace, body: Box::new(Streamed { chunks, part: Default::default() }) })
    }
}

struct Streamed {
    chunks: std::sync::mpsc::Receiver<axum::body::Bytes>,
    part: axum::body::Bytes,
}

impl Read for Streamed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.part.is_empty() {
            match self.chunks.recv() {
                Ok(chunk) => self.part = chunk,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.part.len());
        buf[..n].copy_from_slice(&self.part[..n]);
        self.part = self.part.slice(n..);
        Ok(n)
    }
}
