//! A service endpoint: a device's abilities (a service) offered over iroh, as a part of its device's oned, a process of
//! its own (`oned <name>`). It's an endpoint of its own key, kept in its folder, and gets in when its device's oned lists
//! it as its person's, a part of that device (`Node::list_part`), which tells the relay to let it in. It holds no keys
//! and reads no bucket: the nodes that call it bring it `global`'s entries, whose member entries it replays from the
//! network's first entry (given by oned, so nothing on the way can make it another network's) and checks each caller by
//! itself: a compute endpoint of the network that runs the calls of a person it serves. The program offering it, its
//! app's own (`oned::program`), which this starts, registers over a loopback with the services' API and long-polls for
//! calls, in its sandbox (`sandbox`), reaching out only through this endpoint's proxy. A service writes nothing itself: what it asks for is a call, which a node of its person's runs (`/v1/call` on the loopback).

use crate::config::YOURS;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ed25519_dalek::SigningKey;
use iroh::endpoint::{Connection, QuicTransportConfig, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Entry, Hash, Key, hex, short, unhex};
use crate::database::scopes::{self, ScopeDb};
use crate::members::{self, Members};
use crate::services::Services;

/// The most an ask or its answer may carry, as between nodes.
const MESSAGE_LIMIT: usize = 64 << 20;
/// The most a computer this service doesn't know of yet may send it: `global`'s entries.
const STRANGER_LIMIT: usize = 16 << 20;
/// How many messages a computer this service doesn't know of may send on a connection: a hello, the network's
/// entries, and a hello again.
const STRANGER_MESSAGES: usize = 3;
/// How many computers it doesn't know of may be connected at once.
const STRANGERS: usize = 4;
/// How many streams a node may have open on a connection at once.
const STREAMS: u32 = 32;
/// How long reaching a node may take.
const CONNECT: Duration = Duration::from_secs(10);

/// The network a service is in, as its device's oned gives it: its address, where its relay and sign-in are, the hash
/// of its first entry, which its member entries are replayed from, and the person whose service it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Network {
    pub address: String,
    pub root: String,
    pub user: String,
}

impl Network {
    /// Where oned keeps it, in the service's folder: still `install.json`, which services of older builds read.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("install.json")
    }

    /// Gives it to the service kept in `dir`, the first time: a service is another network's or person's only once
    /// its folder goes.
    pub fn give(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        match std::fs::read(Self::path(dir)).ok().map(|b| serde_json::from_slice::<Network>(&b)).transpose()? {
            Some(kept) => ensure!(kept == *self, "{} is someone else's, or in another network ({}): remove it to move it", dir.display(), kept.address),
            None => write_private(&Self::path(dir), &serde_json::to_vec_pretty(self)?)?,
        }
        Ok(())
    }
}

/// A service endpoint, offering one service.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

struct Inner {
    name: String,
    dir: PathBuf,
    key: SigningKey,
    network: Network,
    /// `global`'s entries, as the nodes brought them, and what its member entries add up to.
    known: Mutex<(Vec<Entry>, ScopeDb)>,
    /// Once `global`'s log is frozen, its scope database's steps nodes brought, by position from the first
    /// (`database::steps`), kept in its folder as `steps`.
    steps: Mutex<Vec<Option<crate::database::steps::Given>>>,
    /// The depths in `global`'s replay order of the entries its member entries follow that nodes didn't bring: a service
    /// is brought only member entries, and orders them as the nodes do with these.
    depths: Mutex<HashMap<Hash, u64>>,
    /// Where the program offering it registers and waits for calls.
    services: Services,
    /// What that program sends to say it's on this machine.
    token: String,
    endpoint: Mutex<Option<Endpoint>>,
    /// A connection to each node it asks for calls.
    conns: Mutex<HashMap<Key, Connection>>,
    strangers: AtomicUsize,
    /// The network's sign-in service, whose keys check a sign-in a node passes on to a service that serves everyone.
    issuer: crate::cloudflare::Issuer,
    /// What its app declares its program may reach, as oned gave it (`declared.json`), if it did.
    declared: Option<crate::sandbox::Declared>,
}

/// Where oned puts what a service's app declares it may reach, in its folder.
pub const DECLARED: &str = "declared.json";


/// Where a service keeps its key, its network and what it knows of it.
pub fn folder(name: &str) -> PathBuf {
    crate::home().join("services").join(name)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Service {
    /// The service `name` kept in `dir`: its key (made the first time), and the network oned gave it.
    pub fn open(dir: &Path, name: &str) -> Result<Service> {
        use std::os::unix::fs::PermissionsExt;
        ensure!(members::valid_app_name(name), "a service's name is lowercase words joined by -");
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let key_file = dir.join("key");
        let key = match std::fs::read_to_string(&key_file) {
            Ok(secret) => SigningKey::from_bytes(&unhex(secret.trim()).context("its key file isn't a key")?),
            Err(_) => {
                let key = SigningKey::from_bytes(&rand::random());
                write_private(&key_file, hex(&key.to_bytes()).as_bytes())?;
                key
            }
        };
        let network: Network = serde_json::from_slice(&std::fs::read(Network::path(dir)).with_context(|| format!("{name} is started by its device's oned, which says which network it's in"))?)?;
        let token = match std::fs::read_to_string(dir.join(crate::api::LOCAL_TOKEN)) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                let t = hex(&rand::random::<[u8; 32]>());
                write_private(&dir.join(crate::api::LOCAL_TOKEN), t.as_bytes())?;
                t
            }
        };
        let root = Hash::from_hex(&network.root)?;
        let network_address = network.address.clone();
        let entries = std::fs::read(dir.join("entries")).ok().and_then(|b| crate::entry::unpack(&b).ok()).unwrap_or_default();
        let depths: HashMap<Hash, u64> = std::fs::read(dir.join("depths")).ok().and_then(|b| serde_json::from_slice::<HashMap<String, u64>>(&b).ok()).map(|m| m.into_iter().filter_map(|(h, d)| Some((Hash::from_hex(&h).ok()?, d))).collect()).unwrap_or_default();
        let steps = read_steps(&dir.join("steps"));
        let (entries, members) = placed(root, entries, &depths, &steps);
        Ok(Service(Arc::new(Inner {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            key,
            network,
            known: Mutex::new((entries, members)),
            steps: Mutex::new(steps),
            depths: Mutex::new(depths),
            services: Services::default(),
            token,
            endpoint: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            strangers: AtomicUsize::new(0),
            issuer: crate::cloudflare::Issuer::new(&network_address),
            declared: std::fs::read(dir.join(DECLARED)).ok().and_then(|b| serde_json::from_slice(&b).ok()),
        })))
    }

    pub fn key(&self) -> Key {
        *self.0.key.verifying_key().as_bytes()
    }

    pub fn endpoint(&self) -> crate::keys::Endpoint {
        crate::keys::Endpoint::of(&self.0.key)
    }

    /// Where its endpoint is reached now, once it's bound.
    pub fn addr(&self) -> Option<EndpointAddr> {
        self.0.endpoint.lock().unwrap().as_ref().map(|e| e.addr())
    }

    /// Where the program offering it registers.
    pub fn services(&self) -> &Services {
        &self.0.services
    }

    pub fn token(&self) -> &str {
        &self.0.token
    }

    /// Whose service it is, once a person's device approved it.
    pub fn owner(&self) -> Option<String> {
        self.owner_in(&self.0.known.lock().unwrap().1)
    }

    fn owner_in(&self, m: &ScopeDb) -> Option<String> {
        // Only a listing as the person whose link it was given counts: nobody else can make it theirs.
        m.service(&self.key()).filter(|s| s.service.as_deref() == Some(self.0.name.as_str()) && s.user == self.0.network.user).map(|s| s.user)
    }

    /// Starts it: its endpoint, through the network's relay (which lets it in once it's approved), asking to be
    /// approved until it is, and the loopback the program offering it registers at. → that loopback's address.
    pub async fn start(&self) -> Result<std::net::SocketAddr> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let at = listener.local_addr()?;
        let router = self.loopback();
        // A program in a sandbox reaches it through a socket in its sandbox's folder.
        #[cfg(target_os = "linux")]
        if matches!(crate::sandbox::how(self.0.declared.as_ref(), ""), crate::sandbox::How::Unit | crate::sandbox::How::Person) {
            use std::os::unix::fs::PermissionsExt;
            let dir = crate::sandbox::dir(&self.0.name);
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(crate::sandbox::SOCKET);
            let _ = std::fs::remove_file(&path);
            let socket = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
            let router = router.clone();
            tokio::spawn(async move {
                if let Err(err) = axum::serve(socket, router).await {
                    tracing::warn!("the service's socket stopped: {err:#}");
                }
            });
            // What a unit's program says, which systemd would keep where its person can't read it.
            let path = dir.join(crate::sandbox::LOG);
            let _ = std::fs::remove_file(&path);
            let log = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
            tokio::spawn(async move {
                while let Ok((mut said, _)) = log.accept().await {
                    tokio::spawn(async move {
                        let _ = tokio::io::copy(&mut said, &mut tokio::io::stderr()).await;
                    });
                }
            });
        }
        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                tracing::warn!("the service's loopback stopped: {err:#}");
            }
        });
        tokio::spawn(self.clone().keep_endpoint());
        tokio::spawn(self.clone().ask_to_be_listed());
        Ok(at)
    }

    /// Binds its endpoint, then takes the connections nodes open, for good.
    async fn keep_endpoint(self) {
        let mut wait = Duration::from_secs(1);
        let endpoint = loop {
            match self.bind().await {
                Ok(endpoint) => break endpoint,
                Err(err) => tracing::warn!("{}'s endpoint: {err:#}", self.0.name),
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
        };
        *self.0.endpoint.lock().unwrap() = Some(endpoint.clone());
        while let Some(incoming) = endpoint.accept().await {
            let service = self.clone();
            tokio::spawn(async move {
                match incoming.accept() {
                    Ok(accepting) => match accepting.await {
                        Ok(conn) => service.take(conn).await,
                        Err(err) => tracing::debug!("a connection that didn't open: {err:#}"),
                    },
                    Err(err) => tracing::debug!("a connection that didn't open: {err:#}"),
                }
            });
        }
    }

    async fn bind(&self) -> Result<Endpoint> {
        let (_, map) = crate::node::relay(&self.0.network.address, None)?;
        let transport = QuicTransportConfig::builder().max_concurrent_bidi_streams(STREAMS.into()).max_concurrent_uni_streams(0u32.into()).build();
        Ok(Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&self.0.key.to_bytes()))
            .alpns(vec![crate::node::service_alpn()])
            .transport_config(transport)
            .relay_mode(RelayMode::Custom(map))
            .bind()
            .await?)
    }

    /// Asks its device's oned to list it, once the program offering it has said what it is (so whether it serves
    /// everyone): `ask` in its folder, its endpoint and that, which oned reads and lists, until it's listed as its
    /// person's and as part of a device (one listed before parts named their device is listed again).
    async fn ask_to_be_listed(self) {
        let ask = self.0.dir.join("ask");
        loop {
            let registered = !self.0.services.describe().is_empty();
            let listed = self.owner().is_some() && self.0.known.lock().unwrap().1.service(&self.key()).is_some_and(|s| s.device.is_some());
            if listed || !registered {
                let _ = std::fs::remove_file(&ask);
            } else if !ask.exists() {
                let me = self.endpoint();
                let said = json!({ "key": hex(&me.key), "seal": hex(&me.seal), "shared": self.0.services.serves_everyone(&self.0.name) });
                if let Err(err) = write_private(&ask, said.to_string().as_bytes()) {
                    tracing::warn!("asking oned to list {}: {err:#}", self.0.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Answers what a computer asks on a connection it opened: a node of the network anything a node asks a service;
    /// one it doesn't know of only a hello and the network's entries, a few messages, a few at once, since anyone who
    /// reaches it directly can connect.
    async fn take(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        let known = self.is_node(&remote);
        if !known && self.0.strangers.fetch_add(1, SeqCst) >= STRANGERS {
            self.0.strangers.fetch_sub(1, SeqCst);
            return conn.close(1u32.into(), b"not a device of this network");
        }
        let mut messages = 0;
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            messages += 1;
            let node = self.is_node(&remote);
            if !node && messages > STRANGER_MESSAGES {
                break;
            }
            let service = self.clone();
            let answering = async move {
                let limit = if node { MESSAGE_LIMIT } else { STRANGER_LIMIT };
                let msg: Value = match recv.read_to_end(limit).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what {} sent: {err:#}", short(&remote)),
                };
                let answer = service.answer(remote, &msg).await;
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering {}: {err:#}", short(&remote));
                }
            };
            // A node's asks run at once; a stranger's one at a time.
            if node {
                tokio::spawn(answering);
            } else {
                answering.await;
            }
        }
        if !known {
            self.0.strangers.fetch_sub(1, SeqCst);
            conn.close(1u32.into(), b"not a device of this network");
        }
    }

    async fn answer(&self, remote: Key, msg: &Value) -> Value {
        let body = &msg["body"];
        match msg["kind"].as_str() {
            Some("hello") => self.hello(remote, body["entries"].as_u64().unwrap_or(0) as usize, body["steps"].as_u64().unwrap_or(0) as usize),
            Some("steps") => match self.take_steps(&remote, &body["steps"]) {
                Ok(()) => json!({ "ok": true }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            },
            Some("entries") => match self.take_entries(&remote, body["pack"].as_str().unwrap_or(""), &body["depths"]) {
                Ok(()) => json!({ "ok": true }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            },
            Some("ask") if self.is_node(&remote) => self.run(remote, &body["call"]).await,
            _ => json!({ "error": format!("only {YOURS}'s devices are answered") }),
        }
    }

    /// A node's hello, with how many of `global`'s entries it has: → what this service offers and whom it
    /// serves, to a node of the network that may call it; that it's behind, if the node has more entries than it.
    fn hello(&self, remote: Key, entries: usize, steps: usize) -> Value {
        let (count, may) = {
            let known = self.0.known.lock().unwrap();
            (known.0.len(), known.1.compute(&remote).is_some() && known.1.service(&self.key()).is_some())
        };
        if entries > count || steps > self.0.steps.lock().unwrap().len() {
            return json!({ "behind": true });
        }
        if !may {
            return json!({ "error": format!("only {YOURS}'s devices are answered") });
        }
        let Some(mut describe) = self.0.services.describe().into_iter().next() else {
            return json!({ "error": format!("{} isn't running here yet", self.0.name) });
        };
        describe["runs"] = json!("service");
        let known = self.0.known.lock().unwrap();
        let m = &known.1;
        // Whom it serves is what its listing says, not what the program offering it claims.
        let users: Vec<String> = m.users().into_iter().map(|u| u.id).filter(|u| m.may(u, "use", &scopes::service(&self.key()))).collect();
        json!({ "describe": describe, "users": users })
    }

    /// Takes `global`'s member entries a node of the network brought (anyone's, while it knows of no node yet, since
    /// they must replay from the network's first entry): added to those it has, never in place of any, each signed,
    /// replayed from the first entry; those signed by no endpoint the network ever listed are passed over.
    fn take_entries(&self, remote: &Key, pack: &str, depths: &Value) -> Result<()> {
        let bytes = data_encoding::BASE64.decode(pack.as_bytes()).map_err(|_| anyhow!("the entries aren't base64"))?;
        let given = crate::entry::unpack(&bytes)?;
        let root = Hash::from_hex(&self.0.network.root)?;
        let mut known = self.0.known.lock().unwrap();
        ensure!(known.1.computes().is_empty() || known.1.compute(remote).is_some(), "only {YOURS}'s devices bring its entries");
        let mut depths_known = self.0.depths.lock().unwrap();
        let mut grown = depths_known.clone();
        for (hash, depth) in depths.as_object().into_iter().flatten() {
            if let (Ok(hash), Some(depth)) = (Hash::from_hex(hash), depth.as_u64()) {
                grown.insert(hash, depth);
            }
        }
        let more_depths = grown.len() > depths_known.len();
        let steps = self.0.steps.lock().unwrap().clone();
        let (entries, members) = match merged(root, &known.0, given, &grown, &steps) {
            Some(merged) => merged,
            // Only more depths: what was known may order better with them.
            None if more_depths => placed(root, known.0.clone(), &grown, &steps),
            None => return Ok(()),
        };
        ensure!(!members.users().is_empty(), "those aren't {YOURS}'s entries");
        write_private(&self.0.dir.join("entries"), &crate::entry::pack(&entries.iter().collect::<Vec<_>>()))?;
        let text: HashMap<String, u64> = grown.iter().map(|(h, d)| (h.to_hex(), *d)).collect();
        write_private(&self.0.dir.join("depths"), &serde_json::to_vec(&text)?)?;
        *known = (entries, members);
        *depths_known = grown;
        Ok(())
    }

    /// Takes `global`'s scope database's steps a node of the network brought (`{ at, step }` each, as
    /// `GET /v1/steps/global` gives them): those after the ones it has, each checked as from anyone, then who's who
    /// from them, once its log is frozen.
    fn take_steps(&self, remote: &Key, given: &Value) -> Result<()> {
        let root = Hash::from_hex(&self.0.network.root)?;
        let mut known = self.0.known.lock().unwrap();
        ensure!(known.1.compute(remote).is_some(), "only {YOURS}'s devices bring its steps");
        let mut steps = self.0.steps.lock().unwrap();
        for s in given.as_array().into_iter().flatten() {
            let at = s["at"].as_u64().unwrap_or(0);
            if at == steps.len() as u64 + 1 {
                let bytes = s["step"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok());
                steps.push(crate::database::steps::given(&crate::entry::Scope::Global, at, bytes.as_deref()));
            }
        }
        let kept: Vec<Value> = steps.iter().enumerate().map(|(i, s)| json!({ "at": i + 1, "step": s.as_ref().map(|s| data_encoding::BASE64URL_NOPAD.encode(&crate::database::steps::encoded(s))) })).collect();
        write_private(&self.0.dir.join("steps"), &serde_json::to_vec(&kept)?)?;
        let depths = self.0.depths.lock().unwrap().clone();
        *known = placed(root, known.0.clone(), &depths, &steps);
        Ok(())
    }

    /// Whether `key` is a node of the network, as this service knows it.
    fn is_node(&self, key: &Key) -> bool {
        self.0.known.lock().unwrap().1.compute(key).is_some()
    }

    /// Runs a call a node asked for, if it may: → `{ ok }`, `{ error }`, or `{ elsewhere }` for a call this service
    /// can't run here.
    async fn run(&self, remote: Key, call: &Value) -> Value {
        let me = &call["me"];
        let (key, user) = match (me["key"].as_str().map(unhex), me["user"].as_str()) {
            (Some(Ok(key)), Some(user)) => (key, user.to_string()),
            _ => return json!({ "error": "who's asking?" }),
        };
        // A sign-in a node passed on counts only once this service has checked it's that person's itself.
        let signed_in = match me["token"].as_str() {
            Some(token) => self.signed_in(token, &user).await,
            None => false,
        };
        let caller = {
            let known = self.0.known.lock().unwrap();
            let m = &known.1;
            if let Err(err) = may_call(m, &self.key(), &remote, &key, &user, signed_in) {
                return json!({ "error": format!("{err:#}") });
            }
            let person = m.user(&user);
            crate::apps::Me {
                key: hex(&key),
                user: user.clone(),
                name: m.machine(&key).map(|d| d.name).unwrap_or_default(),
                role: person.as_ref().map(|u| u.role),
                user_name: person.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
                handle: person.map(|u| u.handle).unwrap_or_default(),
                agent: me["agent"].as_str().map(str::to_string),
            }
        };
        let input = if call["input"].is_null() { json!({}) } else { call["input"].clone() };
        let from = call["from"].as_str().map(str::to_string);
        match self.0.services.call(&self.0.name, call["op"].as_str().unwrap_or(""), input, caller, from).await {
            Ok(answer) => json!({ "ok": answer.ok }),
            Err(err) if err.is::<crate::services::Elsewhere>() => json!({ "elsewhere": err.to_string() }),
            Err(err) => json!({ "error": format!("{err:#}") }),
        }
    }

    /// Whether `token` is an access token the network's sign-in service signed for `user`, unexpired.
    async fn signed_in(&self, token: &str, user: &str) -> bool {
        let (service, token) = (self.clone(), token.to_string());
        let claims = tokio::task::spawn_blocking(move || service.0.issuer.verify(&token, now_ms())).await;
        matches!(claims, Ok(Ok(c)) if c.sub == user)
    }

    /// Asks a node of its person's own to run a call, as them: the first that answers. A service asks only for its own
    /// app, in its person's own scope (`limited`), which their node runs with its own keys until grants land.
    pub async fn ask_node(&self, call: Value) -> Result<Value> {
        let owner = self.owner().context("this service isn't approved yet")?;
        let mut call = limited(&self.0.name, &call)?;
        call["me"] = json!({ "key": hex(&self.key()), "user": owner });
        let nodes: Vec<Key> = {
            let known = self.0.known.lock().unwrap();
            let m = &known.1;
            m.computes().into_iter().filter(|c| c.user == owner && m.runs_for(&c.key, &crate::entry::Scope::User(owner.clone()))).map(|c| c.key).collect()
        };
        let mut why = Vec::new();
        for node in nodes {
            match self.send(node, &json!({ "kind": "ask", "body": { "call": call } })).await {
                Ok(answer) if answer.get("elsewhere").is_some() => why.push(answer["elsewhere"].as_str().unwrap_or("").to_string()),
                Ok(answer) => return Ok(answer),
                Err(err) => why.push(format!("{}: {err:#}", short(&node))),
            }
        }
        bail!("no device of yours could run it ({})", why.join("; "))
    }

    async fn send(&self, to: Key, msg: &Value) -> Result<Value> {
        let endpoint = self.0.endpoint.lock().unwrap().clone().with_context(|| format!("this service isn't connected to {YOURS}'s relay yet"))?;
        let held = self.0.conns.lock().unwrap().get(&to).filter(|c| c.close_reason().is_none()).cloned();
        let conn = match held {
            Some(conn) => conn,
            None => {
                let (url, _) = crate::node::relay(&self.0.network.address, None)?;
                let addr = EndpointAddr::new(PublicKey::from_bytes(&to)?).with_relay_url(url);
                let conn = tokio::time::timeout(CONNECT, endpoint.connect(addr, &crate::node::alpn())).await.context("it isn't connected")??;
                self.0.conns.lock().unwrap().insert(to, conn.clone());
                conn
            }
        };
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(msg.to_string().as_bytes()).await?;
        send.finish()?;
        let answer = tokio::time::timeout(Duration::from_secs(600), recv.read_to_end(MESSAGE_LIMIT)).await.context("it didn't answer")??;
        Ok(serde_json::from_slice(&answer)?)
    }

    /// The loopback the program offering it registers at, with the local token in its folder: the services' API, its
    /// machine as `/v1/state` says, and `/v1/call`, which a node of its person's runs.
    fn loopback(&self) -> Router {
        Router::new()
            .route("/v1/services/{app}", post(register))
            .route("/v1/services/{app}/calls", get(next_call))
            .route("/v1/services/{app}/calls/{id}", post(reply))
            .route("/v1/services/{app}/calls/{id}/vault", post(fill))
            .route("/v1/state", get(state))
            .route("/v1/call", post(call))
            // Its program's way out, from its sandbox: `CONNECT host:port`, to a host its app declares.
            .fallback(proxy)
            .with_state(self.clone())
    }

    /// What `/v1/state` says here: this service as the device, and the others offering it, as far as it knows.
    fn state(&self) -> Value {
        let known = self.0.known.lock().unwrap();
        let m = &known.1;
        // Whether another is on, it can't know: asked, one that isn't says so.
        let devices: Vec<Value> = m.offering(&self.0.name).into_iter().map(|s| json!({ "key": hex(&s.key), "name": s.name, "here": s.key == self.key(), "online": true, "services": [self.0.name] })).collect();
        // The scopes its device runs, whose part it is: a program here may reach what they hold.
        let listed = m.service(&self.key());
        let device = listed.as_ref().and_then(|s| s.device);
        let runs_for: Vec<String> = m.scopes().into_iter().filter(|s| device.is_some_and(|d| m.runs_for(&d, s))).map(|s| s.to_string()).collect();
        let name = listed.as_ref().map(|s| s.name.as_str()).unwrap_or("");
        json!({
            "device": { "key": hex(&self.key()), "name": name, "runs_for": runs_for },
            "me": { "key": hex(&self.key()), "name": name },
            "user": self.owner_in(m).map(|u| json!({ "id": u, "name": m.user(&u).map(|p| p.name) })),
            "install": { "name": m.name(), "root": self.0.network.root, "devices": devices, "settings": crate::state::settings(&m.settings()) },
        })
    }
}

/// Whether a node, `remote`, may ask this service (`me`) for a call by `user` from their device `key`: this service
/// serves them (its own person, or everyone, as its listing says), and the node is one of their own, calling for its
/// own person; or, for a service that serves everyone, a node that runs their calls bringing their sign-in, which this
/// service checked (`signed_in`). A node of someone else's, an admin's too, never calls as them otherwise.
pub fn may_call(m: &ScopeDb, me: &Key, remote: &Key, key: &Key, user: &str, signed_in: bool) -> Result<()> {
    let service = m.service(me).with_context(|| format!("this service isn't one of {YOURS}'s yet"))?;
    let node = m.compute(remote).with_context(|| format!("only {YOURS}'s devices are answered"))?;
    ensure!(m.may(user, "use", &scopes::service(me)), "{} serves only its own person", service.service.as_deref().unwrap_or("this service"));
    let own = node.user == user && (*key == *remote || m.client(key).is_some_and(|c| c.user == user));
    ensure!(own || (service.shared && signed_in && m.serves(remote, user)), "{} may not call as that person", short(remote));
    Ok(())
}

/// What a service may ask its person's node to run: only its own app (`name`), in its person's own scope, calling no
/// other app, and naming a device only to reach itself there (Agent Devices listing every device's); until grants
/// land, that app's database there is all it reaches. → the call.
pub fn limited(name: &str, call: &Value) -> Result<Value> {
    ensure!(call["app"] == name && (call["service"].is_null() || call["service"] == name), "a service asks only for its own app, {name}");
    ensure!(call["device"].is_null() || (call["service"] == name && call["device"].is_string()), "a service names a device only to reach itself there");
    let scopes_ok = match &call["scopes"] {
        Value::Null => true,
        Value::Array(s) => s.iter().all(|s| s == "user"),
        _ => false,
    };
    ensure!(scopes_ok, "a service's calls reach only its person's own scope");
    let mut asked = json!({ "app": name, "service": call["service"], "op": call["op"], "input": call["input"], "scopes": ["user"] });
    if call["device"].is_string() {
        asked["device"] = call["device"].clone();
    }
    Ok(asked)
}

/// `known` with `given`'s member entries added, if any are new: → them, replayed from the network's first entry, with
/// those signed by no endpoint it ever listed passed over (junk anyone could sign counts for nothing, and isn't kept).
/// `depths` are those of the entries they follow that nodes don't bring.
fn merged(root: Hash, known: &[Entry], given: Vec<Entry>, depths: &HashMap<Hash, u64>, steps: &[Option<crate::database::steps::Given>]) -> Option<(Vec<Entry>, ScopeDb)> {
    let mut all: HashMap<String, Entry> = known.iter().map(|e| (e.hash().to_hex(), e.clone())).collect();
    let before = all.len();
    for e in given.into_iter().filter(members::is_member_entry) {
        all.entry(e.hash().to_hex()).or_insert(e);
    }
    if all.len() == before {
        return None;
    }
    Some(placed(root, all.into_values().collect(), depths, steps))
}

/// `entries` in the nodes' replay order, those that can be placed in it and are signed by an endpoint the network
/// listed, and the scope database they add up to.
fn placed(root: Hash, entries: Vec<Entry>, depths: &HashMap<Hash, u64>, steps: &[Option<crate::database::steps::Given>]) -> (Vec<Entry>, ScopeDb) {
    let ordered = crate::entry::causal(entries, depths);
    // Who's who is the scope database the freeze the network's recovery key signed anchors, as its genesis among the
    // entries names it, and the steps after.
    let built = crate::database::steps::recovery_of(root, &ordered)
        .context("its genesis isn't among the entries")
        .and_then(|recovery| crate::database::steps::Truth::read(steps, recovery, now_ms())?.map(|t| t.db).context("who's who has no freeze yet"));
    let db = built.unwrap_or_else(|err| {
        tracing::warn!("building the scope database: {err:#}");
        ScopeDb::build(&Members::default(), &crate::entry::Scope::Global, &Default::default(), 0).expect("an empty scope database builds")
    });
    let listed: std::collections::HashSet<Key> = db.machines().iter().map(|d| d.key).collect();
    (ordered.into_iter().filter(|e| e.hash() == root || listed.contains(&e.author())).collect(), db)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// The loopback's routes.

fn refused(code: StatusCode, why: impl std::fmt::Display) -> Response {
    (code, Json(json!({ "error": why.to_string() }))).into_response()
}

/// Whether a request is the program's on this machine: it sends the token in the service's folder.
fn ours(s: &Service, headers: &HeaderMap) -> Result<(), Response> {
    match headers.get(crate::api::LOCAL_TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        Some(t) if crate::keys::same_secret(t.as_bytes(), s.0.token.as_bytes()) => Ok(()),
        _ => Err(refused(StatusCode::UNAUTHORIZED, "send the local token in the service's folder")),
    }
}

fn this_one(s: &Service, app: &str) -> Result<(), Response> {
    match app == s.0.name {
        true => Ok(()),
        false => Err(refused(StatusCode::NOT_FOUND, format!("this is {}'s service", s.0.name))),
    }
}

async fn register(State(s): State<Service>, headers: HeaderMap, UrlPath(app): UrlPath<String>, Json(describe): Json<Value>) -> Response {
    if let Err(r) = ours(&s, &headers).and_then(|_| this_one(&s, &app)) {
        return r;
    }
    s.0.services.register(&app, describe);
    Json(json!({})).into_response()
}

async fn next_call(State(s): State<Service>, headers: HeaderMap, UrlPath(app): UrlPath<String>) -> Response {
    if let Err(r) = ours(&s, &headers).and_then(|_| this_one(&s, &app)) {
        return r;
    }
    match s.0.services.next(&app).await {
        Ok(Some(call)) => Json(json!(call)).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
    }
}

async fn reply(State(s): State<Service>, headers: HeaderMap, UrlPath((app, id)): UrlPath<(String, u64)>, Json(body): Json<Value>) -> Response {
    if let Err(r) = ours(&s, &headers).and_then(|_| this_one(&s, &app)) {
        return r;
    }
    match s.0.services.reply(&app, id, body) {
        Ok(()) => Json(json!({})).into_response(),
        Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
    }
}

async fn fill(State(s): State<Service>, headers: HeaderMap) -> Response {
    if let Err(r) = ours(&s, &headers) {
        return r;
    }
    refused(StatusCode::NOT_IMPLEMENTED, "a service can't fill in the Vault's items yet: the call's node fills them in before it asks")
}

async fn proxy(State(s): State<Service>, req: axum::extract::Request) -> Response {
    crate::sandbox::connect(s.0.declared.as_ref(), req).await
}

async fn state(State(s): State<Service>, headers: HeaderMap) -> Response {
    if let Err(r) = ours(&s, &headers) {
        return r;
    }
    Json(s.state()).into_response()
}

async fn call(State(s): State<Service>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = ours(&s, &headers) {
        return r;
    }
    match s.ask_node(body).await {
        Ok(answer) if answer.get("error").is_some() => refused(StatusCode::BAD_REQUEST, answer["error"].as_str().unwrap_or("failed")),
        Ok(answer) => Json(json!({ "ok": answer["ok"] })).into_response(),
        Err(err) => refused(StatusCode::BAD_GATEWAY, format!("{err:#}")),
    }
}

/// Whether this part should stop: this program's binary was replaced, by an update, so the new oned starts it again;
/// or the oned that started it is gone.
fn stop_check(parent: u32) -> Result<impl Fn() -> bool> {
    use std::os::unix::fs::MetadataExt;
    // Asked to stop (oned switching it off): it stops what it started first, on a Mac too, where nothing else would.
    extern "C" fn asked(_: libc::c_int) {
        ASKED.store(true, SeqCst);
    }
    // SAFETY: the handler only stores to an atomic.
    unsafe { libc::signal(libc::SIGTERM, asked as *const () as libc::sighandler_t) };
    let exe = std::env::current_exe()?.canonicalize()?;
    let ino = std::fs::metadata(&exe)?.ino();
    Ok(move || ASKED.load(SeqCst) || std::fs::metadata(&exe).map(|m| m.ino()).ok() != Some(ino) || std::os::unix::process::parent_id() != parent)
}

/// Whether this part was asked to stop.
static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `oned <name>`: the service `name` as a part of this device's oned, which started it, until oned stops or this
/// program is replaced: its program (`oned::program`), started here and again whenever it stops, told where to register (`ONE_ADDR`), where the token it sends is (`ONE_HOME`,
/// the service's folder) and where its data goes (`<NAME>_DATA`). One runs at a time: a part an update's oned starts
/// waits for the one before to stop.
pub fn run(name: &str) -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into());
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(filter).try_init();
    let dir = folder(name);
    std::fs::create_dir_all(&dir)?;
    let parent = std::os::unix::process::parent_id();
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock"))?;
    // SAFETY: a plain flock on a file this keeps open: held until this process exits. While another holds it, this
    // waits, unless the oned that started this one goes meanwhile.
    while unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        ensure!(std::os::unix::process::parent_id() == parent, "{name}: the oned that started it stopped");
        std::thread::sleep(Duration::from_millis(500));
    }
    let stop = stop_check(parent)?;
    let program = crate::oned::program(name);
    #[cfg(target_os = "macos")]
    let program = match crate::oned::helper(name) {
        Some(carried) => crate::oned::placed(&carried)?,
        None => program,
    };
    ensure!(program.exists(), "{name}'s program, {}, isn't there", program.display());
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let service = Service::open(&dir, name)?;
    let at = runtime.block_on(service.start())?;
    let data = crate::home().join(name);
    std::fs::create_dir_all(&data)?;
    host(name, &program, &data, at, &service, &stop)?;
    eprintln!("{name}: stopping, for oned to start it again");
    Ok(())
}

/// Where the endpoint says why its program can't start in its sandbox, in its folder, which oned shows on it.
pub const PROBLEM: &str = "problem";

/// A service's program running: a process of this one's, or its unit.
enum Running {
    Child(std::process::Child),
    Unit(String),
    /// A Mac helper app's program (`oned::helper`), a process macOS holds responsible for itself.
    #[cfg(target_os = "macos")]
    Own(libc::pid_t),
}

impl Running {
    /// How it stopped, once it has.
    fn done(&mut self) -> Result<Option<String>> {
        Ok(match self {
            Running::Child(child) => child.try_wait()?.map(|s| s.to_string()),
            Running::Unit(unit) => {
                let state = crate::sandbox::systemctl(&["is-active", unit])?;
                (!["active", "activating", "reloading"].contains(&state.as_str())).then_some(state)
            }
            #[cfg(target_os = "macos")]
            Running::Own(pid) => {
                let mut status = 0;
                // SAFETY: waits without blocking for a child of this process.
                match unsafe { libc::waitpid(*pid, &mut status, libc::WNOHANG) } {
                    0 => None,
                    -1 => Some("gone".into()),
                    _ => Some(format!("wait status {status}")),
                }
            }
        })
    }

    fn end(&mut self) {
        match self {
            Running::Child(child) => end(child),
            Running::Unit(unit) => {
                if let Err(err) = crate::sandbox::systemctl(&["stop", unit]) {
                    eprintln!("stopping {unit}: {err:#}");
                }
            }
            #[cfg(target_os = "macos")]
            Running::Own(pid) => {
                // SAFETY: signals to, and waits for, a child of this process, which it hasn't reaped.
                unsafe {
                    libc::kill(*pid, libc::SIGTERM);
                    for _ in 0..100 {
                        if libc::waitpid(*pid, std::ptr::null_mut(), libc::WNOHANG) != 0 {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    libc::kill(*pid, libc::SIGKILL);
                    libc::waitpid(*pid, std::ptr::null_mut(), 0);
                }
            }
        }
    }
}

/// Starts `command` as a process macOS holds responsible for itself, not for this one: so a Mac helper app's
/// permissions are asked for and recorded under its own name, not the Mac app's that started oned. Its id.
#[cfg(target_os = "macos")]
fn disclaimed(command: &std::process::Command) -> Result<libc::pid_t> {
    use std::ffi::{CString, OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        // libSystem's, as Chromium and LLDB start what they run.
        fn responsibility_spawnattrs_setdisclaim(attr: *mut libc::posix_spawnattr_t, disclaim: libc::c_int) -> libc::c_int;
    }
    let c = |s: &OsStr| CString::new(s.as_bytes());
    let program = c(command.get_program())?;
    let args: Vec<CString> = std::iter::once(command.get_program()).chain(command.get_args()).map(c).collect::<Result<_, _>>()?;
    let mut vars: std::collections::BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (k, v) in command.get_envs() {
        match v {
            Some(v) => vars.insert(k.into(), v.into()),
            None => vars.remove(k),
        };
    }
    let env: Vec<CString> = vars.iter().map(|(k, v)| CString::new([k.as_bytes(), b"=", v.as_bytes()].concat())).collect::<Result<_, _>>()?;
    let pointers = |all: &[CString]| all.iter().map(|a| a.as_ptr() as *mut libc::c_char).chain(std::iter::once(std::ptr::null_mut())).collect::<Vec<_>>();
    let (argv, envp) = (pointers(&args), pointers(&env));
    // SAFETY: the attributes are made, used and destroyed here; every string outlives the call.
    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        ensure!(libc::posix_spawnattr_init(&mut attr) == 0, "posix_spawnattr_init failed");
        let mut pid = 0;
        let mut r = responsibility_spawnattrs_setdisclaim(&mut attr, 1);
        if r == 0 {
            r = libc::posix_spawn(&mut pid, program.as_ptr(), std::ptr::null(), &attr, argv.as_ptr(), envp.as_ptr());
        }
        libc::posix_spawnattr_destroy(&mut attr);
        ensure!(r == 0, "{}", std::io::Error::from_raw_os_error(r));
        Ok(pid)
    }
}

/// Starts a service's program, in its sandbox as its app declares and this machine can give it.
fn begin(name: &str, program: &Path, data: &Path, at: std::net::SocketAddr, service: &Service) -> Result<Running> {
    use crate::sandbox::{How, Launch};
    let declared = service.0.declared.clone().unwrap_or_default();
    let data_var = format!("{}_DATA", name.to_uppercase().replace('-', "_"));
    let launcher = PathBuf::from(std::env::current_exe()?.to_string_lossy().trim_end_matches(" (deleted)"));
    let mut command = match crate::sandbox::how(service.0.declared.as_ref(), "") {
        How::Unit => {
            let launch = Launch { name: name.into(), own_network: !declared.any_host(), ports: declared.local_ports(), data: None };
            crate::sandbox::prepare(name, program, &launcher, service.token(), &launch)?;
            let unit = crate::sandbox::unit(name);
            // One left by an endpoint that stopped without stopping it goes first: two mustn't share its data.
            let _ = crate::sandbox::systemctl(&["stop", &unit]);
            let _ = crate::sandbox::systemctl(&["reset-failed", &unit]);
            crate::sandbox::systemctl(&["start", &unit])?;
            return Ok(Running::Unit(unit));
        }
        How::Person => {
            let launch = Launch { name: name.into(), own_network: !declared.any_host(), ports: declared.local_ports(), data: Some(data.to_path_buf()) };
            let dir = crate::sandbox::prepare(name, program, &launcher, service.token(), &launch)?;
            crate::oned::command(&[&format!("--sandboxed={}", dir.display())])?
        }
        How::Seatbelt => {
            let token = service.0.dir.join(crate::api::LOCAL_TOKEN);
            let profile = crate::sandbox::seatbelt(&declared, at.port(), data, &token, program);
            let mut command = std::process::Command::new("/usr/bin/sandbox-exec");
            command.arg("-p").arg(profile).arg(program);
            let proxy = format!("http://{at}");
            for var in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
                command.env(var, &proxy);
            }
            command.env("NO_PROXY", "localhost,127.0.0.1").env("no_proxy", "localhost,127.0.0.1");
            if !declared.person {
                let tmp = data.join("tmp");
                std::fs::create_dir_all(&tmp)?;
                command.env("TMPDIR", tmp);
            }
            command
        }
        How::Plain(why) => {
            if let Some(why) = why {
                eprintln!("{name} runs without its sandbox: {why}");
            }
            std::process::Command::new(program)
        }
    };
    command.env("ONE_ADDR", at.to_string()).env("ONE_HOME", &service.0.dir).env("ONE_CLI", crate::config::CLI).env(&data_var, data);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // It stops with this part, even one killed: two of a service on the same data (WhatsApp's link) mustn't run.
        // SAFETY: only an async-signal-safe call, between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(target_os = "macos")]
    if crate::oned::helper(name).is_some() {
        return Ok(Running::Own(disclaimed(&command).with_context(|| format!("starting {}", program.display()))?));
    }
    Ok(Running::Child(command.spawn().with_context(|| format!("starting {}", program.display()))?))
}

/// Runs a service's program in another language, started again whenever it stops, until `stop`. One that can't start
/// in its sandbox says why (`PROBLEM`), and is tried again a minute on.
fn host(name: &str, program: &Path, data: &Path, at: std::net::SocketAddr, service: &Service, stop: &impl Fn() -> bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let built = || std::fs::metadata(program).map(|m| (m.ino(), m.mtime())).ok();
    let problem = service.0.dir.join(PROBLEM);
    let before = crate::sandbox::dir(name).join(crate::sandbox::BEFORE);
    while !stop() {
        let started = built();
        let mut running = match begin(name, program, data, at, service) {
            Ok(r) => {
                let _ = std::fs::remove_file(&problem);
                r
            }
            Err(err) => {
                eprintln!("{name}: {err:#}");
                let _ = std::fs::write(&problem, format!("Couldn’t start it in its sandbox: {err:#}"));
                for _ in 0..60 {
                    if stop() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                continue;
            }
        };
        loop {
            if let Some(status) = running.done()? {
                eprintln!("{} stopped ({status}): starting it again", program.display());
                std::thread::sleep(Duration::from_secs(2));
                break;
            }
            // Its data from before, moved into its sandbox's own folder before it started, is done with once it's up.
            if service.services().has(name) && before.exists() {
                let _ = std::fs::remove_dir_all(&before);
            }
            // Stopped for good, or for its new build, put in its place (`<cli> services try`).
            let renewed = built() != started;
            if stop() || renewed {
                running.end();
                if renewed {
                    eprintln!("{}: a new build is there, so it starts again", program.display());
                }
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}

/// Stops a program: asked, then made to, if it hasn't in ten seconds, so it closes what it keeps (WhatsApp's link).
fn end(child: &mut std::process::Child) {
    // SAFETY: a signal to a child of this process, which it hasn't reaped.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    for _ in 0..100 {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The steps a service kept (`take_steps`).
fn read_steps(path: &Path) -> Vec<Option<crate::database::steps::Given>> {
    let kept: Vec<Value> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    kept.iter()
        .enumerate()
        .map(|(i, s)| crate::database::steps::given(&crate::entry::Scope::Global, i as u64 + 1, s["step"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok()).as_deref()))
        .collect()
}
