//! How a client device reaches its install's nodes: an iroh endpoint of the device's own key, through the relay at
//! the install's address (and natively, directly where a path opens), a connection kept open to each node it asks,
//! and each request HTTP/1.1 on a stream of its own, which the node answers with its local API as the device's person
//! (src/node/net.rs). The same natively, for the CLI and the apps, and in browsers as WebAssembly (src/wasm.rs).
//!
//! A device whose key can't be read out (a browser's, in WebCrypto) is reached as an endpoint of a key of the session's,
//! which the device vouches for with a certificate (`keys::certify` in the node): shown to the relay as its auth token,
//! and sent with each request (`x-one-endpoint`), so the node takes it as the device.
//!
//! Every method takes `&self`, so many requests run at once on one client: on a connection, opening a stream costs no
//! round trip, so a request takes one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{Connection, RecvStream, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher as _};
use n0_future::time::{Duration, Instant, timeout};

pub mod members;
#[cfg(feature = "relayed")]
pub mod relayed;
#[cfg(target_family = "wasm")]
mod wasm;

#[cfg(not(target_family = "wasm"))]
mod bucket;
#[cfg(not(target_family = "wasm"))]
mod device;
#[cfg(not(target_family = "wasm"))]
pub use device::{Device, new_secret};

/// One of the client's own screens (its pages, Services, a Vault request, Settings') as its page, made from the screens
/// built into this client, never asked of a node: they act as its person (`one::screen::client_page`). `params` is JSON.
#[cfg(not(target_family = "wasm"))]
pub fn client_screen(name: &str, params: &str) -> String {
    one::screen::client_page(name, &serde_json::from_str(params).unwrap_or_else(|_| serde_json::json!({})))
}

mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

/// How long a request waits to reach a node before it asks the next (the last it waits for as long as `PATIENT`): a
/// node that doesn't answer is given up on quickly.
const CONNECT: Duration = Duration::from_secs(2);
/// How long a node that isn't being asked for a request (one given up on, tried again in the background, or the first
/// connected to) has to be reached.
const PATIENT: Duration = Duration::from_secs(10);
/// How long a connected node has to start answering a GET or HEAD, before its connection is taken as gone quiet. (Other
/// requests may run as long as they need: that one is for the connection's own idle timeout.)
const ANSWER: Duration = Duration::from_secs(5);
/// How long a node that didn't answer is skipped, before the next request tries it again in the background.
const SKIP: Duration = Duration::from_secs(10);
/// The most a response's head may be.
const HEAD_LIMIT: usize = 64 << 10;
/// How much of a body is read at once.
const READ: usize = 64 << 10;

/// What a session's endpoint sends its certificate in.
const ENDPOINT_HEADER: &str = "x-one-endpoint";
/// What a request in developer mode asks for a trace with, and a node answers its own part in (src/node/trace.rs).
pub const TRACE_HEADER: &str = "x-one-trace";

/// What this white label's clients speak to its nodes.
fn alpn() -> Vec<u8> {
    format!("{}/http/1", config::CLI).into_bytes()
}

/// Why a request failed; `sent` if it may have reached the node, so that sending it again might run it twice.
#[derive(Debug)]
pub struct Error {
    pub sent: bool,
    /// The install doesn't list this device (yet, or any more): what an app shows is its own screen to approve it.
    pub not_let_in: bool,
    message: String,
    /// The nodes it tried, for a request that asked for a trace.
    pub trace: Option<serde_json::Value>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

fn unsent(message: impl std::fmt::Display) -> Error {
    Error { sent: false, not_let_in: false, message: message.to_string(), trace: None }
}

#[cfg(not(target_family = "wasm"))]
fn not_let_in(message: impl std::fmt::Display) -> Error {
    Error { not_let_in: true, ..unsent(message) }
}

fn sent(message: impl std::fmt::Display) -> Error {
    Error { sent: true, not_let_in: false, message: message.to_string(), trace: None }
}

/// A request to a node's local API: `path` with its query, from `/`.
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Sent,
}

/// What a request sends: bytes, or natively a file, streamed from disk and read again for each try, so a big body (a
/// git push's pack) is never held whole.
#[derive(Clone)]
pub enum Sent {
    Bytes(Vec<u8>),
    #[cfg(not(target_family = "wasm"))]
    File(std::path::PathBuf),
}

impl From<Vec<u8>> for Sent {
    fn from(bytes: Vec<u8>) -> Sent {
        Sent::Bytes(bytes)
    }
}

impl Sent {
    fn len(&self) -> Result<u64, Error> {
        match self {
            Sent::Bytes(bytes) => Ok(bytes.len() as u64),
            #[cfg(not(target_family = "wasm"))]
            Sent::File(path) => Ok(std::fs::metadata(path).map_err(|e| unsent(format!("the file to send: {e}")))?.len()),
        }
    }

    /// The bytes, if it's bytes.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Sent::Bytes(bytes) => bytes,
            #[cfg(not(target_family = "wasm"))]
            Sent::File(_) => &[],
        }
    }

    /// Writes it all to `send`, read as it goes.
    async fn write(&self, send: &mut iroh::endpoint::SendStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self {
            Sent::Bytes(bytes) => send.write_all(bytes).await?,
            #[cfg(not(target_family = "wasm"))]
            Sent::File(path) => {
                use std::io::Read;
                let (mut file, mut part) = (std::fs::File::open(path)?, vec![0; READ]);
                loop {
                    let n = file.read(&mut part)?;
                    if n == 0 {
                        break;
                    }
                    send.write_all(&part[..n]).await?;
                }
            }
        }
        Ok(())
    }
}

/// A node's response, its body read as it comes.
pub struct Response {
    /// The node that answered, as its connection proved: the one a 428 may be believed from, and keys sealed to.
    pub node: [u8; 32],
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    /// For a request that asked for one (`TRACE_HEADER`): `{ tries, node }`, each node it tried in order, with how
    /// long it waited and why it moved on, and what the node that answered did, as it said.
    pub trace: Option<serde_json::Value>,
}

/// A client device's way to its install's nodes. Clones share it.
#[derive(Clone)]
pub struct Client(Arc<Inner>);

struct Inner {
    secret: SecretKey,
    way: Way,
    /// A session's endpoint's certificate, if this key is one.
    certificate: Mutex<Option<String>>,
    /// The nodes to ask, in the order things run on them: the first that answers is asked.
    nodes: Mutex<Vec<PublicKey>>,
    /// The endpoint, made again after the network was lost.
    endpoint: tokio::sync::Mutex<Option<Endpoint>>,
    /// A connection to each node asked, held while it's being opened.
    conns: Mutex<HashMap<PublicKey, Arc<tokio::sync::Mutex<Option<Connection>>>>>,
    /// The nodes that didn't answer, and when: asked after the others, until one is reached again.
    down: Mutex<HashMap<PublicKey, Down>>,
}

struct Down {
    since: Instant,
    /// Whether a connection to it is being tried in the background.
    probing: bool,
}

impl Client {
    /// The client of the device whose key is `secret`, in the install at `address`, asking `nodes` (in the order things
    /// run on them); or of a session's endpoint whose key is `secret`, with its device's `certificate`. It starts
    /// connecting to the first node at once.
    pub async fn start(secret: [u8; 32], certificate: Option<String>, address: &str, nodes: &[[u8; 32]]) -> Result<Client, Error> {
        let url: RelayUrl = address.trim_end_matches('/').parse().map_err(|_| unsent("the install's address isn't a URL"))?;
        // A Worker has no UDP, so the relay serves no QUIC address discovery.
        let mut config = RelayConfig::from(url.clone());
        config.quic = None;
        Client::with(secret, Way::Relay(url, config), certificate, nodes).await
    }

    /// The client of the device whose key is `secret` in a Worker, with no socket: what it sends goes to `send`, and
    /// what the relay hands it for its key to the inbox this gives (`relayed`). It asks `nodes` as `start`'s does.
    #[cfg(feature = "relayed")]
    pub async fn relayed(secret: [u8; 32], send: relayed::Send, nodes: &[[u8; 32]]) -> Result<(Client, relayed::Inbox), Error> {
        let inbox = relayed::Inbox::default();
        let public = *SecretKey::from_bytes(&secret).public().as_bytes();
        let way = Way::Custom(Arc::new(relayed::Relayed::new(&public, inbox.clone(), send)));
        Ok((Client::with(secret, way, None, nodes).await?, inbox))
    }

    async fn with(secret: [u8; 32], way: Way, certificate: Option<String>, nodes: &[[u8; 32]]) -> Result<Client, Error> {
        let client = Client(Arc::new(Inner {
            secret: SecretKey::from_bytes(&secret),
            way,
            certificate: Mutex::new(certificate),
            nodes: Mutex::default(),
            endpoint: tokio::sync::Mutex::default(),
            conns: Mutex::default(),
            down: Mutex::default(),
        }));
        client.endpoint().await?;
        client.set_nodes(nodes)?;
        Ok(client)
    }

    /// This endpoint's key: the device's, or its session's.
    pub fn id(&self) -> [u8; 32] {
        *self.0.secret.public().as_bytes()
    }

    /// A session's new certificate, before its last ends: its requests carry it at once, and it's shown to the relay
    /// on a fresh endpoint, since the relay ends the session's socket when the last one ends.
    pub async fn certify(&self, certificate: String) {
        *self.0.certificate.lock().unwrap() = Some(certificate);
        self.reconnect().await;
    }

    /// The nodes to ask, in the order things run on them, as the install's state says; the first is connected to now.
    pub fn set_nodes(&self, nodes: &[[u8; 32]]) -> Result<(), Error> {
        let nodes = nodes.iter().map(PublicKey::from_bytes).collect::<Result<Vec<_>, _>>().map_err(|_| unsent("a node's key isn't a key"))?;
        *self.0.nodes.lock().unwrap() = nodes.clone();
        if let Some(first) = self.in_order(&nodes).first().copied().filter(|n| !self.is_down(n)) {
            let client = self.clone();
            n0_future::task::spawn(async move {
                let _ = client.conn(&first, false, PATIENT).await;
            });
        }
        Ok(())
    }

    /// `nodes` with those that didn't answer last; and any of them that's been skipped a while tried in the background.
    fn in_order(&self, nodes: &[PublicKey]) -> Vec<PublicKey> {
        let mut retry = Vec::new();
        let mut down = self.0.down.lock().unwrap();
        for (node, d) in down.iter_mut() {
            if !d.probing && d.since.elapsed() >= SKIP && nodes.contains(node) {
                d.probing = true;
                retry.push(*node);
            }
        }
        let (mut up, later): (Vec<_>, Vec<_>) = nodes.iter().partition(|n| !down.contains_key(*n));
        drop(down);
        for node in retry {
            let client = self.clone();
            n0_future::task::spawn(async move {
                let _ = client.conn(&node, true, PATIENT).await;
                if let Some(d) = client.0.down.lock().unwrap().get_mut(&node) {
                    d.probing = false;
                }
            });
        }
        up.extend(later);
        up.into_iter().copied().collect()
    }

    fn is_down(&self, node: &PublicKey) -> bool {
        self.0.down.lock().unwrap().contains_key(node)
    }

    /// `node` didn't answer: the others are asked first, for a while.
    fn give_up(&self, node: &PublicKey) {
        // Its connection is no good to the next try.
        self.0.conns.lock().unwrap().remove(node);
        let mut down = self.0.down.lock().unwrap();
        let d = down.entry(*node).or_insert(Down { since: Instant::now(), probing: false });
        d.since = Instant::now();
    }

    /// Starts again with a fresh endpoint: after the network comes back, which an endpoint that lost it waits out its
    /// relay's backoff to notice (seconds), where a fresh one connects at once.
    pub async fn reconnect(&self) {
        let old = self.0.endpoint.lock().await.take();
        self.0.conns.lock().unwrap().clear();
        self.0.down.lock().unwrap().clear();
        if let Some(old) = old {
            // The relay keeps the newest connection of a key, so the new one needn't wait for the old to close.
            n0_future::task::spawn(async move { old.close().await });
        }
        let nodes: Vec<[u8; 32]> = self.0.nodes.lock().unwrap().iter().map(|k| *k.as_bytes()).collect();
        if self.endpoint().await.is_ok() {
            let _ = self.set_nodes(&nodes);
        }
    }

    /// Closes the client: its connections and its endpoint.
    pub async fn close(&self) {
        self.0.conns.lock().unwrap().clear();
        if let Some(endpoint) = self.0.endpoint.lock().await.take() {
            endpoint.close().await;
        }
    }

    /// Asks the first node that answers, giving up on one within seconds (`CONNECT`, `ANSWER`) and asking the others
    /// first for a while (`SKIP`) after one that didn't. A request that never reached one is tried on the next; one whose connection
    /// was lost, again once (after a fresh endpoint, if this device lost the relay), unless it may have reached the
    /// node and isn't a GET or HEAD.
    pub async fn fetch(&self, req: Request) -> Result<Response, Error> {
        self.fetch_with(req, |_| Vec::new()).await
    }

    /// `fetch`, with headers for whichever node it's sent to (`node`, its key): what's sealed to that node.
    pub async fn fetch_with(&self, req: Request, node: impl Fn(&[u8; 32]) -> Vec<(String, String)>) -> Result<Response, Error> {
        let certificate = self.0.certificate.lock().unwrap().clone();
        let again = matches!(req.method.as_str(), "GET" | "HEAD");
        let nodes = self.0.nodes.lock().unwrap().clone();
        let nodes = self.in_order(&nodes);
        let mut why = unsent("there's no node to ask");
        // Asked for a trace: each try, as it went.
        let traced = req.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(TRACE_HEADER));
        let (start, mut tries) = (Instant::now(), Vec::new());
        let tried = |tries: &mut Vec<serde_json::Value>, key: &PublicKey, began: Instant, result: &str, opened: Option<Duration>| {
            if traced {
                let ms = |d: Duration| d.as_millis() as u64;
                tries.push(serde_json::json!({ "node": data_encoding::HEXLOWER.encode(key.as_bytes()), "at": ms(began - start), "ms": ms(began.elapsed()), "result": result, "connect": opened.map(ms) }));
            }
        };
        let traces = |tries: &mut Vec<serde_json::Value>, node: Option<serde_json::Value>| traced.then(|| serde_json::json!({ "tries": std::mem::take(tries), "node": node }));
        for (at, key) in nodes.iter().enumerate() {
            // The last has no other to give way to: it's given the time to be reached, and to answer.
            let last = at + 1 == nodes.len();
            let mut headers = req.headers.clone();
            headers.extend(node(key.as_bytes()));
            let head = head(&req.method, &req.path, &headers, req.body.len()?, certificate.as_deref())?;
            let node = key;
            for fresh in [false, true] {
                let began = Instant::now();
                let (err, reused) = match self.exchange(node, &head, &req, fresh, last).await {
                    Ok((mut response, opened)) => {
                        tried(&mut tries, node, began, "answered", opened);
                        let theirs = response.headers.iter().find(|h| h.0 == TRACE_HEADER).and_then(|h| data_encoding::BASE64.decode(h.1.as_bytes()).ok()).and_then(|b| serde_json::from_slice(&b).ok());
                        response.trace = traces(&mut tries, theirs);
                        return Ok(response);
                    }
                    Err(e) => e,
                };
                tried(&mut tries, node, began, failure(&err), None);
                if err.sent && !again {
                    return Err(Error { trace: traces(&mut tries, None), ..err });
                }
                why = err;
                if !self.on_relay().await {
                    self.reconnect().await;
                } else if !reused || self.is_down(node) {
                    // A node this device couldn't reach while on the relay, or that went quiet: the next.
                    break;
                }
            }
        }
        Err(Error { trace: traces(&mut tries, None), ..why })
    }

    /// Whether this device's endpoint is connected to the relay.
    async fn on_relay(&self) -> bool {
        if !matches!(self.0.way, Way::Relay(..)) {
            return true;
        }
        self.0.endpoint.lock().await.as_ref().is_some_and(|e| e.home_relay_status().get().iter().any(|r| r.is_connected()))
    }

    async fn endpoint(&self) -> Result<Endpoint, Error> {
        let mut held = self.0.endpoint.lock().await;
        if let Some(endpoint) = held.as_ref().filter(|e| !e.is_closed()) {
            return Ok(endpoint.clone());
        }
        let builder = Endpoint::builder(presets::Minimal).secret_key(self.0.secret.clone());
        let builder = match &self.0.way {
            Way::Relay(_, config) => {
                let mut config = config.clone();
                if let Some(certificate) = self.0.certificate.lock().unwrap().clone() {
                    config = config.with_auth_token(certificate);
                }
                builder.relay_mode(RelayMode::Custom(RelayMap::from(config)))
            }
            #[cfg(feature = "relayed")]
            Way::Custom(transport) => builder.relay_mode(RelayMode::Disabled).add_custom_transport(transport.clone()),
        };
        let endpoint = builder.bind().await.map_err(unsent)?;
        *held = Some(endpoint.clone());
        Ok(endpoint)
    }

    /// A connection to `node`: the one open (and true), or a new one (`fresh`, or if that one closed), within `limit`
    /// (which includes waiting for another to open it).
    async fn conn(&self, node: &PublicKey, fresh: bool, limit: Duration) -> Result<(Connection, bool), Error> {
        let opening = async {
            let slot = self.0.conns.lock().unwrap().entry(*node).or_default().clone();
            let mut held = slot.lock().await;
            if let Some(conn) = held.as_ref().filter(|c| !fresh && c.close_reason().is_none()) {
                return Ok((conn.clone(), true));
            }
            let endpoint = self.endpoint().await?;
            let addr = match &self.0.way {
                Way::Relay(url, _) => EndpointAddr::new(*node).with_relay_url(url.clone()),
                #[cfg(feature = "relayed")]
                Way::Custom(_) => EndpointAddr::from_parts(*node, [iroh::TransportAddr::Custom(relayed::through_relay(node.as_bytes()))]),
            };
            let conn = endpoint.connect(addr, &alpn()).await.map_err(|err| unsent(format!("the node couldn't be reached: {err}")))?;
            *held = Some(conn.clone());
            Ok((conn, false))
        };
        match timeout(limit, opening).await {
            Ok(Ok(opened)) => {
                if !opened.1 {
                    self.0.down.lock().unwrap().remove(node);
                }
                Ok(opened)
            }
            Ok(Err(err)) => {
                self.give_up(node);
                Err(err)
            }
            Err(_) => {
                self.give_up(node);
                Err(unsent("the node isn't connected"))
            }
        }
    }

    /// One request on a stream of its own: → the response and how long a new connection took to open (none if one was
    /// open), or why not and whether the connection was one already open.
    async fn exchange(&self, node: &PublicKey, head: &[u8], req: &Request, fresh: bool, last: bool) -> Result<(Response, Option<Duration>), (Error, bool)> {
        let connecting = Instant::now();
        let (conn, reused) = self.conn(node, fresh, if last { PATIENT } else { CONNECT }).await.map_err(|e| (e, false))?;
        let opened = (!reused).then(|| connecting.elapsed());
        // A connection gone quiet takes no more streams once its peer's allowance is used: nothing was sent.
        let (mut send, mut recv) = match timeout(ANSWER, conn.open_bi()).await {
            Ok(opened) => opened.map_err(|e| (unsent(e), reused))?,
            Err(_) => {
                self.give_up(node);
                return Err((unsent("the node isn't taking requests"), reused));
            }
        };
        let sending = async {
            send.write_all(head).await?;
            req.body.write(&mut send).await?;
            send.finish()?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        };
        sending.await.map_err(|e| (sent(e), reused))?;
        let head_only = req.method == "HEAD";
        let (status, headers, body) = if !last && matches!(req.method.as_str(), "GET" | "HEAD") {
            match timeout(ANSWER, read_response(&mut recv, head_only)).await {
                Ok(read) => read,
                Err(_) => {
                    self.give_up(node);
                    Err(sent("the node didn't answer"))
                }
            }
        } else {
            read_response(&mut recv, head_only).await
        }
        .map_err(|e| (e, reused))?;
        Ok((Response { node: *conn.remote_id().as_bytes(), status, headers, body: Body { recv: Some(recv), ..body }, trace: None }, opened))
    }
}

/// Why a try at a node failed, as a trace says it: which wait it gave up on.
fn failure(err: &Error) -> &'static str {
    match err.message.as_str() {
        "the node isn't connected" => "no connection",
        "the node isn't taking requests" => "not taking requests",
        "the node didn't answer" => "no answer",
        m if m.starts_with("the node couldn't be reached") => "unreachable",
        _ if err.sent => "lost",
        _ => "failed",
    }
}

/// How a client reaches nodes: through the relay at the install's address, or (in a Worker) a transport that passes
/// its datagrams to that relay by RPC.
enum Way {
    Relay(RelayUrl, RelayConfig),
    #[cfg(feature = "relayed")]
    Custom(Arc<relayed::Relayed>),
}

/// A request's head, checked: nothing in it can start another header or request. A session's certificate goes with it.
fn head(method: &str, path: &str, headers: &[(String, String)], len: u64, certificate: Option<&str>) -> Result<Vec<u8>, Error> {
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    if !token(method) {
        return Err(unsent("that isn't a method"));
    }
    if !path.starts_with('/') || path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(unsent("a path starts with / and has no spaces"));
    }
    let mut head = format!("{method} {path} HTTP/1.1\r\nhost: node\r\n");
    for (name, value) in headers {
        if !token(name) || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
            return Err(unsent(format!("{name} isn't a header")));
        }
        // How the request is framed, and who it's from, is this library's to say.
        if ["host", "content-length", "transfer-encoding", "connection", ENDPOINT_HEADER].iter().any(|h| name.eq_ignore_ascii_case(h)) {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(certificate) = certificate.filter(|c| c.bytes().all(|b| b.is_ascii_graphic())) {
        head.push_str(&format!("{ENDPOINT_HEADER}: {certificate}\r\n"));
    }
    head.push_str(&format!("content-length: {len}\r\n\r\n"));
    Ok(head.into_bytes())
}

/// Reads a response's head: → its status, headers, and its body as it's framed, with what was read past the head.
async fn read_response(recv: &mut RecvStream, head_only: bool) -> Result<(u16, Vec<(String, String)>, Body), Error> {
    let mut buf = Vec::new();
    loop {
        let mut list = [httparse::EMPTY_HEADER; 100];
        let mut parsed = httparse::Response::new(&mut list);
        match parsed.parse(&buf).map_err(|e| sent(format!("the node's response: {e}")))? {
            httparse::Status::Complete(len) => {
                let status = parsed.code.unwrap_or(0);
                let headers: Vec<(String, String)> = parsed.headers.iter().map(|h| (h.name.to_ascii_lowercase(), String::from_utf8_lossy(h.value).into_owned())).collect();
                let header = |name: &str| headers.iter().find(|h| h.0 == name).map(|h| h.1.to_ascii_lowercase());
                let framing = if head_only || status < 200 || status == 204 || status == 304 {
                    Framing::Done
                } else if header("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
                    Framing::Size
                } else if let Some(length) = header("content-length") {
                    Framing::Length(length.trim().parse().map_err(|_| sent("the node's response has a bad length"))?)
                } else {
                    Framing::Close
                };
                let body = Body { recv: None, buf: buf.split_off(len), framing };
                return Ok((status, headers, body));
            }
            httparse::Status::Partial if buf.len() > HEAD_LIMIT => return Err(sent("the node's response's head is too long")),
            httparse::Status::Partial => {
                if !more(recv, &mut buf).await? {
                    return Err(sent("the node closed the stream without answering"));
                }
            }
        }
    }
}

/// Reads more of a stream onto `buf`: false at its end.
async fn more(recv: &mut RecvStream, buf: &mut Vec<u8>) -> Result<bool, Error> {
    match recv.read_chunk(READ).await.map_err(|e| sent(format!("reading the node's response: {e}")))? {
        Some(chunk) => {
            buf.extend_from_slice(&chunk);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Where a body's reading is, by how it's framed.
#[derive(Clone, Copy)]
enum Framing {
    /// This many bytes are left.
    Length(u64),
    /// A chunk's size line is next.
    Size,
    /// This many bytes of a chunk are left.
    Data(u64),
    /// A chunk's closing line is next.
    DataEnd,
    /// Until the stream ends.
    Close,
    Done,
}

/// A response's body, read as it comes: dropping it before the end stops the node sending it.
pub struct Body {
    recv: Option<RecvStream>,
    buf: Vec<u8>,
    framing: Framing,
}

impl Body {
    /// A body already read whole.
    pub fn whole(bytes: Vec<u8>) -> Body {
        Body { recv: None, framing: Framing::Length(bytes.len() as u64), buf: bytes }
    }

    /// The next part of the body as it arrives, or none at its end.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, Error> {
        loop {
            match self.framing {
                Framing::Done => {
                    self.recv = None;
                    return Ok(None);
                }
                Framing::Length(0) => self.framing = Framing::Done,
                Framing::Length(n) | Framing::Data(n) => {
                    if self.buf.is_empty() && !self.more().await? {
                        return Err(sent("the node's response ended early"));
                    }
                    let take = self.buf.len().min(n.try_into().unwrap_or(usize::MAX));
                    let part: Vec<u8> = self.buf.drain(..take).collect();
                    let left = n - take as u64;
                    self.framing = match self.framing {
                        Framing::Length(_) => Framing::Length(left),
                        _ if left == 0 => Framing::DataEnd,
                        _ => Framing::Data(left),
                    };
                    return Ok(Some(part));
                }
                Framing::Close => {
                    if !self.buf.is_empty() {
                        return Ok(Some(std::mem::take(&mut self.buf)));
                    }
                    if !self.more().await? {
                        self.framing = Framing::Done;
                    }
                }
                Framing::DataEnd => {
                    let line = self.line().await?;
                    if !line.is_empty() {
                        return Err(sent("the node's response is badly chunked"));
                    }
                    self.framing = Framing::Size;
                }
                Framing::Size => {
                    let line = self.line().await?;
                    let size = line.split(|b| *b == b';').next().and_then(|s| std::str::from_utf8(s).ok()).and_then(|s| u64::from_str_radix(s.trim(), 16).ok());
                    match size {
                        Some(0) => {
                            // Trailers, to the empty line.
                            while !self.line().await?.is_empty() {}
                            self.framing = Framing::Done;
                        }
                        Some(n) => self.framing = Framing::Data(n),
                        None => return Err(sent("the node's response is badly chunked")),
                    }
                }
            }
        }
    }

    /// The rest of the body, whole.
    pub async fn bytes(self) -> Result<Vec<u8>, Error> {
        self.bytes_within(usize::MAX).await
    }

    /// The rest of the body, whole, if it's at most `most` bytes: a node that sends more is stopped.
    pub async fn bytes_within(mut self, most: usize) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        while let Some(part) = self.chunk().await? {
            if out.len() + part.len() > most {
                return Err(sent(format!("the node's response is over {most} bytes")));
            }
            out.extend_from_slice(&part);
        }
        Ok(out)
    }

    async fn more(&mut self) -> Result<bool, Error> {
        match self.recv.as_mut() {
            Some(recv) => more(recv, &mut self.buf).await,
            None => Ok(false),
        }
    }

    /// The next line, without its CRLF.
    async fn line(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            if let Some(at) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line = self.buf.drain(..at + 2).take(at).collect();
                return Ok(line);
            }
            if self.buf.len() > 4096 || !self.more().await? {
                return Err(sent("the node's response is badly chunked"));
            }
        }
    }
}
