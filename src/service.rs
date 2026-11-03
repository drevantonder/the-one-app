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

/// The install a service is in: its address, where its relay and sign-in are, the hash of its first entry, which its
/// member entries are replayed from, the person whose service it is to be, and the one-time secret their device minted
/// for it (`one sidecars install-link`), which proves to that device that its request is this service's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Install {
    pub address: String,
    pub root: String,
    pub user: String,
    pub secret: String,
}

impl Install {
    /// From what `one sidecars install-link` prints on its person's device: `<address>#<root>/<user>/<secret>`.
    pub fn parse(link: &str) -> Result<Install> {
        let said = "that's what one sidecars install-link prints on one of your devices: <address>#<hash>/<you>/<secret>";
        let (address, rest) = link.trim().rsplit_once('#').context(said)?;
        let mut parts = rest.split('/');
        let (Some(root), Some(user), Some(secret), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { bail!("{said}") };
        Hash::from_hex(root).context("after the # is the hash of your One App's first entry")?;
        ensure!(crate::entry::is_id(user), "{said}");
        ensure!(unhex(secret).is_ok(), "{said}");
        ensure!(address.starts_with("http://") || address.starts_with("https://"), "before the # is your One App's address");
        Ok(Install { address: address.trim_end_matches('/').to_string(), root: root.to_string(), user: user.to_string(), secret: secret.to_string() })
    }

    pub fn link(&self) -> String {
        format!("{}#{}/{}/{}", self.address, self.root, self.user, self.secret)
    }

    /// The same install for the same person, whatever secret each carries.
    fn same(&self, other: &Install) -> bool {
        (&self.address, &self.root, &self.user) == (&other.address, &other.root, &other.user)
    }
}

/// A service endpoint, offering one sidecar.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

/// Auth's refusal (403) of a service's request: its install link was used or ran out, or the request isn't signed as it should be.
#[derive(Debug)]
struct LinkRefused(String);

impl std::fmt::Display for LinkRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LinkRefused {}

struct Inner {
    name: String,
    dir: PathBuf,
    key: SigningKey,
    /// Its machine's name, which its request to join shows.
    machine: String,
    install: Install,
    /// `global`'s entries, as the nodes brought them, and what its member entries add up to.
    known: Mutex<(Vec<Entry>, Members)>,
    /// The depths in `global`'s replay order of the entries its member entries follow that nodes didn't bring: a service
    /// is brought only member entries, and orders them as the nodes do with these.
    depths: Mutex<HashMap<Hash, u64>>,
    /// Where the program offering it registers and waits for calls.
    sidecars: Sidecars,
    /// What that program sends to say it's on this machine.
    token: String,
    endpoint: Mutex<Option<Endpoint>>,
    /// A connection to each node it asks for calls.
    conns: Mutex<HashMap<Key, Connection>>,
    strangers: AtomicUsize,
    /// The install's sign-in service, whose keys check a sign-in a node passes on to a service that serves everyone.
    issuer: crate::kernel::Issuer,
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
            (Some(kept), Some(given)) if !kept.same(&given) => bail!("{name} here is someone else's, or in another One App ({}): remove {} to move it", kept.address, dir.display()),
            (Some(kept), None) => kept,
            // A new link, from the same person, for a new request once the last one's secret ran out.
            (_, Some(given)) => {
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
        let install_address = install.address.clone();
        let entries = std::fs::read(dir.join("entries")).ok().and_then(|b| crate::entry::unpack(&b).ok()).unwrap_or_default();
        let depths: HashMap<Hash, u64> = std::fs::read(dir.join("depths")).ok().and_then(|b| serde_json::from_slice::<HashMap<String, u64>>(&b).ok()).map(|m| m.into_iter().filter_map(|(h, d)| Some((Hash::from_hex(&h).ok()?, d))).collect()).unwrap_or_default();
        let (entries, members) = placed(root, entries, &depths);
        Ok(Service(Arc::new(Inner {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            key,
            machine: hostname(),
            install,
            known: Mutex::new((entries, members)),
            depths: Mutex::new(depths),
            sidecars: Sidecars::default(),
            token,
            endpoint: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            strangers: AtomicUsize::new(0),
            issuer: crate::kernel::Issuer::new(&install_address),
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
        // Only a listing as the person whose link it was given counts: nobody else can make it theirs.
        m.service(&self.key()).filter(|s| s.service.as_deref() == Some(self.0.name.as_str()) && s.user == self.0.install.user).map(|s| s.user.clone())
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
        let mut asked_for_good = false;
        loop {
            if let Some(owner) = self.owner() {
                let _ = std::fs::remove_file(self.0.dir.join("code"));
                if asked.is_some() {
                    let name = self.0.known.lock().unwrap().1.user(&owner).map(|u| u.name.clone()).unwrap_or(owner);
                    eprintln!("{} is approved: it answers {name}'s devices.", self.0.name);
                }
                return;
            }
            // Once the program offering it has said what it is, so the request says whether it serves everyone.
            let registered = !self.0.sidecars.describe().is_empty();
            if registered && !asked_for_good && asked.is_none_or(|a| a.elapsed() > Duration::from_secs(10 * 60)) {
                match self.request_approval().await {
                    Ok(()) => {
                        let code = self.code();
                        let _ = write_private(&self.0.dir.join("code"), code.as_bytes());
                        eprintln!("To make {} on {} yours, approve it on one of your devices: {} devices approve {code}", self.0.name, self.0.machine, crate::config::CLI);
                        asked = Some(std::time::Instant::now());
                    }
                    // Its install link was used (a device approved it, and a node will bring the entries that say so) or ran
                    // out: asking again with it is no use, so it says what to do once, and waits for a node.
                    Err(err) if err.downcast_ref::<LinkRefused>().is_some() => {
                        eprintln!("{} isn't asking again: {err:#} If it was approved, it answers once a node of your One App reaches it.", self.0.name);
                        asked = Some(std::time::Instant::now());
                        asked_for_good = true;
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

    /// Leaves its request to be approved at auth, with its install link's enrollment and proof.
    pub async fn request_approval(&self) -> Result<()> {
        let me = self.endpoint();
        let path = "/api/auth/devices/requests";
        let install = &self.0.install;
        let secret = unhex(&install.secret)?;
        let fields = json!({
            "route": format!("POST {path}"), "key": hex(&me.key), "seal": hex(&me.seal), "name": self.0.machine, "service": self.0.name,
            "shared": self.0.sidecars.serves_everyone(&self.0.name), "user": install.user,
            "enrollment": hex(&crate::keys::sha256(&secret)), "proof": crate::keys::service_proof(&secret, &me),
        });
        let body = crate::keys::signed_request(&self.0.key, "one/auth", fields, now_ms());
        let url = format!("{}{path}", self.0.install.address);
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
            let mut resp = agent.post(&url).send_json(&body)?;
            let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            let said = out["error"].as_str().unwrap_or("the sign-in service didn't answer");
            if resp.status() == 403 {
                bail!(LinkRefused(said.to_string()));
            }
            ensure!(resp.status().is_success(), "{said}");
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
            Some("entries") => match self.take_entries(&remote, body["pack"].as_str().unwrap_or(""), &body["depths"]) {
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
        // Whom it serves is what its listing says, not what the program offering it claims.
        let users: Vec<&String> = m.users.keys().filter(|u| m.service_serves(&self.key(), u)).collect();
        json!({ "describe": describe, "users": users })
    }

    /// Takes `global`'s member entries a node of the install brought (anyone's, while it knows of no node yet, since
    /// they must replay from the install's first entry): added to those it has, never in place of any, each signed,
    /// replayed from the first entry; those signed by no endpoint the install ever listed are passed over.
    fn take_entries(&self, remote: &Key, pack: &str, depths: &Value) -> Result<()> {
        let bytes = data_encoding::BASE64.decode(pack.as_bytes()).map_err(|_| anyhow!("the entries aren't base64"))?;
        let given = crate::entry::unpack(&bytes)?;
        let root = Hash::from_hex(&self.0.install.root)?;
        let mut known = self.0.known.lock().unwrap();
        ensure!(known.1.computes().next().is_none() || known.1.compute(remote).is_some(), "only your One App's devices bring its entries");
        let mut depths_known = self.0.depths.lock().unwrap();
        let mut grown = depths_known.clone();
        for (hash, depth) in depths.as_object().into_iter().flatten() {
            if let (Ok(hash), Some(depth)) = (Hash::from_hex(hash), depth.as_u64()) {
                grown.insert(hash, depth);
            }
        }
        let more_depths = grown.len() > depths_known.len();
        let (entries, members) = match merged(root, &known.0, given, &grown) {
            Some(merged) => merged,
            // Only more depths: what was known may order better with them.
            None if more_depths => placed(root, known.0.clone(), &grown),
            None => return Ok(()),
        };
        ensure!(!members.users.is_empty(), "those aren't your One App's entries");
        write_private(&self.0.dir.join("entries"), &crate::entry::pack(&entries.iter().collect::<Vec<_>>()))?;
        let text: HashMap<String, u64> = grown.iter().map(|(h, d)| (h.to_hex(), *d)).collect();
        write_private(&self.0.dir.join("depths"), &serde_json::to_vec(&text)?)?;
        *known = (entries, members);
        *depths_known = grown;
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

    /// Whether `token` is an access token the install's sign-in service signed for `user`, unexpired.
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
            m.computes().filter(|c| c.user == owner && m.runs_for(&c.key, &crate::entry::Scope::User(owner.clone()))).map(|c| c.key).collect()
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

/// Whether a node, `remote`, may ask this service (`me`) for a call by `user` from their device `key`: this service
/// serves them (its own person, or everyone, as its listing says), and the node is one of their own, calling for its
/// own person; or, for a service that serves everyone, a node that runs their calls bringing their sign-in, which this
/// service checked (`signed_in`). A node of someone else's, an admin's too, never calls as them otherwise.
pub fn may_call(m: &Members, me: &Key, remote: &Key, key: &Key, user: &str, signed_in: bool) -> Result<()> {
    let service = m.service(me).context("this service isn't one of your One App's yet")?;
    let node = m.compute(remote).context("only your One App's devices are answered")?;
    ensure!(m.service_serves(me, user), "{} serves only its own person", service.service.as_deref().unwrap_or("this service"));
    let own = node.user == user && (*key == *remote || m.client(key).is_some_and(|c| c.user == user));
    ensure!(own || (service.shared && signed_in && m.serves(remote, user)), "{} may not call as that person", short(remote));
    Ok(())
}

/// What a service may ask its person's node to run: only its own app (`name`), on no device named, in its person's
/// own scope, calling no other app; until grants land, that app's database there is all it reaches. → the call.
pub fn limited(name: &str, call: &Value) -> Result<Value> {
    ensure!(call["app"] == name && (call["sidecar"].is_null() || call["sidecar"] == name), "a service asks only for its own app, {name}");
    ensure!(call["device"].is_null(), "a service can't name a device to run its calls");
    let scopes_ok = match &call["scopes"] {
        Value::Null => true,
        Value::Array(s) => s.iter().all(|s| s == "user"),
        _ => false,
    };
    ensure!(scopes_ok, "a service's calls reach only its person's own scope");
    Ok(json!({ "app": name, "sidecar": call["sidecar"], "op": call["op"], "input": call["input"], "scopes": ["user"] }))
}

/// `known` with `given`'s member entries added, if any are new: → them, replayed from the install's first entry, with
/// those signed by no endpoint it ever listed passed over (junk anyone could sign counts for nothing, and isn't kept).
/// `depths` are those of the entries they follow that nodes don't bring.
fn merged(root: Hash, known: &[Entry], given: Vec<Entry>, depths: &HashMap<Hash, u64>) -> Option<(Vec<Entry>, Members)> {
    let mut all: HashMap<String, Entry> = known.iter().map(|e| (e.hash().to_hex(), e.clone())).collect();
    let before = all.len();
    for e in given.into_iter().filter(members::is_member_entry) {
        all.entry(e.hash().to_hex()).or_insert(e);
    }
    if all.len() == before {
        return None;
    }
    Some(placed(root, all.into_values().collect(), depths))
}

/// `entries` in the nodes' replay order, those that can be placed in it and are signed by an endpoint the install
/// listed, and what they add up to.
fn placed(root: Hash, entries: Vec<Entry>, depths: &HashMap<Hash, u64>) -> (Vec<Entry>, Members) {
    let ordered = crate::entry::causal(entries, depths);
    let member_entries: Vec<Entry> = ordered.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
    let members = Members::replay(Some(root), &member_entries, &Default::default());
    (ordered.into_iter().filter(|e| members.machines.contains_key(&e.author())).collect(), members)
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
    match s.ask_node(body).await {
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

    /// Andre's install: his Mac (an admin's, so it runs global and serves Sam there), Sam's box (his own), Andre's
    /// WhatsApp on linux, Andre's push sender shared with everyone, Sam's notes; and a service an admin's device
    /// lists as Sam's, which counts for nothing. → its entries, members and keys.
    fn install() -> (Vec<Entry>, Members, [SigningKey; 7]) {
        let key = || SigningKey::from_bytes(&rand::random());
        let keys = [key(), key(), key(), key(), key(), key(), key()];
        let [mac, sams, whatsapp, push, notes, taken, _stranger] = &keys;
        let mut prev = None;
        let genesis = entry(mac, ANDRE, &mut prev, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(mac), "mac", &[Kind::Client, Kind::Compute], &id(&key()), &"e".repeat(64)));
        let mut entries = vec![
            genesis.clone(),
            entry(mac, ANDRE, &mut prev, members::device(&Endpoint::of(sams), SAM, "sam's box", &[Kind::Client, Kind::Compute], Some(("Sam", Role::Member)))),
            entry(mac, ANDRE, &mut prev, members::service(&Endpoint::of(whatsapp), ANDRE, "linux", "whatsapp", false)),
            entry(mac, ANDRE, &mut prev, members::service(&Endpoint::of(push), ANDRE, "linux", "push", true)),
            entry(mac, ANDRE, &mut prev, members::service(&Endpoint::of(taken), SAM, "sam's mac", "imessage", false)),
        ];
        // Sam's box runs his own scope once his own device lists it, not on an admin's say; and lists his notes.
        let mut sams_prev = None;
        let ext = |prev: &Hash| Ext { log: "global".into(), time: 1, user: SAM.into(), epoch: 0, previous: vec![*prev] };
        let last = entries.last().unwrap().hash();
        let e = Entry::create(sams, ext(&last), sams_prev, members::device(&Endpoint::of(sams), SAM, "sam's box", &[Kind::Client, Kind::Compute], None));
        sams_prev = Some(e.head());
        entries.push(e);
        let last = entries.last().unwrap().hash();
        entries.push(Entry::create(sams, ext(&last), sams_prev, members::service(&Endpoint::of(notes), SAM, "sam's box", "notes", false)));
        let m = placed(genesis.hash(), entries.clone(), &HashMap::new()).1;
        (entries, m, keys)
    }

    const ANDRE: &str = "a000000000000000";
    const SAM: &str = "b000000000000000";

    fn id(k: &SigningKey) -> Key {
        *k.verifying_key().as_bytes()
    }

    #[test]
    fn a_service_answers_only_its_persons_own_nodes_or_their_checked_sign_in() {
        let (entries, m, [mac, sams, whatsapp, push, notes, taken, stranger]) = install();
        let may = |service: &SigningKey, node: &SigningKey, user: &str, signed_in: bool| may_call(&m, &id(service), &id(node), &id(node), user, signed_in);
        may(&whatsapp, &mac, ANDRE, false).unwrap();
        let other = may(&whatsapp, &sams, SAM, false).unwrap_err();
        assert!(format!("{other:#}").contains("serves only its own person"), "{other:#}");
        assert!(may(&whatsapp, &sams, ANDRE, false).is_err(), "Sam's node can't call as Andre");
        assert!(may(&whatsapp, &stranger, ANDRE, false).is_err(), "a stranger is refused");
        // A call another node (the serverless node, say) passed on to Andre's own is that node's, never his own node's.
        assert!(may_call(&m, &id(&whatsapp), &id(&mac), &id(&stranger), ANDRE, false).is_err(), "a call passed on isn't its person's own node's");
        // An admin's node, which runs Sam's calls in global, never calls Sam's own service as him, even with a sign-in.
        may(&notes, &sams, SAM, false).unwrap();
        assert!(m.serves(&id(&mac), SAM));
        assert!(may(&notes, &mac, SAM, false).is_err(), "an admin's node can't call a member's service as them");
        assert!(may(&notes, &mac, SAM, true).is_err(), "a service that's only Sam's answers only Sam's own nodes");
        // A shared one answers each person's own node, and a node that runs their calls only with their sign-in.
        may(&push, &sams, SAM, false).unwrap();
        assert!(may(&push, &mac, SAM, false).is_err(), "Andre's node can't call as Sam unasked");
        may(&push, &mac, SAM, true).unwrap();
        // Whom a service serves is its listing's say.
        assert!(m.service_serves(&id(&push), SAM) && !m.service_serves(&id(&whatsapp), SAM));
        // An admin's device can't list a service as someone else's.
        assert!(m.service(&id(&taken)).is_none(), "only Sam's own device lists his services");
        // Entries that aren't the install's say nothing.
        assert!(placed(Hash::from_bytes(rand::random()), entries.clone(), &HashMap::new()).1.users.is_empty());
    }

    #[test]
    fn a_service_asks_only_for_its_own_app_in_its_persons_scope() {
        let ok = limited("whatsapp", &json!({ "app": "whatsapp", "op": "write", "input": { "a": 1 } })).unwrap();
        assert_eq!(ok, json!({ "app": "whatsapp", "sidecar": null, "op": "write", "input": { "a": 1 }, "scopes": ["user"] }));
        limited("whatsapp", &json!({ "app": "whatsapp", "sidecar": "whatsapp", "op": "status", "scopes": ["user"] })).unwrap();
        for call in [
            json!({ "app": "board", "op": "post" }),
            json!({ "app": "whatsapp", "sidecar": "shell", "op": "run" }),
            json!({ "app": "shell", "sidecar": "shell", "op": "run" }),
            json!({ "app": "whatsapp", "op": "status", "device": "mac" }),
            json!({ "app": "whatsapp", "op": "read", "scopes": ["global"] }),
            json!({ "app": "whatsapp", "op": "read", "scopes": "user" }),
        ] {
            assert!(limited("whatsapp", &call).is_err(), "{call}");
        }
    }

    #[test]
    fn entries_brought_add_to_those_known_and_never_replace_them() {
        let (entries, m, [mac, ..]) = install();
        let root = entries[0].hash();
        let (known, _) = merged(root, &[], entries.clone(), &HashMap::new()).unwrap();
        assert_eq!(known.len(), entries.len());
        // A pack with fewer, or the same, adds nothing and drops nothing.
        assert!(merged(root, &known, entries[..2].to_vec(), &HashMap::new()).is_none());
        // A longer pack of junk signed by keys the install never listed is passed over, and what's known stays.
        let junk: Vec<Entry> = (0..20)
            .map(|_| {
                let k = SigningKey::from_bytes(&rand::random());
                entry(&k, ANDRE, &mut None, members::device(&Endpoint::of(&k), ANDRE, "junk", &[Kind::Client], None))
            })
            .collect();
        let (after, members) = merged(root, &known, junk, &HashMap::new()).unwrap();
        assert_eq!(after.len(), known.len(), "junk isn't kept");
        assert_eq!(members.machines.len(), m.machines.len());
        assert!(members.compute(&id(&mac)).is_some());
    }
}

