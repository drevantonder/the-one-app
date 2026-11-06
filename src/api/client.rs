//! How a client asks its node's local API: the CLI and its MCP server over HTTP, as one of this machine's own programs or
//! as a device of its person's (src/main.rs), and the node's own MCP in the node itself (`InProcess`), with the proof
//! of who asks that the request it answers came with.

use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

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

    /// The devices the last request tried before one answered, for its trace: a device of its person's asks them over
    /// the network (client/src/lib.rs).
    fn tries(&self) -> Option<Value> {
        None
    }
}

/// A client of the node: what the CLI's commands and the MCP server ask it with.
pub struct Client {
    via: Box<dyn Via>,
    elsewhere: bool,
    needed: Mutex<Option<Value>>,
}

/// A call's answer, with what it did as lines when its trace was asked for (`node::trace::said`).
pub struct Called {
    pub ok: Result<Value>,
    pub trace: Option<Vec<String>>,
}

impl Client {
    /// `elsewhere`: it asks for someone who isn't this machine's own person, a device of theirs or a sign-in, whose MCP
    /// request the node answers.
    pub fn new(via: impl Via + 'static, elsewhere: bool) -> Client {
        Client { via: Box::new(via), elsewhere, needed: Mutex::new(None) }
    }

    pub fn elsewhere(&self) -> bool {
        self.elsewhere
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.json(self.via.send("GET", path, &[], None, None)?)
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.json(self.via.send("POST", path, &[("content-type", "application/json")], Some(serde_json::to_vec(&body)?), None)?)
    }

    /// Keeps `bytes` as a blob on the node: → its hash.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        Ok(self.json(self.via.send("POST", "/v1/blobs", &[], Some(bytes.to_vec()), None)?)?["blob"].as_str().unwrap_or("").to_string())
    }

    /// What `path` answers, as it comes, whatever its status: a screen's page, a blob, the node's events.
    pub fn open(&self, path: &str, wait: Option<Duration>) -> Result<Answer> {
        self.via.send("GET", path, &[], None, wait)
    }

    /// Runs a call (`/v1/call`'s body), with what it did when `trace` asks: after its answer, or the reason it failed.
    pub fn call(&self, body: &Value, trace: bool) -> Called {
        drop(self.via.tries());
        let mut headers = vec![("content-type", "application/json")];
        if trace {
            headers.push((crate::node::trace::HEADER, "1"));
        }
        let (ok, node) = match self.via.send("POST", "/v1/call", &headers, Some(serde_json::to_vec(body).unwrap_or_default()), None) {
            Ok(mut answer) => {
                let node = answer.trace.take().and_then(|h| data_encoding::BASE64.decode(h.as_bytes()).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                (self.json(answer), node)
            }
            Err(err) => (Err(err), None),
        };
        let trace = trace.then(|| {
            let tries = self.via.tries();
            if node.is_none() && tries.is_none() {
                return vec![format!("no trace: this ran in {} here, not on a device", crate::config::CLI)];
            }
            let answered = match node {
                Some(_) => String::new(),
                None => self.get("/v1/state").ok().and_then(|s| s["device"]["name"].as_str().map(str::to_string)).unwrap_or_else(|| "the device that answered".into()),
            };
            crate::node::trace::said(tries.as_ref(), node.as_ref(), &answered)
        });
        Called { ok: ok.map(|mut r| r["ok"].take()), trace }
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
        answered(answer.status, body)
    }
}

/// What the node answered, or what it says failed: refused (401, 403 or `code::REFUSED`) as `cli::Failed::Refused`, the
/// rest with its code, if it has one (`protocol::Coded`).
pub fn answered(status: u16, body: Value) -> Result<Value> {
    let Some(err) = body.get("error") else { return Ok(body) };
    let err = err.as_str().unwrap_or("failed").to_string();
    let code = body.get("code").and_then(Value::as_str).map(str::to_string);
    Err(match matches!(status, 401 | 403) || code.as_deref() == Some(crate::protocol::code::REFUSED) {
        true => crate::cli::Failed::Refused(err).into(),
        false => crate::protocol::Coded::or_plain(code, err),
    })
}

/// The node's own routes, asked in the node itself as the request its MCP answers asked them: with that request's proof
/// of who asks (the client device its connection proved, or this machine's own program, signed in or not), which
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
        let proof = [axum::http::header::AUTHORIZATION.as_str(), super::LOCAL_TOKEN_HEADER, crate::database::HEADER];
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
        let trace = parts.headers.get(crate::node::trace::HEADER).and_then(|h| h.to_str().ok()).map(str::to_string);
        Ok(Answer { status: parts.status.as_u16(), trace, body: Box::new(Streamed { chunks, part: Default::default() }) })
    }
}

/// A body read as its chunks come.
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
