//! iroh, how devices reach each other: each is an endpoint whose id is its device key, reached directly where a path
//! opens (a LAN, a tailnet, a port its router maps) and otherwise through the relay at the network's address
//! (relay/), its only relay, which lets in only the network's endpoints. iroh's QUIC is end to end and proves
//! both ends' keys, so what devices say needs no sealing of its own: an ask is one stream, its answer the same stream
//! back. Devices are found by the network's list of them, never n0's discovery.

use ::custom_build::config::YOURS;
use ::custom_build::protocol::message as M;
use iroh::endpoint::{Connection, QuicTransportConfig};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, SecretKey};

use ::api::runtime::NotThere;
use ::api::net::{CONNECT, Endpoints, MESSAGE_LIMIT, alpn, relay};
use super::*;

/// How long a device that isn't on the relay and hasn't talked with this one lately (`Network::reachable`) is waited
/// for: it may have just started, but mostly it's off, and a call shouldn't wait out `CONNECT` for it.
const UNSEEN_CONNECT: Duration = Duration::from_secs(3);
/// How long after one like that couldn't be reached it isn't tried again, unless it's on the relay again.
const DOWN_FOR: Duration = Duration::from_secs(20);
/// How long a request to the relay may take.
const RELAY_WAIT: Duration = Duration::from_secs(15);
/// How long a `changed` waits to be taken before it's sent again on a new connection.
const CHANGED_WAIT: Duration = Duration::from_millis(500);
/// The byte an ask's answer starts with: the call started on the device asked (past its lease, for one that writes), or
/// it didn't, and its answer follows.
const TAKEN: u8 = 1;
const NOT_TAKEN: u8 = 0;
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
/// How long a client device is waited for when this node asks it for something, and the most it may answer.
const CLIENT_ASK: Duration = Duration::from_secs(10);
const CLIENT_ANSWER: usize = 128 << 10;
/// How long a connection, a device's or a client's, stays open with nothing asked on it. Nothing keeps one open to say
/// a device is there: iroh's path pings would keep it up forever, so it's closed here (`close_idle`).
pub(super) const IDLE: Duration = Duration::from_secs(3 * 60);
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
    /// How many streams each device's connection has open, and when its last one ended: `close_idle`.
    streams: Mutex<HashMap<Key, (usize, Instant)>>,
    /// When each device that wasn't reachable couldn't be reached (`DOWN_FOR`).
    down: Mutex<HashMap<Key, Instant>>,
    /// How many computers that aren't the network's are connected, asking to join.
    joining: std::sync::atomic::AtomicUsize,
    /// The removed devices this device told the relay to take out.
    removed: Mutex<std::collections::HashSet<Key>>,
    /// Whose keys this device last told the relay are admins', as who's who says (`tell_roles`).
    admins: Mutex<HashMap<Key, bool>>,
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
struct Held(Host, Key, Connection);

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

/// What client devices of this custom build speak to a node: HTTP/1.1, one request on each stream, to its local API.
fn http_alpn() -> Vec<u8> {
    format!("{}/http/1", ::custom_build::config::CLI).into_bytes()
}

impl Host {
    /// Keeps this device's endpoint, once it's a device of a network with an address.
    pub(super) fn follow_net(&self) {
        self.tell_removed();
        self.tell_roles();
        let wanted = self.with_or(None, |i| Ok(i.r.db().compute(&self.0.id)?.and(i.r.db().setting("address")?)));
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

    /// Closes this device's endpoint as it stops, telling every device connected to it, which then connects to whatever
    /// starts in its place at once, rather than waiting out this one's silence.
    pub(super) async fn close_net(&self) {
        if let Some((_, task)) = self.0.net.task.lock().unwrap().take() {
            task.abort();
        }
        let endpoint = self.0.net.endpoint.lock().unwrap().take();
        if let Some((_, endpoint)) = endpoint {
            endpoint.close().await;
        }
    }

    /// Where this device's endpoint is reached now: its key, its relay and the addresses it's bound to, once it has one.
    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.0.net.endpoint.lock().unwrap().as_ref().map(|(_, e)| e.addr())
    }

    /// Binds this device's endpoint, then takes the connections other devices open, until it's replaced. One that
    /// closes (the relay or the network went away under it) is bound again.
    async fn keep_net(self, address: String) {
        loop {
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
                let host = self.clone();
                tokio::spawn(async move {
                    let conn = match incoming.accept() {
                        Ok(accepting) => accepting.await,
                        Err(err) => return tracing::debug!("a connection that didn't open: {err:#}"),
                    };
                    match conn {
                        Ok(conn) => host.take_conn(conn).await,
                        Err(err) => tracing::debug!("a connection that didn't open: {err:#}"),
                    }
                });
            }
            tracing::warn!("this device's endpoint closed: binding it again");
            self.0.net.conns.lock().unwrap().clear();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn bind(&self, address: &str, ticket: Option<&str>) -> Result<Endpoint> {
        let (_, map) = relay(address, ticket)?;
        let transport = QuicTransportConfig::builder().max_concurrent_bidi_streams(STREAMS.into()).max_concurrent_uni_streams(0u32.into()).build();
        let endpoint = Endpoint::builder(Endpoints)
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
            let host = self.clone();
            let streaming = self.streaming(&remote);
            tokio::spawn(async move {
                let _streaming = streaming;
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what {} sent: {err:#}", short(&remote)),
                };
                host.with_or(None, |i| Ok(i.exchanged.insert(remote, Instant::now())));
                let mut answer = match msg["kind"].as_str() {
                    Some(M::ASK) => {
                        let asked = msg["body"]["trace"] == true;
                        let t = trace::Trace::continuing(asked, msg["body"]["traceparent"].as_str());
                        let (taken, mut told) = tokio::sync::oneshot::channel();
                        let call = super::calls::telling(taken, trace::within(Some(t.clone()), host.accept_call(remote, &msg["body"]["call"])));
                        tokio::pin!(call);
                        // The answer starts with whether the call started here (`TAKEN`), as soon as it has.
                        let (reply, said) = tokio::select! {
                            reply = &mut call => (reply, false),
                            Ok(()) = &mut told => {
                                if let Err(err) = send.write_all(&[TAKEN]).await {
                                    return tracing::debug!("answering {}: {err:#}", short(&remote));
                                }
                                (call.await, true)
                            }
                        };
                        if !said {
                            let byte = if told.try_recv().is_ok() { TAKEN } else { NOT_TAKEN };
                            if let Err(err) = send.write_all(&[byte]).await {
                                return tracing::debug!("answering {}: {err:#}", short(&remote));
                            }
                        }
                        let mut answer = reply.json();
                        if asked {
                            answer["trace"] = t.done(&host.device_name());
                        }
                        answer
                    }
                    Some(M::STATE) => host.state_asked(),
                    Some(M::CHANGED) => {
                        host.changed_from(&msg["body"]);
                        Value::Null
                    }
                    Some(M::SERVICE) => host.carry(&msg["body"]).await,
                    _ => json!({ "error": "that isn't something devices ask each other" }),
                };
                // Every answer says how loaded this device is, for the asker's routing: no device says so unasked.
                if let Value::Object(said) = &mut answer {
                    said.insert("load".into(), json!(::devices::load::current()));
                    said.insert("updating".into(), json!(host.updating()));
                }
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering {}: {err:#}", short(&remote));
                }
            });
        }
    }

    async fn take_cloudflare_node(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            if !self.is_cloudflare_node(&remote) {
                return conn.close(1u32.into(), b"not this network's Cloudflare node");
            }
            let host = self.clone();
            tokio::spawn(async move {
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what the Cloudflare node sent: {err:#}"),
                };
                let answer = match msg["kind"].as_str() {
                    Some(M::ASK) => crate::api::call_from_cloudflare_node(&host, remote, msg["body"]["call"].clone()).await,
                    _ => json!({ "error": "the Cloudflare node only asks for calls" }),
                };
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering the Cloudflare node: {err:#}");
                }
            });
        }
    }

    /// Answers a client device's requests to the local API, as its user's (`api::FromClient`), each stream one
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
        let vouched: Arc<Mutex<Option<(String, key::Session)>>> = Arc::default();
        if !listed {
            let (host, waiting, shown, found) = (self.clone(), conn.clone(), vouched.clone(), found.clone());
            tokio::spawn(async move {
                tokio::time::sleep(UNLISTED_WITHIN).await;
                host.0.net.unlisted.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if shown.lock().unwrap().is_none() && !found.load(std::sync::atomic::Ordering::SeqCst) {
                    waiting.close(1u32.into(), b"not a device of this network");
                }
            });
        }
        let (host, closing) = (self.clone(), conn.clone());
        let api = crate::api::router(self.clone()).layer(axum::middleware::from_fn(move |mut req: axum::extract::Request, next: axum::middleware::Next| {
            let (host, closing, vouched, found) = (host.clone(), closing.clone(), vouched.clone(), found.clone());
            async move {
                // Who asks is the key the connection proved, or the device that vouched for it, and nothing it says of itself.
                for header in [axum::http::header::AUTHORIZATION.as_str(), ::custom_build::protocol::header::LOCAL_TOKEN] {
                    req.headers_mut().remove(header);
                }
                let certificate = req.headers_mut().remove(::custom_build::protocol::header::ENDPOINT);
                let who = if listed || found.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok((remote, u64::MAX))
                } else if certificate.is_none() && host.client_now(&remote).await.is_some() {
                    // A device approved on another, which this one hadn't read yet: it's who its connection proves.
                    found.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok((remote, u64::MAX))
                } else {
                    let shown = certificate.as_ref().and_then(|c| c.to_str().ok());
                    // Its device may be one approved on another, which this one hadn't read yet.
                    match host.vouched(&remote, shown, &vouched) {
                        Err(_) if shown.is_some() => {
                            host.catch_up().await;
                            host.vouched(&remote, shown, &vouched)
                        }
                        who => who,
                    }
                };
                match who {
                    Ok((key, until)) => {
                        req.extensions_mut().insert(crate::api::FromClient(key, until));
                        // What it sends is read in a while and up to a size, but a file's and an app's own requests',
                        // which stream to disk within their user's room (`Host::put_blob_from`).
                        let path = req.uri().path();
                        if !(path == "/api/blobs" || path.starts_with("/api/blobs/") || crate::api::app_route(path)) {
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
            let (send, recv) = match tokio::time::timeout(IDLE, conn.accept_bi()).await {
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

    /// The device a session's endpoint `remote` asks as: the one its certificate names, if it's for `remote`, unexpired,
    /// and by a client device the network still lists, with its `until`. `kept` is the last checked, so a connection
    /// checks one signature.
    fn vouched(&self, remote: &Key, certificate: Option<&str>, kept: &Mutex<Option<(String, key::Session)>>) -> Result<(Key, u64)> {
        let certificate = certificate.with_context(|| format!("that key isn't one of {YOURS}'s devices, and no device vouched for it"))?;
        let now = now_ms();
        let mut kept = kept.lock().unwrap();
        let session = match kept.as_ref().filter(|(c, _)| c == certificate) {
            Some((_, session)) => *session,
            None => {
                let session = key::check_certificate(certificate, now)?;
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
                Some(join) if msg["kind"] == M::ASK => self.joining(remote, join).await,
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
        joined.unwrap_or_else(|err| json!({ "error": format!("{err:#}"), "code": ::custom_build::protocol::Coded::of(&err) }))
    }

    /// Another device's `changed`, that something changed: `{ scope, app?, database?, at?, step?, sig? }`, `step` the sealed step written
    /// at `at` and `sig` the signature beside it, base64.
    fn changed_from(&self, changed: &Value) {
        if let Some(sent) = changed["sent"].as_u64() {
            insights_runtime::write_seen(&self.rt(), sent);
        }
        let Ok(scope) = Scope::parse(changed["scope"].as_str().unwrap_or("")) else { return };
        let wrote = changed["app"].as_str().zip(changed["database"].as_str()).map(|(app, db)| (app, db, changed["at"].as_u64().unwrap_or(0)));
        let bytes = |field: &str| changed[field].as_str().and_then(|s| data_encoding::BASE64.decode(s.as_bytes()).ok());
        self.changed_there(&scope, wrote, bytes("step"), bytes("sig"));
    }

    fn is_device(&self, key: &Key) -> bool {
        self.with_or(false, |i| Ok(i.r.db().compute(key)?.is_some()))
    }

    pub(super) fn is_service(&self, key: &Key) -> bool {
        self.with_or(false, |i| Ok(i.r.db().service(key)?.is_some()))
    }

    /// Whether `key` is the network's Cloudflare node's endpoint, as `cloudflare node deploy` set it.
    pub(super) fn is_cloudflare_node(&self, key: &Key) -> bool {
        self.with_or(false, |i| Ok(i.r.db().setting(scope::scopes::CLOUDFLARE_NODE)?.is_some_and(|k| k == hex(key))))
    }

    /// Closes the connections of devices the network no longer lists: client devices' to the local API, and those
    /// held to devices.
    pub(super) fn close_removed(&self) {
        let (clients, held): (Vec<Key>, Vec<Key>) = (self.0.net.clients.lock().unwrap().keys().copied().collect(), self.0.net.conns.lock().unwrap().keys().copied().collect());
        let (gonets, gone_held): (Vec<Key>, Vec<Key>) = self
            .with(|i| {
                let m = i.r.db();
                // A session's endpoint, listed nowhere, is cut off at its next request instead.
                let mut gonets = Vec::new();
                for k in clients {
                    if m.machine_ever(&k)?.is_some() && m.client(&k)?.is_none() {
                        gonets.push(k);
                    }
                }
                let mut gone_held = Vec::new();
                for k in held {
                    if m.compute(&k)?.is_none() && m.setting(scope::scopes::CLOUDFLARE_NODE)?.is_none_or(|s| s != hex(&k)) {
                        gone_held.push(k);
                    }
                }
                Ok((gonets, gone_held))
            })
            // What can't be read closes nothing; the next look tries again.
            .inspect_err(|err| tracing::warn!("closing removed devices' connections: {err:#}"))
            .unwrap_or_default();
        for key in gonets {
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
        // One that isn't on the relay, nor talked with this one lately, is mostly off: it isn't waited for as long, nor tried again for a while.
        let seen = self.with(|i| Ok(i.reachable(to))).unwrap_or(true);
        if !seen && self.0.net.down.lock().unwrap().get(to).is_some_and(|at| at.elapsed() < DOWN_FOR) {
            bail!(NotThere(self.away(to)));
        }
        let wait = if seen { CONNECT } else { UNSEEN_CONNECT };
        // Off the relay, a device is reached only on a connection it opened: one just started waits to be on it.
        let waiting = std::time::Instant::now();
        if tokio::time::timeout(CONNECT, endpoint.online()).await.is_err() {
            bail!(NotThere(format!("this device isn't connected to {YOURS}'s relay")));
        }
        trace::note(json!({ "step": "online" }), waiting.elapsed());
        let (url, _) = relay(&address, None)?;
        let addr = EndpointAddr::new(PublicKey::from_bytes(to)?).with_relay_url(url);
        let connecting = std::time::Instant::now();
        let conn = match tokio::time::timeout(wait, endpoint.connect(addr, &alpn())).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => {
                self.0.net.down.lock().unwrap().insert(*to, Instant::now());
                bail!(NotThere(format!("{} couldn't be reached: {err:#}", self.name_of(to))))
            }
            Err(_) => {
                self.0.net.down.lock().unwrap().insert(*to, Instant::now());
                bail!(NotThere(self.away(to)))
            }
        };
        self.0.net.down.lock().unwrap().remove(to);
        trace::note(json!({ "step": "connect", "to": short(to) }), connecting.elapsed());
        *held = Some(conn.clone());
        // The other end asks on it too, once it keeps it for its asks (take_conn).
        let (host, to, serving) = (self.clone(), *to, conn.clone());
        tokio::spawn(async move { host.serve_device(to, serving).await });
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

    /// Says in this device's log how each connection it holds goes: each path, whether it's the one sent on, and the
    /// packets and bytes it has carried each way, as iroh counts them. Every packet through the relay is billed by
    /// Cloudflare, so this is what says which devices reach each other directly (`one logs <device> --grep paths`).
    pub(super) fn log_paths(&self) {
        let mut conns: Vec<(Key, Connection)> = self.0.net.conns.lock().unwrap().iter().filter_map(|(k, slot)| Some((*k, slot.try_lock().ok()?.clone()?))).collect();
        conns.extend(self.0.net.clients.lock().unwrap().iter().flat_map(|(k, held)| held.iter().map(|(c, _)| (*k, c.clone()))));
        for (key, conn) in conns.into_iter().filter(|(_, c)| c.close_reason().is_none()) {
            let paths: Vec<String> = conn
                .paths()
                .iter()
                .map(|p| {
                    let s = p.stats();
                    let way = if p.is_relay() { "relay".to_string() } else { p.remote_addr().to_string() };
                    let on = if p.is_selected() { " (sent on)" } else { "" };
                    format!("{way}{on} {}ms: sent {} packets, {} B; got {} packets, {} B", s.rtt.as_millis(), s.udp_tx.datagrams, s.udp_tx.bytes, s.udp_rx.datagrams, s.udp_rx.bytes)
                })
                .collect();
            tracing::info!("paths to {} {}: {}", self.name_of(&key), short(&key), paths.join("; "));
        }
    }

    /// One more stream on the connection to `to`, until what this gives is dropped.
    fn streaming(&self, to: &Key) -> Streaming {
        self.0.net.streams.lock().unwrap().entry(*to).or_insert((0, Instant::now())).0 += 1;
        Streaming(self.clone(), *to)
    }

    /// Closes each connection to another device that had no stream open for `IDLE`: a call or a `changed` opens one
    /// again.
    pub(super) fn close_idle(&self) {
        let idle: Vec<Key> = self.0.net.streams.lock().unwrap().iter().filter(|(_, (open, since))| *open == 0 && since.elapsed() >= IDLE).map(|(k, _)| *k).collect();
        for key in idle {
            self.0.net.streams.lock().unwrap().remove(&key);
            if let Some(slot) = self.0.net.conns.lock().unwrap().remove(&key)
                && let Ok(held) = slot.try_lock()
                && let Some(conn) = held.as_ref()
            {
                conn.close(0u32.into(), b"idle");
            }
        }
    }

    fn drop_conn(&self, to: &Key) {
        self.0.net.conns.lock().unwrap().remove(to);
    }

    /// Fills this device's copy of each database it has open from the bucket now: a connection to another device just
    /// opened, and what it said while there was none was lost.
    fn caught_up(&self) {
        let host = self.clone();
        tokio::spawn(async move { host.fill_copies(None).await });
    }

    /// A stream of its own to `to`, on the connection open or, if that one's gone, a new one, with `kind`'s message
    /// sent on it.
    async fn stream_to(&self, to: &Key, kind: &str, body: &Value) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
        let msg = json!({ "kind": kind, "body": body }).to_string();
        ensure!(msg.len() <= MESSAGE_LIMIT, "it's {} MB, more than another device is sent at once ({} MB)", msg.len() >> 20, MESSAGE_LIMIT >> 20);
        let mut fresh = false;
        let (mut send, recv) = loop {
            let conn = self.conn(to, fresh).await?;
            match conn.open_bi().await {
                Ok(stream) => break stream,
                Err(_) if !fresh => fresh = true,
                Err(err) => bail!(NotThere(format!("{} couldn't be reached: {err:#}", self.name_of(to)))),
            }
        };
        send.write_all(msg.as_bytes()).await?;
        send.finish()?;
        Ok((send, recv))
    }

    /// Sends `to` a message of `kind`: → its answer, waiting `limit` for it, if `answered`.
    pub(super) async fn send(&self, to: &Key, kind: &str, body: &Value, answered: bool, limit: Duration) -> Result<Value> {
        let _streaming = self.streaming(to);
        let (_send, mut recv) = self.stream_to(to, kind, body).await?;
        if !answered {
            return Ok(Value::Null);
        }
        let answer = tokio::time::timeout(limit, recv.read_to_end(MESSAGE_LIMIT)).await.context("timed out")??;
        let answer: Value = serde_json::from_slice(&answer)?;
        self.answered_by(to, &answer);
        Ok(answer)
    }

    /// What another device's answer says of it, kept for routing: how loaded it is, how far, and that the two talked.
    fn answered_by(&self, to: &Key, answer: &Value) {
        let far = self.round_trip(to);
        self.with_or(None, |i| {
            if let Some(far) = far {
                i.far.insert(*to, far);
            }
            if answer.get("load").is_some() {
                i.said.insert(*to, Said { load: serde_json::from_value(answer["load"].clone()).ok(), updating: answer["updating"].as_str().map(str::to_string) });
            }
            Ok(i.exchanged.insert(*to, Instant::now()))
        });
    }

    /// Asks the client device `to` for its log, on a connection it opened to this node (`take_client`), where a client
    /// answers whatever a node asks on one (onet/src/lib.rs's `Asked`): → its answer, or None if it isn't connected here.
    pub(super) async fn ask_client(&self, to: &Key, ask: &Value) -> Result<Option<Value>> {
        let conn = self.0.net.clients.lock().unwrap().get(to).and_then(|open| open.iter().rev().map(|(c, _)| c).find(|c| c.close_reason().is_none()).cloned());
        let Some(conn) = conn else { return Ok(None) };
        let (mut send, mut recv) = tokio::time::timeout(CLIENT_ASK, conn.open_bi()).await.context("the client isn't taking requests")??;
        send.write_all(ask.to_string().as_bytes()).await?;
        send.finish()?;
        let answer = tokio::time::timeout(CLIENT_ASK, recv.read_to_end(CLIENT_ANSWER)).await.context("the client didn't answer")??;
        Ok(Some(serde_json::from_slice(&answer)?))
    }

    /// Asks another device to run a call, waiting `limit` for its answer, and tells `taken` whether it started there as
    /// soon as that device says. An error that's a `NotThere` means it never reached
    /// that device.
    pub(super) async fn ask(&self, to: Key, call: Value, limit: Duration, taken: impl FnOnce(bool)) -> Result<Value> {
        // A traced request asks that device for its own part too, and any call says whose span it's under.
        let mut body = json!({ "call": call });
        if trace::on() {
            body["trace"] = json!(true);
        }
        if let Some(parent) = trace::traceparent() {
            body["traceparent"] = json!(parent);
        }
        let _streaming = self.streaming(&to);
        let answer = tokio::time::timeout(limit, async {
            let (_send, mut recv) = self.stream_to(&to, M::ASK, &body).await?;
            let mut first = [0u8];
            recv.read_exact(&mut first).await?;
            taken(first[0] == TAKEN);
            anyhow::Ok(recv.read_to_end(MESSAGE_LIMIT).await?)
        });
        let answer = answer.await.context("timed out")??;
        let answer: Value = serde_json::from_slice(&answer)?;
        self.answered_by(&to, &answer);
        Ok(answer)
    }

    /// Tells each of `to` that something changed, expecting no answer.
    pub(super) fn notify(&self, to: &[Key], changed: &Value) {
        let mut changed = changed.clone();
        changed["sent"] = json!(now_ms());
        for device in to.iter().copied() {
            let (host, body) = (self.clone(), changed.clone());
            tokio::spawn(async move {
                // Answered, so one lost on a connection that died unseen is sent again on a new one.
                if let Err(err) = host.send(&device, M::CHANGED, &body, true, CHANGED_WAIT).await {
                    host.drop_conn(&device);
                    if let Err(err) = host.send(&device, M::CHANGED, &body, true, CHANGED_WAIT).await {
                        tracing::debug!("telling {}: {err:#}", short(&device));
                    }
                    tracing::debug!("telling {}, tried again: {err:#}", short(&device));
                }
            });
        }
    }

    /// Tells the relay to take out the devices removed from the network, if this device is an admin's: the relay lets
    /// in only its list, which nothing on it can read the network's log for.
    fn tell_removed(&self) {
        let Ok(removed) = self.with(|i| {
            let m = i.r.db();
            let mine = m.machine(&self.0.id)?.is_some_and(|me| m.may(&me.user, "invite", scopes::NETWORK));
            Ok(if mine { m.machines()?.into_iter().filter(|d| d.removed).map(|d| d.key).collect::<Vec<Key>>() } else { Vec::new() })
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

    /// Tells the relay whose keys are admins' now, if this device is an admin's: it decides by that who may change its
    /// list, and can't read who's who. What it hasn't been told since this device started, and each change after that.
    fn tell_roles(&self) {
        let Ok(wanted) = self.with(|i| {
            let m = i.r.db();
            let mine = m.machine(&self.0.id)?.is_some_and(|me| m.may(&me.user, "invite", scopes::NETWORK));
            Ok(if mine { m.machines()?.into_iter().filter(|d| !d.removed && d.service.is_none()).map(|d| (d.key, m.may(&d.user, "invite", scopes::NETWORK))).collect() } else { HashMap::new() })
        }) else {
            return;
        };
        let mut told = self.0.net.admins.lock().unwrap();
        let changed: serde_json::Map<String, Value> = wanted.iter().filter(|(key, admin)| told.get(*key) != Some(*admin)).map(|(key, admin)| (hex(key), json!(admin))).collect();
        if !changed.is_empty() {
            self.tell_relay(json!({ "admins": changed }));
        }
        *told = wanted;
    }

    /// Whether that device didn't answer lately, so it's tried after those that may.
    pub(super) fn down(&self, device: &Key) -> bool {
        self.0.net.down.lock().unwrap().get(device).is_some_and(|at| at.elapsed() < DOWN_FOR)
    }

    /// Whether that device can be reached now: what a guard tries before it says the device is off.
    pub(super) async fn reaches(&self, device: &Key) -> bool {
        self.conn(device, false).await.is_ok()
    }

    /// That device didn't answer in its usual time (`down`), or did.
    pub(super) fn set_down(&self, device: Key, down: bool) {
        let mut all = self.0.net.down.lock().unwrap();
        match down {
            true => all.insert(device, Instant::now()),
            false => all.remove(&device),
        };
    }

    /// Reads which of the network's devices are connected to the relay. What it last said stands until it answers.
    pub(super) fn read_connected(&self) {
        let Some(address) = self.with_or(None, |i| Ok(i.r.db().setting("address")?)) else { return };
        let key = ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes());
        let url = format!("{}/relay/computes", address.trim_end_matches('/'));
        let host = self.clone();
        // What a device is on is its device's to say: an app runtime of its own key is on its device's build.
        let mut asked = json!({ "computes": true, "reads": ::custom_build::protocol::READS });
        if matches!(self.0.holds, holds::Holds::Key(_)) {
            asked["version"] = json!(::custom_build::protocol::version());
        }
        tokio::spawn(async move {
            let body = key::signed_request(&key, ::custom_build::protocol::RELAY, asked, key::next_at(now_ms()));
            let said = match post_relay(url, body).await {
                Ok((200, said)) => said,
                // Too old to be told anything else, it can still be asked to update.
                Ok((426, said)) => return host.heard_ask(&said["asked"]),
                Ok((code, _)) => return tracing::debug!("reading who's connected: the relay answered {code}"),
                Err(err) => return tracing::debug!("reading who's connected: {err:#}"),
            };
            host.heard_ask(&said["asked"]);
            host.with_or((), |i| {
                i.versions = said["versions"].clone();
                i.workers = said["workers"].clone();
                i.moved = said["moved"].is_object().then(|| (Instant::now(), said["moved"].clone()));
                Ok(())
            });
            let connected: std::collections::HashSet<Key> = said["online"].as_array().into_iter().flatten().filter_map(|k| key::unhex(k.as_str()?).ok()).collect();
            let seen: HashMap<Key, u64> = said["seen"].as_object().into_iter().flatten().filter_map(|(k, at)| Some((key::unhex(k).ok()?, at.as_u64()?))).collect();
            let was = host.with_or(None, |i| {
                i.last_seen = seen;
                let was = std::mem::replace(&mut i.connected, connected.clone());
                Ok((was != connected).then_some(was))
            });
            if let Some(was) = was {
                // One whose socket opened again since is tried again at once.
                host.0.net.down.lock().unwrap().retain(|k, _| was.contains(k) || !connected.contains(k));
                insights_runtime::relay_says(&host.rt(), &was, &connected);
                host.emit(Event::Devices);
            }
        });
    }

    /// Tells the relay at the network's address something, signed by this device's key (`holds`): a device it admitted
    /// (`add`), or a pairing code it made (`pairing`, the hash of its ticket, and `until`). Tried a few times, in the
    /// background.
    pub(super) fn tell_relay(&self, fields: Value) {
        let Some(address) = self.with_or(None, |i| Ok(i.r.db().setting("address")?)) else {
            return tracing::debug!("telling the relay: {YOURS} has no address yet");
        };
        let host = self.clone();
        let url = format!("{}/relay/tell", address.trim_end_matches('/'));
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(1);
            for _ in 0..5 {
                let at = key::next_at(now_ms());
                let body = match key::signed_request_by(&host.0.holds, ::custom_build::protocol::RELAY, fields.clone(), at).await {
                    Ok(body) => body,
                    Err(err) => return tracing::debug!("telling the relay: {err:#}"),
                };
                match post_relay(url.clone(), body).await {
                    Ok((code, _)) if code < 300 => return,
                    // Another process of this device signed in the same millisecond: again at once, with a new `at`.
                    Ok((_, said)) if said["code"] == ::custom_build::protocol::code::REPLAYED => {
                        tokio::time::sleep(key::again_in(at)).await;
                        continue;
                    }
                    Ok((code, _)) if code < 500 => return tracing::debug!("the relay refused what this device told it ({code})"),
                    Ok(_) => tracing::debug!("telling the relay: it had a problem"),
                    Err(err) => tracing::debug!("telling the relay: {err:#}"),
                }
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
        });
    }
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

/// Posts to the relay: → what it answered. A slow relay gives up before the next ask.
/// A stream open on the connection to a device, for `close_idle`.
struct Streaming(Host, Key);

impl Drop for Streaming {
    fn drop(&mut self) {
        if let Some((open, since)) = self.0.0.net.streams.lock().unwrap().get_mut(&self.1) {
            *open = open.saturating_sub(1);
            *since = Instant::now();
        }
    }
}

async fn post_relay(url: String, body: Value) -> Result<(u16, Value)> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(RELAY_WAIT)).build().into();
    tokio::task::spawn_blocking(move || {
        let request = ureq::http::Request::post(&url).header("content-type", "application/json").body(serde_json::to_vec(&body)?)?;
        let (mut resp, _) = platform::hedge::run(&agent, "relay", request, platform::hedge::Twice::Never)?;
        let said: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        anyhow::Ok((resp.status().as_u16(), said))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_at_its_cap_replaces_its_oldest_idle_connection_and_never_exceeds_it() {
        assert_eq!(oldest_idle([2, 0, 1, 0].into_iter()), Some(1));
        assert_eq!(oldest_idle([1; CLIENT_CONNS].into_iter()), None);
    }
}
