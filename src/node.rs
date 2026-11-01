//! A node: this server's install (an edition holds one), its replica and views, its iroh endpoint, and where calls run.

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
use crate::schedules::{self, Schedule};
use crate::store::{Frontier, Store};
use crate::sync::{self, Driver, Handled, Msg, Replica};
use crate::vault;

const JOIN_ALPN: &[u8] = b"one/join/1";
const CALL_ALPN: &[u8] = b"one/call/1";
pub const CLIENT_ALPN: &[u8] = b"one/client/1";
const BLOB_ALPN: &[u8] = b"one/blob/1";
/// How long a call may run before it's stopped.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A driver's call may take longer, as a browser or a desktop starts: as long as a provider is waited for.
const DRIVER_LIMIT: Duration = Duration::from_secs(600);
/// The most a call or a client's request (or its answer) may carry: an email of a claim's receipts, base64, fits.
const MESSAGE_LIMIT: usize = 64 << 20;
/// How long a waking server waits for its first round of dialing before it may call itself ready.
const FIRST_DIAL: Duration = Duration::from_secs(5);
/// How many app-to-app calls deep a call may be: a person's call of A, A's of B, B's of C, C's of D.
pub const MAX_DEPTH: u32 = 3;
/// How many calls an app may make of other apps, or runs of its schedules, a minute on one server.
pub const RATE_LIMIT: u32 = 600;
/// How long an app that looped stays stopped on a server, unless it's deployed again sooner.
const STOPPED_FOR: u64 = 15 * 60_000;
/// How often a server looks for schedules to run.
const SCHEDULE_TICK: Duration = Duration::from_secs(10);
/// How long a call's work may carry on after it returns (`ctx.waitUntil`).
const LATER_LIMIT: Duration = Duration::from_secs(600);
/// How many of an app's failures a server records an hour; the rest are only counted on the next hour's.
const FAILURES_KEPT: u32 = 30;
/// How long a request for an item waits for its answer.
const REQUEST_LIFE: u64 = 86_400_000;
/// How long an app's page says a deploy changed or removed one of its schedules.
const CHANGED_SHOWN: u64 = 14 * 86_400_000;

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
    /// How loaded the peer said it is.
    load: Option<crate::load::Load>,
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
    /// The user the machine joins as, when it isn't the asker's: an admin's code for someone else's machine, or a new
    /// user's id, chosen when the code is made. Codes made before this have none, and a new user's id is chosen then.
    #[serde(default)]
    admits: Option<String>,
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
    /// The agent that asked through it, by name ("Claude Code"), when one did.
    pub agent: Option<String>,
}

/// An operation to run: of an app, or of a driver.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallRequest {
    /// The app, or for a driver's call, the driver (older machines name a driver here).
    #[serde(default)]
    pub app: String,
    /// The driver whose operation this is, when it is one: an app and the driver it wraps may share a name.
    #[serde(default)]
    pub driver: Option<String>,
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
    /// The agent that asked (an MCP client's name), credited on the call.
    #[serde(default)]
    pub agent: Option<String>,
    /// How many app-to-app calls deep it is: 0 when a person, an agent or a schedule made it.
    #[serde(default)]
    pub depth: u32,
    /// The app whose `ctx.apps` made it, if one did.
    #[serde(default)]
    pub from: Option<String>,
    /// What made it, for its app's insights: `cli`, `mcp`, `mac`, `phone`, `browser`, `schedule` or `app`.
    #[serde(default)]
    pub client: Option<String>,
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
    /// The bundle of each app this release has built in, kept as a blob: any install may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png` as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The logos of the sites apps and drivers connect to.
    logos: crate::logos::Logos,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The port apps' requests go out through.
    outbound: u16,
    woke: Instant,
    /// What each app did in its last minute, and the apps stopped here for looping.
    guard: Mutex<Guard>,
    /// The schedules' slots running here now: `<scope>/<app>/<schedule>/<slot>`.
    running: Mutex<std::collections::HashSet<String>>,
    /// The calls that returned with work still running (`ctx.waitUntil`), by token: told when it's done.
    later: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Who used which app today from which client, as recorded here: (app, user, client, day).
    seen: Mutex<std::collections::HashSet<(String, String, String, u64)>>,
    /// How many failures of each app were recorded here each hour.
    failures: Mutex<HashMap<(String, u64), u32>>,
}

/// Loop guards: an app that calls other apps, or runs its schedules, more than `RATE_LIMIT` times a minute on this
/// server is stopped here for `STOPPED_FOR`, or until it's deployed again.
#[derive(Default)]
struct Guard {
    /// Each app's calls or runs since the minute it started counting (milliseconds): a minute from its first, not
    /// the clock's, so a loop that runs across the turn of a minute is still caught.
    counts: HashMap<(String, &'static str), (u64, u32)>,
    /// Each app stopped here: why, until when, and the bundle it was running.
    stopped: HashMap<String, (String, u64, Option<String>)>,
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
    /// How many app-to-app calls deep the call is.
    pub depth: u32,
    /// The app whose `ctx.apps` made the call, if one did.
    pub from: Option<String>,
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
        let mut built_in = BTreeMap::new();
        for (app, files) in crate::config::BUILT_IN {
            built_in.insert(app.to_string(), release::put_blob(dir, &built_in_bundle(files)?)?);
        }
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
            built_in,
            images: Mutex::new(HashMap::new()),
            logos: crate::logos::Logos::open(dir),
            grants: Mutex::new(HashMap::new()),
            outbound: outbound.local_addr()?.port(),
            woke: Instant::now(),
            guard: Mutex::new(Guard::default()),
            running: Mutex::new(Default::default()),
            later: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            failures: Mutex::new(HashMap::new()),
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
        tokio::spawn(node.clone().schedule_loop());
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
        if let Some(why) = self.removed_here(app) {
            bail!("{why}");
        }
        if let Some(domain) = describe["domain"].as_str() {
            self.logo(domain, app);
        }
        self.0.providers.register(app, describe);
        self.say_hello();
        Ok(())
    }

    /// Removes a driver from a server (this one if none is named), or with `installed` lets it register there again,
    /// for someone acting for one of the server's owners.
    pub async fn set_driver(&self, caller: &Caller, server: Option<&str>, name: &str, installed: bool) -> Result<()> {
        let server = server.map(str::to_string).unwrap_or_else(|| self.server_name());
        let content = self.with(|i| driver_entry(&i.r.members, &caller.user, &server, name, installed))?;
        self.member_content(caller, content).await?;
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

    /// Refuses a second install: an edition holds one, and another context is another edition.
    fn ensure_no_install(&self) -> Result<()> {
        if self.has_install() {
            bail!("this server already belongs to an install, and {} holds one; for another, install that context's own app", crate::config::NAME);
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

    /// Creates this server's install, with its local user as the first admin, and deploys the config's apps in
    /// `global`. Returns the recovery key's secret, given only this once.
    pub async fn create(&self, name: &str, user_name: &str) -> Result<String> {
        self.create_with(name, user_name, crate::config::STARTERS).await
    }

    /// Creates the install, starting with these built-in apps.
    pub async fn create_with(&self, name: &str, user_name: &str, apps: &[&str]) -> Result<String> {
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
        let me = Caller { key: self.0.id, user, agent: None };
        for app in apps {
            self.bring_in(&me, app).await?;
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
        let forgotten: Vec<String> = handled.new.iter().filter(|e| e.content.app == members::APP && e.content.kind == "forget").filter_map(|e| e.content.data["blob"].as_str().map(str::to_string)).collect();
        let mut apps: Vec<String> = jobs.iter().map(|j| j.app.clone()).collect();
        apps.extend(handled.new.iter().filter(|e| e.content.app != members::APP).map(|e| e.content.app.clone()));
        apps.sort();
        apps.dedup();
        self.0.apps.replay(jobs).await;
        self.emit(Event::Changed { apps });
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
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
            let wanted: BTreeMap<String, String> = i.r.members.apps.values().filter(|a| !a.erased).filter_map(|a| Some((a.name.clone(), a.bundle.clone()?))).collect();
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
        if running.as_deref() != Some(crate::config::BUILD) {
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
        let me = Caller { key: self.0.id, user: self.user()?, agent: req.agent.clone() };
        self.route(req, me).await
    }

    /// Tries the servers that might run a call, nearest first, one at a time, the next only if one can't be reached
    /// or says to go elsewhere.
    async fn route(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let req = self.resolve(req);
        if req.server.as_deref() == Some("all") {
            return self.everywhere(req, me).await;
        }
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

    /// Runs a driver's call on every server that runs it for the caller, one at a time: `{ <server>: answer }`, or
    /// `{ error }` for one that failed. Only for drivers: an app's call runs once.
    async fn everywhere(&self, req: CallRequest, me: Caller) -> Result<Value> {
        ensure!(req.driver.is_some(), "{} is an app: its calls run once, on one server", req.app);
        let req = CallRequest { server: None, ..req };
        let mut out = serde_json::Map::new();
        for server in self.candidates(&req, &me)? {
            let reply = match server == self.0.id {
                true => Ok(self.run(&req, &me).await),
                false => self.ask_server(server, &req, &me).await,
            };
            let answer = match reply {
                Ok(Reply::Ok(v)) => v,
                Ok(Reply::Error(e) | Reply::Elsewhere(e)) => json!({ "error": e }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            };
            out.insert(self.name_of(&server), answer);
        }
        ensure!(!out.is_empty(), "no server runs {} for you", req.app);
        Ok(Value::Object(out))
    }

    /// The servers that might run a call, nearest first.
    fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        if let Some(server) = &req.server {
            return Ok(vec![self.find_server(server)?]);
        }
        let here = self.runs_driver_here(&req.app);
        self.with(|i| {
            let m = &i.r.members;
            let mut servers: Vec<Key> = if req.driver.is_none() {
                m.servers().map(|s| s.key).filter(|s| m.serves(s, &me.user)).collect()
            } else {
                let mut s: Vec<Key> = i.sessions.iter().filter(|(_, s)| s.drivers.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect();
                if here && (me.user == i.user || (self.0.providers.serves_everyone(&req.app) && i.r.members.serves(&self.0.id, &me.user))) {
                    s.push(self.0.id);
                }
                s
            };
            let rank = |k: &Key| (*k != self.0.id, !i.sessions.contains_key(k), i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)));
            servers.sort_by_key(rank);
            Ok(servers)
        })
    }

    /// A call as the node runs it: one that names a driver is that driver's, whatever app shares its name. As older
    /// machines ask for a driver, so is one that names an app with no deploy, or that runs on every server.
    fn resolve(&self, mut req: CallRequest) -> CallRequest {
        if let Some(driver) = &req.driver {
            req.app = driver.clone();
        } else if req.app != members::APP && (!self.deployed(&req.app) || req.server.as_deref() == Some("all")) {
            req.driver = Some(req.app.clone());
        }
        req
    }

    fn deployed(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.bundle.is_some()))).unwrap_or(false)
    }

    /// The drivers serving on this server, whom each serves, and what each says it is: those built into the node,
    /// then those registered. A driver one of its owners removed from this server isn't served here.
    fn drivers_here(&self, i: &Install) -> Vec<Driver> {
        let m = &i.r.members;
        let everyone: Vec<String> = m.users.keys().filter(|u| m.serves(&self.0.id, u)).cloned().collect();
        let removed = |name: &str| m.drivers.get(&(self.0.id, name.to_string())).is_some_and(|d| d.removed());
        let built_in = |n: &crate::native::Native| {
            let mut d = (n.describe)();
            d["name"] = json!(n.name);
            d["built_in"] = json!(true);
            d
        };
        let mut drivers: Vec<Driver> = self.0.native.iter().filter(|n| !removed(n.name)).map(|n| (n.name.to_string(), vec![i.user.clone()], built_in(n))).collect();
        drivers.extend(self.0.providers.describe().into_iter().filter(|d| !removed(d["name"].as_str().unwrap_or(""))).map(|d| {
            let users = if d["serves"] == "everyone" { everyone.clone() } else { vec![i.user.clone()] };
            (d["name"].as_str().unwrap_or("").to_string(), users, d)
        }));
        drivers
    }

    /// Why a driver isn't served here, if one of this server's owners removed it.
    fn removed_here(&self, name: &str) -> Option<String> {
        self.with(|i| {
            let m = &i.r.members;
            Ok(m.drivers.get(&(self.0.id, name.to_string())).filter(|d| d.removed()).map(|d| {
                let by = m.users.get(&d.by).map(|u| u.name.as_str()).unwrap_or("an owner");
                format!("{name} was removed from {} by {by}; an owner of it installs it again", self.server_name())
            }))
        })
        .ok()
        .flatten()
    }

    fn runs_driver_here(&self, app: &str) -> bool {
        (self.0.native.iter().any(|n| n.name == app) || self.0.providers.has(app)) && self.removed_here(app).is_none()
    }

    /// Asks another server to run a call.
    async fn ask_server(&self, server: Key, req: &CallRequest, me: &Caller) -> Result<Reply> {
        let conn = tokio::time::timeout(Duration::from_secs(10), self.0.endpoint.connect(EndpointId::from_bytes(&server)?, CALL_ALPN)).await.context("timed out")??;
        let (mut send, mut recv) = conn.open_bi().await?;
        let request = json!({ "app": req.app, "driver": req.driver, "op": req.op, "input": req.input, "scopes": req.scopes, "as": req.as_user, "depth": req.depth, "from": req.from, "client": req.client, "me": { "key": hex(&me.key), "user": me.user, "agent": me.agent } });
        send.write_all(&serde_json::to_vec(&request)?).await?;
        send.finish()?;
        let reply = tokio::time::timeout(self.limit(req).await + Duration::from_secs(5), recv.read_to_end(MESSAGE_LIMIT)).await.context("timed out")??;
        Ok(Reply::from_json(serde_json::from_slice(&reply)?))
    }

    /// Runs a call here, for `me`, or says why it can't.
    async fn run(&self, req: &CallRequest, me: &Caller) -> Reply {
        let req = &self.resolve(req.clone());
        let limit = self.limit(req).await;
        let result = tokio::time::timeout(limit, self.run_here(req, me, limit)).await;
        match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(err)) => Reply::Error(format!("{err:#}")),
            Err(_) => Reply::Error(format!("it took longer than {}s and was stopped", limit.as_secs())),
        }
    }

    /// How long a call may run: an app's, CALL_LIMIT; a driver's, and that of an app that uses drivers (which waits
    /// on theirs), DRIVER_LIMIT.
    async fn limit(&self, req: &CallRequest) -> Duration {
        if req.app == members::APP {
            return CALL_LIMIT;
        }
        if req.driver.is_some() || !self.uses_here(&req.app).await.1.is_empty() { DRIVER_LIMIT } else { CALL_LIMIT }
    }

    async fn run_here(&self, req: &CallRequest, me: &Caller, limit: Duration) -> Result<Reply> {
        let input = if req.input.is_null() { json!({}) } else { req.input.clone() };
        if req.app == members::APP {
            return self.node_op(&req.op, &input, me).await.map(Reply::Ok);
        }
        if req.driver.is_some() {
            if !self.runs_driver_here(&req.app) {
                return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
            }
            return match self.run_driver(&req.app, &req.op, input, me, req.from.clone()).await {
                Err(err) if err.is::<crate::native::Elsewhere>() => Ok(Reply::Elsewhere(err.to_string())),
                other => other.map(Reply::Ok),
            };
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
        if self.deleted(&req.app) {
            return Ok(Reply::Error(format!("{} was deleted: its owners can restore it from Recently Deleted in the App Store", req.app)));
        }
        if let Some(why) = self.stopped(&req.app) {
            self.record(&req.app, req, me, Some(&why));
            return Ok(Reply::Error(why));
        }
        let mut ctx = ctx;
        ctx.from = req.from.clone();
        let granted = self.grant(&req.app, me, &mut ctx, req.depth);
        // Just under the call's own limit, so a call that's stopped is recorded first.
        let ran = tokio::time::timeout(limit - Duration::from_secs(1), async {
            let mut outcome = self.0.apps.call(&req.app, &req.op, input, &ctx, limit).await?;
            self.write_appends(&req.app, std::mem::take(&mut outcome.appends), &scopes, me, req.from.as_deref()).await?;
            anyhow::Ok(outcome)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("it took longer than {}s and was stopped", limit.as_secs())));
        let outcome = match ran {
            Ok(outcome) => outcome,
            Err(err) => {
                self.record(&req.app, req, me, Some(&format!("{err:#}")));
                return Err(err);
            }
        };
        self.record(&req.app, req, me, None);
        if outcome.later {
            self.keep_for_later(granted);
        }
        Ok(Reply::Ok(outcome.result))
    }

    // Insights.

    /// Records, for an app's owners, a call that ran here: the first each day by each user from each client
    /// (`used`), and every failure (`failed`), at most `FAILURES_KEPT` an hour per app on this server. They go in the
    /// owner's scope when the caller may use it and this server holds it, else in `global`; in the background.
    fn record(&self, app: &str, req: &CallRequest, me: &Caller, error: Option<&str>) {
        let now = now_ms();
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let content = match error {
            None => {
                let day = now / 86_400_000;
                if !self.0.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
                    return;
                }
                members::node("used", json!({ "app": app, "client": client, "agent": me.agent, "day": day }))
            }
            Some(error) => {
                let hour = now / 3_600_000;
                let mut failures = self.0.failures.lock().unwrap();
                let n = failures.entry((app.to_string(), hour)).or_default();
                *n += 1;
                if *n > FAILURES_KEPT {
                    return;
                }
                let error: String = error.lines().next().unwrap_or("").chars().take(300).collect();
                members::node("failed", json!({ "app": app, "op": req.op, "error": error, "client": client, "agent": me.agent }))
            }
        };
        self.write_insight(app, me, content);
    }

    /// Writes an insight about `app` for `me`'s call: in the app owner's scope when `me` may use it and this server
    /// holds it, else in `global`; in the background.
    fn write_insight(&self, app: &str, me: &Caller, content: Content) {
        let Ok(scope) = self.with(|i| {
            let m = &i.r.members;
            let owner = m.apps.get(app).map(|a| a.owner.clone()).context("no such app")?;
            let own = match owner {
                Principal::User(u) => Scope::User(u),
                Principal::Group(g) => Scope::Group(g),
                _ => Scope::Global,
            };
            Ok(if m.may_use(&me.user, &own) && i.r.holds(&own) { own } else { Scope::Global })
        }) else {
            return;
        };
        let (node, user, by) = (self.clone(), me.user.clone(), (me.key != self.0.id).then_some(me.key));
        tokio::spawn(async move {
            if let Err(err) = node.append_as(&scope, &user, by, content).await {
                tracing::debug!("insights: {err:#}");
            }
        });
    }

    /// What an app's owners see of it, from every scope `viewer` may use here: who used it, when and from what;
    /// its errors grouped, with counts; and what it spent on AI, by model. Only for someone acting for its owner.
    fn insights_of(&self, viewer: &Caller, app: &str) -> Result<Option<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let Some(a) = m.apps.get(app) else { return Ok(None) };
            if !m.acts_for(&viewer.user, &a.owner) {
                return Ok(None);
            }
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut people: BTreeMap<(String, String), (u64, u64, Option<String>)> = BTreeMap::new();
            let mut errors: BTreeMap<(String, String), (u64, u64, u64, String)> = BTreeMap::new();
            let mut cost: BTreeMap<String, (f64, u64, u64, u64)> = BTreeMap::new();
            for (log, (_, state)) in &i.views {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = state.insights.get(app).filter(|_| m.may_use(&viewer.user, &scope)) else { continue };
                for ((user, client), u) in &ins.used {
                    let p = people.entry((user.clone(), client.clone())).or_default();
                    p.0 += u.days;
                    p.1 = p.1.max(u.last);
                    if u.agent.is_some() {
                        p.2 = u.agent.clone();
                    }
                }
                for (key, e) in &ins.errors {
                    let x = errors.entry(key.clone()).or_insert((0, u64::MAX, 0, String::new()));
                    x.0 += e.count;
                    x.1 = x.1.min(e.first);
                    if e.last >= x.2 {
                        x.2 = e.last;
                        x.3 = name(&e.user);
                    }
                }
                for (model, c) in &ins.cost {
                    let x = cost.entry(model.clone()).or_default();
                    x.0 += c.usd;
                    x.1 += c.input;
                    x.2 += c.output;
                    x.3 += c.calls;
                }
            }
            let mut usage: Vec<Value> = people.into_iter().map(|((u, client), (days, last, agent))| json!({ "user": name(&u), "client": client, "agent": agent, "days": days, "last": last })).collect();
            usage.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let mut errors: Vec<Value> = errors.into_iter().map(|((op, error), (count, first, last, user))| json!({ "op": op, "error": error, "count": count, "first": first, "last": last, "user": user })).collect();
            errors.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let cost: Vec<Value> = cost.into_iter().map(|(model, (usd, input, output, calls))| json!({ "model": model, "usd": usd, "input": input, "output": output, "calls": calls })).collect();
            Ok(Some(json!({ "usage": usage, "errors": errors, "cost": cost })))
        })
    }

    /// Writes a call's appends as its caller. Every one is checked before any is written: a call that fails writes
    /// nothing. Installs come last, since installing a built-in app the install doesn't have yet adds it first.
    async fn write_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<()> {
        // What it spent on AI counts against the app that called it, if one did.
        let (costs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Cost { .. }));
        for cost in costs {
            if let Append::Cost { data } = cost {
                let to = from.unwrap_or(app);
                let data = json!({ "app": to, "model": data["model"], "usd": data["usd"], "input": data["input"], "output": data["output"], "by": app });
                self.write_insight(to, me, members::node("cost", data));
            }
        }
        let (installs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Install { .. }));
        let mut writes = Vec::new();
        for append in appends {
            writes.push(self.with(|i| check_append(i, app, append, scopes, me))?);
        }
        for append in installs {
            if let Append::Install { app, scope, installed } = append {
                let scope = if scope == "user" { Scope::User(me.user.clone()) } else { Scope::parse(&scope)? };
                writes.extend(self.install_entries(me, &app, &scope, installed).await?);
            }
        }
        let by = (me.key != self.0.id).then_some(me.key);
        for (scope, content) in writes {
            self.append_as(&scope, &me.user, by, content).await?;
        }
        Ok(())
    }

    /// Keeps a call's grant after it returned, while the work it left running (`ctx.waitUntil`) goes on: its requests
    /// still go out as the call's, and it hands in its appends (`POST /appends`) until it says it's done, or for
    /// `LATER_LIMIT` at most.
    fn keep_for_later(&self, granted: Granted) {
        let done = Arc::new(tokio::sync::Notify::new());
        self.0.later.lock().unwrap().insert(granted.1.clone(), done.clone());
        let node = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(LATER_LIMIT, done.notified()).await;
            node.0.later.lock().unwrap().remove(&granted.1);
            drop(granted);
        });
    }

    /// Runs a native app or provider here, for this server's local user.
    async fn run_driver(&self, app: &str, op: &str, input: Value, me: &Caller, from: Option<String>) -> Result<Value> {
        let everyone = self.0.providers.serves_everyone(app) && self.with(|i| Ok(i.r.members.serves(&self.0.id, &me.user)))?;
        let user = if everyone { me.user.clone() } else { self.user()? };
        ensure!(me.user == user, "{app} runs only for {}'s own user", self.server_name());
        let caller = self.me_for(me)?;
        let native = self.0.native.iter().find(|n| n.name == app);
        let (result, appends) = match native {
            Some(n) => (n.call)(op.to_string(), input, caller, if n.entries { self.app_entries(app)? } else { vec![] }).await?,
            None => self.0.providers.call(app, op, input, caller, from).await?,
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
                let mut data = json!({ "server": hex(&self.0.id) });
                if let Some(agent) = &me.agent {
                    data["agent"] = json!(agent);
                }
                self.append_as(&scope, &me.user, by, members::node("open", data)).await?;
                Ok(json!(format!("Opened for an hour on {}.", self.server_name())))
            }
            other => bail!("the node has no operation {other}"),
        }
    }

    /// Whether `app` is in Recently Deleted (or erased): it doesn't run, show or run its schedules until restored.
    fn deleted(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.deleted.is_some() || a.erased))).unwrap_or(false)
    }

    /// Deletes an app (`deleted`), restores it (`restored`) or erases it for good (`erased`), for someone acting for
    /// its owner. A deleted app stops running and leaves every page, but keeps its data until it's erased.
    pub async fn retire(&self, caller: &Caller, app: &str, how: &str) -> Result<()> {
        let (_, content) = self.with(|i| check_append(i, app, Append::Retire { app: app.into(), how: how.into() }, &[], caller))?;
        self.member_content(caller, content).await
    }

    // Schedules.

    /// Now and then, runs the schedules this server is next in line for.
    async fn schedule_loop(self) {
        loop {
            tokio::time::sleep(SCHEDULE_TICK).await;
            if self.has_install() && self.ready() {
                if let Err(err) = self.run_schedules().await {
                    tracing::warn!("schedules: {err:#}");
                }
            }
        }
    }

    /// Each loaded app's schedules, as its latest deploy declares them, with why any don't make sense.
    async fn declared(&self) -> Vec<(String, Vec<std::result::Result<Schedule, (String, String)>>)> {
        let mut out = Vec::new();
        for app in self.0.apps.names().into_iter().filter(|a| !self.deleted(a)) {
            if let Ok(d) = self.0.apps.describe(&app).await {
                let list = Schedule::all(&d);
                if !list.is_empty() {
                    out.push((app, list));
                }
            }
        }
        out
    }

    /// Runs every schedule's slot that's due, hasn't run, and that this server is next in line for: in each scope it
    /// holds where the app is installed, as whoever installed it there or switched the schedule on since. A server
    /// waits `GRACE` for each server before it in line that it's connected to; one it can't reach doesn't count.
    async fn run_schedules(&self) -> Result<()> {
        let now = now_ms();
        let declared = self.declared().await;
        let due = self.with(|i| {
            let m = &i.r.members;
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(scope) = Scope::parse(log) else { continue };
                for (app, list) in &declared {
                    if !state.installed.contains(app) || !i.r.holds(&scope) {
                        continue;
                    }
                    for s in list.iter().filter_map(|s| s.as_ref().ok()) {
                        let Some(user) = runs_as(m, state, &scope, app, &s.name) else { continue };
                        let Some(slot) = s.slot(now) else { continue };
                        let installed = state.installers.get(app).map(|(_, t)| *t).unwrap_or(0);
                        let ran = state.runs.get(&(app.clone(), s.name.clone())).is_some_and(|r| r.iter().any(|r| r.slot == slot));
                        if slot < installed || ran || !schedules::due(s, slot, now) {
                            continue;
                        }
                        let line = schedules::line(m, &scope, &user, app, &s.name);
                        let Some(at) = line.iter().position(|k| *k == self.0.id) else { continue };
                        let ahead = line[..at].iter().filter(|k| i.sessions.contains_key(*k)).count() as u64;
                        if now - slot >= ahead * schedules::GRACE {
                            out.push((scope.clone(), app.clone(), s.clone(), slot, user));
                        }
                    }
                }
            }
            Ok(out)
        })?;
        for (scope, app, schedule, slot, user) in due {
            let key = format!("{scope}/{app}/{}/{slot}", schedule.name);
            if !self.0.running.lock().unwrap().insert(key.clone()) {
                continue;
            }
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(err) = node.run_schedule(&scope, &app, &schedule, slot, &user).await {
                    tracing::warn!("{app}'s schedule {}: {err:#}", schedule.name);
                }
                node.0.running.lock().unwrap().remove(&key);
            });
        }
        Ok(())
    }

    /// Runs one slot of a schedule here: an ordinary call as `user`, touching only `scope`; then writes how it went.
    async fn run_schedule(&self, scope: &Scope, app: &str, schedule: &Schedule, slot: u64, user: &str) -> Result<()> {
        let started = Instant::now();
        let reply = match self.count(app, "schedule").await {
            Err(err) => Reply::Error(format!("{err:#}")),
            Ok(()) => {
                let req = CallRequest { app: app.into(), op: schedule.op.clone(), input: schedule.input.clone(), scopes: Some(vec![scope.to_string()]), client: Some("schedule".into()), ..Default::default() };
                self.run(&req, &Caller { key: self.0.id, user: user.into(), agent: None }).await
            }
        };
        let (ok, error) = match reply {
            Reply::Ok(_) => (true, None),
            Reply::Error(e) => (false, Some(e.chars().take(500).collect::<String>())),
            // Not here yet (not ready, or without the latest deploy): the next tick tries again.
            Reply::Elsewhere(why) => {
                tracing::debug!("{app}'s schedule {} waits: {why}", schedule.name);
                return Ok(());
            }
        };
        let data = json!({ "app": app, "schedule": schedule.name, "slot": slot, "ok": ok, "error": error, "ms": started.elapsed().as_millis() as u64, "spec": schedule.spec() });
        self.append_as(scope, user, None, members::node("ran", data)).await?;
        Ok(())
    }

    /// An app's schedules where it's installed for `viewer` and this server holds the scope: what each runs and when,
    /// whether it's on, whom it runs as, when it runs next, its last run, and whether a deploy changed or removed it;
    /// and its runs, newest first.
    fn schedules_of(&self, viewer: &Caller, app: &str, declared: &[std::result::Result<Schedule, (String, String)>]) -> Result<(Vec<Value>, Vec<Value>)> {
        let now = now_ms();
        self.with(|i| {
            let m = &i.r.members;
            let user_name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let (mut list, mut activity) = (Vec::new(), Vec::new());
            for scope in reachable(i, app, &viewer.user) {
                let Some((_, state)) = i.views.get(&scope.to_string()) else { continue };
                if !state.installed.contains(app) {
                    continue;
                }
                let scope_name = handles(m, std::slice::from_ref(&scope), &viewer.user).remove(0).name;
                let base = |name: &str| {
                    let key = (app.to_string(), name.to_string());
                    let runs = state.runs.get(&key);
                    let last = runs.and_then(|r| r.last()).map(|r| json!({ "time": r.time, "ok": r.ok, "error": r.error }));
                    let changed = state.changed.get(&key).filter(|(.., t)| now.saturating_sub(*t) < CHANGED_SHOWN).map(|(was, is, t)| json!({ "was": schedules::say(was), "is": schedules::say(is), "time": t }));
                    json!({ "name": name, "scope": scope.to_string(), "scope_name": scope_name, "may_switch": members::manages_scope(m, &viewer.user, &scope), "last": last, "changed": changed })
                };
                for s in declared {
                    let mut v = match s {
                        Ok(s) => {
                            let mut v = base(&s.name);
                            let user = runs_as(m, state, &scope, app, &s.name);
                            let switched = state.switches.get(&(app.to_string(), s.name.clone()));
                            v["description"] = json!(s.description);
                            v["when"] = json!(s.say());
                            v["op"] = json!(s.op);
                            v["on"] = json!(switched.is_none_or(|(on, ..)| *on));
                            v["switched"] = json!(switched.map(|(on, by, t)| json!({ "on": on, "by": user_name(by), "time": t })));
                            v["runs_as"] = json!(user.as_deref().map(user_name));
                            v["next"] = json!(user.and(s.next(now)));
                            v
                        }
                        Err((name, why)) => {
                            let mut v = base(name);
                            v["invalid"] = json!(why);
                            v
                        }
                    };
                    v["removed"] = json!(false);
                    list.push(v);
                }
                // Schedules a deploy removed that ran lately.
                for ((a, name), runs) in &state.runs {
                    let gone = a == app && !declared.iter().any(|s| match s {
                        Ok(s) => &s.name == name,
                        Err((n, _)) => n == name,
                    });
                    if gone && runs.last().is_some_and(|r| now.saturating_sub(r.time) < CHANGED_SHOWN) {
                        let mut v = base(name);
                        v["removed"] = json!(true);
                        v["was"] = json!(runs.last().map(|r| schedules::say(&r.spec)));
                        list.push(v);
                    }
                }
                for ((a, name), runs) in state.runs.iter().filter(|((a, _), _)| a == app) {
                    let _ = a;
                    for r in runs {
                        activity.push(json!({ "schedule": name, "scope": scope.to_string(), "scope_name": scope_name, "time": r.time, "ok": r.ok, "error": r.error, "ms": r.ms, "server": m.machines.get(&r.server).map(|s| s.name.clone()), "user": user_name(&r.user) }));
                    }
                }
            }
            activity.sort_by_key(|r| std::cmp::Reverse(r["time"].as_u64().unwrap_or(0)));
            activity.truncate(50);
            Ok((list, activity))
        })
    }

    /// Lets the requests of a call, widget or hook of `app` through the node as `caller`'s, while it runs.
    fn grant(&self, app: &str, caller: &Caller, ctx: &mut Context, depth: u32) -> Granted {
        let token = hex(&rand::random::<[u8; 32]>());
        let scopes = ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect();
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new(), depth, from: ctx.from.clone() };
        self.0.grants.lock().unwrap().insert(token.clone(), grant);
        ctx.token = token.clone();
        Granted(self.clone(), token)
    }

    /// Counts one of `app`'s calls of other apps, or one run of its schedules (`kind`). Past `RATE_LIMIT` this minute,
    /// the app is stopped here: it's looping.
    async fn count(&self, app: &str, kind: &'static str) -> Result<()> {
        let now = now_ms();
        let tripped = {
            let mut g = self.0.guard.lock().unwrap();
            let (since, n) = g.counts.entry((app.to_string(), kind)).or_insert((now, 0));
            if now.saturating_sub(*since) >= 60_000 {
                (*since, *n) = (now, 0);
            }
            *n += 1;
            *n == RATE_LIMIT + 1
        };
        if tripped {
            let what = if kind == "call" { "called other apps" } else { "ran its schedules" };
            let why = format!("{app} {what} more than {RATE_LIMIT} times in a minute on {}, so it was stopped there: is it looping? It runs again once it's deployed again, or in {} minutes.", self.server_name(), STOPPED_FOR / 60_000);
            let bundle = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            self.0.guard.lock().unwrap().stopped.insert(app.to_string(), (why.clone(), now_ms() + STOPPED_FOR, bundle));
            tracing::warn!("{why}");
        }
        match self.stopped(app) {
            Some(why) => bail!("{why}"),
            None => Ok(()),
        }
    }

    /// Why `app` is stopped here, if it is: it looped, and hasn't been deployed again or waited out its time since.
    fn stopped(&self, app: &str) -> Option<String> {
        let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
        let mut g = self.0.guard.lock().unwrap();
        let (why, until, bundle) = g.stopped.get(app)?;
        if now_ms() < *until && *bundle == latest {
            return Some(why.clone());
        }
        g.stopped.remove(app);
        None
    }

    /// The ports this node serves on this machine: its local API, workerd, and apps' outbound requests.
    pub fn own_ports(&self) -> Vec<u16> {
        let api = crate::api::addr().rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(crate::config::PORT);
        vec![api, self.0.apps.port(), self.0.outbound]
    }

    /// The grant of a call running now.
    pub fn grant_of(&self, token: &str) -> Option<Grant> {
        self.0.grants.lock().unwrap().get(token).cloned()
    }

    // The Vault.

    /// What a call's placeholder names, as (hosts, value; no hosts for a plain field): `<item id>:field` of an item
    /// in a scope its caller may use, `field` of the oldest item that has it in the caller's own scope, else in
    /// `global`, or `<group id>/field` in that group's; always of an item the call's app is switched on for.
    pub fn vault_for(&self, grant: &Grant, name: &str) -> Option<(Vec<String>, String)> {
        self.with(|i| {
            let m = &i.r.members;
            let user = &grant.caller.user;
            let items = |scope: &Scope| i.views.get(&scope.to_string()).map(|(_, s)| &s.items);
            let oldest = |scope: &Scope, field: &str| items(scope).and_then(|its| vault::find(its, &grant.app, field)).and_then(|(_, it)| vault::filled(it, field));
            Ok(match (name.split_once(':'), name.split_once('/')) {
                (Some((id, field)), _) => i
                    .views
                    .iter()
                    .filter(|(log, _)| Scope::parse(log).is_ok_and(|s| m.may_use(user, &s)))
                    .find_map(|(_, (_, s))| s.items.get(id))
                    .filter(|it| it.apps.contains(&grant.app))
                    .and_then(|it| vault::filled(it, field)),
                (None, Some((group, field))) => {
                    let scope = Scope::Group(group.to_string());
                    if m.may_use(user, &scope) { oldest(&scope, field) } else { None }
                }
                (None, None) => oldest(&Scope::User(user.clone()), name).or_else(|| oldest(&Scope::Global, name)),
            })
        })
        .ok()
        .flatten()
    }

    /// The items `app` is switched on for in `scopes`, as its calls see them (`ctx.vault.items()`).
    fn vault_of(&self, app: &str, scopes: &[Scope]) -> Result<Vec<Value>> {
        self.with(|i| {
            let mut out = Vec::new();
            for scope in scopes {
                let log = scope.to_string();
                let Some((_, state)) = i.views.get(&log) else { continue };
                let mut mine: Vec<(&String, &vault::Item)> = state.items.iter().filter(|(_, it)| it.apps.contains(app)).collect();
                mine.sort_by_key(|(_, it)| it.made);
                out.extend(mine.into_iter().map(|(id, it)| vault::for_app(id, &log, it)));
            }
            Ok(out)
        })
    }

    /// The items in the scopes `viewer` manages (or `scope`), as a person sees them: never a secret field's value.
    /// `wanted_by` is the apps installed for them that ask the Vault for fields it has.
    pub async fn vault(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
        let needs = self.vault_needs(viewer).await?;
        let listed = self.with(|i| {
            let m = &i.r.members;
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                let scope_name = handles(m, std::slice::from_ref(&s), &viewer.user).remove(0).name;
                for (id, it) in &state.items {
                    let wanted: Vec<&String> = needs.iter().filter(|(_, _, fields)| fields.iter().all(|f| it.fields.contains_key(f))).map(|(app, ..)| app).collect();
                    let mut wanted_by: Vec<&String> = wanted.into_iter().chain(&it.apps).collect();
                    wanted_by.sort();
                    wanted_by.dedup();
                    out.push(json!({
                        "id": id, "scope": log, "scope_name": scope_name, "title": it.title, "type": it.kind, "domain": it.domain,
                        "hosts": it.hosts, "fields": vault::fields_shown(it), "apps": it.apps, "wanted_by": wanted_by, "app": it.app,
                        "user": name(&it.user), "time": it.time, "made": it.made,
                    }));
                }
            }
            out.sort_by_key(|v| (v["scope"].as_str().map(|s| (!s.starts_with("user:"), s == "global", s.to_string())), v["made"].as_u64()));
            Ok(out)
        })?;
        let mut out = listed;
        for item in &mut out {
            if let (Some(domain), Some(app)) = (item["domain"].as_str().map(str::to_string), item["app"].as_str().or(item["apps"][0].as_str()).map(str::to_string)) {
                if let Some(image) = self.logo(&domain, &app) {
                    item["image"] = json!(image);
                }
            }
        }
        Ok(out)
    }

    /// What the apps installed for `viewer` ask the Vault for: (app, need, its fields), from their `vault`.
    async fn vault_needs(&self, viewer: &Caller) -> Result<Vec<(String, String, Vec<String>)>> {
        let mut out = Vec::new();
        for app in self.apps(viewer).await? {
            let (Some(name), Some(needs)) = (app["name"].as_str(), app["vault"].as_object()) else { continue };
            if app["installed"].as_array().is_none_or(|i| i.is_empty()) {
                continue;
            }
            for (need, d) in needs {
                let fields: Vec<String> = d["fields"].as_object().map(|f| f.keys().cloned().collect()).unwrap_or_default();
                if !fields.is_empty() {
                    out.push((name.to_string(), need.clone(), fields));
                }
            }
        }
        Ok(out)
    }

    /// Makes (with no `id`) or changes an item in `scope`, for a person or agent (`caller`) → its id.
    pub async fn set_item(&self, caller: &Caller, scope: &Scope, mut data: Value) -> Result<String> {
        let id = data["id"].as_str().map(str::to_string).unwrap_or_else(new_id);
        data["id"] = json!(id);
        // Only the node says which app's call made an item.
        data.as_object_mut().context("an item is an object")?.remove("app");
        let content = self.with(|i| item_entry(i, &caller.user, scope, data))?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        Ok(id)
    }

    /// Asks a person for an item `app` may use, in `scope`, for `caller` (an agent, or an app's page), who never sees
    /// its secret fields: what `app` declares as `need`, or `ask`'s `{ title, fields, hosts }`. → `{ id, link, scope }`.
    pub async fn request_item(&self, caller: &Caller, scope: &Scope, app: &str, need: Option<&str>, ask: &Value, why: &str) -> Result<Value> {
        let mut data = match need {
            Some(need) => {
                let d = self.0.apps.describe(app).await.ok().map(|d| d["vault"][need].clone()).filter(|d| d.is_object()).with_context(|| format!("{app} asks the Vault for no {need}"))?;
                let fields: serde_json::Map<String, Value> = d["fields"].as_object().into_iter().flatten().map(|(n, f)| (n.clone(), json!({ "secret": f["secret"] == true }))).collect();
                json!({ "title": d["title"], "fields": fields, "hosts": d["hosts"], "need": need })
            }
            None => json!({ "title": ask["title"], "fields": ask["fields"], "hosts": ask["hosts"] }),
        };
        let id = new_id();
        data["id"] = json!(id);
        data["app"] = json!(app);
        data["why"] = json!(why);
        data["agent"] = json!(caller.agent);
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, scope), "you can't change the Vault in {scope}");
            ensure!(i.r.members.apps.get(app).is_some_and(|a| !a.erased), "no app named {app}");
            Ok(())
        })?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, members::node("vault-request", data)).await?;
        Ok(json!({ "id": id, "scope": scope.to_string(), "link": format!("{}://vault?id={id}&scope={scope}", crate::config::CLI) }))
    }

    /// Answers a request in `scope`: switches its app on for `item`, or makes an item with `fields` (name → value)
    /// and `title`; with `declined`, declines it.
    pub async fn answer_request(&self, caller: &Caller, scope: &Scope, id: &str, answer: &Value) -> Result<()> {
        let content = self.with(|i| {
            let state = &i.views.get(&scope.to_string()).with_context(|| format!("no request {id} in {scope}"))?.1;
            let r = state.requests.get(id).with_context(|| format!("no request {id} in {scope}"))?;
            ensure!(r.answer.is_none(), "that request was already answered");
            if answer["declined"] == true {
                ensure!(members::manages_scope(&i.r.members, &caller.user, scope), "you can't change the Vault in {scope}");
                return Ok(members::node("vault-request", json!({ "id": id, "declined": true })));
            }
            let data = match answer["item"].as_str() {
                Some(item) => {
                    let it = state.items.get(item).context("no such item there")?;
                    let mut apps = it.apps.clone();
                    apps.insert(r.app.clone());
                    json!({ "id": item, "apps": apps, "request": id })
                }
                None => {
                    let given = answer["fields"].as_object().context("give the item's fields")?;
                    let mut fields = serde_json::Map::new();
                    for (name, secret) in &r.fields {
                        let value = given.get(name).and_then(Value::as_str).filter(|v| !v.is_empty()).with_context(|| format!("give {name}"))?;
                        fields.insert(name.clone(), json!({ "value": value, "secret": secret }));
                    }
                    let title = answer["title"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(&r.title);
                    json!({ "id": new_id(), "title": title, "fields": fields, "hosts": r.hosts, "apps": [r.app], "request": id })
                }
            };
            item_entry(i, &caller.user, scope, data)
        })?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        Ok(())
    }

    /// The requests `viewer` may answer (here, or in `scope`, or just `id`), newest first, with their state:
    /// `waiting`, `set`, `declined` or `expired` (a day after it was made), and the items there that have the
    /// fields it asks for. Never a secret field's value.
    pub fn item_requests(&self, viewer: &Caller, scope: Option<&Scope>, id: Option<&str>) -> Result<Vec<Value>> {
        let now = now_ms();
        self.with(|i| {
            let m = &i.r.members;
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                let scope_name = handles(m, std::slice::from_ref(&s), &viewer.user).remove(0).name;
                for (rid, r) in state.requests.iter().filter(|(rid, _)| id.is_none_or(|want| want == rid.as_str())) {
                    let state_now = match &r.answer {
                        Some((how, ..)) => how.as_str(),
                        None if now.saturating_sub(r.time) > REQUEST_LIFE => "expired",
                        None => "waiting",
                    };
                    let answered = r.answer.as_ref().map(|(_, by, t)| json!({ "by": name(by), "time": t }));
                    let matches: Vec<Value> = state
                        .items
                        .iter()
                        .filter(|(_, it)| r.fields.iter().all(|(f, _)| it.fields.contains_key(f)))
                        .map(|(iid, it)| json!({ "id": iid, "title": it.title, "type": it.kind, "domain": it.domain, "apps": it.apps }))
                        .collect();
                    let fields: Vec<Value> = r.fields.iter().map(|(n, secret)| json!({ "name": n, "secret": secret })).collect();
                    out.push(json!({
                        "id": rid, "scope": log, "scope_name": scope_name, "app": r.app, "title": r.title, "fields": fields, "need": r.need,
                        // For shells built before the Vault, which ask for one secret by name.
                        "name": r.fields.first().map(|(n, _)| n.clone()),
                        "hosts": r.hosts, "why": r.why, "by": name(&r.user), "agent": r.agent, "time": r.time, "state": state_now,
                        "answered": answered, "matches": matches,
                    }));
                }
            }
            out.sort_by_key(|v| std::cmp::Reverse(v["time"].as_u64().unwrap_or(0)));
            Ok(out)
        })
    }

    /// The Vault's routes, for the local API and clients alike, `/v1/vault…` and their earlier names `/v1/secrets…`;
    /// `body` is a GET's query or a POST's body. None for another path.
    pub async fn vault_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        let scope = |default: Option<&str>| body["scope"].as_str().or(default).map(|s| self.scope_for(me, s)).transpose();
        let hosts = || json!(body["hosts"].as_array().cloned().unwrap_or_default());
        let me = &Caller { agent: body["agent"].as_str().map(str::to_string).or(me.agent.clone()), ..me.clone() };
        Ok(Some(match (method, path) {
            ("GET", "/v1/vault") => json!(self.vault(me, scope(None)?.as_ref()).await?),
            ("POST", "/v1/vault") => {
                let scope = scope(Some("user"))?.unwrap();
                let mut data = body.clone();
                data.as_object_mut().context("an item is an object")?.remove("scope");
                json!({ "id": self.set_item(me, &scope, data).await? })
            }
            ("GET", "/v1/vault/requests" | "/v1/secrets/requests") => json!(self.item_requests(me, scope(None)?.as_ref(), body["id"].as_str())?),
            ("POST", "/v1/vault/request") => {
                let scope = scope(Some("user"))?.unwrap();
                self.request_item(me, &scope, body["app"].as_str().unwrap_or(""), body["need"].as_str(), body, body["why"].as_str().unwrap_or("")).await?
            }
            ("POST", "/v1/vault/answer") => {
                self.answer_request(me, &scope(Some("user"))?.unwrap(), body["id"].as_str().unwrap_or(""), body).await?;
                json!({})
            }
            ("GET", "/v1/secrets") => json!(self.secrets(me, scope(None)?.as_ref())?),
            ("POST", "/v1/secrets") => {
                let scope = scope(Some("user"))?.unwrap();
                let (app, name) = (body["app"].as_str().unwrap_or(""), body["name"].as_str().unwrap_or(""));
                self.set_secret(me, &scope, app, name, hosts(), body["value"].as_str()).await?;
                json!({})
            }
            ("POST", "/v1/secrets/request") => {
                let scope = scope(Some("user"))?.unwrap();
                let name = body["name"].as_str().unwrap_or("");
                let ask = json!({ "title": name, "fields": { name: { "secret": true } }, "hosts": hosts() });
                let mut r = self.request_item(me, &scope, body["app"].as_str().unwrap_or(""), None, &ask, body["why"].as_str().unwrap_or("")).await?;
                r["link"] = json!(r["link"].as_str().unwrap_or("").replace("://vault?", "://secret?"));
                r
            }
            ("POST", "/v1/secrets/answer") => {
                let scope = scope(Some("user"))?.unwrap();
                let id = body["id"].as_str().unwrap_or("");
                let answer = match body["value"].as_str() {
                    None => json!({ "declined": true }),
                    Some(value) => {
                        let r = self.item_requests(me, Some(&scope), Some(id))?.into_iter().next().with_context(|| format!("no request {id} in {scope}"))?;
                        json!({ "fields": { r["name"].as_str().unwrap_or(""): value } })
                    }
                };
                self.answer_request(me, &scope, id, &answer).await?;
                json!({})
            }
            _ => return Ok(None),
        }))
    }

    /// Sets field `name` of the item `app` uses in `scope` that has it (or with no value, removes it), or makes an
    /// item of its own: what the earlier `POST /v1/secrets` did.
    async fn set_secret(&self, caller: &Caller, scope: &Scope, app: &str, name: &str, hosts: Value, value: Option<&str>) -> Result<()> {
        let found = self.with(|i| Ok(i.views.get(&scope.to_string()).and_then(|(_, s)| vault::find(&s.items, app, name).map(|(id, it)| (id.clone(), it.fields.len())))))?;
        let data = match (found, value) {
            (Some((id, 1)), None) => json!({ "id": id, "deleted": true }),
            (Some((id, _)), None) => json!({ "id": id, "fields": { name: null } }),
            (None, None) => bail!("{app} has no {name} in {scope}"),
            (Some((id, _)), Some(value)) => {
                let mut d = json!({ "id": id, "fields": { name: { "value": value, "secret": true } } });
                if hosts.as_array().is_some_and(|h| !h.is_empty()) {
                    d["hosts"] = hosts;
                }
                d
            }
            (None, Some(value)) => json!({ "title": name, "fields": { name: { "value": value, "secret": true } }, "hosts": hosts, "apps": [app] }),
        };
        self.with(|i| {
            ensure!(i.r.members.apps.get(app).is_some_and(|a| !a.erased), "no app named {app}");
            Ok(())
        })?;
        self.set_item(caller, scope, data).await.map(|_| ())
    }

    /// Every field of the items `viewer` may change, here or in `scope`, as the earlier `GET /v1/secrets` listed
    /// secrets: scope, app, name, hosts, who set it and when, `given` (not an app's own), and its item. Never a value.
    pub fn secrets(&self, viewer: &Caller, scope: Option<&Scope>) -> Result<Vec<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let mut out = Vec::new();
            for (log, (_, state)) in &i.views {
                let Ok(s) = Scope::parse(log) else { continue };
                if scope.is_some_and(|want| *want != s) || !members::manages_scope(m, &viewer.user, &s) {
                    continue;
                }
                for (id, it) in &state.items {
                    let app = it.app.clone().or_else(|| it.apps.first().cloned()).unwrap_or_default();
                    for name in it.fields.keys() {
                        out.push(json!({ "scope": log, "app": app, "name": name, "hosts": it.hosts, "user": it.user, "time": it.time, "given": it.app.is_none(), "item": id, "title": it.title }));
                    }
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
            Ok(Context { me, now: now_ms(), scopes: handles(&i.r.members, &scopes, &viewer.user), read_only: true, token: String::new(), from: None })
        })
    }

    /// Every widget, rendered for `viewer`; with `scope` (`user`, `group:<id>` or `global`), those of the apps
    /// installed there, for a shell's page of that scope, each reaching only it. Each comes at its own options at every
    /// size it draws, with its options (choices worked out now), and as each widget the viewer put on that page with
    /// options of their own (`placed`, by its id, at its size).
    pub async fn widgets(&self, viewer: Caller, scope: Option<&str>) -> Result<Vec<Value>> {
        let page = scope.map(|s| if s == format!("user:{}", viewer.user) { "user" } else { s });
        let placed: Vec<Value> = page
            .and_then(|p| self.arrangements(&viewer.user).remove(p))
            .and_then(|a| a["items"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item["widget"].is_string() && item["options"].as_object().is_some_and(|o| !o.is_empty()))
            .collect();
        let none = json!({});
        let mut out = Vec::new();
        for (app, ctx, _granted) in self.page_contexts(&viewer, scope)? {
            let describe = self.0.apps.describe(&app).await?;
            for w in describe["widgets"].as_array().into_iter().flatten() {
                let widget = w["name"].as_str().unwrap_or_default();
                let size = w["size"].as_str().unwrap_or("medium");
                let options = match w["options"].as_array().is_some_and(|o| !o.is_empty()) {
                    true => self.0.apps.widget_options(&app, widget, &ctx).await.unwrap_or_else(|_| w["options"].clone()),
                    false => Value::Null,
                };
                match self.0.apps.widget(&app, widget, size, &none, &ctx).await {
                    Ok(root) => {
                        // Each other size it draws, as its own tree; one that fails is left out, and shells draw `root`.
                        let mut sizes = serde_json::Map::new();
                        for other in w["sizes"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).filter(|s| *s != size) {
                            if let Ok(tree) = self.0.apps.widget(&app, widget, other, &none, &ctx).await {
                                sizes.insert(other.to_string(), tree);
                            }
                        }
                        sizes.insert(size.to_string(), root.clone());
                        let mut mine = serde_json::Map::new();
                        for item in placed.iter().filter(|i| i["app"] == app.as_str() && i["widget"] == widget) {
                            let at = item["size"].as_str().unwrap_or(size);
                            let tree = match self.0.apps.widget(&app, widget, at, &item["options"], &ctx).await {
                                Ok(tree) => json!({ "sizes": { at: tree } }),
                                Err(err) => json!({ "error": format!("{err:#}") }),
                            };
                            mine.insert(item["id"].as_str().unwrap_or_default().to_string(), tree);
                        }
                        let mut one = json!({ "app": app, "widget": widget, "title": w["title"], "description": w["description"], "size": size, "sizes": sizes, "root": root });
                        if !options.is_null() {
                            one["options"] = options;
                        }
                        if !mine.is_empty() {
                            one["placed"] = Value::Object(mine);
                        }
                        out.push(one)
                    }
                    // A shell that knows `error` says the widget is broken; one that doesn't still draws `root`.
                    Err(err) => out.push(json!({
                        "app": app, "widget": widget, "title": w["title"], "description": w["description"], "size": size, "error": format!("{err:#}"),
                        "root": { "t": "text", "text": "This widget stopped working.", "tone": "muted" },
                    })),
                }
            }
        }
        Ok(out)
    }

    /// Each app's badge, the count its icon shows, for `viewer`, as `widgets` reaches scopes; apps with none, or 0,
    /// are left out.
    pub async fn badges(&self, viewer: Caller, scope: Option<&str>) -> Result<Value> {
        let mut out = serde_json::Map::new();
        for (app, ctx, _granted) in self.page_contexts(&viewer, scope)? {
            if self.0.apps.describe(&app).await?["badge"] != true {
                continue;
            }
            match self.0.apps.badge(&app, &ctx).await {
                Ok(n) if n.as_f64().is_some_and(|n| n > 0.0) => drop(out.insert(app, n)),
                Ok(_) => {}
                Err(err) => tracing::warn!("{app} badge: {err:#}"),
            }
        }
        Ok(Value::Object(out))
    }

    /// The apps a page shows and the context each renders with: with `scope`, those installed there, each reaching
    /// only it; without, every app, reaching all it may. Each comes with its grant, which lasts as long as it's kept.
    fn page_contexts(&self, viewer: &Caller, scope: Option<&str>) -> Result<Vec<(String, Context, Granted)>> {
        let scope = match scope {
            Some("user") => Some(Scope::User(viewer.user.clone())),
            Some(s) => Some(Scope::parse(s)?),
            None => None,
        };
        let mut out = Vec::new();
        for app in self.0.apps.names().into_iter().filter(|a| !self.deleted(a)) {
            let mut ctx = self.read_context(&app, viewer)?;
            let granted = self.grant(&app, viewer, &mut ctx, 0);
            if let Some(scope) = &scope {
                let installed = self.with(|i| Ok(i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.installed.contains(&app))))?;
                ctx.scopes.retain(|s| installed && s.id == scope.to_string());
            }
            if !ctx.scopes.is_empty() {
                out.push((app, ctx, granted));
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
        let caller = Caller { key: self.0.id, user: self.user()?, agent: None };
        let mut ctx = self.read_context(app, &caller)?;
        let _granted = self.grant(app, &caller, &mut ctx, 0);
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
        let deployed = self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.bundle.is_some())))?;
        let body = match deployed {
            // A screen the app doesn't have (an old or mistyped link) is a page that says so.
            true => self.0.apps.screen(app, name).await?.unwrap_or_else(|| "<div id=\"missing\"></div><script>missing.innerHTML = one.notFound()</script>".into()),
            // A driver's screen, for now: its `_screen` operation gives the body, wherever the driver runs.
            false => {
                let input = json!({ "name": name, "params": params });
                let req = CallRequest { app: app.into(), driver: Some(app.into()), op: "_screen".into(), input, ..Default::default() };
                let body = self.route(req, viewer.clone()).await?;
                body.as_str().context("the screen isn't text")?.to_string()
            }
        };
        let me = self.me_for(&viewer)?;
        let made_here = self.with(|i| Ok(settings(&i.r.members).remove("made_here")))?;
        let config = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "made_here": made_here });
        let context = json!({ "app": app, "screen": name, "params": params, "me": me, "scope": scope, "config": config });
        // `</` can't appear inside the inline script, so it can't end it early.
        let context = context.to_string().replace("</", "<\\/");
        Ok(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
             <style>{kit}:root{{--accent:{accent}}}</style><script>window.__one={context};{bridge}</script></head><body>{body}</body></html>",
            kit = include_str!("screen/kit.css"),
            accent = crate::config::ACCENT,
            bridge = concat!(include_str!("screen/bridge.js"), include_str!("screen/kit.js"), include_str!("screen/code.js"), include_str!("screen/markdown.js")),
        ))
    }

    /// Every app in the install, with what each does for `viewer` where it's installed for them; and the drivers
    /// that serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer, false).await?;
        let mut drivers = Vec::new();
        if viewer.user == self.user()? {
            drivers.extend(self.with(|i| Ok(self.drivers_here(i)))?.into_iter().map(|d| d.2));
        }
        let remote = self.with(|i| {
            let mut d: Vec<Value> = i.sessions.values().flat_map(|s| s.drivers.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for mut describe in drivers.into_iter().chain(remote) {
            if out.iter().any(|a| a["name"] == describe["name"]) {
                continue;
            }
            // A driver serves its user, so its screens are on their own page.
            if describe["screens"].as_array().is_some_and(|s| !s.is_empty()) && describe["installed"].is_null() {
                describe["installed"] = json!([Scope::User(viewer.user.clone()).to_string()]);
            }
            if let (Value::Null, Some(domain), Some(name)) = (&describe["image"], describe["domain"].as_str(), describe["name"].as_str()) {
                if let Some((image, kind)) = self.logo(domain, name) {
                    (describe["image"], describe["image_kind"]) = (json!(image), json!(kind));
                }
            }
            out.push(describe);
        }
        Ok(out)
    }

    /// Every app in the install: owner, `from`, forks, deploys, the scopes `viewer` may use it's installed in, its
    /// title and icon, and for one built in, the bundle this release has of it; for those installed for them, what it
    /// does (operations, widgets, screens, instructions). With `not_added`, also the built-in apps the install hasn't
    /// added yet, with no deploys.
    pub async fn catalog(&self, viewer: &Caller, not_added: bool) -> Result<Vec<Value>> {
        let listed = self.with(|i| {
            let m = &i.r.members;
            Ok(m.apps
                .values()
                .filter(|a| !a.erased)
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
                    let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed });
                    if let Some(bundle) = self.built_in(a) {
                        info["built_in"] = json!(bundle);
                    }
                    // A deleted app leaves every page; its owners see it in Recently Deleted.
                    if let Some((time, by)) = &a.deleted {
                        info["installed"] = json!([]);
                        info["deleted"] = json!({ "time": time, "by": m.users.get(by).map(|u| &u.name), "yours": m.acts_for(&viewer.user, &a.owner) });
                        return (a.name.clone(), info, false);
                    }
                    (a.name.clone(), info, !reach.is_empty())
                })
                .collect::<Vec<_>>())
        })?;
        let missing: Vec<Value> = if not_added {
            self.with(|i| Ok(self.0.built_in.iter().filter(|(name, _)| !i.r.members.apps.contains_key(*name)).map(|(name, bundle)| json!({ "name": name, "owner": "admins", "from": null, "forks": [], "deploys": 0, "bundle": null, "history": [], "installed": [], "built_in": bundle })).collect()))?
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for (name, mut info, mine) in listed {
            if self.0.apps.has(&name) {
                if let Ok(Value::Object(d)) = self.0.apps.describe(&name).await {
                    // A fork's module may still give its first name; the install's name for it is the one that counts.
                    let declared = Schedule::all(&Value::Object(d.clone()));
                    for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && k != "schedules" && (mine || matches!(k.as_str(), "title" | "icon" | "color" | "domain" | "description" | "uses"))) {
                        info[k] = v;
                    }
                    if mine {
                        let (list, activity) = self.schedules_of(viewer, &name, &declared)?;
                        info["schedules"] = json!(list);
                        info["activity"] = json!(activity);
                    }
                    if let Some(insights) = self.insights_of(viewer, &name)? {
                        info["insights"] = insights;
                    }
                }
            }
            out.push(info);
        }
        out.extend(missing);
        for info in &mut out {
            if let Some(image) = info["bundle"].as_str().or(info["built_in"].as_str()).and_then(|b| self.image(b)) {
                info["image"] = json!(image);
            } else if let (Some(domain), Some(name)) = (info["domain"].as_str(), info["name"].as_str()) {
                if let Some((image, kind)) = self.logo(domain, name) {
                    (info["image"], info["image_kind"]) = (json!(image), json!(kind));
                }
            }
        }
        Ok(out)
    }

    /// The logo of `domain`, the site `app` connects to, as a data URL, once this server has fetched it. Fetches it
    /// in the background when it hasn't, or did a week ago, then tells shells `app` changed.
    fn logo(&self, domain: &str, app: &str) -> Option<(String, String)> {
        let (url, due) = self.0.logos.get(&self.0.dir, domain, now_ms());
        if due && self.0.logos.begin(domain) {
            let (node, domain, app) = (self.clone(), domain.to_string(), app.to_string());
            tokio::spawn(async move {
                let d = domain.clone();
                let logo = match tokio::task::spawn_blocking(move || crate::logos::fetch(&d)).await {
                    Ok(Ok(logo)) => Some(logo),
                    Ok(Err(err)) => {
                        tracing::warn!("{app}'s logo: {err:#}");
                        None
                    }
                    Err(_) => None,
                };
                if node.0.logos.finish(&node.0.dir, &domain, logo, now_ms()) {
                    node.emit(Event::Changed { apps: vec![app] });
                }
            });
        }
        url
    }

    /// The picture a bundle's app shows on its tile instead of its symbol: `icon.png` in its folder, as a data URL.
    /// Only from bundles this server has.
    fn image(&self, bundle: &str) -> Option<String> {
        if let Some(image) = self.0.images.lock().unwrap().get(bundle) {
            return image.clone();
        }
        let bytes = std::fs::read(release::blob_path(&self.0.dir, bundle).ok()?).ok()?;
        let files = crate::bundle::unpack(&bytes).ok()?;
        let png = files.into_iter().find(|(path, _)| path == "src/icon.png").map(|(_, png)| format!("data:image/png;base64,{}", data_encoding::BASE64.encode(&png)));
        self.0.images.lock().unwrap().insert(bundle.to_string(), png.clone());
        png
    }

    /// The bundle this release has of `app`, if it's the built-in app of that name: owned by admins, not a fork.
    fn built_in(&self, app: &members::App) -> Option<&String> {
        self.0.built_in.get(&app.name).filter(|_| app.owner == Principal::Admins && app.from.is_none())
    }

    /// Adds a built-in app the install doesn't have yet, owned by admins, as this release has it. Admins only.
    async fn bring_in(&self, caller: &Caller, app: &str) -> Result<()> {
        let Some(bundle) = self.0.built_in.get(app) else { return Ok(()) };
        let (present, admin) = self.with(|i| Ok((i.r.members.apps.contains_key(app), i.r.members.is_admin(&caller.user))))?;
        if present {
            return Ok(());
        }
        ensure!(admin, "{app} is built in, but this install doesn't have it yet: an admin adds it first");
        self.member_as(caller, "app", json!({ "name": app, "owner": "admins" })).await?;
        self.member_content(caller, deploy(app, bundle)).await
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
        self.uses_here(app).await
    }

    /// What the deploy of `app` this server runs says it uses: (apps, drivers).
    async fn uses_here(&self, app: &str) -> (Vec<String>, Vec<String>) {
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
            self.bring_in(caller, &wanted[i]).await?;
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

    /// Arranges a page for `caller` alone: the order of its apps and widgets, which widgets, their sizes and options,
    /// and the apps taken off it, kept in their own scope so it follows them to every device.
    pub async fn arrange(&self, caller: &Caller, page: &str, items: Value, hidden: Value) -> Result<()> {
        let scope = Scope::User(caller.user.clone());
        let content = self.with(|i| {
            ensure!(members::valid_page(page), "a page is user, group:<id> or global");
            ensure!(i.r.holds(&scope), "this server doesn't hold your data");
            Ok(members::node("arrange", json!({ "page": page, "items": items, "hidden": if hidden.is_null() { json!([]) } else { hidden } })))
        })?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(&scope, &caller.user, by, content).await?;
        Ok(())
    }

    /// How `user` arranged their pages, by page, as their own scope's `arrange` entries say.
    fn arrangements(&self, user: &str) -> BTreeMap<String, Value> {
        self.with(|i| Ok(i.views.get(&Scope::User(user.to_string()).to_string()).map(|(_, s)| s.arrangements.clone()).unwrap_or_default())).unwrap_or_default()
    }

    /// Switches one of an app's schedules on or off in `scope`, for `caller`; switching one on makes it run as them.
    pub async fn switch_schedule(&self, caller: &Caller, app: &str, schedule: &str, scope: &Scope, on: bool) -> Result<()> {
        let content = self.with(|i| switch_entry(i, &caller.user, scope, app, schedule, on))?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
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
    /// /blob/<hash>` gets one the caller may read; `GET /catalog` lists the install's apps, `GET /servers` its servers.
    pub async fn internal(&self, token: &str, method: &str, path: &str, body: Vec<u8>) -> Result<axum::response::Response> {
        use axum::response::IntoResponse;
        let grant = &self.grant_of(token).context("only a running call asks the node")?;
        let ok = |v: Value| axum::Json(v).into_response();
        Ok(match (method, path) {
            ("POST", "/call") => {
                ensure!(!grant.read_only, "this call only reads: it can't run other apps' operations");
                let req: Value = serde_json::from_slice(&body)?;
                let (app, op) = (req["app"].as_str().unwrap_or(""), req["op"].as_str().unwrap_or(""));
                // `ctx.drivers` says it's a driver's; `ctx.apps`, an app's (or, from older shims, either).
                let driver = req["driver"] == true;
                let (apps, drivers) = self.uses(&grant.app).await;
                let used = if driver { drivers.iter().any(|u| u == app) } else { apps.iter().chain(&drivers).any(|u| u == app) };
                ensure!(used, "{} doesn't use {app}", grant.app);
                ensure!(grant.depth < MAX_DEPTH, "{} called {app} {} calls deep, and apps call each other at most {MAX_DEPTH} deep: is it looping?", grant.app, grant.depth + 1);
                self.count(&grant.app, "call").await?;
                // An app may say which server runs it, like the one with a local model.
                let server = req["server"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                let call = CallRequest { app: app.into(), driver: driver.then(|| app.into()), op: op.into(), input: req["input"].clone(), server, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("GET", "/vault") => ok(json!(self.vault_of(&grant.app, &grant.scopes)?)),
            ("POST", "/vault/match") => {
                // The item a sign-in would change rather than make: one of that type whose plain field has that value.
                let req: Value = serde_json::from_slice(&body)?;
                let scope = match req["scope"].as_str() {
                    None | Some("user") => Scope::User(grant.caller.user.clone()),
                    Some(s) => Scope::parse(s)?,
                };
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let (kind, field, value) = (req["type"].as_str().unwrap_or(""), req["field"].as_str().unwrap_or(""), req["value"].as_str());
                let id = self.with(|i| {
                    Ok(i.views.get(&scope.to_string()).and_then(|(_, s)| {
                        s.items.iter().filter(|(_, it)| it.kind == kind && it.fields.get(field).is_some_and(|f| !f.secret && Some(f.value.as_str()) == value)).min_by_key(|(_, it)| it.made).map(|(id, _)| id.clone())
                    }))
                })?;
                ok(json!({ "id": id }))
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
                let built_in = self.0.built_in.values().any(|h| h == hash);
                // A call asked on this machine may read what its local API would give it: a file put there to attach.
                let local = grant.caller.key == self.0.id && release::blob_path(&self.0.dir, hash).is_ok_and(|p| p.exists());
                ensure!(built_in || local || grant.puts.iter().any(|h| h == hash) || self.blob_named_for(hash, &grant.caller.user)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            ("POST", "/appends") => {
                ensure!(self.0.later.lock().unwrap().contains_key(token), "only work a call left running after it returned hands in appends later");
                let req: Value = serde_json::from_slice(&body)?;
                let appends = crate::apps::appends(&grant.app, &req["appends"])?;
                self.write_appends(&grant.app, appends, &grant.scopes, &grant.caller, grant.from.as_deref()).await?;
                ok(json!({}))
            }
            ("POST", "/done") => {
                let req: Value = serde_json::from_slice(&body).unwrap_or_default();
                if let Some(err) = req["error"].as_str() {
                    tracing::warn!("{}'s work after a call failed: {err}", grant.app);
                }
                if let Some(done) = self.0.later.lock().unwrap().remove(token) {
                    done.notify_one();
                }
                ok(json!({}))
            }
            ("GET", "/catalog") => ok(json!(self.catalog(&grant.caller, true).await?)),
            ("GET", "/servers") => ok(self.state_for(&grant.caller)["install"]["servers"].take()),
            _ => bail!("the node has no {method} {path}"),
        })
    }

    /// Sets one of the install's settings, for an admin; `None` puts back the edition's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_as(caller, "setting", json!({ "name": name, "value": value })).await
    }

    /// Everything a shell needs to draw this server and its install, for this server's local user.
    pub fn state(&self) -> Value {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller { key: self.0.id, user, agent: None })
    }

    /// The state as `viewer`, this server or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Value {
        let ready = self.ready();
        let waiting = self.item_requests(viewer, None, None).map(|r| r.iter().filter(|r| r["state"] == "waiting").count()).unwrap_or(0);
        let arrangements = self.arrangements(&viewer.user);
        let install = self.0.install.lock().unwrap();
        let viewer_name = if viewer.key == self.0.id { self.server_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let user = install.as_ref().and_then(|i| i.r.members.user(&viewer.user)).map(|u| json!({ "id": u.id, "name": u.name, "role": u.role }));
        let settings = settings(install.as_ref().map(|i| &i.r.members).unwrap_or(&Members::default()));
        let install = install.as_ref().map(|i| {
            let m = &i.r.members;
            let me = &viewer.user;
            let list = |ps: Vec<Principal>| ps.iter().map(|p| p.to_string()).collect::<Vec<_>>();
            let here_drivers = self.drivers_here(i);
            let servers: Vec<Value> = m
                .servers()
                .map(|s| {
                    let session = i.sessions.get(&s.key);
                    // The containers on a server are its owners' business.
                    let load = if s.key == self.0.id { Some(crate::load::current()) } else { session.and_then(|s| s.load.clone()) }.map(|mut l| {
                        if !m.owners(&s.key).iter().any(|o| m.acts_for(me, o)) {
                            l.containers.clear();
                        }
                        l
                    });
                    json!({
                        "key": hex(&s.key), "name": s.name, "user": s.user, "me": s.key == viewer.key, "here": s.key == self.0.id,
                        "online": s.key == self.0.id || session.is_some(),
                        "owners": list(m.owners(&s.key)), "users": list(m.users_of(&s.key)), "privileged": m.privileged(&s.key),
                        "trusted_by": m.users.keys().filter(|u| m.trusts(u, &s.key)).collect::<Vec<_>>(),
                        "serves_me": m.serves(&s.key, me),
                        "build": i.builds.get(&s.key).map(|r| &r.commit), "target": i.builds.get(&s.key).map(|r| &r.target),
                        "drivers": if s.key == self.0.id { here_drivers.iter().map(|d| d.0.clone()).collect() } else { session.map(|s| s.drivers.iter().map(|d| d.0.clone()).collect::<Vec<_>>()).unwrap_or_default() },
                        "installed_drivers": installed_drivers(m, s, if s.key == self.0.id { &here_drivers } else { session.map(|s| s.drivers.as_slice()).unwrap_or_default() }, i.builds.get(&s.key).map(|r| r.commit.as_str())),
                        "removed_drivers": m.drivers.iter().filter(|((k, _), d)| *k == s.key && d.removed()).map(|((_, name), d)| json!({ "name": name, "by": m.users.get(&d.by).map(|u| &u.name), "time": d.time })).collect::<Vec<_>>(),
                        "load": load,
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
                .map(|(_, s)| s.opens.iter().map(|(by, server, time, agent)| json!({ "by": by, "by_name": m.users.get(by).map(|u| &u.name), "agent": agent, "server": hex(server), "time": time, "until": time + members::OPEN_FOR })).collect())
                .unwrap_or_default();
            json!({
                "name": m.name, "root": i.r.root.map(|r| r.to_hex()), "user": me, "role": m.user(me).map(|u| u.role),
                "member": m.server(&self.0.id).is_some(),
                "users": users, "groups": groups, "servers": servers, "clients": clients, "opens": opens,
                "entries": i.r.store.count().unwrap_or(0), "settings": settings,
            })
        });
        let opens = install.as_ref().map(|i| i["opens"].clone()).unwrap_or(json!([]));
        let config = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "accent": crate::config::ACCENT, "store": settings["store"], "push": settings["push"] });
        let mut state = json!({
            // `brand` is the same, for shells built before it was `config`.
            "config": config, "brand": config,
            "me": { "key": hex(&viewer.key), "name": viewer_name, "user": viewer.user },
            "server": { "key": hex(&self.0.id), "name": self.server_name(), "ready": ready },
            "user": user,
            "install": install,
            "opens": opens,
            // Requests for an item waiting for this user to answer (see GET /v1/vault/requests); and for shells
            // built before the Vault.
            "vault_requests": waiting,
            "secret_requests": waiting,
            // How this user arranged each of their pages (see `arrange`).
            "arrangements": arrangements,
        });
        if viewer.key != self.0.id {
            state["client"] = json!({ "key": hex(&viewer.key), "name": viewer_name });
        }
        state
    }

    // Members.

    /// Writes one of `global`'s node entries as the local user, after checking it would count.
    pub async fn member(&self, kind: &str, data: Value) -> Result<()> {
        self.member_as(&Caller { key: self.0.id, user: self.user()?, agent: None }, kind, data).await
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
    pub async fn open(&self, user: &str, server: &str, agent: Option<String>) -> Result<Value> {
        let req = CallRequest { app: members::APP.into(), op: "open".into(), input: json!({ "user": user }), server: Some(server.into()), agent, ..Default::default() };
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
            let me = Caller { key: caller, user, agent: None };
            tokio::spawn(async move {
                let result = async {
                    let request: Value = serde_json::from_slice(&recv.read_to_end(MESSAGE_LIMIT).await?)?;
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
            ("GET", "/v1/badges") => self.badges(me.clone(), body["scope"].as_str()).await?,
            ("GET", "/v1/screen") => {
                let (app, name) = (body["app"].as_str().unwrap_or(""), body["screen"].as_str().unwrap_or("main"));
                json!({ "html": self.screen(me.clone(), app, name, body["params"].clone(), body["scope"].as_str()).await? })
            }
            ("POST", "/v1/call") => {
                let mut req = self.resolve(serde_json::from_value(body)?);
                (req.from, req.depth, req.client) = (None, 0, Some("phone".into()));
                let me = &Caller { agent: req.agent.clone(), ..me.clone() };
                // An app's call runs here or the client tries another server; a driver's goes where the driver is.
                match req.driver.is_none() {
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
            ("POST", "/v1/retire") => {
                self.retire(me, body["app"].as_str().unwrap_or(""), body["how"].as_str().unwrap_or("")).await?;
                json!({})
            }
            ("POST", "/v1/arrange") => {
                self.arrange(me, body["page"].as_str().unwrap_or(""), body["items"].clone(), body["hidden"].clone()).await?;
                json!({})
            }
            ("POST", "/v1/schedule") => {
                let scope = self.scope_for(me, body["scope"].as_str().unwrap_or("user"))?;
                self.switch_schedule(me, body["app"].as_str().unwrap_or(""), body["schedule"].as_str().unwrap_or(""), &scope, body["on"] != false).await?;
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
            ("POST", "/v1/settings") => {
                self.set_setting(me, body["name"].as_str().unwrap_or(""), body["value"].as_str()).await?;
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
            (m, p) if p.starts_with("/v1/vault") || p.starts_with("/v1/secrets") => self.vault_route(me, m, p, &body).await?.with_context(|| format!("no {m} {p}"))?,
            ("POST", "/v1/name") => {
                let name = body["name"].as_str().unwrap_or("").trim();
                self.member_as(me, "rename", json!({ "key": hex(&me.key), "name": name })).await?;
                json!({})
            }
            ("POST", "/v1/pair") => {
                let (code, user) = self.pair_as(me, new_user(&body)?, body["user"].as_str())?;
                json!({ "code": code, "user": user })
            }
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
        Ok(self.pair_local(new_user, None)?.0)
    }

    /// Like `pair`, or for an admin, a code for another machine of `user` (an id or a name). With the id of the user
    /// the code admits.
    pub fn pair_local(&self, new_user: Option<(String, Role)>, user: Option<&str>) -> Result<(String, String)> {
        self.pair_as(&Caller { key: self.0.id, user: self.user()?, agent: None }, new_user, user)
    }

    fn pair_as(&self, asker: &Caller, new_user: Option<(String, Role)>, user: Option<&str>) -> Result<(String, String)> {
        let user = user.map(|u| self.find_user(u)).transpose()?;
        self.with(|i| {
            let m = &i.r.members;
            ensure!(m.server(&self.0.id).is_some(), "this server isn't in the install yet");
            ensure!(new_user.is_none() || m.is_admin(&asker.user), "only admins add users; pair one of your own machines instead");
            ensure!(new_user.is_none() || user.is_none(), "a code is for a new user or an existing one, not both");
            ensure!(user.as_ref().is_none_or(|u| *u == asker.user) || m.is_admin(&asker.user), "only admins pair other people's machines");
            ensure!(m.serves(&self.0.id, &asker.user), "{} doesn't serve you", self.server_name());
            Ok(())
        })?;
        let admits = user.or_else(|| new_user.as_ref().map(|_| new_id()));
        let joins_as = admits.clone().unwrap_or_else(|| asker.user.clone());
        let secret: [u8; 16] = rand::random();
        let by = (asker.key != self.0.id).then(|| hex(&asker.key));
        self.add_pairing(secret, Pairing { user: asker.user.clone(), by, new_user, admits, until: now_ms() + PAIRING_LIFE })?;
        let code = PairingCode { from: self.0.endpoint.addr(), secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&code, &mut bytes)?;
        Ok((data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase(), joins_as))
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
        let user = match (&pairing.admits, &pairing.new_user) {
            (Some(user), _) => user.clone(),
            (None, Some(_)) => new_id(),
            (None, None) => pairing.user.clone(),
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
        self.member_as(&Caller { key: by.unwrap_or(self.0.id), user: pairing.user.clone(), agent: None }, "admit", admit.data).await.context("this server can no longer admit that machine")?;
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
        let drivers = self.drivers_here(i);
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
            i.sessions.insert(peer, Session { tx: tx.clone(), frontier: None, drivers: Vec::new(), load: None, caught_up: false });
            Ok(())
        })?;
        self.emit(Event::Servers);
        tracing::info!("syncing with {}", self.name_of(&peer));
        let writer = async {
            while let Some(mut msg) = rx.recv().await {
                // Hellos made inside sync carry no drivers or load; every hello this server sends does.
                if let Msg::Hello { drivers, load, .. } = &mut msg {
                    if drivers.is_empty() {
                        if let Ok(Msg::Hello { drivers: d, .. }) = self.with(|i| self.hello(i)) {
                            *drivers = d;
                        }
                    }
                    *load = Some(crate::load::current());
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
            let Msg::Hello { frontier, drivers, load } = &msg else { return Ok(false) };
            let Some(s) = i.sessions.get_mut(&peer) else { return Ok(false) };
            s.frontier = Some(frontier.clone());
            s.load = load.clone();
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
        let request: Value = serde_json::from_slice(&recv.read_to_end(MESSAGE_LIMIT).await?)?;
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
        let agent = request["me"]["agent"].as_str().map(str::to_string);
        Ok(Caller { key, user, agent })
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
    // An erase takes the app's secrets from every scope's view, as replaying the scope again would.
    for e in new.iter().filter(|e| members::is_member_entry(e)) {
        for (_, state) in i.views.values_mut() {
            members::forget_erased(&i.r.members, state, e);
        }
    }
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
        user_name: m.users.get(e.user()).map(|u| u.name.clone()).unwrap_or_default(),
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
        let open = i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.opens.iter().any(|(by, server, t, _)| by == &me.user && *server == here && now_ms() < t + members::OPEN_FOR));
        ensure!(open, "that user's data isn't open for you on {}", node.server_name());
        let caller = Caller { key: me.key, user: target.clone(), agent: me.agent.clone() };
        let ctx = Context { me: node_me(node, m, &caller), now: now_ms(), scopes: handles(m, std::slice::from_ref(&scope), target), read_only: true, token: String::new(), from: None };
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
    let ctx = Context { me: node_me(node, m, me), now: now_ms(), scopes: handles(m, &scopes, &me.user), read_only: false, token: String::new(), from: None };
    Ok(Ok((ctx, scopes)))
}

fn node_me(node: &Node, m: &Members, caller: &Caller) -> Me {
    let name = if caller.key == node.0.id { node.server_name() } else { m.machine(&caller.key).map(|k| k.name.clone()).unwrap_or_default() };
    let user = m.user(&caller.user);
    Me {
        key: hex(&caller.key),
        user: caller.user.clone(),
        name,
        role: user.map(|u| u.role),
        user_name: user.map(|u| u.name.clone()).unwrap_or_default(),
        agent: caller.agent.clone(),
    }
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
        Append::Switch { app, schedule, scope, on } => {
            let scope = if scope == "user" { Scope::User(me.user.clone()) } else { Scope::parse(&scope)? };
            (scope.clone(), switch_entry(i, &me.user, &scope, &app, &schedule, on)?)
        }
        Append::Forget { scope, blob } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blob.len() == 64 && blob.bytes().all(|b| b.is_ascii_hexdigit()), "a blob is named by its hash");
            (scope, members::node("forget", json!({ "blob": blob.to_lowercase() })))
        }
        Append::Retire { app: name, how } => {
            let m = &i.r.members;
            let a = m.apps.get(&name).with_context(|| format!("no app named {name}"))?;
            ensure!(m.acts_for(&me.user, &a.owner), "only someone acting for {name}'s owner deletes, restores or erases it");
            let mut data = json!({ "name": name, "owner": a.owner.to_string() });
            match how.as_str() {
                "deleted" => data["deleted"] = json!(true),
                "restored" => data["deleted"] = json!(false),
                "erased" if a.deleted.is_some() => data["erased"] = json!(true),
                "erased" => bail!("delete {name} before erasing it"),
                other => bail!("an app is deleted, restored or erased, not {other}"),
            }
            (Scope::Global, members::node("app", data))
        }
        Append::Driver { server, name, installed } => (Scope::Global, driver_entry(&i.r.members, &me.user, &server, &name, installed)?),
        Append::Vault { id, scope, data, same, release } => vault_entry(i, app, me, scopes, &id, scope, data, same, release)?,
        Append::Install { .. } | Append::Cost { .. } => bail!("installs and costs are written on their own"),
    })
}

/// What a server says of each driver serving there, as `GET /v1/state` lists it: what it is, its version (a
/// driver's own, or for one built into the node, the node's build), whom it serves, and who installed it.
fn installed_drivers(m: &Members, server: &members::Machine, drivers: &[Driver], build: Option<&str>) -> Vec<Value> {
    let name_of = |u: &String| m.users.get(u).map(|u| u.name.clone());
    drivers
        .iter()
        .map(|(name, users, d)| {
            let built_in = d["built_in"] == true;
            let installed = m.drivers.get(&(server.key, name.clone())).filter(|i| i.local);
            // Registered over the local API, a driver counts as installed by the server's local user.
            let by = match installed {
                Some(i) => name_of(&i.by),
                None if built_in => None,
                None => name_of(&server.user),
            };
            json!({
                "name": name, "title": d["title"].as_str().unwrap_or(name), "icon": d["icon"], "color": d["color"], "domain": d["domain"], "image": d["image"], "image_kind": d["image_kind"], "local_app": d["local_app"],
                "description": d["description"], "version": d["version"].as_str().or(built_in.then_some(build).flatten()),
                "built_in": built_in, "installed_by": by, "installed": installed.map(|i| i.time), "users": users.iter().filter_map(name_of).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// The `driver` entry that removes a driver from a server, found by name or key prefix, or with `installed` lets it
/// register there again; for someone acting for one of the server's owners.
fn driver_entry(m: &Members, user: &str, server: &str, name: &str, installed: bool) -> Result<Content> {
    let found: Vec<&members::Machine> = m.servers().filter(|s| s.name.eq_ignore_ascii_case(server) || (!server.is_empty() && hex(&s.key).starts_with(server))).collect();
    let [s] = found.as_slice() else { bail!("no one server called {server}") };
    ensure!(m.owners(&s.key).iter().any(|o| m.acts_for(user, o)), "only someone acting for one of {}'s owners installs or removes its drivers", s.name);
    Ok(members::node("driver", json!({ "server": hex(&s.key), "name": name, "blob": null, "local": installed })))
}

/// Whom a schedule of `app` in a scope runs as: whoever switched it on last, or else installed the app there, while
/// they still manage the scope; nobody if it's switched off.
fn runs_as(m: &Members, state: &ScopeState, scope: &Scope, app: &str, schedule: &str) -> Option<String> {
    let manages = |u: &String| members::manages_scope(m, u, scope);
    match state.switches.get(&(app.to_string(), schedule.to_string())) {
        Some((false, ..)) => None,
        Some((true, user, _)) if manages(user) => Some(user.clone()),
        _ => state.installers.get(app).map(|(u, _)| u.clone()).filter(manages),
    }
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
fn switch_entry(i: &Install, user: &str, scope: &Scope, app: &str, schedule: &str, on: bool) -> Result<Content> {
    ensure!(members::manages_scope(&i.r.members, user, scope), "you can't switch schedules in {scope}");
    ensure!(i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.installed.contains(app)), "{app} isn't installed in {scope}");
    ensure!(i.r.holds(scope), "this server doesn't hold {scope}");
    Ok(members::node("schedule", json!({ "app": app, "schedule": schedule, "on": on })))
}

/// An earlier app's `secret(name, value, hosts)`: the `secret` entry `user` may write in `scope`, the item of its
/// own that `app` keeps.
fn secret_entry(i: &Install, user: &str, scope: &Scope, app: &str, name: &str, hosts: Vec<String>, value: Option<String>) -> Result<Content> {
    let m = &i.r.members;
    ensure!(members::manages_scope(m, user, scope), "you can't change the Vault in {scope}");
    ensure!(m.apps.get(app).is_some_and(|a| !a.erased), "no app named {app}");
    ensure!(vault::valid_field(name), "a field's name is letters, digits, _, - and .");
    ensure!(value.is_none() || (!hosts.is_empty() && hosts.iter().all(|h| members::valid_host(h))), "give the hosts it's sent to, as host names (api.example.com)");
    ensure!(i.r.holds(scope), "this server doesn't hold {scope}");
    Ok(members::node("secret", json!({ "app": app, "name": name, "hosts": hosts, "value": value })))
}

/// An `item` entry `user` may write in `scope`, checked as replaying it will check it.
fn item_entry(i: &Install, user: &str, scope: &Scope, data: Value) -> Result<Content> {
    let m = &i.r.members;
    ensure!(members::manages_scope(m, user, scope), "you can't change the Vault in {scope}");
    ensure!(i.r.holds(scope), "this server doesn't hold {scope}");
    let mut items = i.views.get(&scope.to_string()).map(|(_, s)| s.items.clone()).unwrap_or_default();
    vault::apply(&mut items, &data, user, now_ms(), |a| !m.apps.get(a).is_some_and(|a| a.erased))?;
    Ok(members::node("item", data))
}

/// A call's change to the Vault, by `app` for its caller: an item it makes (switched on for it, or with `same`, an
/// item of that type whose field matches, all its secret fields given anew), one it's switched on for that it
/// changes (its hosts only with every secret field given anew), or one it lets go of (`release`), deleted once no
/// app uses it.
fn vault_entry(i: &Install, app: &str, me: &Caller, scopes: &[Scope], id: &str, scope: Option<String>, mut data: Value, same: Option<String>, release: bool) -> Result<(Scope, Content)> {
    ensure!(crate::entry::is_id(id), "an item's id is 16 hex characters");
    let fields_given = |it: &vault::Item, data: &Value| it.fields.iter().filter(|(_, f)| f.secret).all(|(n, _)| data["fields"][n]["value"].is_string());
    let obj = data.as_object_mut().context("an item is an object")?;
    for key in ["id", "app", "apps", "deleted", "request"] {
        obj.remove(key);
    }
    let held = |s: &Scope| i.views.get(&s.to_string()).and_then(|(_, st)| st.items.get(id));
    let found = scopes.iter().find_map(|s| held(s).map(|it| (s.clone(), it)));
    let (scope, data) = match found {
        Some((scope, it)) if release => {
            ensure!(it.apps.contains(app), "{app} isn't switched on for that item");
            let mut apps = it.apps.clone();
            apps.remove(app);
            (scope, if apps.is_empty() { json!({ "id": id, "deleted": true }) } else { json!({ "id": id, "apps": apps }) })
        }
        Some((scope, it)) => {
            let matched = same.as_deref().is_some_and(|f| it.kind == data["type"].as_str().unwrap_or(&it.kind) && it.fields.get(f).is_some_and(|x| !x.secret && data["fields"][f]["value"].as_str() == Some(x.value.as_str())));
            ensure!(it.apps.contains(app) || (matched && fields_given(it, &data)), "{app} isn't switched on for that item");
            ensure!(data["hosts"].is_null() || fields_given(it, &data), "an app changes an item's hosts only with every secret field given anew");
            let mut apps = it.apps.clone();
            apps.insert(app.to_string());
            data["id"] = json!(id);
            data["apps"] = json!(apps);
            (scope, data)
        }
        None => {
            ensure!(!release, "no such item");
            let scope = match scope.as_deref() {
                None | Some("user") => Scope::User(me.user.clone()),
                Some(s) => Scope::parse(s)?,
            };
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            data["id"] = json!(id);
            data["apps"] = json!([app]);
            data["app"] = json!(app);
            (scope, data)
        }
    };
    Ok((scope.clone(), item_entry(i, &me.user, &scope, data)?))
}

/// A deploy of `app`: the bundle at blob `bundle`.
/// A built-in app's bundle: its `app.js` as the module, and every file of its folder as its source. One with npm
/// packages (a `package.json`) is built as `<cli> deploy` builds any app.
fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.iter().any(|(p, _)| *p == "package.json") {
        return crate::deploy::build_files(files);
    }
    let module = files.iter().find(|(p, _)| *p == "app.js").context("a built-in app has no app.js")?.1;
    let mut bundle = vec![("app.js".to_string(), module.to_vec())];
    bundle.extend(files.iter().map(|(p, b)| (format!("src/{p}"), b.to_vec())));
    crate::bundle::pack(&bundle)
}

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
        release::verify(crate::config::RELEASE_KEY, &bytes, signature).context("no node would take it")?;
        let blob = self.put_blob(&bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into() };
        self.append(&Scope::Global, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node if a newer build for its platform was published, signed with the release key.
    fn consider_update(&self) {
        let latest = self.with(|i| Ok(release::latest(crate::config::RELEASE_KEY, &i.r.store.scope("global")?, crate::config::TARGET)));
        let Ok(Some((r, author))) = latest else { return };
        let marker = self.0.dir.join("release");
        if !release::wanted(&r, crate::config::BUILD, crate::config::COMMITTED) || std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == r.blob) {
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
        release::verify(crate::config::RELEASE_KEY, &bytes, &r.signature)?;
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

    /// Whether an entry in a scope `holds` says yes to names the blob, and that scope still needs it: an app's entry's
    /// blob stops counting once its scope forgets it; a node entry's (a deploy's bundle, a release's binary) never does.
    fn blob_named(&self, hash: &str, holds: impl Fn(&Scope) -> bool) -> Result<bool> {
        self.with(|i| {
            for log in i.r.store.logs()? {
                let forgotten = i.views.get(&log).is_some_and(|(_, s)| s.forgotten.contains(hash));
                let names = |e: &Entry| e.content.blobs.iter().any(|b| b == hash) && (!forgotten || e.content.app == members::APP);
                if Scope::parse(&log).is_ok_and(|s| holds(&s)) && i.r.store.scope(&log)?.iter().any(names) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    /// Deletes the blobs here that scopes forgot, once no entry this server holds still needs them.
    fn delete_forgotten(&self, hashes: Vec<String>) {
        for hash in hashes {
            if self.0.built_in.values().any(|b| *b == hash) || self.blob_named(&hash, |_| true).unwrap_or(true) {
                continue;
            }
            if let Ok(path) = release::blob_path(&self.0.dir, &hash) {
                if std::fs::remove_file(&path).is_ok() {
                    tracing::info!("deleted blob {} that nothing needs any more", &hash[..12]);
                }
            }
        }
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

/// The install's settings: the edition's defaults (config.toml's `[settings]`), and what its admins set over them.
fn settings(m: &Members) -> serde_json::Map<String, Value> {
    let mut out: serde_json::Map<String, Value> = crate::config::SETTINGS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    out.extend(m.settings.iter().map(|(k, v)| (k.clone(), json!(v))));
    for key in ["store", "push"] {
        out.entry(key).or_insert(json!(""));
    }
    out
}
