//! iroh, how devices reach each other: each is an endpoint whose id is its device key, reached directly where a path
//! opens (a LAN, a tailnet, a port its router maps) and otherwise through the relay at the network's address
//! (cloudflare/relay/), its only relay, which lets in only the network's endpoints. iroh's QUIC is end to end and proves
//! both ends' keys, so what devices say needs no sealing of its own: an ask is one stream, its answer the same stream
//! back. Devices are found by the network's list of them, never n0's discovery.

use crate::config::YOURS;
use iroh::endpoint::{Connection, QuicTransportConfig, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey};

use super::*;

/// How long reaching a device may take.
const CONNECT: Duration = Duration::from_secs(10);
/// How long a notice waits to be taken before it's sent again on a new connection.
const NOTICE_WAIT: Duration = Duration::from_millis(500);
/// The most an ask or its answer may carry: an email of a claim's receipts, base64, fits.
const MESSAGE_LIMIT: usize = 64 << 20;
/// The most a computer that isn't the network's yet may send: one ask to join.
const JOIN_LIMIT: usize = 4 << 10;
/// How many streams another device may have open on a connection at once.
const STREAMS: u32 = 32;
/// How many computers that aren't the network's yet may be connected at once, each asking to join.
const JOINING: usize = 4;
/// How long a client device may take to send a request's head on a stream.
const REQUEST_HEAD: Duration = Duration::from_secs(30);
/// How many connections a client device may have open to this one at once.
const CLIENT_CONNS: usize = 4;
/// The most a client device's request may send, but a file's or an app's own request's, which stream to disk.
const CLIENT_BODY: usize = 4 << 20;
/// How long a client device's request may take to send its body.
const CLIENT_BODY_TIME: Duration = Duration::from_secs(60);
/// How long a client device's connection stays open with nothing asked on it.
const CLIENT_IDLE: Duration = Duration::from_secs(10 * 60);
/// How many connections of keys the network doesn't list (a session's endpoint, until its device's certificate is
/// checked; or a stranger) may be waiting at once, and how long one may take to show that certificate.
const UNLISTED: usize = 32;
const UNLISTED_WITHIN: Duration = Duration::from_secs(10);

/// This device's endpoint, and its connections to other devices.
#[derive(Default)]
pub(super) struct Net {
    /// The endpoint, with the network's address its relay is at.
    endpoint: Mutex<Option<(String, Endpoint)>>,
    /// The task binding the endpoint and taking what others open, with the address it's for.
    task: Mutex<Option<(String, tokio::task::JoinHandle<()>)>>,
    /// A connection to each device, opened by either end; each held while it's being opened.
    conns: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Option<Connection>>>>>,
    /// How many computers that aren't the network's are connected, asking to join.
    joining: std::sync::atomic::AtomicUsize,
    /// The removed devices this device told the relay to take out.
    removed: Mutex<std::collections::HashSet<Key>>,
    /// The connections each client device has open to this one's local API, oldest first, each with how many of its
    /// requests are still being answered.
    clients: Mutex<HashMap<Key, Vec<(Connection, Arc<std::sync::atomic::AtomicUsize>)>>>,
    /// How many connections of keys the network doesn't list are waiting to show a certificate.
    unlisted: std::sync::atomic::AtomicUsize,
}

/// Which of a device's connections, oldest first with how many requests each is answering, makes room for a new one:
/// the oldest answering none. None if all are busy, so a device never holds more than `CLIENT_CONNS` answering at once.
fn oldest_idle(busy: impl Iterator<Item = usize>) -> Option<usize> {
    busy.enumerate().find(|(_, busy)| *busy == 0).map(|(i, _)| i)
}

/// A client's connection in `Net::clients`, taken out once it's done with.
struct Held(Node, Key, Connection);

impl Drop for Held {
    fn drop(&mut self) {
        let mut clients = self.0.0.net.clients.lock().unwrap();
        if let Some(open) = clients.get_mut(&self.1) {
            open.retain(|(c, _)| c.stable_id() != self.2.stable_id() && c.close_reason().is_none());
            if open.is_empty() {
                clients.remove(&self.1);
            }
        }
    }
}

/// Why an ask never reached the device.
#[derive(Debug)]
pub(super) struct NotThere(pub(super) String);

impl std::fmt::Display for NotThere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotThere {}

/// What devices of this custom build speak: another's never meet them.
pub fn alpn() -> Vec<u8> {
    format!("{}/1", crate::config::CLI).into_bytes()
}

/// What client devices of this custom build speak to a node: HTTP/1.1, one request on each stream, to its local API.
fn http_alpn() -> Vec<u8> {
    format!("{}/http/1", crate::config::CLI).into_bytes()
}

/// What service endpoints of this custom build answer (src/service.rs): a node asks one with it.
pub fn service_alpn() -> Vec<u8> {
    format!("{}/service/1", crate::config::CLI).into_bytes()
}

/// The relay at the network's address, which serves no QUIC address discovery, a Worker having no UDP; with `ticket`,
/// a pairing code's, which lets a computer joining in.
pub fn relay(address: &str, ticket: Option<&str>) -> Result<(RelayUrl, RelayMap)> {
    let url: RelayUrl = address.trim_end_matches('/').parse().context("the network's address isn't a URL")?;
    let mut config = RelayConfig::from(url.clone());
    config.quic = None;
    if let Some(ticket) = ticket {
        config = config.with_auth_token(ticket);
    }
    Ok((url, config.into()))
}

impl Node {
    /// Keeps this device's endpoint, once it's a device of a network with an address.
    pub(super) fn follow_net(&self) {
        if super::HANDING_OVER.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.tell_removed();
        let wanted = self.with(|i| Ok(i.r.db().compute(&self.0.id).and(i.r.db().setting("address")))).ok().flatten();
        let mut task = self.0.net.task.lock().unwrap();
        if task.as_ref().map(|(a, _)| a) == wanted.as_ref() {
            return;
        }
        if let Some((_, old)) = task.take() {
            old.abort();
        }
        if let Some((_, endpoint)) = self.0.net.endpoint.lock().unwrap().take() {
            tokio::spawn(async move { endpoint.close().await });
        }
        self.0.net.conns.lock().unwrap().clear();
        if let Some(address) = wanted {
            *task = Some((address.clone(), tokio::spawn(self.clone().keep_net(address))));
        }
    }

    /// Closes this device's endpoint, for good: the node taking over from this one has its own.
    pub(super) fn close_net(&self) {
        if let Some((_, old)) = self.0.net.task.lock().unwrap().take() {
            old.abort();
        }
        if let Some((_, endpoint)) = self.0.net.endpoint.lock().unwrap().take() {
            tokio::spawn(async move { endpoint.close().await });
        }
        self.0.net.conns.lock().unwrap().clear();
    }

    /// Where this device's endpoint is reached now: its key, its relay and the addresses it's bound to, once it has one.
    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.0.net.endpoint.lock().unwrap().as_ref().map(|(_, e)| e.addr())
    }

    /// Binds this device's endpoint, then takes the connections other devices open, until it's replaced.
    async fn keep_net(self, address: String) {
        let mut wait = Duration::from_secs(1);
        let endpoint = loop {
            match self.bind(&address, None).await {
                Ok(endpoint) => break endpoint,
                Err(err) => tracing::warn!("this device's endpoint: {err:#}"),
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
        };
        *self.0.net.endpoint.lock().unwrap() = Some((address.clone(), endpoint.clone()));
        tracing::info!("this device is reached through the relay at {address}");
        while let Some(incoming) = endpoint.accept().await {
            let node = self.clone();
            tokio::spawn(async move {
                let conn = match incoming.accept() {
                    Ok(accepting) => accepting.await,
                    Err(err) => return tracing::debug!("a connection that didn't open: {err:#}"),
                };
                match conn {
                    Ok(conn) => node.take_conn(conn).await,
                    Err(err) => tracing::debug!("a connection that didn't open: {err:#}"),
                }
            });
        }
    }

    async fn bind(&self, address: &str, ticket: Option<&str>) -> Result<Endpoint> {
        let (_, map) = relay(address, ticket)?;
        let transport = QuicTransportConfig::builder().max_concurrent_bidi_streams(STREAMS.into()).max_concurrent_uni_streams(0u32.into()).build();
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(self.0.key.as_bytes()))
            .alpns(vec![alpn(), http_alpn()])
            .transport_config(transport)
            .relay_mode(RelayMode::Custom(map))
            .bind()
            .await?;
        Ok(endpoint)
    }

    /// Answers what another device asks on a connection it opened. One that isn't the network's yet is let ask only to
    /// join it, once, a few at a time, and only while this device has a pairing code out: anyone who reaches this
    /// device directly (its LAN, its tailnet, a port its router maps) can connect, which the relay never sees.
    async fn take_conn(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        if conn.alpn() == http_alpn() {
            return self.take_client(conn).await;
        }
        if !self.is_device(&remote) && self.is_cloudflare_node(&remote) {
            return self.take_cloudflare_node(conn).await;
        }
        if !self.is_device(&remote) && self.is_service(&remote) {
            return self.take_service(conn).await;
        }
        if !self.is_device(&remote) {
            return self.take_joining(conn).await;
        }
        self.caught_up();
        // Asks go back on it too, unless one's open already.
        let slot = self.conn_slot(&remote);
        if let Ok(mut held) = slot.try_lock() {
            if held.as_ref().is_none_or(|c| c.close_reason().is_some()) {
                *held = Some(conn.clone());
            }
        }
        self.serve_device(remote, conn).await;
    }

    /// Answers what another device asks on a connection, whichever of the two opened it: both ends serve one they hold, as
    /// each may reach the other on whichever is open.
    async fn serve_device(&self, remote: Key, conn: Connection) {
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            // A device removed while connected is answered no more.
            if !self.is_device(&remote) {
                return conn.close(1u32.into(), b"not a device of this network");
            }
            let node = self.clone();
            tokio::spawn(async move {
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what {} sent: {err:#}", short(&remote)),
                };
                let answer = match msg["kind"].as_str() {
                    Some("ask") if msg["body"]["trace"] == true => {
                        let t = trace::Trace::new();
                        let mut answer = trace::within(Some(t.clone()), node.accept_call(remote, &msg["body"]["call"])).await.json();
                        answer["trace"] = t.done(&node.device_name());
                        answer
                    }
                    Some("ask") => node.accept_call(remote, &msg["body"]["call"]).await.json(),
                    Some("notice") => {
                        node.changed_from(&msg["body"]["changed"]);
                        Value::Null
                    }
                    _ => json!({ "error": "that isn't something devices ask each other" }),
                };
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering {}: {err:#}", short(&remote));
                }
            });
        }
    }

    /// The network's Cloudflare node, which asks only for calls, each its person's, with the keys they handed it for
    /// them, sealed to this device: as a client device's call over iroh is taken.
    async fn take_cloudflare_node(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            if !self.is_cloudflare_node(&remote) {
                return conn.close(1u32.into(), b"not this network's Cloudflare node");
            }
            let node = self.clone();
            tokio::spawn(async move {
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what the Cloudflare node sent: {err:#}"),
                };
                let answer = match msg["kind"].as_str() {
                    Some("ask") => crate::api::call_from_cloudflare_node(&node, remote, msg["body"]["call"].clone()).await,
                    _ => json!({ "error": "the Cloudflare node only asks for calls" }),
                };
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering the Cloudflare node: {err:#}");
                }
            });
        }
    }

    /// Answers a client device's requests to the local API, as its person's (`api::FromClient`), each stream one
    /// request and its response. A key the network lists as a client is who its connection proves; any other only as
    /// a session's endpoint its device vouches for, by the certificate each request carries (`x-one-endpoint`), and is
    /// closed at the first that doesn't.
    async fn take_client(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        let busy = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let listed = self.client(&remote).is_some();
        // One the network doesn't list waits its turn among a few, and shows its device's certificate soon, or is found
        // listed once this device reads `global` again, or is let go.
        let found = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if !listed {
            use std::sync::atomic::Ordering::SeqCst;
            if self.0.net.unlisted.fetch_add(1, SeqCst) >= UNLISTED {
                self.0.net.unlisted.fetch_sub(1, SeqCst);
                return conn.close(2u32.into(), b"too many connections");
            }
        }
        {
            let mut clients = self.0.net.clients.lock().unwrap();
            let open = clients.entry(remote).or_default();
            open.retain(|(c, _)| c.close_reason().is_none());
            // A device opening another when it has its few already is one whose old one is dead, or a command that
            // ended without closing it: the oldest not answering anything is let go for it. Only the device's own
            // are ever let go, so what it may hold stays the same.
            if open.len() >= CLIENT_CONNS {
                match oldest_idle(open.iter().map(|(_, busy)| busy.load(std::sync::atomic::Ordering::SeqCst))) {
                    Some(oldest) => open.remove(oldest).0.close(0u32.into(), b"replaced by a newer connection from this device"),
                    None => {
                        drop(clients);
                        if !listed {
                            self.0.net.unlisted.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        return conn.close(2u32.into(), b"this device has too many things open to this computer at once: let one finish, then try again");
                    }
                }
            }
            open.push((conn.clone(), busy.clone()));
        }
        let _held = Held(self.clone(), remote, conn.clone());
        // The certificate checked last, and the session it vouched for: one check a connection, unless it's renewed.
        let vouched: Arc<Mutex<Option<(String, crate::keys::Session)>>> = Arc::default();
        if !listed {
            let (node, waiting, shown, found) = (self.clone(), conn.clone(), vouched.clone(), found.clone());
            tokio::spawn(async move {
                tokio::time::sleep(UNLISTED_WITHIN).await;
                node.0.net.unlisted.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if shown.lock().unwrap().is_none() && !found.load(std::sync::atomic::Ordering::SeqCst) {
                    waiting.close(1u32.into(), b"not a device of this network");
                }
            });
        }
        let (node, closing) = (self.clone(), conn.clone());
        let api = crate::api::router(self.clone()).layer(axum::middleware::from_fn(move |mut req: axum::extract::Request, next: axum::middleware::Next| {
            let (node, closing, vouched, found) = (node.clone(), closing.clone(), vouched.clone(), found.clone());
            async move {
                // Who asks is the key the connection proved, or the device that vouched for it, and nothing it says of itself.
                for header in [axum::http::header::AUTHORIZATION.as_str(), crate::api::LOCAL_TOKEN_HEADER] {
                    req.headers_mut().remove(header);
                }
                let certificate = req.headers_mut().remove(crate::api::ENDPOINT_HEADER);
                let who = if listed || found.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok((remote, u64::MAX))
                } else if certificate.is_none() && node.client_now(&remote).await.is_some() {
                    // A device approved on another, which this one hadn't read yet: it's who its connection proves.
                    found.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok((remote, u64::MAX))
                } else {
                    let shown = certificate.as_ref().and_then(|c| c.to_str().ok());
                    // Its device may be one approved on another, which this one hadn't read yet.
                    match node.vouched(&remote, shown, &vouched) {
                        Err(_) if shown.is_some() => {
                            node.catch_up().await;
                            node.vouched(&remote, shown, &vouched)
                        }
                        who => who,
                    }
                };
                match who {
                    Ok((key, until)) => {
                        req.extensions_mut().insert(crate::api::FromClient(key, until));
                        // What it sends is read in a while and up to a size, but a file's and an app's own requests',
                        // which stream to disk within their person's room (`Node::put_blob_from`).
                        let path = req.uri().path();
                        if !(path == "/v1/blobs" || path.starts_with("/v1/blobs/") || crate::api::app_route(path)) {
                            let (parts, body) = req.into_parts();
                            let Ok(Ok(bytes)) = tokio::time::timeout(CLIENT_BODY_TIME, axum::body::to_bytes(body, CLIENT_BODY)).await else {
                                let said = format!("a request sends at most {} MiB, within {} s", CLIENT_BODY >> 20, CLIENT_BODY_TIME.as_secs());
                                return axum::response::IntoResponse::into_response((axum::http::StatusCode::PAYLOAD_TOO_LARGE, said));
                            };
                            req = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
                        }
                        next.run(req).await
                    }
                    Err(err) => {
                        tracing::debug!("{}: {err:#}", short(&remote));
                        closing.close(1u32.into(), b"not a device of this network");
                        axum::response::IntoResponse::into_response(axum::http::StatusCode::FORBIDDEN)
                    }
                }
            }
        }));
        let service = hyper_util::service::TowerToHyperService::new(api);
        loop {
            use std::sync::atomic::Ordering::SeqCst;
            let (send, recv) = match tokio::time::timeout(CLIENT_IDLE, conn.accept_bi()).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(_)) => break,
                // Nothing asked a while, and nothing still answering (events, say): it's let go.
                Err(_) if busy.load(SeqCst) == 0 => return conn.close(0u32.into(), b"idle"),
                Err(_) => continue,
            };
            // A device removed while connected is answered no more (a session's, at its next request).
            if listed && self.client(&remote).is_none() {
                return conn.close(1u32.into(), b"not a device of this network");
            }
            let http = hyper::server::conn::http1::Builder::new()
                .keep_alive(false)
                // A client finishes its side once it has sent its request, and still reads the response.
                .half_close(true)
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(REQUEST_HEAD)
                .serve_connection(hyper_util::rt::TokioIo::new(Stream { send, recv }), service.clone());
            let busy = busy.clone();
            busy.fetch_add(1, SeqCst);
            tokio::spawn(async move {
                if let Err(err) = http.await {
                    tracing::debug!("answering {}: {err:#}", short(&remote));
                }
                busy.fetch_sub(1, SeqCst);
            });
        }
    }

    /// A service endpoint of the network, which asks only for calls, as its own person: a service doesn't write
    /// itself, it asks a node to.
    async fn take_service(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            if !self.is_service(&remote) {
                return conn.close(1u32.into(), b"not a service of this network");
            }
            let node = self.clone();
            tokio::spawn(async move {
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what service {} sent: {err:#}", short(&remote)),
                };
                let answer = match msg["kind"].as_str() {
                    Some("ask") => node.accept_service_call(remote, &msg["body"]["call"]).await.json(),
                    _ => json!({ "error": "a service only asks for calls" }),
                };
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering service {}: {err:#}", short(&remote));
                }
            });
        }
    }

    /// The device a session's endpoint `remote` asks as: the one its certificate names, if it's for `remote`, unexpired,
    /// and by a client device the network still lists, with its `until`. `kept` is the last checked, so a connection
    /// checks one signature.
    fn vouched(&self, remote: &Key, certificate: Option<&str>, kept: &Mutex<Option<(String, crate::keys::Session)>>) -> Result<(Key, u64)> {
        let certificate = certificate.with_context(|| format!("that key isn't one of {YOURS}'s devices, and no device vouched for it"))?;
        let now = now_ms();
        let mut kept = kept.lock().unwrap();
        let session = match kept.as_ref().filter(|(c, _)| c == certificate) {
            Some((_, session)) => *session,
            None => {
                let session = crate::keys::check_certificate(certificate, now)?;
                ensure!(session.endpoint == *remote, "that certificate is for another endpoint");
                *kept = Some((certificate.to_string(), session));
                session
            }
        };
        ensure!(session.until > now, "that session has ended");
        ensure!(self.client(&session.by).is_some(), "the device that vouched for this endpoint isn't one of {YOURS}'s");
        Ok((session.by, session.until))
    }

    /// A computer that isn't the network's: → its one ask to join answered, then the connection closed.
    async fn take_joining(&self, conn: Connection) {
        use std::sync::atomic::Ordering::SeqCst;
        let remote = *conn.remote_id().as_bytes();
        let pairing = self.0.pairings.lock().unwrap().values().any(|p| p.until > now_ms());
        if !pairing || self.0.net.joining.fetch_add(1, SeqCst) >= JOINING {
            if pairing {
                self.0.net.joining.fetch_sub(1, SeqCst);
            }
            tracing::debug!("{} isn't a device of {YOURS}", short(&remote));
            return conn.close(1u32.into(), b"not a device of this network");
        }
        let answered = tokio::time::timeout(Duration::from_secs(30), async {
            let (mut send, mut recv) = conn.accept_bi().await?;
            let msg: Value = serde_json::from_slice(&recv.read_to_end(JOIN_LIMIT).await?)?;
            let answer = match msg["body"].get("join") {
                Some(join) if msg["kind"] == "ask" => self.joining(remote, join).await,
                _ => json!({ "error": format!("only {YOURS}'s devices are answered") }),
            };
            send.write_all(answer.to_string().as_bytes()).await?;
            send.finish()?;
            // Until the joiner has read it.
            let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
            anyhow::Ok(())
        })
        .await;
        if let Ok(Err(err)) = answered {
            tracing::debug!("{} asking to join: {err:#}", short(&remote));
        }
        conn.close(0u32.into(), b"answered");
        self.0.net.joining.fetch_sub(1, SeqCst);
    }

    /// Admits a computer joining with a pairing code: → its reply, or why not.
    async fn joining(&self, remote: Key, join: &Value) -> Value {
        let joined = match serde_json::from_value(join.clone()) {
            Ok(request) => self.admit(remote, request).await.and_then(|r| Ok(serde_json::to_value(r)?)),
            Err(err) => Err(err.into()),
        };
        joined.unwrap_or_else(|err| json!({ "error": format!("{err:#}") }))
    }

    /// A notice that something changed: `{ scope, app?, database?, at?, step?, sig? }`, `step` the sealed step written
    /// at `at` and `sig` the signature beside it, base64.
    fn changed_from(&self, changed: &Value) {
        let Ok(scope) = Scope::parse(changed["scope"].as_str().unwrap_or("")) else { return };
        let wrote = changed["app"].as_str().zip(changed["database"].as_str()).map(|(app, db)| (app, db, changed["at"].as_u64().unwrap_or(0)));
        let bytes = |field: &str| changed[field].as_str().and_then(|s| data_encoding::BASE64.decode(s.as_bytes()).ok());
        self.changed_there(&scope, wrote, bytes("step"), bytes("sig"));
    }

    fn is_device(&self, key: &Key) -> bool {
        self.with(|i| Ok(i.r.db().compute(key).is_some())).unwrap_or(false)
    }

    pub(super) fn is_service(&self, key: &Key) -> bool {
        self.with(|i| Ok(i.r.db().service(key).is_some())).unwrap_or(false)
    }

    /// Whether `key` is the network's Cloudflare node's endpoint, as `cloudflare node deploy` set it.
    fn is_cloudflare_node(&self, key: &Key) -> bool {
        self.with(|i| Ok(i.r.db().setting("serverless").is_some_and(|k| k == hex(key)))).unwrap_or(false)
    }

    /// Closes the connections of devices the network no longer lists: client devices' to the local API, and those
    /// held to devices and services.
    pub(super) fn close_removed(&self) {
        let (clients, held): (Vec<Key>, Vec<Key>) = (self.0.net.clients.lock().unwrap().keys().copied().collect(), self.0.net.conns.lock().unwrap().keys().copied().collect());
        let (gone_clients, gone_held): (Vec<Key>, Vec<Key>) = self
            .with(|i| {
                let m = i.r.db();
                // A session's endpoint, listed nowhere, is cut off at its next request instead.
                Ok((clients.into_iter().filter(|k| m.machine_ever(k).is_some() && m.client(k).is_none()).collect(), held.into_iter().filter(|k| m.compute(k).is_none() && m.service(k).is_none() && m.setting("serverless").is_none_or(|s| s != hex(k))).collect()))
            })
            .unwrap_or_default();
        for key in gone_clients {
            for (conn, _) in self.0.net.clients.lock().unwrap().remove(&key).unwrap_or_default() {
                conn.close(1u32.into(), b"removed from this network");
            }
        }
        for key in gone_held {
            if let Some(slot) = self.0.net.conns.lock().unwrap().remove(&key)
                && let Ok(held) = slot.try_lock()
                && let Some(conn) = held.as_ref()
            {
                conn.close(1u32.into(), b"removed from this network");
            }
        }
    }

    fn conn_slot(&self, to: &Key) -> Arc<tokio::sync::Mutex<Option<Connection>>> {
        self.0.net.conns.lock().unwrap().entry(*to).or_default().clone()
    }

    /// A connection to `to`: the one open, or a new one (`fresh`, or if that one closed). A `NotThere` if it can't
    /// be reached.
    async fn conn(&self, to: &Key, fresh: bool) -> Result<Connection> {
        let (address, endpoint) = self.0.net.endpoint.lock().unwrap().clone().ok_or_else(|| NotThere(format!("this device isn't connected to {YOURS}'s relay")))?;
        let slot = self.conn_slot(to);
        let mut held = slot.lock().await;
        if let Some(conn) = held.as_ref().filter(|c| !fresh && c.close_reason().is_none()) {
            return Ok(conn.clone());
        }
        // Off the relay, a device is reached only on a connection it opened: one just started waits to be on it.
        if tokio::time::timeout(CONNECT, endpoint.online()).await.is_err() {
            bail!(NotThere(format!("this device isn't connected to {YOURS}'s relay")));
        }
        let (url, _) = relay(&address, None)?;
        let addr = EndpointAddr::new(PublicKey::from_bytes(to)?).with_relay_url(url);
        let alpn = if self.is_service(to) { service_alpn() } else { alpn() };
        let conn = match tokio::time::timeout(CONNECT, endpoint.connect(addr, &alpn)).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => bail!(NotThere(format!("{} couldn't be reached: {err:#}", short(to)))),
            Err(_) => bail!(NotThere(format!("{} isn't connected", short(to)))),
        };
        *held = Some(conn.clone());
        if !self.is_service(to) {
            // The other end asks on it too, once it keeps it for its asks (take_conn).
            let (node, to, conn) = (self.clone(), *to, conn.clone());
            tokio::spawn(async move { node.serve_device(to, conn).await });
        }
        self.caught_up();
        Ok(conn)
    }

    /// How long a message takes there and back on the connection open to `to`, as iroh measures it on the path it sends
    /// on. None if there's no connection open.
    pub(super) fn round_trip(&self, to: &Key) -> Option<Duration> {
        let slot = self.0.net.conns.lock().unwrap().get(to)?.clone();
        let held = slot.try_lock().ok()?;
        let conn = held.as_ref().filter(|c| c.close_reason().is_none())?;
        conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt())
    }

    /// Forgets the connection to `to`, so the next message opens a new one.
    fn drop_conn(&self, to: &Key) {
        self.0.net.conns.lock().unwrap().remove(to);
    }

    /// Fills this device's copy of each database it has open from the bucket now: a connection to another device just
    /// opened, and what it said while there was none was lost.
    fn caught_up(&self) {
        let node = self.clone();
        tokio::spawn(async move { node.fill_copies().await });
    }

    /// Sends `to` a message of `kind` on a stream of its own, on the connection open or, if that one's gone, a new
    /// one: → its answer, waiting `limit` for it, if `answered`.
    async fn send(&self, to: &Key, kind: &str, body: &Value, answered: bool, limit: Duration) -> Result<Value> {
        let msg = json!({ "kind": kind, "body": body }).to_string();
        ensure!(msg.len() <= MESSAGE_LIMIT, "it's {} MB, more than another device is sent at once ({} MB)", msg.len() >> 20, MESSAGE_LIMIT >> 20);
        let mut fresh = false;
        let (mut send, mut recv) = loop {
            let conn = self.conn(to, fresh).await?;
            match conn.open_bi().await {
                Ok(stream) => break stream,
                Err(_) if !fresh => fresh = true,
                Err(err) => bail!(NotThere(format!("{} couldn't be reached: {err:#}", short(to)))),
            }
        };
        send.write_all(msg.as_bytes()).await?;
        send.finish()?;
        if !answered {
            return Ok(Value::Null);
        }
        let answer = tokio::time::timeout(limit, recv.read_to_end(MESSAGE_LIMIT)).await.context("timed out")??;
        Ok(serde_json::from_slice(&answer)?)
    }

    /// Asks another device to run a call, waiting `limit` for its answer. An error that's a `NotThere` means it never
    /// reached that device.
    pub(super) async fn ask(&self, to: Key, call: Value, limit: Duration) -> Result<Value> {
        // A traced request asks that device for its own part too, which an older one leaves out.
        let body = if trace::on() { json!({ "call": call, "trace": true }) } else { json!({ "call": call }) };
        self.send(&to, "ask", &body, true, limit).await
    }

    /// Says hello to a service endpoint, with the network's genesis and how many of `global`'s steps this device has:
    /// it answers what it offers once it lists this device as one that may call it; until then, that it's behind, and
    /// is sent them, which it checks itself. → what it offers, and whom it serves.
    pub(super) async fn hello_service(&self, to: Key) -> Result<Option<(Value, Vec<String>)>> {
        // `global`'s scope database's steps (`database::steps`), its freeze among them.
        let mut steps = Vec::new();
        if self.frozen() {
            let (bucket, keys) = (self.bucket()?, self.step_keys(&Scope::Global).await?);
            loop {
                let read = crate::database::steps::read(&bucket, &keys, &Scope::Global, steps.len() as u64).await?;
                steps.extend(read.steps.iter().cloned());
                if !read.junk_at_end(self.with(|i| Ok(i.r.epoch_made(&Scope::Global)))?) {
                    break;
                }
                steps.push(None);
            }
        }
        let hello = json!({ "entries": 1, "steps": steps.len() });
        let mut answer = self.send(&to, "hello", &hello, true, CONNECT).await?;
        if answer["behind"] == true {
            let genesis = crate::database::steps::genesis(&self.bucket()?, &self.step_keys(&Scope::Global).await?).await?;
            let pack = data_encoding::BASE64.encode(&crate::entry::pack(&genesis.iter().collect::<Vec<_>>()));
            let taken = self.send(&to, "entries", &json!({ "pack": pack }), true, CONNECT).await?;
            if let Some(err) = taken["error"].as_str() {
                bail!("{} didn't take the network's genesis: {err}", short(&to));
            }
            answer = self.send(&to, "hello", &hello, true, CONNECT).await?;
        }
        if answer["behind"] == true && !steps.is_empty() {
            let list: Vec<Value> = steps.iter().enumerate().map(|(i, s)| json!({ "at": i + 1, "step": s.as_ref().map(|s| data_encoding::BASE64URL_NOPAD.encode(&crate::database::steps::encoded(s))) })).collect();
            let taken = self.send(&to, "steps", &json!({ "steps": list }), true, CONNECT).await?;
            if let Some(err) = taken["error"].as_str() {
                bail!("{} didn't take the scope database's steps: {err}", short(&to));
            }
            answer = self.send(&to, "hello", &hello, true, CONNECT).await?;
        }
        if let Some(err) = answer["error"].as_str() {
            bail!("{err}");
        }
        let users = answer["users"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)).collect();
        Ok(answer.get("describe").filter(|d| d.is_object()).map(|d| (d.clone(), users)))
    }

    /// Tells each of `to` that something changed, expecting no answer.
    pub(super) fn notify(&self, to: &[Key], changed: &Value) {
        for device in to.iter().copied() {
            let (node, body) = (self.clone(), json!({ "changed": changed }));
            tokio::spawn(async move {
                // Answered, so one lost on a connection that died unseen is sent again on a new one.
                if let Err(err) = node.send(&device, "notice", &body, true, NOTICE_WAIT).await {
                    node.drop_conn(&device);
                    if let Err(err) = node.send(&device, "notice", &body, true, NOTICE_WAIT).await {
                        tracing::debug!("telling {}: {err:#}", short(&device));
                    }
                    tracing::debug!("telling {}, tried again: {err:#}", short(&device));
                }
            });
        }
    }

    /// Asks the device that made a pairing code to admit this one, through the relay at `address`, which lets this
    /// one in for the code's `ticket`. The device that made it may not have told the relay of it yet: it's tried for a
    /// few seconds.
    pub(super) async fn ask_to_join(&self, address: &str, ticket: &str, to: Key, join: Value) -> Result<Value> {
        let endpoint = self.bind(address, Some(ticket)).await?;
        let (url, _) = relay(address, None)?;
        let addr = EndpointAddr::new(PublicKey::from_bytes(&to)?).with_relay_url(url);
        let mut tried = 0;
        let conn = loop {
            match tokio::time::timeout(CONNECT, endpoint.connect(addr.clone(), &alpn())).await {
                Ok(Ok(conn)) => break conn,
                _ if tried < 3 => tried += 1,
                Ok(Err(err)) => bail!("the device that made the pairing code couldn't be reached: {err:#}"),
                Err(_) => bail!("the device that made the pairing code isn't on"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        let answer = async {
            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_all(json!({ "kind": "ask", "body": { "join": join } }).to_string().as_bytes()).await?;
            send.finish()?;
            let answer = tokio::time::timeout(Duration::from_secs(30), recv.read_to_end(MESSAGE_LIMIT)).await.context("the device that made the pairing code didn't answer")??;
            anyhow::Ok(serde_json::from_slice(&answer)?)
        }
        .await;
        conn.close(0u32.into(), b"joined");
        endpoint.close().await;
        answer
    }

    /// Tells the relay to take out the devices removed from the network, if this device is an admin's: the relay lets
    /// in only its list, which nothing on it can read the network's log for.
    fn tell_removed(&self) {
        let Ok(removed) = self.with(|i| {
            let m = i.r.db();
            let mine = m.machine(&self.0.id).is_some_and(|me| m.may(&me.user, "invite", scopes::NETWORK));
            Ok(if mine { m.machines().into_iter().filter(|d| d.removed).map(|d| d.key).collect::<Vec<Key>>() } else { Vec::new() })
        }) else {
            return;
        };
        let mut told = self.0.net.removed.lock().unwrap();
        for key in removed {
            if told.insert(key) {
                self.tell_relay(json!({ "remove": hex(&key) }));
            }
        }
    }

    /// Tells the relay at the network's address something, signed by this device: a device it admitted (`add`), or a
    /// pairing code it made (`pairing`, the hash of its ticket, and `until`). Tried a few times, in the background.
    pub(super) fn tell_relay(&self, fields: Value) {
        let Some(address) = self.with(|i| Ok(i.r.db().setting("address"))).ok().flatten() else {
            return tracing::debug!("telling the relay: {YOURS} has no address yet");
        };
        let key = ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes());
        let url = format!("{}/relay/tell", address.trim_end_matches('/'));
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(1);
            for _ in 0..5 {
                let at = crate::keys::next_at(now_ms());
                let body = crate::keys::signed_request(&key, "one/relay", fields.clone(), at);
                let url = url.clone();
                let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
                let sent = tokio::task::spawn_blocking(move || {
                    let mut resp = agent.post(&url).send_json(&body)?;
                    let said: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
                    anyhow::Ok((resp.status().as_u16(), said["error"].as_str().unwrap_or("").to_string()))
                })
                .await;
                match sent {
                    Ok(Ok((code, _))) if code < 300 => return,
                    // Another process of this device signed in the same millisecond: again at once, with a new `at`.
                    Ok(Ok((_, said))) if crate::keys::taken_already(&said) => {
                        tokio::time::sleep(crate::keys::again_in(at)).await;
                        continue;
                    }
                    Ok(Ok((code, _))) if code < 500 => return tracing::debug!("the relay refused what this device told it ({code})"),
                    Ok(Ok(_)) => tracing::debug!("telling the relay: it had a problem"),
                    Ok(Err(err)) => tracing::debug!("telling the relay: {err}"),
                    Err(err) => tracing::debug!("telling the relay: {err}"),
                }
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
        });
    }
}

/// What a computer joining shows the relay for a pairing code's `secret`: not the secret, which admits it.
pub(super) fn pairing_ticket(secret: &[u8; 16]) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::new().chain_update(b"relay pairing:").chain_update(secret).finalize())
}

/// What a device that made a pairing code tells the relay of it: the hash of its ticket.
pub(super) fn ticket_hash(ticket: &str) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(ticket.as_bytes()))
}

/// A stream a client device opened, as one connection to the local API.
struct Stream {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

impl tokio::io::AsyncRead for Stream {
    fn poll_read(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut self.recv), cx, buf)
    }
}

impl tokio::io::AsyncWrite for Stream {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut self.send), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device at its connections' cap gets a new one by an old idle one going, never by holding more: all busy, it's refused.
    #[test]
    fn a_device_at_its_cap_replaces_its_oldest_idle_connection_and_never_exceeds_it() {
        assert_eq!(oldest_idle([2, 0, 1, 0].into_iter()), Some(1));
        assert_eq!(oldest_idle([1; CLIENT_CONNS].into_iter()), None);
    }
}
