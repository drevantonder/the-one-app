//! A node: this server's install (a white label holds one), its replica and views, its iroh endpoint, and where calls run.

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
use tokio::sync::broadcast;

use crate::apps::{Apps, Context, Me, ScopeHandle};
use crate::call::Append;
use crate::backups::{KEY_OBJECT, KEYS};
use crate::bucket::{Copied, Place};
use crate::db::Db;
use crate::entry::{Content, Entry, Key, Order, Scope, hex, new_id, short, unhex};
use crate::members::{self, Members, Principal, Role, ScopeState};
use crate::rank;
use crate::release::{self, Release};
use crate::schedules::{self, Schedule};
use crate::store::Store;
use crate::replica::{Handled, Replica};

mod apps;
mod backups;
mod blobs;
mod calls;
mod databases;
mod logs;
mod pairing;
mod releases;
mod scheduled;
mod screens;
mod sessions;
mod sidecars;
mod state;
mod status;
mod storage;
mod tunnel;
mod vault;

const JOIN_ALPN: &[u8] = b"one/join/1";
const CALL_ALPN: &[u8] = b"one/call/1";
const BLOB_ALPN: &[u8] = b"one/blob/1";
/// How long a call may run before it's stopped.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A sidecar's call may take longer, as a browser or a desktop starts: as long as a sidecar is waited for.
const SIDECAR_LIMIT: Duration = Duration::from_secs(600);
/// The most a call or a client's request (or its answer) may carry: an email of a claim's receipts, base64, fits.
const MESSAGE_LIMIT: usize = 64 << 20;
/// How many app-to-app calls deep a call may be: a person's call of A, A's of B, B's of C, C's of D.
pub const MAX_DEPTH: u32 = 3;
/// How many calls an app may make of other apps, or runs of its schedules, a minute on one server.
pub const RATE_LIMIT: u32 = 600;
/// How long an app that looped stays stopped on a server, unless it's deployed again sooner.
/// A server whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

/// Whether a server has no room for more: its memory, or its CPU of late, over 90% used.
fn full(load: &crate::load::Load) -> bool {
    let busy = load.cpu.iter().rev().take(3).map(|c| *c as u32).sum::<u32>() / 3;
    load.memory.is_some_and(|m| m.total > 0 && m.used * 10 > m.total * 9) || busy > 90
}

const STOPPED_FOR: u64 = 15 * 60_000;
/// How often a server reads what other servers wrote and said.
const TICK: Duration = Duration::from_secs(10);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this server's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// The largest step a ping carries: a bigger one, the servers it tells read from the bucket.
const PINGED_STEP: usize = 256 << 10;
/// How long a call's work may carry on after it returns (`ctx.waitUntil`).
const LATER_LIMIT: Duration = Duration::from_secs(600);
/// How many of an app's failures a server records an hour; the rest are only counted on the next hour's.
const FAILURES_KEPT: u32 = 30;
/// How long a request for an item waits for its answer.
const REQUEST_LIFE: u64 = 86_400_000;
/// How long an app's page says a deploy changed or removed one of its schedules.
const CHANGED_SHOWN: u64 = 14 * 86_400_000;
/// The object in an install's bucket that says whose it is: the install's genesis.
const INSTALL_OBJECT: &str = "install";
/// How often a server says hello to the others: which sidecars serve there, and how loaded it is.
const PRESENCE_EVERY: Duration = Duration::from_secs(15);
/// What an app's own database is called: SQLite's name for a connection's first.
const DATABASE: &str = "main";
/// What a built-in sidecar's database of its records is called, under its name.
const SIDECAR_DATABASE: &str = "sidecar";

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
    /// The install's bucket, as a place's fields: a folder's `path`, or S3's. None if this server can't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
}

/// A sidecar serving on a server: its name, the users it serves, and what it does (its operations), which callers
/// list until apps wrap sidecars.
pub type Sidecar = (String, Vec<String>, Value);

/// Another server, as it last said hello: the sidecars serving there, and how loaded it is.
struct Peer {
    sidecars: Vec<Sidecar>,
    load: Option<crate::load::Load>,
    seen: Instant,
}

struct Install {
    r: Replica,
    user: String,
    /// The install's bucket, and its fields, which a server joining is given.
    bucket: Option<(Place, BTreeMap<String, String>)>,
    /// Each other server, as it last said hello.
    peers: HashMap<Key, Peer>,
    /// The build each server last said it runs.
    builds: HashMap<Key, release::Running>,
    /// Each scope's rows of how things went, as this server last read them.
    status: HashMap<String, status::Status>,
    /// Each scope's views: how far they've replayed, and what the scope's node entries add up to.
    views: HashMap<String, (Order, ScopeState)>,
    /// How long each server took to answer a call lately.
    nearest: HashMap<Key, Duration>,
    /// How far each server is: its connection's round trip, when this server last asked it something.
    far: HashMap<Key, Duration>,
    /// Whether this server has read every scope it holds from the bucket since it woke.
    read: bool,
}

impl Install {
    /// Whether another server said hello lately.
    fn online(&self, server: &Key) -> bool {
        self.peers.get(server).is_some_and(|p| p.seen.elapsed() < 3 * PRESENCE_EVERY)
    }
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
    /// The user the machine joins as, when it isn't the asker's: an admin's code for someone else's machine.
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
    /// For a server: the install's bucket, as a place's fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
}

/// Who asked for something: this server or another, or a client, as one user.
#[derive(Clone, Debug)]
pub struct Caller {
    pub key: Key,
    pub user: String,
    /// The agent that asked through it, by name ("Claude Code"), when one did.
    pub agent: Option<String>,
    /// The kernel access token it came with, when it's a sign-in's (or a run the schedules service handed in): a call
    /// gets its keys and bucket URLs from the vault with it, and passes it on with the call to another server.
    pub token: Option<String>,
}

/// An operation to run: of an app, or of a sidecar.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallRequest {
    /// Names this call wherever it's tried, so a call that writes runs on one server only: the first to take its lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The app, or for a sidecar's call, the sidecar (older machines name a sidecar here).
    #[serde(default)]
    pub app: String,
    /// The sidecar whose operation this is, when it is one: an app and the sidecar it wraps may share a name.
    #[serde(default)]
    pub sidecar: Option<String>,
    /// What `sidecar` was called first, which machines built before send; a call takes it as `sidecar`.
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
    built_in_sidecars: Vec<crate::sidecars::BuiltIn>,
    pub sidecars: crate::sidecars::Sidecars,
    install: Mutex<Option<Install>>,
    pairings: Mutex<HashMap<[u8; 16], Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this server.
    changed: tokio::sync::Notify,
    /// Wakes the task that updates this node to a newer release.
    released: tokio::sync::Notify,
    /// The bundle each app runs here.
    bundles: Mutex<BTreeMap<String, String>>,
    /// The bundle of each app this release has built in, kept as a blob: any install may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png` as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The logos of the sites apps and sidecars connect to.
    logos: crate::logos::Logos,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The blobs sign-ins put here (their files in an MCP answer, say), by hash: for whom, so they may read them back.
    put_by: Mutex<HashMap<String, std::collections::HashSet<String>>>,
    /// The port apps' requests go out through.
    outbound: u16,
    /// What each app did in its last minute, and the apps stopped here for looping.
    guard: Mutex<Guard>,
    /// The calls that returned with work still running (`ctx.waitUntil`), by token: told when it's done.
    later: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Who used which app today from which client, as recorded here: (app, user, client, day).
    seen: Mutex<std::collections::HashSet<(String, String, String, u64)>>,
    /// How many failures of each app were recorded here each hour.
    failures: Mutex<HashMap<(String, u64), u32>>,
    /// Wakes the backups to run now, and whether they run whether or not an hour passed.
    backup_now: tokio::sync::Notify,
    backup_forced: Mutex<bool>,
    /// The newest snapshot this server knows of each (place, scope), with its key's id.
    backed: Mutex<HashMap<(String, String), (crate::backups::Snapshot, String)>>,
    /// When each place's last run here failed, so it's tried again an hour later, not every minute.
    backup_failed: Mutex<HashMap<String, u64>>,
    /// When this server last pruned each place, by its id.
    pruned: Mutex<HashMap<String, u64>>,
    /// When this server last copied each (place, scope)'s databases from the bucket.
    backup_looked: Mutex<HashMap<(String, String), u64>>,
    /// Each database of each scope, by its app and name (`board/main`), as this server last read it.
    dbs: Mutex<HashMap<(Scope, String), Arc<tokio::sync::Mutex<Db>>>>,
    /// The databases another server said it wrote without sending the step, which a read catches up from the bucket.
    behind: Mutex<std::collections::HashSet<(Scope, String)>>,
    /// Each scope's key, once this server has it.
    scope_keys: Mutex<HashMap<Scope, [u8; 32]>>,
    /// Whether this server is moving to the install's new storage now.
    moving: Mutex<bool>,
    /// Where the install's data went: every bucket this server has used writes there, after a move (`Copied`).
    moved: Arc<tokio::sync::RwLock<Option<Place>>>,
    /// This server's connector for the install's tunnel, with the token it runs with.
    tunnel: Mutex<Option<(String, tokio::task::JoinHandle<()>)>>,
    /// Held while this server makes or finds a scope's key.
    making_keys: tokio::sync::Mutex<()>,
    /// The last position this server wrote an entry of each scope at.
    written: Mutex<HashMap<Scope, u64>>,
    /// The step of each scope's entries this server last moved their newest hint on to, by scope.
    hinted: Mutex<HashMap<String, u64>>,
    /// The bundles this server has put in the bucket, or found there.
    shared: Mutex<std::collections::HashSet<String>>,
    /// What the vault gave this server for the sign-ins its calls came with.
    sessions: sessions::Sessions,
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
    /// The scopes whose databases it has brought up to date, which it reads as they are from then on.
    pub caught_up: Vec<Scope>,
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
    /// Starts this server's node, with the built-in sidecars this build has.
    pub async fn start(dir: &Path, built_in_sidecars: Vec<crate::sidecars::BuiltIn>) -> Result<(Node, Router)> {
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
            built_in_sidecars,
            sidecars: Default::default(),
            install: Mutex::new(None),
            pairings: Mutex::new(load_pairings(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
            bundles: Mutex::new(BTreeMap::new()),
            built_in,
            images: Mutex::new(HashMap::new()),
            logos: crate::logos::Logos::open(dir),
            grants: Mutex::new(HashMap::new()),
            put_by: Mutex::new(HashMap::new()),
            outbound: outbound.local_addr()?.port(),
            guard: Mutex::new(Guard::default()),
            later: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            failures: Mutex::new(HashMap::new()),
            backup_now: tokio::sync::Notify::new(),
            backup_forced: Mutex::new(false),
            backed: Mutex::new(HashMap::new()),
            backup_failed: Mutex::new(HashMap::new()),
            pruned: Mutex::new(HashMap::new()),
            backup_looked: Mutex::new(HashMap::new()),
            dbs: Mutex::new(HashMap::new()),
            behind: Mutex::new(Default::default()),
            scope_keys: Mutex::new(HashMap::new()),
            moving: Mutex::new(false),
            moved: Arc::default(),
            tunnel: Mutex::new(None),
            making_keys: tokio::sync::Mutex::new(()),
            written: Mutex::new(HashMap::new()),
            hinted: Mutex::new(HashMap::new()),
            shared: Mutex::new(Default::default()),
            sessions: Default::default(),
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
            node.keep_alone_here()?;
            node.follow_storage();
            node.follow_tunnel();
            node.read_all_status();
            let reader = node.clone();
            tokio::spawn(async move {
                if let Err(err) = reader.read_logs().await {
                    tracing::warn!("reading the bucket: {err:#}");
                }
            });
        }
        let router = Router::builder(endpoint)
            .accept(JOIN_ALPN, JoinHandler(node.clone()))
            .accept(CALL_ALPN, CallHandler(node.clone()))
            .accept(BLOB_ALPN, BlobHandler(node.clone()))
            .spawn();
        tokio::spawn(node.clone().presence_loop());
        tokio::spawn(node.clone().tick_loop());
        tokio::spawn(node.clone().backup_loop());
        tokio::spawn(node.clone().update_loop());
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
            for sidecar in node.0.built_in_sidecars.iter().filter(|n| n.sync.is_some()) {
                node.sync_built_in(sidecar.name);
            }
        }
        Ok((node, router))
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    /// Runs `f` on this server's install.
    fn with<R>(&self, f: impl FnOnce(&mut Install) -> Result<R>) -> Result<R> {
        let mut install = self.0.install.lock().unwrap();
        f(install.as_mut().context("this device is in no install yet: create one, or join with a pairing code")?)
    }

    pub fn has_install(&self) -> bool {
        self.0.install.lock().unwrap().is_some()
    }

    /// This server's local user.
    pub fn user(&self) -> Result<String> {
        self.with(|i| Ok(i.user.clone()))
    }

    /// Refuses a second install: a white label holds one install, and another context is another white label.
    fn ensure_no_install(&self) -> Result<()> {
        if self.has_install() {
            bail!("this device already belongs to an install, and {} holds one; for another, install that context's own app", crate::config::NAME);
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
        let r = Replica::new(self.0.key.clone(), store, Some(Hash::from_bytes(root)))?;
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((Place::from_fields(&f)?, f))).transpose()?;
        let install = Install { r, user: file.user, bucket, peers: HashMap::new(), builds: HashMap::new(), status: HashMap::new(), views: HashMap::new(), nearest: HashMap::new(), far: HashMap::new(), read: false };
        *self.0.install.lock().unwrap() = Some(install);
        Ok(())
    }

    /// An install made before it kept its data in a bucket, with no server but this one: it keeps it in a folder here.
    fn keep_alone_here(&self) -> Result<()> {
        let alone = self.with(|i| Ok(i.bucket.is_none() && i.r.members.servers().all(|s| s.key == self.0.id)))?;
        if !alone {
            return Ok(());
        }
        let dir = self.install_dir();
        let mut file: InstallFile = serde_json::from_slice(&std::fs::read(dir.join("install.json"))?)?;
        let fields: BTreeMap<String, String> = [("path".to_string(), dir.join("bucket").display().to_string())].into();
        let place = Place::from_fields(&fields)?;
        place.create(INSTALL_OBJECT, file.root.as_bytes())?;
        file.bucket = Some(fields.clone());
        self.save_install(&file)?;
        self.with(|i| {
            i.bucket = Some((place, fields));
            Ok(())
        })
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
        // Until it's moved to S3-compatible storage, the install's bucket is a folder here.
        let bucket: BTreeMap<String, String> = [("path".to_string(), self.install_dir().join("bucket").display().to_string())].into();
        Place::from_fields(&bucket)?.create(INSTALL_OBJECT, entry.hash().to_hex().as_bytes())?;
        self.save_install(&InstallFile { root: entry.hash().to_hex(), user: user.clone(), bucket: Some(bucket) })?;
        Store::open(&self.install_dir().join("entries.db"))?.insert_own(&entry, None)?;
        self.open_install()?;
        self.replay_all().await?;
        self.put_entry(&Scope::Global, &entry).await?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })?;
        let me = Caller { key: self.0.id, user, agent: None, token: None };
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
        // What's in the bucket first, so the entry comes after all of it.
        self.read_log_now(scope).await?;
        let entry = self.with(|i| i.r.make(scope, user, by, content, now_ms()))?;
        self.put_entry(scope, &entry).await?;
        let handled = self.with(|i| i.r.add_own(&entry))?;
        self.after(handled).await?;
        self.ping(scope, None);
        Ok(entry)
    }

    /// Passes on new entries, follows what changed in the members, and brings each scope's state up to date.
    async fn after(&self, handled: Handled) -> Result<()> {
        if handled.new.is_empty() && handled.dropped.is_empty() && !handled.members {
            return Ok(());
        }
        self.with(|i| {
            if handled.members {
                let m = &i.r.members;
                i.peers.retain(|peer, _| m.server(peer).is_some());
            }
            for log in &handled.dropped {
                i.views.remove(log);
            }
            update_views(i, &handled.new)
        })?;
        // A scope this server no longer holds: it forgets what it read of it.
        self.0.dbs.lock().unwrap().retain(|(scope, _), _| !handled.dropped.contains(&scope.to_string()));
        if let Ok(bucket) = self.bucket() {
            for log in &handled.dropped {
                if let Err(err) = bucket.forget(&crate::bucket::folder(log)) {
                    tracing::warn!("forgetting this server's copy of {log}: {err:#}");
                }
            }
        }
        if handled.new.iter().any(|e| e.content.app == release::APP && e.content.kind == "node-release") {
            self.0.released.notify_one();
        }
        if handled.members {
            self.0.changed.notify_one();
            self.emit(Event::Servers);
            self.give_keys();
        }
        if handled.members || handled.new.iter().any(|e| e.log() == "global" && e.content.app == members::APP) {
            self.follow_storage();
            self.follow_tunnel();
        }
        let forgotten: Vec<String> = handled.new.iter().filter(|e| e.content.app == members::APP && e.content.kind == "forget").filter_map(|e| e.content.data["blob"].as_str().map(str::to_string)).collect();
        self.emit(Event::Changed { apps: Vec::new() });
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
        Ok(())
    }

    /// Replays every scope's state from the start (when the node starts).
    async fn replay_all(&self) -> Result<()> {
        let _write = self.0.writes.lock().await;
        self.with(|i| {
            i.views.clear();
            let all = i.r.store.ordered()?;
            update_views(i, &all)
        })
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
        tokio::spawn(self.clone().share_bundles(wanted.values().cloned().collect()));
        let changed: Vec<String> = {
            let bundles = self.0.bundles.lock().unwrap();
            modules.iter().map(|(a, _)| a.clone()).filter(|a| bundles.get(a) != wanted.get(a)).collect()
        };
        if !changed.is_empty() || modules.len() != self.0.apps.names().len() {
            // Writes wait for the new deploys, so what's written next runs on them.
            let _write = self.0.writes.lock().await;
            self.0.apps.load(modules.clone()).await?;
            *self.0.bundles.lock().unwrap() = modules.iter().map(|(a, _)| (a.clone(), wanted[a].clone())).collect();
            // Read again, so each is brought to its new deploy's schema.
            self.0.dbs.lock().unwrap().retain(|(_, db), _| !changed.iter().any(|app| db.starts_with(&format!("{app}/"))));
            self.emit(Event::Changed { apps: changed });
        }
        let running = self.with(|i| Ok(i.builds.get(&self.0.id).map(|r| r.commit.clone())))?;
        if running.as_deref() != Some(crate::config::BUILD) {
            self.say(&Scope::Global, status::Said::Build { server: self.0.id, build: crate::config::BUILD.into(), target: crate::config::TARGET.into() }).await?;
        }
        Ok(())
    }

    /// Puts each of `bundles` in the bucket that this server hasn't since it started (`bundle::bucket_name`), so a node
    /// with no other way to get an app's latest deploy reads it there.
    async fn share_bundles(self, bundles: Vec<String>) {
        use crate::bucket::Bucket;
        let Ok(bucket) = self.bucket() else { return };
        let Ok(key) = self.database_key(&Scope::Global, members::APP, crate::bundle::DATABASE).await else { return };
        for hash in bundles {
            if self.0.shared.lock().unwrap().contains(&hash) {
                continue;
            }
            let Ok(bytes) = self.blob(&hash).await else { continue };
            let name = crate::bundle::bucket_name(&hash);
            match Bucket::create(&bucket, &name, &crate::db::seal(&key, &name, &bytes)).await {
                Ok(_) => drop(self.0.shared.lock().unwrap().insert(hash)),
                Err(err) => tracing::debug!("{name}: {err:#}"),
            }
        }
    }

    /// Whether this server is ready to run calls: it's a server of the install, and has read every scope it holds
    /// from the bucket since it woke.
    pub fn ready(&self) -> bool {
        self.with(|i| {
            let alone = i.r.members.servers().all(|s| s.key == self.0.id);
            Ok(i.r.members.server(&self.0.id).is_some() && (i.read || alone))
        })
        .unwrap_or(false)
    }

    /// Now and then, reads what other servers wrote and said, and syncs built-in sidecars' records.
    async fn tick_loop(self) {
        for tick in 1u64.. {
            tokio::time::sleep(TICK).await;
            // What other servers wrote and said, every tick; and wrote of a built-in sidecar's records, every minute.
            if self.has_install() {
                if let Err(err) = self.read_logs().await {
                    tracing::debug!("reading the bucket: {err:#}");
                }
                self.fill_copies().await;
                self.forget_sessions();
                self.read_all_status();
            }
            if tick % 6 == 0 && self.has_install() {
                for sidecar in self.0.built_in_sidecars.iter().filter(|n| n.sync.is_some()) {
                    self.sync_built_in(sidecar.name);
                }
            }
        }
    }
}

/// Which views to replay for new entries: the new entries that count, onto each scope's views, or, where one sorts
/// before what a scope's views have replayed (a member entry of `global` included), that whole scope again.
/// Only `apps`' stores are replayed.
/// Brings each scope's state up to date with new entries: from the ones after what it has, or from the start when one
/// sorts before them or the members changed earlier.
fn update_views(i: &mut Install, new: &[Entry]) -> Result<()> {
    // An erase takes the app's secrets from every scope's state, as replaying the scope again would.
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
        let (_, state) = members::replay_from(i.r.root, &i.r.member_entries, &entries, state, |_, _, _| {});
        let position = match view {
            Some((pos, _)) if !rebuild => pos.clone().max(last),
            _ => last,
        };
        i.views.insert(log.clone(), (position, state));
    }
    Ok(())
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

/// Checks one of a call's appends: into a scope it named, which the caller may write.
fn check_append(i: &Install, app: &str, append: Append, scopes: &[Scope], me: &Caller) -> Result<(Scope, Content)> {
    Ok(match append {
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
        Append::Source { app: name, link } => {
            let m = &i.r.members;
            let a = m.apps.get(&name).filter(|a| !a.erased).with_context(|| format!("no app named {name}"))?;
            ensure!(m.acts_for(&me.user, &a.owner), "only someone acting for {name}'s owner sets its source link");
            let link = members::source_link(&json!(link))?;
            (Scope::Global, members::node("app", json!({ "name": name, "owner": a.owner.to_string(), "source": link })))
        }
        Append::Sidecar { server, name, installed } => (Scope::Global, sidecar_entry(&i.r.members, &me.user, &server, &name, installed)?),
        Append::Vault { id, scope, data, same, release } => vault_entry(i, app, me, scopes, &id, scope, data, same, release)?,
        Append::Install { .. } | Append::Cost { .. } | Append::Exec { .. } => bail!("installs, costs and statements are written on their own"),
    })
}

/// The `sidecar` entry that removes a sidecar from a server, found by name or key prefix, or with `installed` lets it
/// register there again; for someone acting for one of the server's owners.
fn sidecar_entry(m: &Members, user: &str, server: &str, name: &str, installed: bool) -> Result<Content> {
    let found: Vec<&members::Machine> = m.servers().filter(|s| s.name.eq_ignore_ascii_case(server) || (!server.is_empty() && hex(&s.key).starts_with(server))).collect();
    let [s] = found.as_slice() else { bail!("no one device called {server}") };
    ensure!(m.owners(&s.key).iter().any(|o| m.acts_for(user, o)), "only someone acting for one of {}'s owners installs or removes its sidecars", s.name);
    Ok(members::node("sidecar", json!({ "server": hex(&s.key), "name": name, "blob": null, "local": installed })))
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
fn switch_entry(i: &Install, user: &str, scope: &Scope, app: &str, schedule: &str, on: bool) -> Result<Content> {
    ensure!(members::manages_scope(&i.r.members, user, scope), "you can't switch schedules in {scope}");
    ensure!(i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.installed.contains(app)), "{app} isn't installed in {scope}");
    ensure!(i.r.holds(scope), "this device doesn't hold {scope}");
    Ok(members::node("schedule", json!({ "app": app, "schedule": schedule, "on": on })))
}

/// An `item` entry `user` may write in `scope`, checked as replaying it will check it.
fn item_entry(i: &Install, user: &str, scope: &Scope, data: Value) -> Result<Content> {
    let m = &i.r.members;
    ensure!(members::manages_scope(m, user, scope), "you can't change the Vault in {scope}");
    ensure!(i.r.holds(scope), "this device doesn't hold {scope}");
    let mut items = i.views.get(&scope.to_string()).map(|(_, s)| s.items.clone()).unwrap_or_default();
    crate::vault::apply(&mut items, &data, user, now_ms(), |a| !m.apps.get(a).is_some_and(|a| a.erased))?;
    Ok(members::node("item", data))
}

/// A call's change to the Vault, by `app` for its caller: an item it makes (switched on for it, or with `same`, an
/// item of that type whose field matches, all its secret fields given anew), one it's switched on for that it
/// changes (its hosts only with every secret field given anew), or one it lets go of (`release`), deleted once no
/// app uses it.
fn vault_entry(i: &Install, app: &str, me: &Caller, scopes: &[Scope], id: &str, scope: Option<String>, mut data: Value, same: Option<String>, release: bool) -> Result<(Scope, Content)> {
    ensure!(crate::entry::is_id(id), "an item's id is 16 hex characters");
    let fields_given = |it: &crate::vault::Item, data: &Value| it.fields.iter().filter(|(_, f)| f.secret).all(|(n, _)| data["fields"][n]["value"].is_string());
    crate::vault::generate(&mut data);
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

/// A built-in app's bundle: its `app.js` as the module, and every file of its folder as its source. One with npm
/// packages (a `package.json`) is built as `<cli> deploy` builds any app.
pub fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.iter().any(|(p, _)| *p == "package.json") {
        return crate::deploy::build_files(files);
    }
    let module = files.iter().find(|(p, _)| *p == "app.js").context("a built-in app has no app.js")?.1;
    let mut bundle = vec![("app.js".to_string(), module.to_vec())];
    bundle.extend(files.iter().map(|(p, b)| (format!("src/{p}"), b.to_vec())));
    crate::bundle::pack(&bundle)
}

/// A deploy of `app`: the bundle at blob `bundle`.
pub fn deploy(app: &str, bundle: &str) -> Content {
    Content { blobs: vec![bundle.into()], ..members::node("deploy", json!({ "app": app, "bundle": bundle })) }
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
struct JoinHandler(Node);
#[derive(Clone, Debug)]
struct CallHandler(Node);
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
