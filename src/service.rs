//! A service endpoint: a device's abilities (a sidecar) offered over iroh, with no node on its machine. It's an
//! endpoint of its own key, kept in its folder, and gets in when a person types the code it shows into one of their
//! devices (`one devices approve`), which lists it as their service offering it and tells the relay to let it in. It
//! holds no keys and reads no bucket: the nodes that call it bring it `global`'s entries, whose member entries it
//! replays from the install's first entry (given where it's installed, by its person's own machine, so nothing on the
//! way can make it another install's) and checks each caller by itself: a compute endpoint of the install that runs
//! the calls of a person it serves. The program offering it registers over a loopback with the sidecars' API and long-polls for calls,
//! whether it's this one (`one sidecars serve`, `sidecars::answer_hosted`) or one in another language (`one sidecars
//! host`, which starts it), so the Go sidecars don't change. A service writes nothing itself: what it asks for is a
//! call, which a node of its person's runs (`/v1/call` on the loopback).

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
use crate::members::{self, Members};
use crate::sidecars::Sidecars;

/// The most an ask or its answer may carry, as between nodes.
const MESSAGE_LIMIT: usize = 64 << 20;
/// The most a computer this service doesn't know of yet may send it: `global`'s entries.
const STRANGER_LIMIT: usize = 16 << 20;
/// How many messages a computer this service doesn't know of may send on a connection: a hello, the install's
/// entries, and a hello again.
const STRANGER_MESSAGES: usize = 3;
/// How many computers it doesn't know of may be connected at once.
const STRANGERS: usize = 4;
/// How many streams a node may have open on a connection at once.
const STREAMS: u32 = 32;
/// How long reaching a node may take.
const CONNECT: Duration = Duration::from_secs(10);

/// The install a service is in: its address, where its relay and sign-in are, and the hash of its first entry, which
/// its member entries are replayed from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Install {
    pub address: String,
    pub root: String,
}

impl Install {
    /// From what `one sidecars install-link` prints on a device of the install: `<address>#<root>`.
    pub fn parse(link: &str) -> Result<Install> {
        let (address, root) = link.trim().rsplit_once('#').context("that's your One App's address and its first entry: <address>#<hash>, as one sidecars install-link prints it")?;
        Hash::from_hex(root).context("after the # is the hash of your One App's first entry")?;
        ensure!(address.starts_with("http://") || address.starts_with("https://"), "before the # is your One App's address");
        Ok(Install { address: address.trim_end_matches('/').to_string(), root: root.to_string() })
    }

    pub fn link(&self) -> String {
        format!("{}#{}", self.address, self.root)
    }
}

/// A service endpoint, offering one sidecar.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

struct Inner {
    name: String,
    dir: PathBuf,
    key: SigningKey,
    /// Its machine's name, which its request to join shows.
    machine: String,
    install: Install,
    /// `global`'s entries, as the nodes brought them, and what its member entries add up to.
    known: Mutex<(Vec<Entry>, Members)>,
    /// Where the program offering it registers and waits for calls.
    sidecars: Sidecars,
    /// What that program sends to say it's on this machine.
    token: String,
    endpoint: Mutex<Option<Endpoint>>,
    /// A connection to each node it asks for calls.
    conns: Mutex<HashMap<Key, Connection>>,
    strangers: AtomicUsize,
}

/// Where a service keeps its key, its install and what it knows of it.
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
    /// The service `name` kept in `dir`: its key (made the first time), and the install it's in, `install` the first
    /// time, which can't change after.
    pub fn open(dir: &Path, name: &str, install: Option<Install>) -> Result<Service> {
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
        let install_file = dir.join("install.json");
        let kept: Option<Install> = std::fs::read(&install_file).ok().map(|b| serde_json::from_slice(&b)).transpose()?;
        let install = match (kept, install) {
            (Some(kept), Some(given)) if kept != given => bail!("{name} here is in another One App already ({}): remove {} to move it", kept.address, dir.display()),
            (Some(kept), _) => kept,
            (None, Some(given)) => {
                write_private(&install_file, &serde_json::to_vec_pretty(&given)?)?;
                given
            }
            (None, None) => bail!("say which One App {name} is for: --install <link>, as one sidecars install-link prints it on one of its devices"),
        };
        let token = match std::fs::read_to_string(dir.join(crate::api::LOCAL_TOKEN)) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                let t = hex(&rand::random::<[u8; 32]>());
                write_private(&dir.join(crate::api::LOCAL_TOKEN), t.as_bytes())?;
                t
            }
        };
        let root = Hash::from_hex(&install.root)?;
        let entries = std::fs::read(dir.join("entries")).ok().and_then(|b| crate::entry::unpack(&b).ok()).unwrap_or_default();
        let members = replay(root, &entries);
        Ok(Service(Arc::new(Inner {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            key,
            machine: hostname(),
            install,
            known: Mutex::new((entries, members)),
            sidecars: Sidecars::default(),
            token,
            endpoint: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            strangers: AtomicUsize::new(0),
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

    /// The code a person types on one of their devices to make it theirs.
    pub fn code(&self) -> String {
        crate::keys::join_code(&self.endpoint())
    }

    /// Where the program offering it registers.
    pub fn sidecars(&self) -> &Sidecars {
        &self.0.sidecars
    }

    pub fn token(&self) -> &str {
        &self.0.token
    }

    /// Whose service it is, once a person's device approved it.
    pub fn owner(&self) -> Option<String> {
        self.owner_in(&self.0.known.lock().unwrap().1)
    }

    fn owner_in(&self, m: &Members) -> Option<String> {
        m.service(&self.key()).filter(|s| s.service.as_deref() == Some(self.0.name.as_str())).map(|s| s.user.clone())
    }

    /// Starts it: its endpoint, through the install's relay (which lets it in once it's approved), asking to be
    /// approved until it is, and the loopback the program offering it registers at. → that loopback's address.
    pub async fn start(&self) -> Result<std::net::SocketAddr> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let at = listener.local_addr()?;
        let router = self.loopback();
        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                tracing::warn!("the service's loopback stopped: {err:#}");
            }
        });
        tokio::spawn(self.clone().keep_endpoint());
        tokio::spawn(self.clone().ask_to_join());
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
        let (_, map) = crate::node::relay(&self.0.install.address, None)?;
        let transport = QuicTransportConfig::builder().max_concurrent_bidi_streams(STREAMS.into()).max_concurrent_uni_streams(0u32.into()).build();
        Ok(Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&self.0.key.to_bytes()))
            .alpns(vec![crate::node::service_alpn()])
            .transport_config(transport)
            .relay_mode(RelayMode::Custom(map))
            .bind()
            .await?)
    }

    /// Leaves its request to join at the install's sign-in service, signed with its own key, and says the code to
    /// approve it with; again before it runs out, until a device approves it.
    async fn ask_to_join(self) {
        let mut asked: Option<std::time::Instant> = None;
        loop {
            if let Some(owner) = self.owner() {
                let _ = std::fs::remove_file(self.0.dir.join("code"));
                if asked.is_some() {
                    let name = self.0.known.lock().unwrap().1.user(&owner).map(|u| u.name.clone()).unwrap_or(owner);
                    eprintln!("{} is approved: it answers {name}'s devices.", self.0.name);
                }
                return;
            }
            if asked.is_none_or(|a| a.elapsed() > Duration::from_secs(10 * 60)) {
                match self.leave_request().await {
                    Ok(()) => {
                        let code = self.code();
                        let _ = write_private(&self.0.dir.join("code"), code.as_bytes());
                        eprintln!("To make {} on {} yours, approve it on one of your devices: {} devices approve {code}", self.0.name, self.0.machine, crate::config::CLI);
                        asked = Some(std::time::Instant::now());
                    }
                    Err(err) => {
                        eprintln!("asking your One App to approve {}: {err:#}", self.0.name);
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn leave_request(&self) -> Result<()> {
        let me = self.endpoint();
        let path = "/api/auth/devices/requests";
        let fields = json!({ "route": format!("POST {path}"), "key": hex(&me.key), "seal": hex(&me.seal), "name": self.0.machine, "service": self.0.name });
        let body = crate::keys::signed_request(&self.0.key, "one/auth", fields, now_ms());
        let url = format!("{}{path}", self.0.install.address);
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
            let mut resp = agent.post(&url).send_json(&body)?;
            let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            ensure!(resp.status().is_success(), "{}", out["error"].as_str().unwrap_or("the sign-in service didn't answer"));
            Ok(())
        })
        .await?
    }

    /// Answers what a computer asks on a connection it opened: a node of the install anything a node asks a service;
    /// one it doesn't know of only a hello and the install's entries, a few messages, a few at once, since anyone who
    /// reaches it directly can connect.
    async fn take(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        let known = self.is_node(&remote);
        if !known && self.0.strangers.fetch_add(1, SeqCst) >= STRANGERS {
            self.0.strangers.fetch_sub(1, SeqCst);
            return conn.close(1u32.into(), b"not a device of this install");
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
            conn.close(1u32.into(), b"not a device of this install");
        }
    }

    async fn answer(&self, remote: Key, msg: &Value) -> Value {
        let body = &msg["body"];
        match msg["kind"].as_str() {
            Some("hello") => self.hello(remote, body["entries"].as_u64().unwrap_or(0) as usize),
            Some("entries") => match self.take_entries(body["pack"].as_str().unwrap_or("")) {
                Ok(()) => json!({ "ok": true }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            },
            Some("ask") if self.is_node(&remote) => self.run(remote, &body["call"]).await,
            _ => json!({ "error": "only your One App's devices are answered" }),
        }
    }

    /// A node's hello, with how many of `global`'s entries it has: → what this service offers and whom it
    /// serves, to a node of the install that may call it; that it's behind, if the node has more entries than it.
    fn hello(&self, remote: Key, entries: usize) -> Value {
        let (count, may) = {
            let known = self.0.known.lock().unwrap();
            (known.0.len(), known.1.compute(&remote).is_some() && known.1.service(&self.key()).is_some())
        };
        if entries > count {
            return json!({ "behind": true });
        }
        if !may {
            return json!({ "error": "only your One App's devices are answered" });
        }
        let Some(mut describe) = self.0.sidecars.describe().into_iter().next() else {
            return json!({ "error": format!("{} isn't running here yet", self.0.name) });
        };
        describe["runs"] = json!("service");
        let known = self.0.known.lock().unwrap();
        let m = &known.1;
        let users: Vec<&String> = match (m.service(&self.key()), self.0.sidecars.serves_everyone(&self.0.name)) {
            (_, true) => m.users.keys().filter(|u| m.user(u).is_some()).collect(),
            (Some(me), false) => vec![&me.user],
            (None, false) => Vec::new(),
        };
        json!({ "describe": describe, "users": users })
    }

    /// Takes `global`'s entries a node brought, if there are more than it has and they're the install's: each signed,
    /// its member entries replayed from its first entry.
    fn take_entries(&self, pack: &str) -> Result<()> {
        let bytes = data_encoding::BASE64.decode(pack.as_bytes()).map_err(|_| anyhow!("the entries aren't base64"))?;
        let entries = crate::entry::unpack(&bytes)?;
        let root = Hash::from_hex(&self.0.install.root)?;
        let members = replay(root, &entries);
        ensure!(!members.users.is_empty(), "those aren't your One App's entries");
        let mut known = self.0.known.lock().unwrap();
        if entries.len() <= known.0.len() {
            return Ok(());
        }
        write_private(&self.0.dir.join("entries"), &crate::entry::pack(&entries.iter().collect::<Vec<_>>()))?;
        *known = (entries, members);
        Ok(())
    }

    /// Whether `key` is a node of the install, as this service knows it.
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
        let everyone = self.0.sidecars.serves_everyone(&self.0.name);
        let caller = {
            let known = self.0.known.lock().unwrap();
            let m = &known.1;
            if let Err(err) = may_call(m, &self.key(), &remote, &key, &user, everyone) {
                return json!({ "error": format!("{err:#}") });
            }
            let person = m.user(&user);
            crate::apps::Me {
                key: hex(&key),
                user: user.clone(),
                name: m.machine(&key).map(|d| d.name.clone()).unwrap_or_default(),
                role: person.map(|u| u.role),
                user_name: person.map(|u| u.name.clone()).unwrap_or_default(),
                agent: me["agent"].as_str().map(str::to_string),
            }
        };
        let input = if call["input"].is_null() { json!({}) } else { call["input"].clone() };
        let from = call["from"].as_str().map(str::to_string);
        match self.0.sidecars.call(&self.0.name, call["op"].as_str().unwrap_or(""), input, caller, from).await {
            Ok(answer) => json!({ "ok": answer.ok }),
            Err(err) if err.is::<crate::sidecars::Elsewhere>() => json!({ "elsewhere": err.to_string() }),
            Err(err) => json!({ "error": format!("{err:#}") }),
        }
    }

    /// Asks a node of its person's to run a call, as them: the first that answers, of those that run their calls.
    pub async fn ask_node(&self, mut call: Value) -> Result<Value> {
        let owner = self.owner().context("this service isn't approved yet")?;
        call["me"] = json!({ "key": hex(&self.key()), "user": owner });
        let nodes: Vec<Key> = {
            let known = self.0.known.lock().unwrap();
            let m = &known.1;
            m.computes().filter(|c| m.serves(&c.key, &owner)).map(|c| c.key).collect()
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
        let endpoint = self.0.endpoint.lock().unwrap().clone().context("this service isn't connected to your One App's relay yet")?;
        let held = self.0.conns.lock().unwrap().get(&to).filter(|c| c.close_reason().is_none()).cloned();
        let conn = match held {
            Some(conn) => conn,
            None => {
                let (url, _) = crate::node::relay(&self.0.install.address, None)?;
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

    /// The loopback the program offering it registers at, with the local token in its folder: the sidecars' API, its
    /// machine as `/v1/state` says, and `/v1/call`, which a node of its person's runs.
    fn loopback(&self) -> Router {
        Router::new()
            .route("/v1/sidecars/{app}", post(register))
            .route("/v1/sidecars/{app}/calls", get(next_call))
            .route("/v1/sidecars/{app}/calls/{id}", post(reply))
            .route("/v1/sidecars/{app}/calls/{id}/vault", post(fill))
            .route("/v1/state", get(state))
            .route("/v1/call", post(call))
            .with_state(self.clone())
    }

    /// What `/v1/state` says here: this service as the device, and the others offering it, as far as it knows.
    fn state(&self) -> Value {
        let known = self.0.known.lock().unwrap();
        let m = &known.1;
        let devices: Vec<Value> = m.services(&self.0.name).map(|s| json!({ "key": hex(&s.key), "name": s.name, "online": s.key == self.key(), "sidecars": [self.0.name] })).collect();
        json!({
            "device": { "key": hex(&self.key()), "name": self.0.machine },
            "user": self.owner_in(m).map(|u| json!({ "id": u, "name": m.user(&u).map(|p| p.name.clone()) })),
            "install": { "name": m.name, "root": self.0.install.root, "devices": devices, "settings": crate::state::settings(m) },
        })
    }
}

/// Whether a node, `remote`, may ask this service (`me`) for a call by `user` from their device `key`: it's a compute
/// endpoint of the install that runs their calls, `key` is their own device or such a node, and this service serves
/// them, as its own person's, or everyone's (`everyone`).
pub fn may_call(m: &Members, me: &Key, remote: &Key, key: &Key, user: &str, everyone: bool) -> Result<()> {
    let service = m.service(me).context("this service isn't one of your One App's yet")?;
    ensure!(m.compute(remote).is_some(), "only your One App's devices are answered");
    let theirs = m.client(key).is_some_and(|c| c.user == user) || m.serves(key, user);
    ensure!(m.serves(remote, user) && theirs, "{} doesn't run that person's calls", short(remote));
    ensure!(service.user == user || (everyone && m.user(user).is_some()), "{} serves only its own person", service.service.as_deref().unwrap_or("this service"));
    Ok(())
}

fn replay(root: Hash, entries: &[Entry]) -> Members {
    let ordered = crate::entry::causal(entries.to_vec(), &HashMap::new());
    let member_entries: Vec<Entry> = ordered.into_iter().filter(members::is_member_entry).collect();
    Members::replay(Some(root), &member_entries)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn hostname() -> String {
    std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "device".into())
}

// The loopback's routes.

fn refused(code: StatusCode, why: impl std::fmt::Display) -> Response {
    (code, Json(json!({ "error": why.to_string() }))).into_response()
}

/// Whether a request is the program's on this machine: it sends the token in the service's folder.
fn ours(s: &Service, headers: &HeaderMap) -> Result<(), Response> {
    match headers.get(crate::api::LOCAL_TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        Some(t) if t == s.0.token => Ok(()),
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
    s.0.sidecars.register(&app, describe);
    Json(json!({})).into_response()
}

async fn next_call(State(s): State<Service>, headers: HeaderMap, UrlPath(app): UrlPath<String>) -> Response {
    if let Err(r) = ours(&s, &headers).and_then(|_| this_one(&s, &app)) {
        return r;
    }
    match s.0.sidecars.next(&app).await {
        Ok(Some(call)) => Json(json!(call)).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => refused(StatusCode::BAD_REQUEST, format!("{err:#}")),
    }
}

async fn reply(State(s): State<Service>, headers: HeaderMap, UrlPath((app, id)): UrlPath<(String, u64)>, Json(body): Json<Value>) -> Response {
    if let Err(r) = ours(&s, &headers).and_then(|_| this_one(&s, &app)) {
        return r;
    }
    match s.0.sidecars.reply(&app, id, body) {
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
    let call = json!({ "app": body["app"], "sidecar": body["sidecar"], "op": body["op"], "input": body["input"], "device": body["device"], "scopes": body["scopes"] });
    match s.ask_node(call).await {
        Ok(answer) if answer.get("error").is_some() => refused(StatusCode::BAD_REQUEST, answer["error"].as_str().unwrap_or("failed")),
        Ok(answer) => Json(json!({ "ok": answer["ok"] })).into_response(),
        Err(err) => refused(StatusCode::BAD_GATEWAY, format!("{err:#}")),
    }
}

/// Whether this program's binary was replaced, by an update: it stops, for its service manager to start the new one.
fn replaced_check() -> Result<impl Fn() -> bool> {
    use std::os::unix::fs::MetadataExt;
    let exe = std::env::current_exe()?.canonicalize()?;
    let ino = std::fs::metadata(&exe)?.ino();
    Ok(move || std::fs::metadata(&exe).map(|m| m.ino()).ok() != Some(ino))
}

/// Starts the service `name` on a runtime of its own, kept in its folder, in `install` the first time.
fn started(name: &str, install: Option<&str>) -> Result<(Service, std::net::SocketAddr, tokio::runtime::Runtime)> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into());
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(filter).try_init();
    let install = install.map(Install::parse).transpose()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let service = Service::open(&folder(name), name, install)?;
    let at = runtime.block_on(service.start())?;
    Ok((service, at, runtime))
}

/// `one sidecars serve <name>`: a sidecar this program hosts (`sidecars::hosted`), as a service endpoint, until this
/// program is replaced.
pub fn serve(name: &str, install: Option<&str>) -> Result<()> {
    crate::sidecars::hosted().iter().find(|h| h.name == name).ok_or_else(|| anyhow!("{name} isn't a sidecar this program hosts"))?;
    let replaced = replaced_check()?;
    let (service, at, _runtime) = started(name, install)?;
    let data = crate::home().join(name);
    std::fs::create_dir_all(&data)?;
    crate::sidecars::answer_hosted(name, data, at.to_string(), service.token().to_string(), &replaced)?;
    eprintln!("{name}: this program was replaced, so it stops for the new one");
    Ok(())
}

/// `one sidecars host <name> <program> [args…]`: a sidecar in another language as a service endpoint, the program
/// started here and again whenever it stops, told where to register (`ONE_ADDR`) and where the token it sends is
/// (`ONE_HOME`, the service's folder), until this program is replaced.
pub fn host(name: &str, program: &[String], install: Option<&str>) -> Result<()> {
    let (exe, args) = program.split_first().context("say which program offers it")?;
    let replaced = replaced_check()?;
    let (service, at, _runtime) = started(name, install)?;
    while !replaced() {
        let mut child = std::process::Command::new(exe)
            .args(args)
            .env("ONE_ADDR", at.to_string())
            .env("ONE_HOME", &service.0.dir)
            .env("ONE_CLI", crate::config::CLI)
            .spawn()
            .with_context(|| format!("starting {exe}"))?;
        loop {
            if let Some(status) = child.try_wait()? {
                eprintln!("{exe} stopped ({status}): starting it again");
                std::thread::sleep(Duration::from_secs(2));
                break;
            }
            if replaced() {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    eprintln!("{name}: this program was replaced, so it stops for the new one");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Content, Ext, Head};
    use crate::keys::Endpoint;
    use crate::members::{Kind, Role};

    /// Signs `content` as `user` with `key`, after `prev`, its previous entry in `global`.
    fn entry(key: &SigningKey, user: &str, prev: &mut Option<Head>, content: Content) -> Entry {
        let ext = Ext { log: "global".into(), time: 1, user: user.into(), epoch: 0, previous: Vec::new() };
        let e = Entry::create(key, ext, *prev, content);
        *prev = Some(e.head());
        e
    }

    #[test]
    fn a_service_answers_only_nodes_that_run_its_persons_calls() {
        let key = || SigningKey::from_bytes(&rand::random());
        let (mac, sams, service, stranger) = (key(), key(), key(), key());
        let id = |k: &SigningKey| *k.verifying_key().as_bytes();
        let (andre, sam) = ("a000000000000000", "b000000000000000");
        let mut prev = None;
        let genesis = entry(&mac, andre, &mut prev, members::genesis("Home", andre, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client, Kind::Compute], &id(&key())));
        let entries = vec![
            genesis.clone(),
            entry(&mac, andre, &mut prev, members::device(&Endpoint::of(&sams), sam, "sam's box", &[Kind::Client, Kind::Compute], Some(("Sam", Role::Member)))),
            entry(&mac, andre, &mut prev, members::service(&Endpoint::of(&service), andre, "linux", "whatsapp")),
        ];
        let m = replay(genesis.hash(), &entries);
        assert_eq!(m.service(&id(&service)).map(|s| s.user.as_str()), Some(andre));
        let may = |node: &SigningKey, user: &str, everyone: bool| may_call(&m, &id(&service), &id(node), &id(node), user, everyone);
        may(&mac, andre, false).unwrap();
        let other = may(&sams, sam, false).unwrap_err();
        assert!(format!("{other:#}").contains("serves only its own person"), "{other:#}");
        assert!(may(&sams, andre, false).is_err(), "a node that doesn't run Andre's calls can't call as him");
        assert!(may(&stranger, andre, false).is_err(), "a stranger is refused");
        may(&sams, sam, true).unwrap();
        // Entries that aren't the install's say nothing.
        assert!(replay(Hash::from_bytes(rand::random()), &entries).users.is_empty());
    }
}
