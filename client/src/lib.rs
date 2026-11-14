//! The client's Rust half, with no window: it holds the client's key, reaches a server by its key, and keeps what
//! servers answered (`kept`). The CLI (`cli`) and MCP (`mcp`) are this, run from a terminal or by an agent, and the
//! app is a window on it (`shell`).
//!
//! A client asks a server in MCP's JSON-RPC: one message on a stream of its own, and the answer back on that stream.
//! A server tells a client something the same way, on a stream the server opens. No connection is kept open for
//! that: a client with an app on screen says it's watching, and a server connects to it when there's something to
//! say.

pub mod cli;
mod kept;
pub mod mcp;

use std::io::{ErrorKind, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use futures_util::future::join_all;
use futures_util::stream::FuturesUnordered;
use iroh::endpoint::{Connection, QuicTransportConfig, RecvStream, RelayMode, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, NetReportConfig, RelayConfig, RelayUrl, SecretKey};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::{sleep, timeout};

pub use kept::Shown;

/// What servers and clients speak.
pub const ALPN: &[u8] = b"one/mcp/1";
/// The most one message may carry: a plugin's files fit.
pub const LIMIT: usize = 64 << 20;
/// How long reaching a server may take: long enough for a phone to find the relay and be let in, and for a first
/// packet that's lost to be sent again.
pub const CONNECT: Duration = Duration::from_secs(3);
/// How long a server that's there takes, at most, to say so. Nothing is sent over a connection that's asked nothing,
/// so a server that has gone is found out by asking it, and this soon. A server taken for gone that wasn't is only
/// asked again over a new connection, or has its lease taken by a spare, and neither loses anything.
pub const THERE: Duration = Duration::from_secs(1);
/// How long a connection that's asked nothing stays open. To keep one open, something has to be said over it every
/// few seconds, and through a relay each of those wakes the relay. So a connection is let go once it's idle, and
/// made again when there's something to ask.
const IDLE: Duration = Duration::from_secs(2);
/// How long a connection lasts once nothing arrives over it: longer than `IDLE`, so that one that's let go says so,
/// and sooner than iroh would say anything to keep it. iroh sends what it has for a machine only where a
/// connection it still has with it last was, so the one a server that stopped left behind has to end before that
/// server is found where it starts again.
const SILENT: Duration = Duration::from_secs(3);

/// A network's address, and the pass this machine shows there: what says it's one of the network's own.
#[derive(Clone, Deserialize)]
pub struct Network {
    pub address: String,
    pub pass: String,
}

impl Network {
    /// The headers of an ask of `path` at the address, with `body`: `key`'s signature of it, and the pass. The pieces
    /// there check both (`address/ask.js`), and a server's Elixir signs the same bytes (`OneCore.Address`).
    pub fn signed(&self, key: &SecretKey, path: &str, body: &str) -> Result<[(&'static str, String); 4]> {
        let at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string();
        let signature = key.sign(format!("one/ask/1\n{path}\n{at}\n{body}").as_bytes());
        Ok([
            ("one-key", key.public().to_string()),
            ("one-at", at),
            ("one-signature", hex(&signature.to_bytes())),
            ("one-pass", self.pass.clone()),
        ])
    }

    /// The servers acting for `of`, a user's or a group's prefix, as the signer at the address says: the ones on their
    /// list and then the ones acting for the network, which act for everyone. Each is its key, reached through the
    /// relay there until a straighter path is found.
    async fn servers(&self, key: &SecretKey, of: &str) -> Result<Vec<EndpointAddr>> {
        let mut ask = web()?
            .get(format!("{}/signer/servers", self.address.trim_end_matches('/')))
            .timeout(CONNECT)
            .query(&[("of", of)]);
        for (name, value) in self.signed(key, "/signer/servers", "")? {
            ask = ask.header(name, value);
        }
        let answer = ask.send().await?;
        let (status, said): (_, Value) = (answer.status(), answer.json().await?);
        if !status.is_success() {
            bail!("{}", said["error"].as_str().unwrap_or("the signer refused"))
        }
        let relay: RelayUrl = self.address.parse().with_context(|| format!("{} isn't an address", self.address))?;
        let keys = said["servers"].as_array().context("the signer listed no servers")?;
        keys.iter()
            .map(|key| {
                let key: EndpointId = key.as_str().unwrap_or_default().parse().context("that isn't a server's key")?;
                Ok(EndpointAddr::new(key).with_relay_url(relay.clone()))
            })
            .collect()
    }
}

/// What asks over the web: the signer at a network's address, and the bucket by a link. Its TLS is the one iroh
/// reaches the relay with.
fn web() -> Result<reqwest::Client> {
    let tls = iroh::tls::CaTlsConfig::default().client_config(iroh::tls::default_provider())?;
    Ok(reqwest::Client::builder().tls_backend_preconfigured(tls).connect_timeout(CONNECT).build()?)
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// An endpoint holding the key kept in `key_file`, which is made on first use. It reaches others, and is reached, by
/// key: straight to the addresses it's told and the ones it finds, and otherwise through the relay at its network's
/// address, which is also the only thing it times. With no other relay and no address lookup, nothing about a
/// network goes to anyone outside it. A network on one machine has no address, and so no relay. A server's add-on
/// binds this same endpoint.
pub async fn endpoint(key_file: &Path, network: Option<&Network>) -> Result<Endpoint> {
    let mut probes = NetReportConfig::minimal();
    let relay = match network {
        Some(Network { address, pass }) => {
            let address = address.parse().with_context(|| format!("{address} isn't a network's address"))?;
            // iroh takes the relay it timed as its own, so with none timed it would have none.
            probes.https_probes = true;
            RelayMode::Custom(RelayConfig::new(address, None).with_auth_token(pass).into())
        }
        None => RelayMode::Disabled,
    };
    Endpoint::builder(presets::Minimal)
        .secret_key(key(key_file)?)
        .alpns(vec![ALPN.to_vec()])
        .transport_config(QuicTransportConfig::builder().max_idle_timeout(Some(SILENT.try_into()?)).build())
        .relay_mode(relay)
        .net_report_config(probes)
        .bind()
        .await
        .context("couldn't open a socket")
}

/// Where `endpoint` is reached, as another is told it: its key, the machine's addresses on its networks, and the
/// relay at its network's address, which it's told now, before the endpoint has connected there. Another on this
/// machine reaches it with no network too.
pub fn address(endpoint: &Endpoint, network: Option<&Network>) -> Result<EndpointAddr> {
    let here = endpoint.bound_sockets().into_iter().filter(SocketAddr::is_ipv4);
    let address = here.fold(endpoint.addr(), |address, socket| {
        address.with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, socket.port())))
    });
    match network {
        Some(Network { address: at, .. }) => {
            Ok(address.with_relay_url(at.parse().with_context(|| format!("{at} isn't an address"))?))
        }
        None => Ok(address),
    }
}

/// The key kept in `file`, which is made on first use.
pub fn key(file: &Path) -> Result<SecretKey> {
    fs::create_dir_all(file.parent().context("a key is kept in a folder")?)?;
    let mut new = fs::OpenOptions::new();
    new.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut new, 0o600);
    match new.open(file) {
        Ok(mut file) => {
            let key = SecretKey::generate();
            file.write_all(&key.to_bytes())?;
            Ok(key)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let bytes = fs::read(file)?.try_into().map_err(|_| anyhow!("{} doesn't hold a key", file.display()))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(error) => Err(error).with_context(|| format!("couldn't make {}", file.display())),
    }
}

/// The folder a client on a computer is kept in: `ONE_HOME` if that's set, as a network made to build or test with
/// sets it, and otherwise where the system keeps an app's data, under a name of this client's own. A phone gives
/// its app a folder instead.
pub fn home() -> Result<PathBuf> {
    match env::var_os("ONE_HOME") {
        Some(home) => Ok(home.into()),
        None => Ok(dirs::data_dir().context("this system has no folder for an app's data: set ONE_HOME")?.join(NAME)),
    }
}

/// What the app is to the system it's on: the name of its folder, and of the app among a phone's. The app before
/// this one is `one` and `The One App` there, and this is neither.
pub const NAME: &str = "app.theone.client";

/// Why a server didn't answer what it was asked.
#[derive(Debug, PartialEq)]
pub enum Unanswered {
    /// It couldn't be reached, so it was asked nothing.
    Unreachable,
    /// It stopped answering after it was asked, so it may have done what was asked.
    Stopped,
}

/// An open connection to a server, and when the server last answered on it. A client asks a server over one, and a
/// server asks another the same way. `gone` is whether the server was found not to be there.
pub struct Reached {
    connection: Connection,
    heard: Arc<std::sync::Mutex<Instant>>,
    gone: AtomicBool,
}

impl Reached {
    /// Connects to the server at `address`, or gives nothing if it doesn't answer `within` that long.
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr, within: Duration) -> Option<Self> {
        let connection = timeout(within, endpoint.connect(address, ALPN)).await.ok()?.ok()?;
        let heard = Arc::new(std::sync::Mutex::new(Instant::now()));
        let (open, last) = (connection.clone(), heard.clone());
        tokio::spawn(async move {
            while open.close_reason().is_none() {
                let idle = last.lock().expect("no ask panics").elapsed();
                match IDLE.checked_sub(idle) {
                    Some(left) => sleep(left).await,
                    None => open.close(0u32.into(), b"idle"),
                }
            }
        });
        Some(Self { connection, heard, gone: AtomicBool::new(false) })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Whether the connection is still one to ask over: it hasn't closed, and its server hasn't been found gone.
    pub fn open(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Whether the server didn't say it was there when it was asked over this connection.
    pub fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    /// Asks the server one message, and gives its answer. A server that hasn't answered anything lately is first
    /// asked whether it's there, and so is one that's taking its time over this.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>, Unanswered> {
        if !self.open() || (self.heard.lock().expect("no ask panics").elapsed() > THERE && !self.there().await) {
            return Err(Unanswered::Unreachable);
        }
        let mut receive = self.send(message).await.ok_or(Unanswered::Unreachable)?;
        let answer = receive.read_to_end(LIMIT);
        tokio::pin!(answer);
        loop {
            tokio::select! {
                answer = &mut answer => {
                    *self.heard.lock().expect("no ask panics") = Instant::now();
                    return answer.map_err(|_| Unanswered::Stopped);
                }
                _ = sleep(THERE) => if !self.there().await {
                    return Err(Unanswered::Stopped);
                }
            }
        }
    }

    /// Whether the server is there: an empty message, which a server answers at once with nothing. One that doesn't
    /// in time is gone, and so is this connection to it.
    async fn there(&self) -> bool {
        let asked = async { self.send(b"").await?.read_to_end(LIMIT).await.ok() };
        let there = timeout(THERE, asked).await.is_ok_and(|answer| answer.is_some());
        if there {
            *self.heard.lock().expect("no ask panics") = Instant::now();
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
struct Reach {
    open: Option<Arc<Reached>>,
    failed: Option<Instant>,
}

/// A client, kept in its home folder: its `key`, where its `network` is or its `servers` are, and what servers
/// answered (`kept`).
pub struct Client {
    home: PathBuf,
    endpoint: Endpoint,
    /// The network's address, if it has one: the client asks there which servers act for its user. A network on one
    /// machine has none, and whatever starts it tells the client where its servers are, in `servers`.
    network: Option<Network>,
    reach: Mutex<Reach>,
    told: broadcast::Sender<Value>,
    /// What every request says of who it's from and for, where the environment says: `ONE_LINK` is a sharing link,
    /// whose holder asks, `ONE_AS` names a user to act as, which only a server of a network being built allows, and
    /// `ONE_FOR` names the group it's for (`groups/family`), or `network`.
    says: Vec<(&'static str, String)>,
    /// Whether an app of this client's is on screen, so its servers are to tell it what changes.
    watching: AtomicBool,
    /// Whether a server answered the last thing asked.
    answering: watch::Sender<bool>,
}

impl Client {
    pub async fn open(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        let network = match fs::read(home.join("network")) {
            Ok(network) => Some(serde_json::from_slice(&network).context("`network` isn't a network's address")?),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("couldn't read `network`"),
        };
        let endpoint = endpoint(&home.join("key"), network.as_ref()).await?;
        let says = [("as", "ONE_AS"), ("link", "ONE_LINK"), ("for", "ONE_FOR")];
        let says = says.into_iter().filter_map(|(says, variable)| Some((says, env::var(variable).ok()?))).collect();
        let told = broadcast::channel(16).0;
        // A server with something to say connects to this client to say it. Only one of the network's own reaches
        // a client, and all it can make one do is ask again.
        let (accepting, tell) = (endpoint.clone(), told.clone());
        tokio::spawn(async move {
            while let Some(incoming) = accepting.accept().await {
                let tell = tell.clone();
                tokio::spawn(async move {
                    if let Ok(connection) = incoming.await {
                        hear(connection, tell).await
                    }
                });
            }
        });
        let (watching, answering) = (AtomicBool::new(false), watch::Sender::new(true));
        Ok(Self { home, endpoint, network, reach: Mutex::default(), told, says, watching, answering })
    }

    /// The network's address, if it has one.
    pub fn address(&self) -> Option<&str> {
        self.network.as_ref().map(|network| network.address.as_str())
    }

    /// Whether a server answered the last thing this client asked, as that changes. It's found out by asking:
    /// nothing is said over a connection to learn it.
    pub fn answering(&self) -> watch::Receiver<bool> {
        self.answering.subscribe()
    }

    /// Says whether an app of this client's is on screen. While one is, every server acting for its user tells it
    /// when something changed. A server that isn't up now is told when it's next asked anything, and one that
    /// isn't told the app has gone finds out when it next has something to say.
    pub async fn watch(&self, watching: bool) {
        if self.watching.swap(watching, Ordering::Relaxed) == watching {
            return;
        }
        let Ok(Some(servers)) = self.servers(false).await else { return };
        let told = servers.into_iter().map(|server| async move {
            if let Some(open) = self.connect(server).await
                && !watching
            {
                let _ = open.ask(json!({"jsonrpc": "2.0", "method": WATCH, "params": {}}).to_string().as_bytes()).await;
            }
        });
        join_all(told).await;
    }

    /// Connects to `server`, hears what it tells this client over the connection, and if this client is watching,
    /// says so first, with where a server reaches it.
    async fn connect(&self, server: EndpointAddr) -> Option<Reached> {
        let open = Reached::connect(&self.endpoint, server, CONNECT).await?;
        tokio::spawn(hear(open.connection().clone(), self.told.clone()));
        if self.watching.load(Ordering::Relaxed) {
            let here = address(&self.endpoint, self.network.as_ref()).ok()?;
            let watch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {"address": here}});
            open.ask(watch.to_string().as_bytes()).await.ok()?;
        }
        Some(open)
    }

    /// What servers tell this client: each a JSON-RPC notification.
    pub fn told(&self) -> broadcast::Receiver<Value> {
        self.told.subscribe()
    }

    /// Asks a server one JSON-RPC message and gives its answer, which for a notification is nothing. A server that
    /// has gone was asked nothing, so whichever answers now is asked instead.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>> {
        let message = self.saying(message);
        let asked = async {
            for _ in 0..2 {
                match self.reached().await?.ask(&message).await {
                    Ok(answer) => return Ok(answer),
                    Err(Unanswered::Unreachable) => continue,
                    Err(Unanswered::Stopped) => bail!(
                        "The server stopped answering partway, so this may or may not have been done. Look before \
                         trying again."
                    ),
                }
            }
            bail!(UNANSWERED)
        };
        let answer = asked.await;
        self.answering.send_if_modified(|answering| std::mem::replace(answering, answer.is_ok()) != answer.is_ok());
        answer
    }

    /// `message` with what this client says of who it's from, in the request's `_meta`.
    fn saying<'a>(&self, message: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match serde_json::from_slice::<Value>(message) {
            Ok(mut request) if !self.says.is_empty() && request["method"].is_string() => {
                for (says, value) in &self.says {
                    request["params"]["_meta"][*says] = json!(value);
                }
                request.to_string().into_bytes().into()
            }
            _ => message.into(),
        }
    }

    /// Asks the server for `method`, and gives its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let answer = self.ask(message.to_string().as_bytes()).await?;
        let mut answer: Value = serde_json::from_slice(&answer).context("the server's answer isn't JSON")?;
        if let Some(error) = answer.get("error") {
            bail!("{}", error["message"].as_str().unwrap_or("the server refused"))
        }
        let mut result = answer["result"].take();
        self.keep_files(&mut result).await?;
        Ok(result)
    }

    /// Ends the connection, if one was made, and tells the server. The goodbye leaves this machine at once. What
    /// else closing an endpoint waits for is a server that was tried and never answered, which QUIC gives three
    /// seconds, so that isn't waited for.
    pub async fn close(&self) {
        if let Some(open) = self.reach.lock().await.open.take() {
            open.connection().close(0u32.into(), b"done");
            let _ = timeout(Duration::from_millis(100), self.endpoint.close()).await;
        }
    }

    /// The connection to a server, made when there's none open: to whichever of the servers acting for this client's
    /// user answers first. Which they are is what the client kept from when it last asked, and what it's told when
    /// it asks now if none of those answers.
    async fn reached(&self) -> Result<Arc<Reached>> {
        let asked = Instant::now();
        let mut reach = self.reach.lock().await;
        if let Some(open) = reach.open.as_ref().filter(|open| open.open()) {
            return Ok(open.clone());
        }
        reach.open = None;
        // Many asks at once share one try: no server answered while this one waited its turn.
        if reach.failed.is_some_and(|failed| failed > asked) {
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
        let Some(network) = &self.network else {
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
        // Until signing in says who this client's user is, it's who it says it acts as, or the one user a network
        // has without it.
        let user = self.says.iter().find(|(says, _)| *says == "as").map_or("anyone", |(_, user)| user);
        let of = format!("users/{user}/");
        let asked = (SERVERS, json!({"of": of}));
        if !now {
            return Ok(self.kept(asked.0, &asked.1).and_then(|kept| serde_json::from_value(kept).ok()));
        }
        let servers = network
            .servers(self.endpoint.secret_key(), &of)
            .await
            .with_context(|| format!("Couldn't ask {} which servers to reach", network.address))?;
        self.keep(asked.0, &asked.1, &serde_json::to_value(&servers)?)?;
        Ok(Some(servers))
    }
}

/// What a person is told when no server answered: what went wrong, and what to do about it.
const UNANSWERED: &str = "No server is answering. Check this device is online and a server is on, then try again.";

/// What a client tells a server to be told what changes, at the `address` it gives, or with none, no longer.
const WATCH: &str = "notifications/one/watch";

/// What the servers the signer listed are kept as.
const SERVERS: &str = "signer/servers";

/// Tells whoever `connection` is to one message, on a stream of its own, which isn't answered. Gives whether it
/// was sent.
pub async fn tell(connection: &Connection, message: &[u8]) -> bool {
    let told = async {
        let mut send = connection.open_uni().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()
    };
    told.await.is_some()
}

/// Passes on what a server tells this client over `connection`, until the connection ends.
async fn hear(connection: Connection, told: broadcast::Sender<Value>) {
    while let Ok(mut receive) = connection.accept_uni().await {
        let told = told.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            if let Ok(message) = serde_json::from_slice(&message) {
                // With nobody listening, as in the CLI, it's dropped.
                let _ = told.send(message);
            }
        });
    }
}
