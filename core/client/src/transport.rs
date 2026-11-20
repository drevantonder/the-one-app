//! How a client reaches servers and is reached by them: its endpoint, its connection to a server (`Reached`), which
//! servers act for its user, and what a server says or asks over a connection it opens.

use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use futures_util::future::join_all;
use futures_util::stream::FuturesUnordered;
use iroh::endpoint::{QuicTransportConfig, ReadError, RecvStream, RelayMode, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, NetReportConfig, RelayConfig, SecretKey, TransportAddr};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};

use crate::{Client, hex, auth::Network, rpc::{UNANSWERED, NOBODY}, state::State};

/// What servers and clients speak.
pub const ALPN: &[u8] = b"one/mcp/1";

/// The most one message may carry: an app's files fit.
pub const LIMIT: usize = 64 << 20;

/// How long reaching a server may take: long enough for a phone to find the relay and be let in, and for a first
/// packet that's lost to be sent again.
pub const CONNECT: Duration = Duration::from_secs(3);

/// How long a server that's there takes, at most, to say so. Nothing is sent over a connection that's asked nothing,
/// so a server that has gone is found out by asking it, and this soon. A server taken for gone that wasn't is only
/// asked again over a new connection, or has its lease taken by a standby, and neither loses anything. A server
/// running what it was asked says so this often too, as MCP's progress notifications, on the request's stream.
pub const THERE: Duration = Duration::from_secs(1);

/// What a server ends a stream with when it doesn't answer what was asked on it: `REFUSED` where it took nothing of
/// it, as one that's stopping takes nothing new, and `STOPPED` where it went away after it took it, so it may have
/// done it.
pub const REFUSED: VarInt = VarInt::from_u32(1);

pub const STOPPED: VarInt = VarInt::from_u32(2);

/// How long a connection that's asked nothing stays open. To keep one open, something has to be said over it every
/// few seconds, and through a relay each of those wakes the relay. So a connection is let go once it's idle, and
/// made again when there's something to ask.
const IDLE: Duration = Duration::from_secs(2);

/// How long a connection lasts once nothing arrives over it, which is how a server that went away while it ran a
/// call, or before it answered one just sent, is found out: both ends send a QUIC keep-alive every `THERE`, which
/// the other's QUIC acknowledges at once, and one that's running a call says so every `THERE` too. So a few of
/// those missed in a row is a machine that's gone, and whatever was asked of it is asked elsewhere this soon after
/// it went. QUIC takes the shorter of the two sides' `SILENT`. Whoever opens a connection closes it once it's
/// `IDLE`, and one opened by a server that stopped lasts this long on the other side. Meanwhile iroh sends what it
/// has for that server only where that connection was, so a server that starts again listens on the port it did
/// (`one_core_iroh`'s `start`).
const SILENT: Duration = Duration::from_secs(4);

/// An endpoint holding `key`. It reaches others, and is reached, by key: straight to the addresses it's told and the
/// ones it finds, and otherwise through the relay at its network's address, which is also the only thing it times.
/// With no other relay and no address lookup, nothing about a network goes to anyone outside it. A network on one
/// machine has no address, and so no relay. It listens on `port`, or on any that's free with `0`. A server's NIF
/// binds this same endpoint.
pub async fn endpoint(key: SecretKey, network: Option<&Network>, port: u16) -> Result<Endpoint> {
    let mut probes = NetReportConfig::minimal();
    let relay = match network {
        Some(Network { address, token }) => {
            let address = address.parse().with_context(|| format!("{address} isn't a network's address"))?;
            // iroh takes the relay it timed as its own, so with none timed it would have none.
            probes.https_probes = true;
            RelayMode::Custom(RelayConfig::new(address, None).with_auth_token(token).into())
        }
        None => RelayMode::Disabled,
    };
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .transport_config(
            QuicTransportConfig::builder()
                .max_idle_timeout(Some(SILENT.try_into()?))
                .keep_alive_interval(THERE)
                // The Android emulator's network drops a batch of UDP datagrams sent as one (GSO) without an error, so
                // a direct path there passes its handshake and pings but no answer of any size: a phone's datagrams
                // go one at a time, which its radio's speed doesn't notice.
                .enable_segmentation_offload(!cfg!(target_os = "android"))
                .build(),
        )
        .relay_mode(relay)
        .net_report_config(probes)
        .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?
        .bind()
        .await
        .context("couldn't open a socket")
}

/// Where `endpoint` is reached, as another is told it: its key, the machine's addresses on its networks, and the
/// relay at its network's address, which it's told now, before the endpoint has connected there. With no network,
/// whatever reaches it is on this machine, by its loopback address. With one, it isn't told that: on another machine
/// that's another machine's.
pub fn address(endpoint: &Endpoint, network: Option<&Network>) -> Result<EndpointAddr> {
    match network {
        Some(Network { address: at, .. }) => {
            Ok(endpoint.addr().with_relay_url(at.parse().with_context(|| format!("{at} isn't an address"))?))
        }
        None => {
            let here = endpoint.bound_sockets().into_iter().filter(SocketAddr::is_ipv4);
            Ok(here.fold(endpoint.addr(), |address, socket| {
                address.with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, socket.port())))
            }))
        }
    }
}

/// Why a server didn't answer what it was asked.
#[derive(Debug, PartialEq)]
pub enum RequestError {
    /// It couldn't be reached, so it was asked nothing.
    Unreachable,
    /// It stopped answering after it was asked, so it may have done what was asked.
    Stopped,
}

/// An open connection to a server, and when the server last answered on it. A client asks a server over one, and a
/// server asks another the same way. `gone` is whether the server was found not to be there, or to be going: it
/// stopped answering, or refused what it was asked, so nothing more is asked over it.
pub struct Reached {
    connection: iroh::endpoint::Connection,
    /// The run's attestation for the endpoint it was made from (`Link`), which what's asked over it says: a run made
    /// again (`relink`) has another, which a server checks against the connection's key, so one said over a connection
    /// of the run before has the server find the device nobody's.
    pub(crate) attestation: String,
    received: Arc<std::sync::Mutex<Instant>>,
    asking: Arc<AtomicUsize>,
    gone: AtomicBool,
}

/// One message being asked over a connection, for as long as it's held: a connection that's asking isn't idle.
struct Asking(Arc<AtomicUsize>);

impl Asking {
    fn new(asking: &Arc<AtomicUsize>) -> Self {
        asking.fetch_add(1, Ordering::Relaxed);
        Self(asking.clone())
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Why a server wasn't reached.
#[derive(Debug)]
pub enum Unreached {
    /// Nothing answered within that long.
    TimedOut(Duration),
    /// The connection failed, as QUIC or iroh says.
    Failed(String),
}

impl std::fmt::Display for Unreached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut(within) => write!(f, "nothing answered within {} ms", within.as_millis()),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// How `endpoint` reaches the relay at its network's address now: `connected`, or not with the last error, or `none`
/// with no network. Without it, a server that can't be reached directly isn't reached at all.
pub fn relay(endpoint: &Endpoint) -> String {
    use iroh::Watcher;
    let statuses = endpoint.home_relay_status().get();
    if statuses.is_empty() {
        return "none".into();
    }
    let said = statuses.iter().map(|status| match (status.is_connected(), status.last_error()) {
        (true, _) => "connected".to_string(),
        (false, Some(error)) => format!("not connected: {error:#}"),
        (false, None) => "not connected yet".to_string(),
    });
    said.collect::<Vec<_>>().join("; ")
}

impl Reached {
    /// Connects to the server at `address`, or says why not if it doesn't answer `within` that long.
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr, within: Duration) -> Result<Self, Unreached> {
        let connection = match timeout(within, endpoint.connect(address, ALPN)).await {
            Err(_) => return Err(Unreached::TimedOut(within)),
            Ok(Err(error)) => return Err(Unreached::Failed(format!("{error:#}"))),
            Ok(Ok(connection)) => connection,
        };
        let received = Arc::new(std::sync::Mutex::new(Instant::now()));
        let asking = Arc::new(AtomicUsize::new(0));
        let (open, last, busy) = (connection.clone(), received.clone(), asking.clone());
        tokio::spawn(async move {
            while open.close_reason().is_none() {
                let idle = last.lock().expect("no ask panics").elapsed();
                match IDLE.checked_sub(idle) {
                    Some(left) => sleep(left).await,
                    None if busy.load(Ordering::Relaxed) > 0 => sleep(IDLE).await,
                    None => open.close(0u32.into(), b"idle"),
                }
            }
        });
        Ok(Self { connection, received, asking, gone: AtomicBool::new(false), attestation: String::new() })
    }

    pub fn connection(&self) -> &iroh::endpoint::Connection {
        &self.connection
    }

    /// How the connection reaches its server now: `direct`, or through a `relay`.
    pub fn path(&self) -> &'static str {
        match self.connection.paths().iter().find(|path| path.is_selected()) {
            Some(path) if path.is_relay() => "relay",
            Some(_) => "direct",
            None => "none",
        }
    }

    /// Whether the connection is still one to ask over: it hasn't closed, and its server hasn't been found gone.
    pub fn open(&self) -> bool {
        self.connection.close_reason().is_none() && !self.gone()
    }

    /// Whether the server didn't say it was there when it was asked over this connection.
    pub fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    /// Asks the server one message, and gives its answer, handing `notified` each notification the server sends
    /// about it first, as MCP has a server send them before its response: one a line, with the response last. A
    /// server that hasn't answered anything lately is first asked whether it's there. One running a request that
    /// carries a `progressToken` sends a progress notification every `THERE` while it runs, which keeps the
    /// connection from being `IDLE` or `SILENT` however long it takes. One that goes silent, or ends the stream
    /// without an answer, is gone: what's asked over this connection from now on is asked elsewhere, while what was
    /// asked already is still answered.
    pub async fn ask(&self, message: &[u8], mut notified: impl FnMut(&[u8])) -> Result<Vec<u8>, RequestError> {
        if !self.open() || (self.received.lock().expect("no ask panics").elapsed() > THERE && !self.there().await) {
            return Err(RequestError::Unreachable);
        }
        let _asking = Asking::new(&self.asking);
        let mut receive = self.send(message).await.ok_or(RequestError::Unreachable)?;
        let (mut read, mut scanned) = (Vec::new(), 0);
        loop {
            let chunk = receive.read_chunk(LIMIT).await.map_err(|unanswered| {
                self.gone.store(true, Ordering::Relaxed);
                match unanswered {
                    ReadError::Reset(REFUSED) => RequestError::Unreachable,
                    _ => RequestError::Stopped,
                }
            })?;
            *self.received.lock().expect("no ask panics") = Instant::now();
            let Some(chunk) = chunk else { return Ok(read) };
            if read.len() + chunk.len() > LIMIT {
                self.gone.store(true, Ordering::Relaxed);
                return Err(RequestError::Stopped);
            }
            read.extend_from_slice(&chunk);
            // A response is JSON on one line, so whatever ends in a newline before it is a notification.
            while let Some(end) = read[scanned..].iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = read.drain(..=scanned + end).collect();
                notified(&line[..line.len() - 1]);
                scanned = 0;
            }
            scanned = read.len();
        }
    }

    /// Whether the server is there: an empty message, which a server answers at once with nothing. One that doesn't
    /// in time is gone, and so is this connection to it.
    async fn there(&self) -> bool {
        let probe = async { self.send(b"").await?.read_to_end(LIMIT).await.ok() };
        let there = timeout(THERE, probe).await.is_ok_and(|response| response.is_some());
        if there {
            *self.received.lock().expect("no ask panics") = Instant::now();
        } else {
            self.gone.store(true, Ordering::Relaxed);
            self.connection.close(0u32.into(), b"gone");
        }
        there
    }

    async fn send(&self, message: &[u8]) -> Option<RecvStream> {
        let (mut send, receive) = self.connection.open_bi().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()?;
        Some(receive)
    }
}

/// The connection to a server, once one is open, and when trying to open one last failed.
#[derive(Default)]
pub(crate) struct Reach {
    pub(crate) open: Option<Arc<Reached>>,
    failed: Option<Instant>,
}

/// A client's endpoint, and its network: the network's address, if it has one, where the client asks which servers
/// act for its user, with the token it shows there. A network on one machine has none, and whatever starts it tells
/// the client where its servers are, in `servers`. `who` is whose device this is, once it's someone's: the user it
/// signed in as.
///
/// The endpoint is this run's, with a key of its own that's made with it and kept nowhere, and `attestation` is the
/// device's word for that key: its signature of `one/run/1`, a line, and the key. A device runs many clients at
/// once, the app, each agent's MCP and every command, and iroh reaches one endpoint by a key, as the relay does: two
/// with one key take each other's place there, and neither is answered. So each run is its own endpoint, and what it
/// shows the relay and says to a server names the device it runs for, with the attestation (`Network::shown`, `say`).
/// The presigner at the address and a server check it (`core/edge/presigner`, `OneCore.User`).
#[derive(Clone)]
pub(crate) struct Link {
    pub(crate) endpoint: Endpoint,
    network: Option<Network>,
    pub(crate) who: Option<String>,
    pub(crate) attestation: String,
}

impl Link {
    /// A new run's link for the device with the key `device`, on its `network`.
    pub(crate) async fn run(device: &SecretKey, network: Option<Network>, who: Option<String>) -> Result<Self> {
        let key = SecretKey::generate();
        let attestation = hex(&device.sign(format!("one/run/1\n{}", key.public()).as_bytes()).to_bytes());
        let shown = network.as_ref().map(|network| network.shown(device, &attestation));
        let endpoint = endpoint(key, shown.as_ref(), 0).await?;
        Ok(Self { endpoint, network, who, attestation })
    }
}

/// Keeps where the server at the other end of `connection` is reached straight, each time that's the path taken.
async fn straight(client: std::sync::Weak<Client>, connection: iroh::endpoint::Connection) {
    let server = connection.remote_id();
    let mut paths = connection.paths_stream();
    while let Some(paths) = paths.next().await {
        let selected = paths.iter().find(|path| path.is_selected()).map(|path| path.remote_addr().clone());
        match (client.upgrade(), selected) {
            (Some(client), Some(TransportAddr::Ip(addr))) => client.reached_straight(server, addr),
            (Some(_), _) => {}
            (None, _) => return,
        }
    }
}

/// What a client tells a server to be told what changes, at the `address` it gives, or with none, no longer.
const WATCH: &str = "notifications/one/watch";

/// What the servers the presigner listed are kept as.
const SERVERS: &str = "presigner/servers";

/// A server with something to say or ask connects to this client to say it, at `endpoint`, until it closes. Only one
/// of the network's own reaches a client, and all it can make one do is ask again, or call a tool of the device's own,
/// which only one acting for the device's person may (`Client::answer`).
pub(crate) fn accept(endpoint: Endpoint, client: std::sync::Weak<Client>) {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let client = client.clone();
            tokio::spawn(async move {
                let Ok(connection) = incoming.await else { return };
                let Some(notifications) = client.upgrade().map(|client| client.notifications.clone()) else { return };
                tokio::spawn(answer(client, connection.clone()));
                receive_notifications(connection, notifications).await
            });
        }
    });
}

/// Answers what a server asks this client over `connection`, each message on a stream of its own, as a server answers
/// a client (`Client::answer`), until the connection ends. An empty message only asks whether it's there.
async fn answer(client: std::sync::Weak<Client>, connection: iroh::endpoint::Connection) {
    let from = connection.remote_id();
    while let Ok((mut send, mut receive)) = connection.accept_bi().await {
        let client = client.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            let Some(client) = client.upgrade().filter(|_| !message.is_empty()) else { return drop(send.finish()) };
            let response = client.answer(from, &message).await.to_string();
            if send.write_all(response.as_bytes()).await.is_ok() {
                let _ = send.finish();
            }
        });
    }
}

/// Tells whoever `connection` is to one message, on a stream of its own, which isn't answered. Gives whether it
/// was sent.
pub async fn notify(connection: &iroh::endpoint::Connection, message: &[u8]) -> bool {
    let sent = async {
        let mut send = connection.open_uni().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()
    };
    sent.await.is_some()
}

/// Passes on what a server tells this client over `connection`, until the connection ends.
async fn receive_notifications(connection: iroh::endpoint::Connection, notifications: broadcast::Sender<Value>) {
    while let Ok(mut receive) = connection.accept_uni().await {
        let notifications = notifications.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            if let Ok(message) = serde_json::from_slice(&message) {
                // With nobody listening, as in the CLI, it's dropped.
                let _ = notifications.send(message);
            }
        });
    }
}

impl Client {
    pub(crate) fn link(&self) -> Link {
        self.link.read().expect("no renewal panics").clone()
    }

    /// Says whether this client is to hear what changes, as the app does while it runs, to every server acting for
    /// its user. While it is, each tells it when something changed, and keeps telling it once it starts again. A
    /// server that isn't up now is told when it's next asked anything, and one that isn't told the app has gone
    /// finds out when it next has something to say. It's said again where a server may not know it: from this run's
    /// next endpoint (`relink`), and once a server answers again after none did, as one that's new may not have heard.
    pub async fn watch(&self, watching: bool) {
        self.watching.store(watching, Ordering::Relaxed);
        if !matches!(*self.state.borrow(), State::SignedIn { .. }) {
            return;
        }
        let Ok(Some(servers)) = self.servers(false).await else { return };
        let watches = servers.into_iter().map(|server| async move {
            if let Some(open) = self.connect(server).await
                && !watching
            {
                let mut unwatch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {}});
                self.say(&mut unwatch);
                let _ = open.ask(unwatch.to_string().as_bytes(), |_| {}).await;
            }
        });
        join_all(watches).await;
    }

    /// Connects to `server`, hears what it tells and asks this client over the connection, and if this client is
    /// watching, says so first, with where a server reaches it and the device it runs for.
    async fn connect(&self, server: EndpointAddr) -> Option<Reached> {
        let link = self.link();
        let mut open = Reached::connect(&link.endpoint, server, CONNECT).await.ok()?;
        open.attestation = link.attestation.clone();
        tokio::spawn(receive_notifications(open.connection().clone(), self.notifications.clone()));
        tokio::spawn(answer(self.me.clone(), open.connection().clone()));
        tokio::spawn(straight(self.me.clone(), open.connection().clone()));
        if self.watching.load(Ordering::Relaxed) {
            // Where it says a server reaches it is its network's relay, so it says so once it's there, as a client
            // that has only just opened, as one a push woke, isn't yet.
            if link.network.is_some() {
                let _ = tokio::time::timeout(CONNECT, link.endpoint.online()).await;
            }
            let here = address(&link.endpoint, link.network.as_ref()).ok()?;
            let mut watch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {"address": here}});
            self.say(&mut watch);
            open.ask(watch.to_string().as_bytes(), |_| {}).await.ok()?;
        }
        Some(open)
    }

    /// What this device answers a server that asked it `message`, `from` being the server's key: a call of a tool of
    /// the device's own (`DeviceTools`), as MCP's `tools/call`, from a server acting for this device's person. Anyone
    /// else of the network is refused, as is anything else.
    async fn answer(&self, from: EndpointId, message: &[u8]) -> Value {
        let request: Value = serde_json::from_slice(message).unwrap_or_default();
        let error = |code: i64, message: String| json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": code, "message": message}});
        let ours = matches!(self.servers(false).await, Ok(Some(servers)) if servers.iter().any(|server| server.id == from));
        if !ours {
            return error(NOBODY, "Only a server acting for this device's person asks it".into());
        }
        let name = request["params"]["name"].as_str().unwrap_or_default();
        let found = self.tools.iter().find_map(|tools| Some((tools, name.strip_prefix(tools.app())?.strip_prefix('_')?)));
        let Some((tools, tool)) = found.filter(|_| request["method"] == "tools/call") else {
            return error(-32601, format!("This device has no {} {name}", request["method"].as_str().unwrap_or("method")));
        };
        let result = match tools.call(tool, request["params"]["arguments"].clone()).await {
            Ok(result) => result,
            Err(why) => json!({"content": [{"type": "text", "text": format!("{why:#}")}], "isError": true}),
        };
        json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
    }

    /// Whether this device is offline, as far as it can tell: its network has a relay at its address, and it isn't
    /// connected to it, as a phone with no signal or an app iOS held in the background isn't. Nothing could have
    /// answered it then, so what wasn't answered meanwhile is the device's being offline, not a failure of its
    /// network's. A network being built has no relay, so there it can't tell.
    pub(crate) fn offline(&self) -> bool {
        use iroh::Watcher;
        let link = self.link();
        link.network.is_some() && !link.endpoint.home_relay_status().get().iter().any(|relay| relay.is_connected())
    }

    /// The connection to a server, made when there's none open: to whichever of the servers acting for this client's
    /// user answers first. Which they are is what the client kept from when it last asked, and what it's told when
    /// it asks now if none of those answers.
    pub(crate) async fn reached(&self) -> Result<Arc<Reached>> {
        let started = Instant::now();
        let mut reach = self.reach.lock().await;
        if let Some(open) = reach.open.as_ref().filter(|open| open.open()) {
            return Ok(open.clone());
        }
        reach.open = None;
        // Many asks at once share one try: no server answered while this one waited its turn.
        if reach.failed.is_some_and(|failed| failed > started) {
            bail!(UNANSWERED)
        }
        let mut tried = None;
        for now in [false, true] {
            let Some(servers) = self.servers(now).await? else { continue };
            if tried.as_ref() == Some(&servers) {
                continue;
            }
            let mut connecting =
                FuturesUnordered::from_iter(servers.iter().cloned().map(|server| self.connect(server)));
            while let Some(connected) = connecting.next().await {
                if let Some(open) = connected {
                    return Ok(reach.open.insert(Arc::new(open)).clone());
                }
            }
            tried = Some(servers);
        }
        reach.failed = Some(Instant::now());
        bail!(UNANSWERED)
    }

    /// The servers acting for this client's user: the ones it kept, or with `now` the ones it's told when it asks.
    /// Nothing if it kept none, or has nobody to ask.
    async fn servers(&self, now: bool) -> Result<Option<Vec<EndpointAddr>>> {
        let Some(network) = self.link().network else {
            if now {
                return Ok(None);
            }
            let file = self.home.join("servers");
            let servers = fs::read(&file)
                .with_context(|| format!("{} doesn't say which servers to ask", file.display()))
                .context("This device hasn't joined a network")?;
            let servers = serde_json::from_slice(&servers)
                .with_context(|| format!("{} isn't servers' addresses", file.display()));
            return servers.map(Some);
        };
        let of = self.of()?;
        let cached = self.cached_servers(&of);
        if !now {
            return Ok(cached);
        }
        let servers = network
            .servers(&self.key, &of)
            .await
            .with_context(|| format!("Couldn't ask {} which servers to reach", network.address))?;
        // One that was reached straight before is tried there again, beside the relay.
        let servers: Vec<EndpointAddr> = servers
            .into_iter()
            .map(|server| match cached.iter().flatten().find(|was| was.id == server.id) {
                Some(was) => server.with_addrs(was.addrs.iter().filter(|addr| addr.is_ip()).cloned()),
                None => server,
            })
            .collect();
        self.keep(SERVERS, &json!({"of": of}), &serde_json::to_value(&servers)?)?;
        Ok(Some(servers))
    }

    /// Whose servers this client asks: whoever its device signed in as, or in a network being built, who it says it
    /// acts as. A sharing link's holder asks the servers of whoever the link leads to, which it starts with.
    fn of(&self) -> Result<String> {
        let header = |what| self.says.iter().find(|(says, _)| *says == what).map(|(_, header)| header.as_str());
        Ok(match (header("link").map(|link| link.split('/').collect::<Vec<_>>()), header("as"), &self.link().who) {
            (Some(leads), ..) if leads[0] == "network" => "network/".to_owned(),
            (Some(leads), ..) if leads.len() > 2 => format!("{}/{}/", leads[0], leads[1]),
            (_, Some(acts_as), _) => format!("users/{acts_as}/"),
            (_, _, Some(who)) => format!("{who}/"),
            _ => bail!("This device isn't signed in here. In a network being built, ONE_AS says who it acts as."),
        })
    }

    /// The servers acting for `of` as this client last kept them, each with where it was last reached straight.
    fn cached_servers(&self, of: &str) -> Option<Vec<EndpointAddr>> {
        self.cached(SERVERS, &json!({"of": of})).and_then(|cached| serde_json::from_value(cached).ok())
    }

    /// Keeps `addr` as where `server` is reached straight, as a connection to it found it, so the next run of this
    /// device's clients tries it from the start, beside the relay, rather than waiting for the relay to find it again.
    fn reached_straight(&self, server: EndpointId, addr: SocketAddr) {
        let (Some(_), Ok(of)) = (self.link().network, self.of()) else { return };
        let Some(mut servers) = self.cached_servers(&of) else { return };
        let Some(cached) = servers.iter_mut().find(|cached| cached.id == server) else { return };
        let relay = cached.addrs.iter().filter(|addr| !addr.is_ip()).cloned();
        let now: std::collections::BTreeSet<_> = relay.chain([TransportAddr::Ip(addr)]).collect();
        if now != cached.addrs {
            cached.addrs = now;
            let cached = serde_json::to_value(&servers).map_err(anyhow::Error::from);
            if let Err(error) = cached.and_then(|servers| self.keep(SERVERS, &json!({"of": of}), &servers)) {
                self.report(json!({"name": "one.client.cache", "error": format!("{error:#}")}));
            }
        }
    }
}
