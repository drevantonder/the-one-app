//! A node: this server's install (a brand holds one), its replica and views, its iroh endpoint, and where calls run.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use p2panda_core::{Hash, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

use crate::apps::{Append, AppEntry, Apps, Context, Me, Replay, ScopeHandle};
use crate::entry::{Content, Entry, Key, Order, Scope, hex, new_id, short, unhex};
use crate::members::{self, Members, Principal, Role, ScopeState};
use crate::release::{self, Release};
use crate::store::{Frontier, Store};
use crate::sync::{self, Driver, Handled, Msg, Replica};

const JOIN_ALPN: &[u8] = b"one/join/1";
const CALL_ALPN: &[u8] = b"one/call/1";
pub const CLIENT_ALPN: &[u8] = b"one/client/1";
const BLOB_ALPN: &[u8] = b"one/blob/1";
/// How long a call may run before it's stopped.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// How long a waking server waits for its first round of dialing before it may call itself ready.
const FIRST_DIAL: Duration = Duration::from_secs(5);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Serialize, Deserialize)]
struct ServerFile {
    secret: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct InstallFile {
    /// The hash of the install's genesis.
    root: String,
    /// This server's local user.
    user: String,
    /// The server whose pairing code this one joined with, to sync with until it sees itself admitted.
    #[serde(default)]
    paired_by: Option<String>,
}

struct Session {
    tx: mpsc::UnboundedSender<Msg>,
    /// The frontier the peer last said it had.
    frontier: Option<Frontier>,
    drivers: Vec<Driver>,
    /// Whether this server has received everything the peer's first hello offered it.
    caught_up: bool,
}

struct Install {
    r: Replica,
    user: String,
    sessions: HashMap<Key, Session>,
    /// When each client last asked this server something.
    asked: HashMap<Key, u64>,
    /// The build each server last said it runs.
    builds: HashMap<Key, release::Running>,
    /// Each scope's views: how far they've replayed, and what the scope's node entries add up to.
    views: HashMap<String, (Order, ScopeState)>,
    /// How long each server took to answer a call lately.
    nearest: HashMap<Key, Duration>,
    /// Whether the first round of dialing since this server woke is done.
    dialed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    Changed { apps: Vec<String> },
    Servers,
    /// This server created or joined an install.
    Install,
}

#[derive(Serialize, Deserialize)]
struct Pairing {
    /// The user who asked for it.
    user: String,
    /// The client that asked for it, if one did.
    #[serde(default)]
    by: Option<String>,
    /// For a new user: their name and role.
    #[serde(default)]
    new_user: Option<(String, Role)>,
    /// Milliseconds since 1970 after which it no longer works.
    until: u64,
}

const PAIRING_LIFE: u64 = 7 * 86_400_000;

#[derive(Serialize, Deserialize)]
struct PairingCode {
    from: EndpointAddr,
    secret: [u8; 16],
}

#[derive(Serialize, Deserialize)]
struct JoinRequest {
    secret: [u8; 16],
    name: String,
    #[serde(rename = "as")]
    as_: String,
}

#[derive(Serialize, Deserialize)]
struct JoinReply {
    root: String,
    user: String,
}

/// Who asked for something: this server or another, or a client, as one user.
#[derive(Clone, Debug)]
pub struct Caller {
    pub key: Key,
    pub user: String,
}

/// An operation to run: of an app, or of a driver.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallRequest {
    pub app: String,
    pub op: String,
    #[serde(default)]
    pub input: Value,
    /// The scopes the call touches, named before it runs; by default every scope it may reach.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// The server to run it on, by name or key prefix; by default the nearest that can.
    #[serde(default)]
    pub server: Option<String>,
    /// A user whose data was opened for the caller, to run as, reading only.
    #[serde(default, rename = "as")]
    pub as_user: Option<String>,
}

/// What a server says to a call.
enum Reply {
    Ok(Value),
    Error(String),
    /// It can't run here; try another server.
    Elsewhere(String),
}

impl Reply {
    fn json(self) -> Value {
        match self {
            Reply::Ok(v) => json!({ "ok": v }),
            Reply::Error(e) => json!({ "error": e }),
            Reply::Elsewhere(w) => json!({ "elsewhere": w }),
        }
    }

    fn from_json(v: Value) -> Reply {
        if let Some(w) = v.get("elsewhere") {
            return Reply::Elsewhere(w.as_str().unwrap_or("").into());
        }
        match v.get("error") {
            Some(e) => Reply::Error(e.as_str().unwrap_or("failed").into()),
            None => Reply::Ok(v["ok"].clone()),
        }
    }
}

pub struct Inner {
    dir: PathBuf,
    key: SigningKey,
    id: Key,
    name: Mutex<String>,
    endpoint: Endpoint,
    apps: Apps,
    native: Vec<crate::native::Native>,
    pub providers: crate::providers::Providers,
    install: Mutex<Option<Install>>,
    pairings: Mutex<HashMap<[u8; 16], Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this server.
    changed: tokio::sync::Notify,
    /// The bundle each app runs here.
    bundles: Mutex<BTreeMap<String, String>>,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The port apps' requests go out through.
    outbound: u16,
    woke: Instant,
}

/// What a running call may do through the node: whose it is, which app's, and which scopes it reaches.
#[derive(Clone, Debug)]
pub struct Grant {
    pub app: String,
    pub caller: Caller,
    pub scopes: Vec<Scope>,
    pub read_only: bool,
    /// The blobs it kept, which it may read back before any entry names them.
    pub puts: Vec<String>,
}

/// Holds a grant for as long as its call runs.
struct Granted(Node, String);

impl Drop for Granted {
    fn drop(&mut self) {
        self.0.0.grants.lock().unwrap().remove(&self.1);
    }
}

#[derive(Clone)]
pub struct Node(Arc<Inner>);

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({})", short(&self.0.id))
    }
}

impl Node {
    /// Starts this server's node, with the native apps this build has.
    pub async fn start(dir: &Path, native: Vec<crate::native::Native>) -> Result<(Node, Router)> {
        std::fs::create_dir_all(dir)?;
        let server = load_server(dir)?;
        let secret = unhex(&server.secret)?;
        let key = SigningKey::from(secret);
        let id = *key.verifying_key().as_bytes();
        let endpoint = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&secret)).bind().await?;
        let outbound = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let apps = Apps::start(dir.join("runtime"), outbound.local_addr()?.port()).await?;
        let (events, _) = broadcast::channel(256);
        let node = Node(Arc::new(Inner {
            dir: dir.to_path_buf(),
            key,
            id,
            name: Mutex::new(server.name),
            endpoint: endpoint.clone(),
            apps,
            native,
            providers: Default::default(),
            install: Mutex::new(None),
            pairings: Mutex::new(load_pairings(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            bundles: Mutex::new(BTreeMap::new()),
            grants: Mutex::new(HashMap::new()),
            outbound: outbound.local_addr()?.port(),
            woke: Instant::now(),
        }));
        let outbound_router = crate::outbound::router(node.clone());
        tokio::spawn(async move {
            if let Err(err) = axum::serve(outbound, outbound_router).await {
                tracing::warn!("apps' outbound requests stopped: {err:#}");
            }
        });
        if dir.join("install").join("install.json").exists() {
            node.open_install().context("open the install")?;
            node.replay_all().await?;
        }
        let router = Router::builder(endpoint)
            .accept(sync::ALPN, SyncHandler(node.clone()))
            .accept(JOIN_ALPN, JoinHandler(node.clone()))
            .accept(CALL_ALPN, CallHandler(node.clone()))
            .accept(CLIENT_ALPN, ClientHandler(node.clone()))
            .accept(BLOB_ALPN, BlobHandler(node.clone()))
            .spawn();
        tokio::spawn(node.clone().dial_loop());
        let worker = node.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = worker.settle().await {
                    tracing::warn!("{err:#}");
                }
                worker.0.changed.notified().await;
            }
        });
        if node.has_install() {
            node.consider_update();
            for native in node.0.native.iter().filter(|n| n.sync.is_some()) {
                node.sync_native(native.name);
            }
        }
        Ok((node, router))
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    pub fn providers(&self) -> &crate::providers::Providers {
        &self.0.providers
    }

    /// Registers a provider; the node advertises it to the servers it syncs with.
    pub async fn provide(&self, app: &str, describe: Value) -> Result<()> {
        self.0.providers.register(app, describe);
        self.say_hello();
        Ok(())
    }

    /// Runs `f` on this server's install.
    fn with<R>(&self, f: impl FnOnce(&mut Install) -> Result<R>) -> Result<R> {
        let mut install = self.0.install.lock().unwrap();
        f(install.as_mut().context("this server is in no install yet: create one, or join with a pairing code")?)
    }

    pub fn has_install(&self) -> bool {
        self.0.install.lock().unwrap().is_some()
    }

    /// This server's local user.
    pub fn user(&self) -> Result<String> {
        self.with(|i| Ok(i.user.clone()))
    }

    /// Refuses a second install: a brand holds one, and another context is another brand.
    fn ensure_no_install(&self) -> Result<()> {
        if self.has_install() {
            bail!("this server already belongs to an install, and {} holds one; for another, install that context's own app", crate::brand::NAME);
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.0.events.send(event);
    }

    fn install_dir(&self) -> PathBuf {
        self.0.dir.join("install")
    }

    fn open_install(&self) -> Result<()> {
        let dir = self.install_dir();
        let file: InstallFile = serde_json::from_slice(&std::fs::read(dir.join("install.json"))?)?;
        let store = Store::open(&dir.join("entries.db"))?;
        let root: [u8; 32] = data_encoding::HEXLOWER.decode(file.root.as_bytes())?.try_into().map_err(|_| anyhow!("bad root"))?;
        let paired_by = file.paired_by.and_then(|h| unhex(&h).ok());
        let r = Replica::new(self.0.key.clone(), store, Some(Hash::from_bytes(root)), paired_by)?;
        let install = Install { r, user: file.user, sessions: HashMap::new(), asked: HashMap::new(), builds: HashMap::new(), views: HashMap::new(), nearest: HashMap::new(), dialed: false };
        *self.0.install.lock().unwrap() = Some(install);
        Ok(())
    }

    fn save_install(&self, file: &InstallFile) -> Result<()> {
        let dir = self.install_dir();
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join("install.json"), &serde_json::to_vec_pretty(file)?)?;
        Ok(())
    }

    pub fn server_name(&self) -> String {
        self.0.name.lock().unwrap().clone()
    }

    /// Creates this server's install, with its local user as the first admin, and deploys the brand's apps in
    /// `global`. Returns the recovery key's secret, given only this once.
    pub async fn create(&self, name: &str, user_name: &str) -> Result<String> {
        self.ensure_no_install()?;
        ensure!(!name.trim().is_empty() && !user_name.trim().is_empty(), "give the install a name, and yourself one");
        let user = new_id();
        let recovery = SigningKey::generate();
        let genesis = members::genesis(name.trim(), &user, user_name.trim(), &self.server_name(), recovery.verifying_key().as_bytes());
        let time = now_ms();
        let entry = Entry::create(&self.0.key, crate::entry::Ext { log: "global".into(), time, user: user.clone(), by: None }, None, genesis);
        self.save_install(&InstallFile { root: entry.hash().to_hex(), user: user.clone(), paired_by: None })?;
        Store::open(&self.install_dir().join("entries.db"))?.insert_own(&entry, None)?;
        self.open_install()?;
        self.replay_all().await?;
        for (app, files) in crate::brand::APPS {
            let mut bundle = vec![("app.js".to_string(), files.iter().find(|(p, _)| *p == "app.js").context("a brand app has no app.js")?.1.to_vec())];
            bundle.extend(files.iter().map(|(p, b)| (format!("src/{p}"), b.to_vec())));
            let blob = self.put_blob(&crate::bundle::pack(&bundle)?)?;
            self.append(&Scope::Global, members::node("app", json!({ "name": app, "owner": "admins" }))).await?;
            self.append(&Scope::Global, deploy(app, &blob)).await?;
            self.append(&Scope::Global, members::node("install", json!({ "app": app }))).await?;
        }
        self.settle().await?;
        self.emit(Event::Install);
        Ok(hex(recovery.as_bytes()))
    }

    /// Writes an entry as this server's local user.
    pub async fn append(&self, scope: &Scope, content: Content) -> Result<Entry> {
        let user = self.user()?;
        self.append_as(scope, &user, None, content).await
    }

    /// Writes an entry to this server's log of `scope`, as `user`, for `by` if it ran for someone else.
    async fn append_as(&self, scope: &Scope, user: &str, by: Option<Key>, content: Content) -> Result<Entry> {
        let _write = self.0.writes.lock().await;
        let (entry, handled) = self.with(|i| i.r.append(scope, user, by, content, now_ms()))?;
        self.after(handled, None).await?;
        Ok(entry)
    }

    /// Passes on new entries, follows what changed in the members, and replays the views.
    async fn after(&self, handled: Handled, from: Option<Key>) -> Result<()> {
        if handled.new.is_empty() && handled.dropped.is_empty() && !handled.members {
            return Ok(());
        }
        let jobs = self.with(|i| {
            for (peer, session) in &i.sessions {
                if Some(*peer) != from {
                    if let Some(msg) = i.r.for_peer(peer, &handled.new) {
                        let _ = session.tx.send(msg);
                    }
                }
                if handled.gained {
                    let _ = session.tx.send(i.r.hello(Vec::new())?);
                }
                if let Some((_, logs)) = handled.peers_gained.iter().find(|(k, _)| k == peer) {
                    let frontier = session.frontier.clone().unwrap_or_default();
                    for msg in i.r.missing_in(peer, &frontier, |l| logs.iter().any(|g| g == l))? {
                        let _ = session.tx.send(msg);
                    }
                }
            }
            if handled.members {
                let r = &i.r;
                i.sessions.retain(|peer, _| r.may_sync(peer));
            }
            for log in &handled.dropped {
                i.views.remove(log);
            }
            i.builds.extend(release::builds(&handled.new));
            replay_jobs(i, &handled.new, &self.0.apps.names())
        })?;
        for log in &handled.dropped {
            self.0.apps.drop_scope(log).await;
        }
        if handled.new.iter().any(|e| e.content.app == release::APP && e.content.kind == "node-release") {
            self.consider_update();
        }
        if handled.members {
            self.0.changed.notify_one();
            self.emit(Event::Servers);
        }
        for native in &self.0.native {
            if native.sync.is_some() && handled.new.iter().any(|e| e.content.app == native.name) {
                self.sync_native(native.name);
            }
        }
        let mut apps: Vec<String> = jobs.iter().map(|j| j.app.clone()).collect();
        apps.extend(handled.new.iter().filter(|e| e.content.app != members::APP).map(|e| e.content.app.clone()));
        apps.sort();
        apps.dedup();
        self.0.apps.replay(jobs).await;
        self.emit(Event::Changed { apps });
        Ok(())
    }

    /// Replays every scope's views from the start (when the node starts).
    async fn replay_all(&self) -> Result<()> {
        let _write = self.0.writes.lock().await;
        let jobs = self.with(|i| {
            i.views.clear();
            i.builds = release::builds(&i.r.store.scope("global")?);
            let all = i.r.store.ordered()?;
            replay_jobs(i, &all, &self.0.apps.names())
        })?;
        self.0.apps.replay(jobs).await;
        Ok(())
    }

    /// Brings this server in line with the members: runs each app's latest deploy, fetching bundles it lacks, and
    /// says which build it runs.
    async fn settle(&self) -> Result<()> {
        let Ok((wanted, author)) = self.with(|i| {
            let wanted: BTreeMap<String, String> = i.r.members.apps.values().filter_map(|a| Some((a.name.clone(), a.bundle.clone()?))).collect();
            Ok((wanted, i.r.members.server(&self.0.id).is_some()))
        }) else {
            return Ok(());
        };
        if !author {
            return Ok(());
        }
        let mut modules = Vec::new();
        for (app, bundle) in &wanted {
            match self.blob(bundle).await.and_then(|b| crate::bundle::module(&b)) {
                Ok(module) => modules.push((app.clone(), module)),
                Err(err) => tracing::warn!("{app}'s deploy: {err:#}"),
            }
        }
        let changed: Vec<String> = {
            let bundles = self.0.bundles.lock().unwrap();
            modules.iter().map(|(a, _)| a.clone()).filter(|a| bundles.get(a) != wanted.get(a)).collect()
        };
        if !changed.is_empty() || modules.len() != self.0.apps.names().len() {
            let _write = self.0.writes.lock().await;
            self.0.apps.load(modules.clone()).await?;
            *self.0.bundles.lock().unwrap() = modules.iter().map(|(a, _)| (a.clone(), wanted[a].clone())).collect();
            let jobs = self.with(|i| {
                let all = i.r.store.ordered()?;
                i.views.clear();
                replay_jobs(i, &all, &changed)
            })?;
            self.0.apps.replay(jobs).await;
            self.emit(Event::Changed { apps: changed });
        }
        let running = self.with(|i| Ok(i.builds.get(&self.0.id).map(|r| r.commit.clone())))?;
        if running.as_deref() != Some(crate::brand::BUILD) {
            self.append(&Scope::Global, release::running()).await?;
        }
        Ok(())
    }

    /// Whether this server is ready to run calls: it has replayed what it holds and, since it woke, received
    /// everything each connected server's hello offered it.
    pub fn ready(&self) -> bool {
        self.with(|i| {
            let alone = i.r.members.servers().all(|s| s.key == self.0.id);
            let dialed = i.dialed || alone || self.0.woke.elapsed() > FIRST_DIAL;
            Ok(i.r.members.server(&self.0.id).is_some() && dialed && i.sessions.values().all(|s| s.caught_up))
        })
        .unwrap_or(false)
    }

    // Calls.

    /// Runs an operation for this server's local user: here, or on the nearest server that can.
    pub async fn call(&self, req: CallRequest) -> Result<Value> {
        let me = Caller { key: self.0.id, user: self.user()? };
        self.route(req, me).await
    }

    /// Tries the servers that might run a call, nearest first, one at a time, the next only if one can't be reached
    /// or says to go elsewhere.
    async fn route(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let mut why = Vec::new();
        for server in self.candidates(&req, &me)? {
            let started = Instant::now();
            let reply = match server == self.0.id {
                true => Ok(self.run(&req, &me).await),
                false => self.ask_server(server, &req, &me).await,
            };
            let name = self.name_of(&server);
            match reply {
                Ok(Reply::Ok(v)) => {
                    let _ = self.with(|i| Ok(i.nearest.insert(server, started.elapsed())));
                    return Ok(v);
                }
                Ok(Reply::Error(e)) => bail!("{e}"),
                Ok(Reply::Elsewhere(w)) => why.push(format!("{name}: {w}")),
                Err(err) => {
                    let _ = self.with(|i| Ok(i.nearest.insert(server, Duration::MAX)));
                    why.push(format!("{name}: {err:#}"));
                }
            }
        }
        if why.is_empty() {
            bail!("no server runs {} for you", req.app);
        }
        bail!("no server could run it ({})", why.join("; "))
    }

    /// The servers that might run a call, nearest first.
    fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        if let Some(server) = &req.server {
            return Ok(vec![self.find_server(server)?]);
        }
        let here = self.runs_driver_here(&req.app);
        self.with(|i| {
            let m = &i.r.members;
            let mut servers: Vec<Key> = if m.apps.get(&req.app).is_some_and(|a| a.bundle.is_some()) {
                m.servers().map(|s| s.key).filter(|s| m.serves(s, &me.user)).collect()
            } else {
                let mut s: Vec<Key> = i.sessions.iter().filter(|(_, s)| s.drivers.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect();
                if here && me.user == i.user {
                    s.push(self.0.id);
                }
                s
            };
            let rank = |k: &Key| (*k != self.0.id, !i.sessions.contains_key(k), i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)));
            servers.sort_by_key(rank);
            Ok(servers)
        })
    }

    fn runs_driver_here(&self, app: &str) -> bool {
        self.0.native.iter().any(|n| n.name == app) || self.0.providers.has(app)
    }

    /// Asks another server to run a call.
    async fn ask_server(&self, server: Key, req: &CallRequest, me: &Caller) -> Result<Reply> {
        let conn = tokio::time::timeout(Duration::from_secs(10), self.0.endpoint.connect(EndpointId::from_bytes(&server)?, CALL_ALPN)).await.context("timed out")??;
        let (mut send, mut recv) = conn.open_bi().await?;
        let request = json!({ "app": req.app, "op": req.op, "input": req.input, "scopes": req.scopes, "as": req.as_user, "me": { "key": hex(&me.key), "user": me.user } });
        send.write_all(&serde_json::to_vec(&request)?).await?;
        send.finish()?;
        let reply = tokio::time::timeout(CALL_LIMIT + Duration::from_secs(5), recv.read_to_end(16 << 20)).await.context("timed out")??;
        Ok(Reply::from_json(serde_json::from_slice(&reply)?))
    }

    /// Runs a call here, for `me`, or says why it can't.
    async fn run(&self, req: &CallRequest, me: &Caller) -> Reply {
        let result = tokio::time::timeout(CALL_LIMIT, self.run_here(req, me)).await;
        match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(err)) => Reply::Error(format!("{err:#}")),
            Err(_) => Reply::Error(format!("it took longer than {}s and was stopped", CALL_LIMIT.as_secs())),
        }
    }

    async fn run_here(&self, req: &CallRequest, me: &Caller) -> Result<Reply> {
        let input = if req.input.is_null() { json!({}) } else { req.input.clone() };
        if req.app == members::APP {
            return self.node_op(&req.op, &input, me).await.map(Reply::Ok);
        }
        let deployed = self.with(|i| Ok(i.r.members.apps.get(&req.app).is_some_and(|a| a.bundle.is_some())))?;
        if !deployed {
            if !self.runs_driver_here(&req.app) {
                return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
            }
            return self.run_driver(&req.app, &req.op, input, me).await.map(Reply::Ok);
        }
        let prepared = self.with(|i| prepare(self, i, req, me));
        let (ctx, scopes) = match prepared? {
            Ok(p) => p,
            Err(why) => return Ok(Reply::Elsewhere(why)),
        };
        if !self.ready() {
            return Ok(Reply::Elsewhere("it isn't ready yet".into()));
        }
        let deployed_here = self.0.bundles.lock().unwrap().get(&req.app).cloned();
        let latest = self.with(|i| Ok(i.r.members.apps.get(&req.app).and_then(|a| a.bundle.clone())))?;
        if deployed_here.is_none() || deployed_here != latest {
            return Ok(Reply::Elsewhere(format!("it doesn't have {}'s latest deploy yet", req.app)));
        }
        let mut ctx = ctx;
        let _granted = self.grant(&req.app, me, &mut ctx);
        let outcome = self.0.apps.call(&req.app, &req.op, input, &ctx).await?;
        // Every append is checked before any is written: a call that fails writes nothing.
        let mut writes = Vec::new();
        for append in outcome.appends {
            match append {
                Append::Install { app, scope, installed } => {
                    let scope = if scope == "user" { Scope::User(me.user.clone()) } else { Scope::parse(&scope)? };
                    writes.extend(self.install_entries(me, &app, &scope, installed).await?);
                }
                append => writes.push(self.with(|i| check_append(i, &req.app, append, &scopes, me))?),
            }
        }
        let by = (me.key != self.0.id).then_some(me.key);
        for (scope, content) in writes {
            self.append_as(&scope, &me.user, by, content).await?;
        }
        Ok(Reply::Ok(outcome.result))
    }

    /// Runs a native app or provider here, for this server's local user.
    async fn run_driver(&self, app: &str, op: &str, input: Value, me: &Caller) -> Result<Value> {
        let user = self.user()?;
        ensure!(me.user == user, "{app} runs only for {}'s own user", self.server_name());
        let caller = self.me_for(me)?;
        let native = self.0.native.iter().find(|n| n.name == app);
        let (result, appends) = match native {
            Some(n) => (n.call)(op.to_string(), input, caller, if n.entries { self.app_entries(app)? } else { vec![] }).await?,
            None => self.0.providers.call(app, op, input, caller).await?,
        };
        let by = (me.key != self.0.id).then_some(me.key);
        for content in appends {
            self.append_as(&Scope::User(user.clone()), &user, by, content).await?;
        }
        Ok(result)
    }

    /// The node's own operations, which other servers ask for: `open`.
    async fn node_op(&self, op: &str, input: &Value, me: &Caller) -> Result<Value> {
        match op {
            "open" => {
                let user = input["user"].as_str().context("whose data?")?;
                let scope = Scope::User(user.into());
                self.with(|i| {
                    let m = &i.r.members;
                    ensure!(m.holds(&self.0.id, &scope), "{} doesn't hold that user's data", self.server_name());
                    ensure!(m.owners(&self.0.id).iter().any(|o| m.acts_for(&me.user, o)), "only someone acting for an owner of {} opens data on it", self.server_name());
                    Ok(())
                })?;
                let by = (me.key != self.0.id).then_some(me.key);
                self.append_as(&scope, &me.user, by, members::node("open", json!({ "server": hex(&self.0.id) }))).await?;
                Ok(json!(format!("Opened for an hour on {}.", self.server_name())))
            }
            other => bail!("the node has no operation {other}"),
        }
    }

    /// Lets the requests of a call, widget or hook of `app` through the node as `caller`'s, while it runs.
    fn grant(&self, app: &str, caller: &Caller, ctx: &mut Context) -> Granted {
        let token = hex(&rand::random::<[u8; 32]>());
        let scopes = ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect();
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new() };
        self.0.grants.lock().unwrap().insert(token.clone(), grant);
        ctx.token = token.clone();
        Granted(self.clone(), token)
    }

    /// The ports this node serves on this machine: its local API, workerd, and apps' outbound requests.
    pub fn own_ports(&self) -> Vec<u16> {
        let api = crate::api::addr().rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(crate::brand::PORT);
        vec![api, self.0.apps.port(), self.0.outbound]
    }

    /// The grant of a call running now.
    pub fn grant_of(&self, token: &str) -> Option<Grant> {
        self.0.grants.lock().unwrap().get(token).cloned()
    }

    /// The secret `name` a call's request names, as (hosts, value): its app's in the caller's own scope, else in
    /// `global`; or with `<group id>/name`, in that group's.
    pub fn secret_for(&self, grant: &Grant, name: &str) -> Option<(Vec<String>, String)> {
        self.with(|i| {
            let look = |scope: &Scope, name: &str| {
                let state = &i.views.get(&scope.to_string())?.1;
                state.secrets.get(&(grant.app.clone(), name.to_string())).map(|s| (s.hosts.clone(), s.value.clone()))
            };
            Ok(match name.split_once('/') {
                Some((group, name)) => {
                    let scope = Scope::Group(group.to_string());
                    if i.r.members.may_use(&grant.caller.user, &scope) { look(&scope, name) } else { None }
                }
                None => look(&Scope::User(grant.caller.user.clone()), name).or_else(|| look(&Scope::Global, name)),
            })
        })
        .ok()
        .flatten()
    }

    /// Sets (or with no value, deletes) a secret of `app` in `scope`, for `caller`. Nothing reads it back.
    pub async fn set_secret(&self, caller: &Caller, scope: &Scope, app: &str, name: &str, hosts: Vec<String>, value: Option<String>) -> Result<()> {
        let content = self.with(|i| secret_entry(i, &caller.user, scope, app, name, hosts, value))?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        Ok(())
    }

    /// The secrets `viewer` may set, here or in `scope`: their app, name and hosts, never their values.
    pub fn secrets(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                for ((app, name), secret) in &state.secrets {
                    out.push(json!({ "scope": log, "app": app, "name": name, "hosts": secret.hosts, "user": secret.user, "time": secret.time }));
                }
            }
            out.sort_by_key(|v| (v["scope"].to_string(), v["app"].to_string(), v["name"].to_string()));
            Ok(out)
        })
    }

    /// Who a caller is, as apps see them.
    fn me_for(&self, caller: &Caller) -> Result<Me> {
        self.with(|i| Ok(node_me(self, &i.r.members, caller)))
    }

    /// The context a widget, hook or screen of `app` gets for `viewer`: every scope it may reach that's here.
    fn read_context(&self, app: &str, viewer: &Caller) -> Result<Context> {
        let me = self.me_for(viewer)?;
        self.with(|i| {
            let reach = reachable(i, app, &viewer.user);
            let scopes: Vec<Scope> = reach.into_iter().filter(|s| i.r.holds(s)).collect();
            Ok(Context { me, now: now_ms(), scopes: handles(&i.r.members, &scopes, &viewer.user), read_only: true, token: String::new() })
        })
    }

    /// Every widget, rendered for `viewer`; with `scope` (`user`, `group:<id>` or `global`), those of the apps
    /// installed there, for a shell's page of that scope, each reaching only it.
    pub async fn widgets(&self, viewer: Caller, scope: Option<&str>) -> Result<Vec<Value>> {
        let scope = match scope {
            Some("user") => Some(Scope::User(viewer.user.clone())),
            Some(s) => Some(Scope::parse(s)?),
            None => None,
        };
        let mut out = Vec::new();
        for app in self.0.apps.names() {
            let mut ctx = self.read_context(&app, &viewer)?;
            let _granted = self.grant(&app, &viewer, &mut ctx);
            if let Some(scope) = &scope {
                let installed = self.with(|i| Ok(i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.installed.contains(&app))))?;
                ctx.scopes.retain(|s| installed && s.id == scope.to_string());
            }
            if ctx.scopes.is_empty() {
                continue;
            }
            let describe = self.0.apps.describe(&app).await?;
            for w in describe["widgets"].as_array().into_iter().flatten() {
                let widget = w["name"].as_str().unwrap_or_default();
                match self.0.apps.widget(&app, widget, &ctx).await {
                    Ok(root) => out.push(json!({ "app": app, "widget": widget, "title": w["title"], "root": root })),
                    Err(err) => tracing::warn!("{app} widget {widget}: {err:#}"),
                }
            }
        }
        Ok(out)
    }

    /// Runs an app's hook for an agent harness on this machine (SPEC.md, "Local API"). The node remembers, per
    /// session, when the hook last ran, and tells the app as `since` (null on a session's first call).
    pub async fn hook(&self, app: &str, name: &str, event: Value) -> Result<Value> {
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let key = format!("{app}/{name}/{session}");
        let since = seen.get(&key).cloned().unwrap_or(Value::Null);
        let now = now_ms();
        let caller = Caller { key: self.0.id, user: self.user()? };
        let mut ctx = self.read_context(app, &caller)?;
        let _granted = self.grant(app, &caller, &mut ctx);
        let out = self.0.apps.hook(app, name, json!({ "event": event, "since": since }), &ctx).await?;
        seen.insert(key, json!(now));
        let week_ago = now.saturating_sub(7 * 86_400_000);
        seen.retain(|_, t| t.as_u64().is_some_and(|t| t > week_ago));
        std::fs::write(&path, serde_json::to_vec(&seen)?)?;
        Ok(out)
    }

    /// An app screen as a whole page: the app's HTML with the design kit, the bridge, and who's looking.
    /// `scope` is the page a shell opened it from, a hint the app may follow or not.
    pub async fn screen(&self, viewer: Caller, app: &str, name: &str, params: Value, scope: Option<&str>) -> Result<String> {
        let body = self.0.apps.screen(app, name).await?;
        let me = self.me_for(&viewer)?;
        let context = json!({ "app": app, "screen": name, "params": params, "me": me, "scope": scope });
        // `</` can't appear inside the inline script, so it can't end it early.
        let context = context.to_string().replace("</", "<\\/");
        Ok(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
             <style>{kit}:root{{--accent:{accent}}}</style><script>window.__one={context};{bridge}</script></head><body>{body}</body></html>",
            kit = include_str!("screen/kit.css"),
            accent = crate::brand::ACCENT,
            bridge = include_str!("screen/bridge.js"),
        ))
    }

    /// Every app in the install, with what each does for `viewer` where it's installed for them; and the drivers
    /// that serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer).await?;
        if viewer.user == self.user()? {
            out.extend(self.0.native.iter().map(|n| (n.describe)()));
            out.extend(self.0.providers.describe());
        }
        let remote = self.with(|i| {
            let mut d: Vec<Value> = i.sessions.values().flat_map(|s| s.drivers.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for describe in remote {
            if !out.iter().any(|a| a["name"] == describe["name"]) {
                out.push(describe);
            }
        }
        Ok(out)
    }

    /// Every app in the install: owner, `from`, forks, deploys, the scopes `viewer` may use it's installed in, its
    /// title and icon; and for those installed for them, what it does (operations, widgets, screens, instructions).
    pub async fn catalog(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = &i.r.members;
            Ok(m.apps
                .values()
                .map(|a| {
                    let mut installed: Vec<String> = i
                        .views
                        .iter()
                        .filter(|(scope, (_, s))| s.installed.contains(&a.name) && Scope::parse(scope).is_ok_and(|s| m.may_use(&viewer.user, &s)))
                        .map(|(scope, _)| scope.clone())
                        .collect();
                    // Yours, then groups', then everyone's.
                    installed.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
                    let reach = reachable(i, &a.name, &viewer.user);
                    let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
                    let forks: Vec<&String> = m.apps.values().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
                    let info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed });
                    (a.name.clone(), info, !reach.is_empty())
                })
                .collect::<Vec<_>>())
        })?;
        let mut out = Vec::new();
        for (name, mut info, mine) in listed {
            if self.0.apps.has(&name) {
                if let Ok(Value::Object(d)) = self.0.apps.describe(&name).await {
                    // A fork's module may still give its first name; the install's name for it is the one that counts.
                    for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && (mine || matches!(k.as_str(), "title" | "icon" | "uses"))) {
                        info[k] = v;
                    }
                }
            }
            out.push(info);
        }
        Ok(out)
    }

    /// What an app's latest deploy says it uses: (apps, drivers). Waits a while for this server to run that deploy.
    async fn uses(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let started = Instant::now();
        while started.elapsed() < CALL_LIMIT {
            let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            if latest.is_none() || self.0.bundles.lock().unwrap().get(app) == latest.as_ref() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let d = if self.0.apps.has(app) { self.0.apps.describe(app).await.unwrap_or_default() } else { Value::Null };
        let list = |k: &str| d["uses"][k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        (list("apps"), list("drivers"))
    }

    /// The entries that install (or uninstall) `app` in `scope` for `caller`, with their own rights there. Installing
    /// also installs each app it uses, and each they use, that isn't installed there yet.
    async fn install_entries(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool) -> Result<Vec<(Scope, Content)>> {
        let mut wanted = vec![app.to_string()];
        let mut i = 0;
        while installed && i < wanted.len() {
            for used in self.uses(&wanted[i]).await.0 {
                if !wanted.contains(&used) {
                    wanted.push(used);
                }
            }
            i += 1;
        }
        self.with(|i| {
            let m = &i.r.members;
            ensure!(members::manages_scope(m, &caller.user, scope), "you can't install apps in {scope}");
            ensure!(i.r.holds(scope), "this server doesn't hold {scope}");
            let here = i.views.get(&scope.to_string()).map(|(_, s)| s.installed.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (n, name) in wanted.iter().enumerate().rev() {
                ensure!(m.apps.contains_key(name), "no app named {name}{}", if n > 0 { format!(", which {app} uses") } else { String::new() });
                if n == 0 || !here.contains(name) {
                    out.push((scope.clone(), members::node(if installed { "install" } else { "uninstall" }, json!({ "app": name }))));
                }
            }
            Ok(out)
        })
    }

    /// Installs (or uninstalls) `app` in `scope` for `caller`, and what it uses.
    pub async fn install(&self, caller: &Caller, app: &str, scope: &Scope, installed: bool) -> Result<()> {
        let by = (caller.key != self.0.id).then_some(caller.key);
        for (scope, content) in self.install_entries(caller, app, scope, installed).await? {
            self.append_as(&scope, &caller.user, by, content).await?;
        }
        Ok(())
    }

    /// Forks `app` as a new app called `name`, owned by `owner` (by default the caller), with data of its own; it
    /// runs `app`'s latest deploy until it's deployed itself.
    pub async fn fork(&self, caller: &Caller, app: &str, name: &str, owner: Option<&str>) -> Result<()> {
        let bundle = self.with(|i| {
            let m = &i.r.members;
            ensure!(!m.apps.contains_key(name), "there's already an app called {name}");
            m.apps.get(app).with_context(|| format!("no app named {app}"))?.bundle.clone().with_context(|| format!("{app} has no deploy to fork"))
        })?;
        let owner = owner.map(str::to_string).unwrap_or_else(|| format!("user:{}", caller.user));
        self.member_as(caller, "app", json!({ "name": name, "owner": owner, "from": app })).await?;
        self.member_content(caller, deploy(name, &bundle)).await
    }

    /// What the shim asks the node for a running call (`src/outbound.rs`): `POST /call { app, op, input }` runs an
    /// operation of an app or driver the calling app uses, as its caller; `POST /blob` keeps a blob, `GET
    /// /blob/<hash>` gets one the caller may read; `GET /catalog` lists the install's apps.
    pub async fn internal(&self, token: &str, method: &str, path: &str, body: Vec<u8>) -> Result<axum::response::Response> {
        use axum::response::IntoResponse;
        let grant = &self.grant_of(token).context("only a running call asks the node")?;
        let ok = |v: Value| axum::Json(v).into_response();
        Ok(match (method, path) {
            ("POST", "/call") => {
                ensure!(!grant.read_only, "this call only reads: it can't run other apps' operations");
                let req: Value = serde_json::from_slice(&body)?;
                let (app, op) = (req["app"].as_str().unwrap_or(""), req["op"].as_str().unwrap_or(""));
                let (apps, drivers) = self.uses(&grant.app).await;
                ensure!(apps.iter().chain(&drivers).any(|u| u == app), "{} doesn't use {app}", grant.app);
                let call = CallRequest { app: app.into(), op: op.into(), input: req["input"].clone(), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("POST", "/blob") => {
                let hash = self.put_blob(&body)?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.puts.push(hash.clone());
                }
                ok(json!({ "blob": hash }))
            }
            ("GET", p) if p.starts_with("/blob/") => {
                let hash = &p["/blob/".len()..];
                ensure!(grant.puts.iter().any(|h| h == hash) || self.blob_named_for(hash, &grant.caller.user)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            ("GET", "/catalog") => ok(json!(self.catalog(&grant.caller).await?)),
            _ => bail!("the node has no {method} {path}"),
        })
    }

    /// Everything a shell needs to draw this server and its install, for this server's local user.
    pub fn state(&self) -> Value {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller { key: self.0.id, user })
    }

    /// The state as `viewer`, this server or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Value {
        let ready = self.ready();
        let install = self.0.install.lock().unwrap();
        let viewer_name = if viewer.key == self.0.id { self.server_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let user = install.as_ref().and_then(|i| i.r.members.user(&viewer.user)).map(|u| json!({ "id": u.id, "name": u.name, "role": u.role }));
        let install = install.as_ref().map(|i| {
            let m = &i.r.members;
            let me = &viewer.user;
            let list = |ps: Vec<Principal>| ps.iter().map(|p| p.to_string()).collect::<Vec<_>>();
            let servers: Vec<Value> = m
                .servers()
                .map(|s| {
                    let session = i.sessions.get(&s.key);
                    json!({
                        "key": hex(&s.key), "name": s.name, "user": s.user, "me": s.key == viewer.key, "here": s.key == self.0.id,
                        "online": s.key == self.0.id || session.is_some(),
                        "owners": list(m.owners(&s.key)), "users": list(m.users_of(&s.key)), "privileged": m.privileged(&s.key),
                        "trusted_by": m.users.keys().filter(|u| m.trusts(u, &s.key)).collect::<Vec<_>>(),
                        "serves_me": m.serves(&s.key, me),
                        "build": i.builds.get(&s.key).map(|r| &r.commit), "target": i.builds.get(&s.key).map(|r| &r.target),
                        "drivers": session.map(|s| s.drivers.iter().map(|d| d.0.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                    })
                })
                .collect();
            let clients: Vec<Value> = m
                .clients()
                .map(|c| json!({ "key": hex(&c.key), "name": c.name, "user": c.user, "me": c.key == viewer.key, "asked": i.asked.get(&c.key) }))
                .collect();
            let users: Vec<Value> = m.users.values().filter(|u| !u.removed).map(|u| json!({ "id": u.id, "name": u.name, "role": u.role, "me": &u.id == me })).collect();
            let groups: Vec<Value> = m
                .groups
                .values()
                .map(|g| {
                    let members: Vec<Value> = g.members.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
                    let invited: Vec<Value> = g.invited.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
                    json!({ "id": g.id, "name": g.name, "role": m.group_role(me, &g.id), "members": members, "invited": invited })
                })
                .collect();
            let opens: Vec<Value> = i
                .views
                .get(&Scope::User(me.clone()).to_string())
                .map(|(_, s)| s.opens.iter().map(|(by, server, time)| json!({ "by": by, "by_name": m.users.get(by).map(|u| &u.name), "server": hex(server), "time": time, "until": time + members::OPEN_FOR })).collect())
                .unwrap_or_default();
            json!({
                "name": m.name, "root": i.r.root.map(|r| r.to_hex()), "user": me, "role": m.user(me).map(|u| u.role),
                "member": m.server(&self.0.id).is_some(),
                "users": users, "groups": groups, "servers": servers, "clients": clients, "opens": opens,
                "entries": i.r.store.count().unwrap_or(0),
            })
        });
        let opens = install.as_ref().map(|i| i["opens"].clone()).unwrap_or(json!([]));
        let mut state = json!({
            "brand": { "name": crate::brand::NAME, "cli": crate::brand::CLI, "accent": crate::brand::ACCENT, "store": crate::brand::STORE },
            "me": { "key": hex(&viewer.key), "name": viewer_name, "user": viewer.user },
            "server": { "key": hex(&self.0.id), "name": self.server_name(), "ready": ready },
            "user": user,
            "install": install,
            "opens": opens,
        });
        if viewer.key != self.0.id {
            state["client"] = json!({ "key": hex(&viewer.key), "name": viewer_name });
        }
        state
    }

    // Members.

    /// Writes one of `global`'s node entries as the local user, after checking it would count.
    pub async fn member(&self, kind: &str, data: Value) -> Result<()> {
        self.member_as(&Caller { key: self.0.id, user: self.user()? }, kind, data).await
    }

    async fn member_as(&self, caller: &Caller, kind: &str, data: Value) -> Result<()> {
        self.member_content(caller, members::node(kind, data)).await
    }

    async fn member_content(&self, caller: &Caller, content: Content) -> Result<()> {
        let kind = content.kind.clone();
        let by = (caller.key != self.0.id).then_some(caller.key);
        // One that wouldn't count would change nothing anywhere: refuse it instead.
        let counts = self.with(|i| i.r.would_count(&caller.user, by, content.clone(), now_ms()))?;
        ensure!(counts, "that changes nothing: you may not {kind} that");
        self.append_as(&Scope::Global, &caller.user, by, content).await?;
        Ok(())
    }

    /// Deploys `bundle` as `app`'s code, for someone acting for its owner. Deploying an older bundle again rolls back.
    /// `bundle` may be the start of an earlier deploy's.
    pub async fn deploy(&self, caller: &Caller, app: &str, bundle: &str) -> Result<()> {
        let bundle = self.with(|i| {
            let m = &i.r.members;
            let a = m.apps.get(app).with_context(|| format!("no app named {app}"))?;
            ensure!(m.acts_for(&caller.user, &a.owner), "only someone acting for {app}'s owner ({}) deploys it", a.owner);
            Ok(match a.history.iter().find(|(h, ..)| bundle.len() >= 8 && h.starts_with(bundle)) {
                Some((h, ..)) => h.clone(),
                None => bundle.to_string(),
            })
        })?;
        crate::bundle::module(&self.blob(&bundle).await?)?;
        self.member_content(caller, deploy(app, &bundle)).await
    }

    /// An app's code as deployed, its latest or an earlier deploy's: its files, or one of them. Everyone may read
    /// every app's code.
    pub async fn source(&self, app: &str, bundle: Option<&str>, path: Option<&str>) -> Result<Value> {
        let history = self.with(|i| Ok(i.r.members.apps.get(app).with_context(|| format!("no app named {app}"))?.history.clone()))?;
        let bundle = match bundle {
            Some(b) => history.iter().find(|(h, ..)| h.starts_with(b)).with_context(|| format!("{app} has no deploy {b}"))?.0.clone(),
            None => history.last().with_context(|| format!("{app} has no deploy yet"))?.0.clone(),
        };
        let mut out = crate::deploy::show(&self.blob(&bundle).await?, path)?;
        out["app"] = json!(app);
        out["bundle"] = json!(bundle);
        Ok(out)
    }

    pub async fn rename_server(&self, name: &str) -> Result<()> {
        ensure!(!name.trim().is_empty(), "give a name");
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("server.json");
        let mut file: ServerFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        if self.has_install() {
            self.member("rename", json!({ "key": hex(&self.0.id), "name": name })).await?;
        }
        Ok(())
    }

    /// Opens `user`'s data on `server` for an hour, for the local user (see SPEC.md, "Servers").
    pub async fn open(&self, user: &str, server: &str) -> Result<Value> {
        let req = CallRequest { app: members::APP.into(), op: "open".into(), input: json!({ "user": user }), server: Some(server.into()), ..Default::default() };
        self.call(req).await
    }

    // Clients.

    fn is_client(&self, key: &Key) -> bool {
        self.with(|i| Ok(i.r.members.client(key).is_some())).unwrap_or(false)
    }

    /// Serves a client (a phone) that keeps no log of its own: one JSON request per stream.
    async fn accept_client(&self, conn: Connection) -> Result<()> {
        let caller = *conn.remote_id().as_bytes();
        ensure!(self.is_client(&caller), "{} is not a client of this install", short(&caller));
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };
            let Ok(user) = self.with(|i| Ok(i.r.members.client(&caller).map(|c| c.user.clone()))) else { return Ok(()) };
            let Some(user) = user else { return Ok(()) };
            let _ = self.with(|i| Ok(i.asked.insert(caller, now_ms())));
            let node = self.clone();
            let me = Caller { key: caller, user };
            tokio::spawn(async move {
                let result = async {
                    let request: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
                    let (method, path) = (request["method"].as_str().unwrap_or("GET"), request["path"].as_str().unwrap_or(""));
                    if path == "/v1/events" {
                        let mut events = node.subscribe();
                        while let Ok(event) = events.recv().await {
                            if !node.is_client(&caller) {
                                break;
                            }
                            send.write_all(format!("{}\n", serde_json::to_string(&event)?).as_bytes()).await?;
                        }
                        return anyhow::Ok(());
                    }
                    let reply = match node.client_request(&me, method, path, request["body"].clone()).await {
                        Ok(ok) => ok,
                        Err(err) => json!({ "error": format!("{err:#}") }),
                    };
                    send.write_all(&serde_json::to_vec(&reply)?).await?;
                    send.finish()?;
                    Ok(())
                }
                .await;
                if let Err(err) = result {
                    tracing::debug!("client request: {err:#}");
                }
            });
        }
    }

    async fn client_request(&self, me: &Caller, method: &str, path: &str, body: Value) -> Result<Value> {
        let serves = self.with(|i| Ok(i.r.members.serves(&self.0.id, &me.user)))?;
        if !serves && path != "/v1/state" {
            return Ok(json!({ "elsewhere": format!("{} doesn't serve you", self.server_name()) }));
        }
        Ok(match (method, path) {
            ("GET", "/v1/state") => self.state_for(me),
            ("GET", "/v1/apps") => json!(self.apps(me).await?),
            ("GET", "/v1/widgets") => json!(self.widgets(me.clone(), body["scope"].as_str()).await?),
            ("GET", "/v1/screen") => {
                let (app, name) = (body["app"].as_str().unwrap_or(""), body["screen"].as_str().unwrap_or("main"));
                json!({ "html": self.screen(me.clone(), app, name, body["params"].clone(), body["scope"].as_str()).await? })
            }
            ("POST", "/v1/call") => {
                let req: CallRequest = serde_json::from_value(body)?;
                let deployed = self.with(|i| Ok(i.r.members.apps.get(&req.app).is_some_and(|a| a.bundle.is_some())))?;
                // An app's call runs here or the client tries another server; a driver's goes where the driver is.
                match deployed || req.app == members::APP {
                    true => self.run(&req, me).await.json(),
                    false => match self.route(req, me.clone()).await {
                        Ok(ok) => json!({ "ok": ok }),
                        Err(err) => json!({ "error": format!("{err:#}") }),
                    },
                }
            }
            ("GET", "/v1/source") => self.source(body["app"].as_str().unwrap_or(""), body["bundle"].as_str(), body["path"].as_str()).await?,
            ("POST", "/v1/install") => {
                let scope = self.scope_for(me, body["scope"].as_str().unwrap_or("user"))?;
                self.install(me, body["app"].as_str().unwrap_or(""), &scope, body["installed"] != false).await?;
                json!({})
            }
            ("POST", "/v1/group") => {
                let group = new_id();
                let name = body["name"].as_str().unwrap_or("").trim();
                ensure!(!name.is_empty(), "give the group a name");
                self.member_as(me, "group", json!({ "group": group, "name": name })).await?;
                json!({ "group": group })
            }
            ("POST", "/v1/invite") | ("POST", "/v1/member") => {
                let group = self.find_group(body["group"].as_str().unwrap_or(""))?;
                let user = self.find_user(body["user"].as_str().unwrap_or(""))?;
                let role = match (path, &body["role"]) {
                    ("/v1/invite", Value::Null) => json!("member"),
                    (_, role) => role.clone(),
                };
                let _: Option<Role> = serde_json::from_value(role.clone())?;
                self.member_as(me, &path["/v1/".len()..], json!({ "group": group, "user": user, "role": role })).await?;
                json!({})
            }
            ("POST", "/v1/accept") => {
                self.member_as(me, "accept", json!({ "group": self.find_group(body["group"].as_str().unwrap_or(""))? })).await?;
                json!({})
            }
            ("POST", "/v1/role") => {
                let role: Role = serde_json::from_value(body["role"].clone())?;
                self.member_as(me, "role", json!({ "user": self.find_user(body["user"].as_str().unwrap_or(""))?, "role": role })).await?;
                json!({})
            }
            ("POST", "/v1/remove") => {
                let data = match body["user"].as_str() {
                    Some(user) => json!({ "user": self.find_user(user)? }),
                    None => json!({ "key": hex(&self.find_machine(body["key"].as_str().unwrap_or(""))?) }),
                };
                self.member_as(me, "remove", data).await?;
                json!({})
            }
            ("GET", "/v1/secrets") => {
                let scope = body["scope"].as_str().map(|s| self.scope_for(me, s)).transpose()?;
                json!(self.secrets(me, scope.as_ref())?)
            }
            ("POST", "/v1/secrets") => {
                let scope = self.scope_for(me, body["scope"].as_str().unwrap_or("user"))?;
                let hosts = body["hosts"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect();
                let (app, name) = (body["app"].as_str().unwrap_or(""), body["name"].as_str().unwrap_or(""));
                self.set_secret(me, &scope, app, name, hosts, body["value"].as_str().map(str::to_string)).await?;
                json!({})
            }
            ("POST", "/v1/name") => {
                let name = body["name"].as_str().unwrap_or("").trim();
                self.member_as(me, "rename", json!({ "key": hex(&me.key), "name": name })).await?;
                json!({})
            }
            ("POST", "/v1/pair") => json!({ "code": self.pair_as(me, new_user(&body)?)? }),
            ("POST", "/v1/trust") => {
                let server = self.find_server(body["server"].as_str().unwrap_or(""))?;
                let kind = if body["trust"] == false { "untrust" } else { "trust" };
                self.member_as(me, kind, json!({ "server": hex(&server) })).await?;
                json!({})
            }
            ("GET", p) if p.starts_with("/v1/blobs/") => {
                let hash = &p["/v1/blobs/".len()..];
                ensure!(self.blob_named_for(hash, &me.user)?, "no such blob for you");
                json!({ "blob": data_encoding::BASE64.encode(&self.blob(hash).await?) })
            }
            _ => bail!("no such request: {method} {path}"),
        })
    }

    // Pairing.

    /// Makes a one-time pairing code for another of the local user's machines, or, for an admin, a new user's.
    pub fn pair(&self, new_user: Option<(String, Role)>) -> Result<String> {
        self.pair_as(&Caller { key: self.0.id, user: self.user()? }, new_user)
    }

    fn pair_as(&self, asker: &Caller, new_user: Option<(String, Role)>) -> Result<String> {
        self.with(|i| {
            let m = &i.r.members;
            ensure!(m.server(&self.0.id).is_some(), "this server isn't in the install yet");
            ensure!(new_user.is_none() || m.is_admin(&asker.user), "only admins add users; pair one of your own machines instead");
            ensure!(m.serves(&self.0.id, &asker.user), "{} doesn't serve you", self.server_name());
            Ok(())
        })?;
        let secret: [u8; 16] = rand::random();
        let by = (asker.key != self.0.id).then(|| hex(&asker.key));
        self.add_pairing(secret, Pairing { user: asker.user.clone(), by, new_user, until: now_ms() + PAIRING_LIFE })?;
        let code = PairingCode { from: self.0.endpoint.addr(), secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&code, &mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// Pairing codes are kept in a file, so one still works after the server that made it restarts.
    fn add_pairing(&self, secret: [u8; 16], pairing: Pairing) -> Result<()> {
        let mut pairings = self.0.pairings.lock().unwrap();
        pairings.retain(|_, p| p.until > now_ms());
        pairings.insert(secret, pairing);
        self.save_pairings(&pairings)
    }

    fn take_pairing(&self, secret: &[u8; 16]) -> Result<Option<Pairing>> {
        let mut pairings = self.0.pairings.lock().unwrap();
        let pairing = pairings.remove(secret);
        self.save_pairings(&pairings)?;
        Ok(pairing)
    }

    fn save_pairings(&self, pairings: &HashMap<[u8; 16], Pairing>) -> Result<()> {
        let list: Vec<(String, &Pairing)> = pairings.iter().map(|(k, v)| (data_encoding::HEXLOWER.encode(k), v)).collect();
        write_private(&self.0.dir.join("pairings.json"), &serde_json::to_vec(&list)?)?;
        Ok(())
    }

    /// Joins the install a pairing code is for, as a server.
    pub async fn join(&self, code: &str) -> Result<()> {
        let bytes = data_encoding::BASE32_NOPAD.decode(code.trim().to_uppercase().as_bytes()).context("that isn't a pairing code")?;
        let code: PairingCode = ciborium::from_reader(bytes.as_slice()).context("that isn't a pairing code")?;
        self.ensure_no_install()?;
        let conn = self.0.endpoint.connect(code.from.clone(), JOIN_ALPN).await.context("couldn't reach the server that made the pairing code")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(&JoinRequest { secret: code.secret, name: self.server_name(), as_: "server".into() })?).await?;
        send.finish()?;
        let reply: Value = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the pairing code was refused"));
        }
        let reply: JoinReply = serde_json::from_value(reply)?;
        let paired_by = *code.from.id.as_bytes();
        self.save_install(&InstallFile { root: reply.root, user: reply.user, paired_by: Some(hex(&paired_by)) })?;
        self.open_install()?;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.connect_sync(paired_by).await {
                tracing::warn!("first sync after joining: {err:#}");
            }
        });
        self.emit(Event::Install);
        Ok(())
    }

    async fn accept_join(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: JoinRequest = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        let reply = match self.admit(remote, request).await {
            Ok(reply) => serde_json::to_value(reply)?,
            Err(err) => json!({ "error": format!("{err:#}") }),
        };
        send.write_all(&serde_json::to_vec(&reply)?).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }

    async fn admit(&self, key: Key, request: JoinRequest) -> Result<JoinReply> {
        let pairing = self.take_pairing(&request.secret)?.filter(|p| p.until > now_ms()).context("this pairing code was already used or has expired")?;
        let server = match request.as_.as_str() {
            "server" => true,
            "client" => false,
            other => bail!("can't join as {other}"),
        };
        let user = match &pairing.new_user {
            Some(_) => new_id(),
            None => pairing.user.clone(),
        };
        let root = self.with(|i| {
            let m = &i.r.members;
            // A phone only asks servers: one must serve its user, or the phone could do nothing.
            if !server {
                let served = match &pairing.new_user {
                    Some(_) => m.servers().any(|s| m.privileged(&s.key) && m.users_of(&s.key).contains(&Principal::Install)),
                    None => m.servers().any(|s| m.serves(&s.key, &user)),
                };
                ensure!(served, "this install has no server for you yet: a phone needs one of your own servers, or one the install shares with everyone");
            }
            i.r.root.map(|r| r.to_hex()).context("no install")
        })?;
        let by = pairing.by.as_deref().map(unhex).transpose()?;
        let new_user = pairing.new_user.as_ref().map(|(name, role)| (name.as_str(), *role));
        let admit = members::admit(&key, &user, &request.name, server, new_user);
        self.member_as(&Caller { key: by.unwrap_or(self.0.id), user: pairing.user.clone() }, "admit", admit.data).await.context("this server can no longer admit that machine")?;
        Ok(JoinReply { root, user })
    }

    // Sync.

    async fn dial_loop(self) {
        loop {
            let wanted: Vec<Key> = self
                .with(|i| {
                    let mut servers: Vec<Key> = i.r.members.servers().map(|s| s.key).filter(|k| *k != self.0.id).collect();
                    // Still joining: only the server that made the pairing code knows this one is in.
                    if i.r.members.server(&self.0.id).is_none() {
                        servers.extend(i.r.paired_by);
                    }
                    Ok(servers.into_iter().filter(|s| !i.sessions.contains_key(s)).collect())
                })
                .unwrap_or_default();
            let mut dials = tokio::task::JoinSet::new();
            for server in wanted {
                let node = self.clone();
                dials.spawn(async move {
                    if let Err(err) = node.connect_sync(server).await {
                        tracing::debug!("sync with {}: {err:#}", short(&server));
                    }
                });
            }
            // The first round is done once each dial connected or failed, or soon enough; sessions run on.
            let _ = tokio::time::timeout(FIRST_DIAL, async {
                while self.with(|i| Ok(!i.dialed)).unwrap_or(false) && !dials.is_empty() {
                    let all_tried = self.with(|i| Ok(i.r.members.servers().filter(|s| s.key != self.0.id).all(|s| i.sessions.contains_key(&s.key)))).unwrap_or(true);
                    if all_tried {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await;
            let _ = self.with(|i| Ok(i.dialed = true));
            dials.detach_all();
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    }

    async fn connect_sync(&self, server: Key) -> Result<()> {
        let conn = tokio::time::timeout(Duration::from_secs(20), self.0.endpoint.connect(EndpointId::from_bytes(&server)?, sync::ALPN))
            .await
            .context("timed out")??;
        let (send, recv) = conn.open_bi().await?;
        self.session(server, send, recv).await
    }

    async fn accept_sync(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        ensure!(self.with(|i| Ok(i.r.may_sync(&remote)))?, "{} isn't a server of this install", short(&remote));
        let (send, recv) = conn.accept_bi().await?;
        self.session(remote, send, recv).await
    }

    /// What this server's hello says: its frontier, and the drivers serving its local user here.
    fn hello(&self, i: &Install) -> Result<Msg> {
        let mut drivers: Vec<Driver> = self.0.native.iter().map(|n| (n.name.to_string(), vec![i.user.clone()], (n.describe)())).collect();
        drivers.extend(self.0.providers.describe().into_iter().map(|d| (d["name"].as_str().unwrap_or("").to_string(), vec![i.user.clone()], d)));
        i.r.hello(drivers)
    }

    /// Says hello to every peer again, as when this server's drivers change.
    fn say_hello(&self) {
        let _ = self.with(|i| {
            let hello = self.hello(i)?;
            for s in i.sessions.values() {
                let _ = s.tx.send(hello.clone());
            }
            Ok(())
        });
    }

    /// Runs one sync session until the connection drops.
    async fn session(&self, peer: Key, mut send: iroh::endpoint::SendStream, mut recv: iroh::endpoint::RecvStream) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
        self.with(|i| {
            tx.send(self.hello(i)?).ok();
            i.sessions.insert(peer, Session { tx: tx.clone(), frontier: None, drivers: Vec::new(), caught_up: false });
            Ok(())
        })?;
        self.emit(Event::Servers);
        tracing::info!("syncing with {}", self.name_of(&peer));
        let writer = async {
            while let Some(mut msg) = rx.recv().await {
                // Hellos made inside sync carry no drivers; every hello this server sends does.
                if let Msg::Hello { drivers, .. } = &mut msg {
                    if drivers.is_empty() {
                        if let Ok(Msg::Hello { drivers: d, .. }) = self.with(|i| self.hello(i)) {
                            *drivers = d;
                        }
                    }
                }
                sync::write(&mut send, &msg).await?;
            }
            anyhow::Ok(())
        };
        let hellos = async {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let hello = self.with(|i| self.hello(i))?;
                tx.send(hello).map_err(|_| anyhow!("session closed"))?;
            }
        };
        let reader = async {
            loop {
                let msg = sync::read(&mut recv).await?;
                for reply in self.handle(peer, msg).await? {
                    tx.send(reply).map_err(|_| anyhow!("session closed"))?;
                }
            }
        };
        let result: Result<()> = tokio::select! { r = writer => r, r = hellos => r, r = reader => r };
        let _ = self.with(|i| {
            if i.sessions.get(&peer).is_some_and(|s| s.tx.same_channel(&tx)) {
                i.sessions.remove(&peer);
            }
            Ok(())
        });
        self.emit(Event::Servers);
        result
    }

    async fn handle(&self, peer: Key, msg: Msg) -> Result<Vec<Msg>> {
        let _write = self.0.writes.lock().await;
        let drivers_changed = self.with(|i| {
            let Msg::Hello { frontier, drivers } = &msg else { return Ok(false) };
            let Some(s) = i.sessions.get_mut(&peer) else { return Ok(false) };
            s.frontier = Some(frontier.clone());
            let changed = s.drivers != *drivers;
            s.drivers = drivers.clone();
            Ok(changed)
        })?;
        let mut handled = self.with(|i| i.r.handle(&peer, msg, now_ms()))?;
        let replies = std::mem::take(&mut handled.replies);
        let caught_up = self.with(|i| {
            let Some(frontier) = i.sessions.get(&peer).and_then(|s| s.frontier.clone()) else { return Ok(false) };
            if i.sessions[&peer].caught_up {
                return Ok(false);
            }
            let mine: HashMap<(Key, String), u32> = i.r.store.frontier()?.into_iter().map(|(a, l, n)| ((a, l), n)).collect();
            let lacking = frontier.iter().any(|(a, l, n)| Scope::parse(l).is_ok_and(|s| i.r.holds(&s)) && mine.get(&(*a, l.clone())).copied().unwrap_or(0) < *n);
            if !lacking {
                i.sessions.get_mut(&peer).unwrap().caught_up = true;
            }
            Ok(!lacking)
        })?;
        self.after(handled, Some(peer)).await?;
        if drivers_changed || caught_up {
            self.emit(Event::Servers);
        }
        Ok(replies)
    }

    async fn accept_call(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
        let reply = match self.check_caller(remote, &request) {
            Ok(me) => {
                let req: CallRequest = serde_json::from_value(request)?;
                self.run(&req, &me).await
            }
            Err(err) => Reply::Error(format!("{err:#}")),
        };
        send.write_all(&serde_json::to_vec(&reply.json())?).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }

    /// A call is taken only as a user from one of their clients or a server they trust.
    fn check_caller(&self, remote: Key, request: &Value) -> Result<Caller> {
        let key = unhex(request["me"]["key"].as_str().context("who's asking?")?)?;
        let user = request["me"]["user"].as_str().context("who's asking?")?.to_string();
        self.with(|i| {
            let m = &i.r.members;
            let mine = |k: &Key| m.client(k).is_some_and(|c| c.user == user) || m.trusts(&user, k);
            ensure!(mine(&key) && (remote == key || m.trusts(&user, &remote)), "{} may not call as that user", short(&remote));
            Ok(())
        })?;
        Ok(Caller { key, user })
    }

    /// Finds a server of the install by name or key prefix.
    pub fn find_server(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.servers().filter(|m| m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name)).map(|m| m.key).collect()))?;
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no server called {name}"),
            _ => bail!("several servers match {name}"),
        }
    }

    /// Finds a server or client of the install by name or key prefix.
    pub fn find_machine(&self, name: &str) -> Result<Key> {
        let found: Vec<Key> = self.with(|i| Ok(i.r.members.machines.values().filter(|m| !m.removed && !name.is_empty() && (m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name))).map(|m| m.key).collect()))?;
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no server or client called {name}"),
            _ => bail!("several servers or clients match {name}"),
        }
    }

    /// Finds a user by name or id.
    pub fn find_user(&self, name: &str) -> Result<String> {
        let found: Vec<String> = self.with(|i| Ok(i.r.members.users.values().filter(|u| !u.removed && (u.id == name || u.name.eq_ignore_ascii_case(name))).map(|u| u.id.clone()).collect()))?;
        match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!("no user called {name}"),
            _ => bail!("several users are called {name}; give their id"),
        }
    }

    /// A scope as someone gives it: `user` (their own), `global`, `group:<id>`, or a group's name or id.
    pub fn scope_for(&self, caller: &Caller, scope: &str) -> Result<Scope> {
        Ok(match scope {
            "user" => Scope::User(caller.user.clone()),
            s => Scope::parse(s).or_else(|_| self.find_group(s).map(Scope::Group))?,
        })
    }

    /// Finds a group by name or id.
    pub fn find_group(&self, name: &str) -> Result<String> {
        let found: Vec<String> = self.with(|i| Ok(i.r.members.groups.values().filter(|g| g.id == name || g.name.eq_ignore_ascii_case(name)).map(|g| g.id.clone()).collect()))?;
        match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!("no group called {name}"),
            _ => bail!("several groups are called {name}; give its id"),
        }
    }

    fn name_of(&self, key: &Key) -> String {
        self.with(|i| Ok(i.r.members.machines.get(key).map(|m| m.name.clone()))).ok().flatten().unwrap_or_else(|| short(key))
    }
}

/// Which views to replay for new entries: the new entries that count, onto each scope's views, or, where one sorts
/// before what a scope's views have replayed (a member entry of `global` included), that whole scope again.
/// Only `apps`' stores are replayed.
fn replay_jobs(i: &mut Install, new: &[Entry], apps: &[String]) -> Result<Vec<Replay>> {
    let earliest_member = new.iter().filter(|e| members::is_member_entry(e)).map(Entry::order).min();
    let mut logs: Vec<String> = new.iter().map(|e| e.log().to_string()).collect();
    if let Some(m) = &earliest_member {
        logs.extend(i.views.iter().filter(|(_, (pos, _))| m <= pos).map(|(log, _)| log.clone()));
    }
    logs.sort();
    logs.dedup();
    let mut jobs = Vec::new();
    for log in logs {
        let Ok(scope) = Scope::parse(&log) else { continue };
        if !i.r.holds(&scope) {
            continue;
        }
        let mut fresh: Vec<Entry> = new.iter().filter(|e| e.log() == log).cloned().collect();
        fresh.sort_by_key(Entry::order);
        let view = i.views.get(&log);
        let rebuild = match view {
            None => true,
            Some((pos, _)) => fresh.first().is_some_and(|e| e.order() <= *pos) || earliest_member.as_ref().is_some_and(|m| m <= pos),
        };
        let (entries, state) = match rebuild {
            true => (i.r.store.scope(&log)?, ScopeState::default()),
            false => (fresh, view.unwrap().1.clone()),
        };
        let Some(last) = entries.last().map(Entry::order) else { continue };
        let mut counted: Vec<AppEntry> = Vec::new();
        let (_, state) = members::replay_from(i.r.root, &i.r.member_entries, &entries, state, |e, m, _| {
            if e.content.app != members::APP && apps.contains(&e.content.app) {
                counted.push(app_entry(e, m));
            }
        });
        let position = match view {
            Some((pos, _)) if !rebuild => pos.clone().max(last),
            _ => last,
        };
        i.views.insert(log.clone(), (position, state));
        for app in apps {
            let entries: Vec<AppEntry> = counted.iter().filter(|e| &e.app == app).cloned().collect();
            if rebuild || !entries.is_empty() {
                jobs.push(Replay { app: app.clone(), scope: log.clone(), fresh: rebuild, entries });
            }
        }
    }
    Ok(jobs)
}

/// An entry as apps see it, with the names and roles current when it was written.
fn app_entry(e: &Entry, m: &Members) -> AppEntry {
    let scope = e.scope();
    let named = e.by().unwrap_or(e.author());
    AppEntry {
        app: e.content.app.clone(),
        id: e.id(),
        scope: e.log().to_string(),
        kind: e.content.kind.clone(),
        user: e.user().to_string(),
        name: m.machines.get(&named).map(|k| k.name.clone()).unwrap_or_default(),
        role: m.role_in(e.user(), &scope),
        groups: m.groups_of(e.user()),
        time: e.time(),
        data: e.content.data.clone(),
        blobs: e.content.blobs.clone(),
    }
}

/// The scopes a call of `app` as `user` may reach: their own if it's installed for them at all, each of their groups
/// it's installed for (in the group or in `global`; a group whose installs this server can't see counts), and `global`
/// if it's installed there.
fn reachable(i: &Install, app: &str, user: &str) -> Vec<Scope> {
    let m = &i.r.members;
    let installed = |s: &Scope| i.views.get(&s.to_string()).is_some_and(|(_, st)| st.installed.contains(app));
    let global = installed(&Scope::Global);
    let mut out = Vec::new();
    for g in m.groups_of(user).keys() {
        let s = Scope::Group(g.clone());
        if global || installed(&s) || !i.r.holds(&s) {
            out.push(s);
        }
    }
    let mine = Scope::User(user.into());
    if global || installed(&mine) || out.iter().any(|s| installed(s)) {
        out.insert(0, mine);
    }
    if global {
        out.push(Scope::Global);
    }
    out
}

fn handles(m: &Members, scopes: &[Scope], user: &str) -> Vec<ScopeHandle> {
    scopes
        .iter()
        .map(|s| {
            let name = match s {
                Scope::Global => m.name.clone(),
                Scope::Group(g) => m.groups.get(g).map(|g| g.name.clone()).unwrap_or_default(),
                Scope::User(u) => m.users.get(u).map(|u| u.name.clone()).unwrap_or_default(),
            };
            ScopeHandle { id: s.to_string(), name, role: m.role_in(user, s) }
        })
        .collect()
}

/// Checks a call may run here, and makes its context; or says why it should run elsewhere.
fn prepare(node: &Node, i: &Install, req: &CallRequest, me: &Caller) -> Result<std::result::Result<(Context, Vec<Scope>), String>> {
    let m = &i.r.members;
    let here = node.0.id;
    if let Some(target) = &req.as_user {
        let scope = Scope::User(target.clone());
        let open = i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.opens.iter().any(|(by, server, t)| by == &me.user && *server == here && now_ms() < t + members::OPEN_FOR));
        ensure!(open, "that user's data isn't open for you on {}", node.server_name());
        let caller = Caller { key: me.key, user: target.clone() };
        let ctx = Context { me: node_me(node, m, &caller), now: now_ms(), scopes: handles(m, std::slice::from_ref(&scope), target), read_only: true, token: String::new() };
        return Ok(Ok((ctx, vec![scope])));
    }
    if !m.serves(&here, &me.user) {
        return Ok(Err(format!("{} doesn't serve you", node.server_name())));
    }
    let reach = reachable(i, &req.app, &me.user);
    let scopes: Vec<Scope> = match &req.scopes {
        None => reach.clone(),
        Some(named) => {
            let mut out = Vec::new();
            for s in named {
                let s = if s == "user" { Scope::User(me.user.clone()) } else { Scope::parse(s)? };
                ensure!(reach.contains(&s), "{} can't reach {s} for you", req.app);
                out.push(s);
            }
            out
        }
    };
    if reach.is_empty() {
        bail!("{} isn't installed for you", req.app);
    }
    if let Some(s) = scopes.iter().find(|s| !i.r.holds(s)) {
        return Ok(Err(format!("{} doesn't hold {s}", node.server_name())));
    }
    let ctx = Context { me: node_me(node, m, me), now: now_ms(), scopes: handles(m, &scopes, &me.user), read_only: false, token: String::new() };
    Ok(Ok((ctx, scopes)))
}

fn node_me(node: &Node, m: &Members, caller: &Caller) -> Me {
    let name = if caller.key == node.0.id { node.server_name() } else { m.machine(&caller.key).map(|k| k.name.clone()).unwrap_or_default() };
    Me { key: hex(&caller.key), user: caller.user.clone(), name, role: m.user(&caller.user).map(|u| u.role) }
}

/// Checks one of a call's appends: into a scope it named, or a secret of the calling app's in a scope it named, which
/// the caller may set.
fn check_append(i: &Install, app: &str, append: Append, scopes: &[Scope], me: &Caller) -> Result<(Scope, Content)> {
    Ok(match append {
        Append::Entry { scope, content } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(content.app != members::APP, "apps don't write the node's entries");
            (scope, content)
        }
        Append::Secret { scope, name, value, hosts } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            (scope.clone(), secret_entry(i, &me.user, &scope, app, &name, hosts, value)?)
        }
        Append::Install { .. } => bail!("installs are checked by install_entries"),
    })
}

/// A `secret` entry `user` may write in `scope`, setting (or with no value, deleting) a secret of `app`.
fn secret_entry(i: &Install, user: &str, scope: &Scope, app: &str, name: &str, hosts: Vec<String>, value: Option<String>) -> Result<Content> {
    let m = &i.r.members;
    ensure!(members::manages_scope(m, user, scope), "you can't set secrets in {scope}");
    ensure!(m.apps.contains_key(app), "no app named {app}");
    ensure!(members::valid_secret_name(name), "a secret's name is letters, digits, _, - and .");
    ensure!(value.is_none() || (!hosts.is_empty() && hosts.iter().all(|h| members::valid_host(h))), "give the hosts it's sent to, as host names (api.example.com)");
    ensure!(i.r.holds(scope), "this server doesn't hold {scope}");
    Ok(members::node("secret", json!({ "app": app, "name": name, "hosts": hosts, "value": value })))
}

/// A deploy of `app`: the bundle at blob `bundle`.
pub fn deploy(app: &str, bundle: &str) -> Content {
    Content { blobs: vec![bundle.into()], ..members::node("deploy", json!({ "app": app, "bundle": bundle })) }
}

/// `{ new_user: { name, role } }` in a pairing request.
pub fn new_user(body: &Value) -> Result<Option<(String, Role)>> {
    match &body["new_user"] {
        Value::Null | Value::Bool(false) => Ok(None),
        v => {
            let name = v["name"].as_str().filter(|n| !n.trim().is_empty()).context("give the new user a name")?.trim().to_string();
            let role: Role = serde_json::from_value(v.get("role").cloned().unwrap_or(json!("member")))?;
            Ok(Some((name, role)))
        }
    }
}

impl Node {
    /// Publishes `binary` (a build of this node for `target`, with the release key's `signature` over it) as the
    /// install's newest release.
    pub async fn publish(&self, binary: &Path, commit: &str, target: &str, committed: u64, signature: &str) -> Result<Release> {
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        release::verify(crate::brand::RELEASE_KEY, &bytes, signature).context("no node would take it")?;
        let blob = self.put_blob(&bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into() };
        self.append(&Scope::Global, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node if a newer build for its platform was published, signed with the release key.
    fn consider_update(&self) {
        let latest = self.with(|i| Ok(release::latest(crate::brand::RELEASE_KEY, &i.r.store.scope("global")?, crate::brand::TARGET)));
        let Ok(Some((r, author))) = latest else { return };
        let marker = self.0.dir.join("release");
        if !release::wanted(&r, crate::brand::BUILD, crate::brand::COMMITTED) || std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == r.blob) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            match node.apply(&r, author).await {
                Ok(exe) => {
                    let _ = std::fs::write(&marker, &r.blob);
                    tracing::info!("updated {} to {}; restarting", exe.display(), r.commit);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(0);
                }
                Err(err) => tracing::warn!("ignoring the release of {}: {err:#}", r.commit),
            }
        });
    }

    async fn apply(&self, r: &Release, author: Key) -> Result<PathBuf> {
        let path = release::blob_path(&self.0.dir, &r.blob)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                let mut peers = vec![author];
                peers.extend(self.peers().into_iter().filter(|p| *p != author));
                self.fetch_from(peers, &r.blob, r.size).await?
            }
        };
        release::verify(crate::brand::RELEASE_KEY, &bytes, &r.signature)?;
        release::install(r, &bytes)
    }

    /// Keeps `bytes` as a blob. See SPEC.md, "Blobs".
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        release::put_blob(&self.0.dir, bytes)
    }

    /// A blob from this server, else from a server it syncs with.
    pub async fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) {
            return Ok(bytes);
        }
        self.fetch_from(self.peers(), hash, 1 << 30).await
    }

    /// Fetches a blob from the first of `peers` that has it, and keeps it.
    async fn fetch_from(&self, peers: Vec<Key>, hash: &str, size: u64) -> Result<Vec<u8>> {
        let mut last = anyhow!("no server had it");
        for peer in peers {
            match self.fetch_blob(peer, hash, size).await {
                Ok(bytes) => {
                    release::put_blob(&self.0.dir, &bytes)?;
                    return Ok(bytes);
                }
                Err(err) => last = err,
            }
        }
        Err(last)
    }

    /// Every entry a native app has in the local user's scope, oldest first.
    fn app_entries(&self, app: &str) -> Result<Vec<AppEntry>> {
        self.with(|i| {
            let mut out = Vec::new();
            let scope = Scope::User(i.user.clone()).to_string();
            members::replay(i.r.root, &i.r.member_entries, &i.r.store.scope(&scope)?, |e, m, _| {
                if e.content.app == app {
                    out.push(app_entry(e, m));
                }
            });
            Ok(out)
        })
    }

    fn sync_native(&self, app: &'static str) {
        let Some(sync) = self.0.native.iter().find(|n| n.name == app).and_then(|n| n.sync) else { return };
        let Ok(entries) = self.app_entries(app) else { return };
        tokio::task::spawn_blocking(move || sync(&entries));
    }

    /// The servers this server syncs with now, then the rest.
    fn peers(&self) -> Vec<Key> {
        self.with(|i| {
            let mut peers: Vec<Key> = i.r.members.servers().map(|m| m.key).filter(|d| *d != self.0.id).collect();
            peers.sort_by_key(|k| !i.sessions.contains_key(k));
            Ok(peers)
        })
        .unwrap_or_default()
    }

    async fn fetch_blob(&self, peer: Key, hash: &str, size: u64) -> Result<Vec<u8>> {
        let conn = tokio::time::timeout(Duration::from_secs(10), self.0.endpoint.connect(EndpointId::from_bytes(&peer)?, BLOB_ALPN)).await.context("timed out")??;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(hash.as_bytes()).await?;
        send.finish()?;
        let bytes = recv.read_to_end(size as usize).await?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "the blob didn't match its hash");
        Ok(bytes)
    }

    /// Whether an entry in a scope `holds` says yes to names the blob.
    fn blob_named(&self, hash: &str, holds: impl Fn(&Scope) -> bool) -> Result<bool> {
        self.with(|i| {
            for log in i.r.store.logs()? {
                if Scope::parse(&log).is_ok_and(|s| holds(&s)) && i.r.store.scope(&log)?.iter().any(|e| e.content.blobs.iter().any(|b| b == hash)) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    fn blob_named_for(&self, hash: &str, user: &str) -> Result<bool> {
        let m = self.with(|i| Ok(i.r.members.clone()))?;
        self.blob_named(hash, |s| m.may_use(user, s))
    }

    async fn accept_blob(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let m = self.with(|i| Ok(i.r.members.clone()))?;
        ensure!(m.server(&remote).is_some(), "not a server of this install");
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hash = String::from_utf8(recv.read_to_end(64).await?)?;
        ensure!(self.blob_named(&hash, |s| m.holds(&remote, s))?, "no such blob for you");
        let bytes = std::fs::read(release::blob_path(&self.0.dir, &hash)?).context("no such blob here")?;
        send.write_all(&bytes).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}

/// Writes a file only this account can read: server keys, pairing secrets.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_pairings(dir: &Path) -> HashMap<[u8; 16], Pairing> {
    let list: Vec<(String, Pairing)> = std::fs::read(dir.join("pairings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.into_iter()
        .filter_map(|(k, v)| Some((data_encoding::HEXLOWER.decode(k.as_bytes()).ok()?.try_into().ok()?, v)))
        .collect()
}

fn load_server(dir: &Path) -> Result<ServerFile> {
    let path = dir.join("server.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let file = ServerFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
    Ok(file)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "server".into())
}

#[derive(Clone, Debug)]
struct SyncHandler(Node);
#[derive(Clone, Debug)]
struct JoinHandler(Node);
#[derive(Clone, Debug)]
struct CallHandler(Node);
#[derive(Clone, Debug)]
struct ClientHandler(Node);
#[derive(Clone, Debug)]
struct BlobHandler(Node);

fn accept_err(err: anyhow::Error) -> AcceptError {
    AcceptError::from_boxed(err.into())
}

impl ProtocolHandler for BlobHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_blob(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for ClientHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_client(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_sync(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for JoinHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_join(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for CallHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_call(conn).await.map_err(accept_err)
    }
}
