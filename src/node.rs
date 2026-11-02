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
use crate::vault;

mod backups;
mod scheduled;
mod sessions;
mod status;
mod storage;
mod tunnel;

const JOIN_ALPN: &[u8] = b"one/join/1";
const CALL_ALPN: &[u8] = b"one/call/1";
pub const CLIENT_ALPN: &[u8] = b"one/client/1";
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
/// How often a server looks for schedules to run.
const SCHEDULE_TICK: Duration = Duration::from_secs(10);
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
    /// When each client last asked this server something.
    asked: HashMap<Key, u64>,
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
    /// The schedules' slots running here now: `<scope>/<app>/<schedule>/<slot>`.
    running: Mutex<std::collections::HashSet<String>>,
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
            running: Mutex::new(Default::default()),
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
            .accept(CLIENT_ALPN, ClientHandler(node.clone()))
            .accept(BLOB_ALPN, BlobHandler(node.clone()))
            .spawn();
        tokio::spawn(node.clone().presence_loop());
        tokio::spawn(node.clone().schedule_loop());
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

    pub fn sidecars(&self) -> &crate::sidecars::Sidecars {
        &self.0.sidecars
    }

    /// Registers a sidecar; the node says so in its hello to the other servers, and to shells when that changes it:
    /// an app of its name shows its picture (a Mac app's icon).
    pub async fn register_sidecar(&self, app: &str, describe: Value) -> Result<()> {
        if let Some(why) = self.removed_here(app) {
            bail!("{why}");
        }
        if let Some(domain) = describe["domain"].as_str() {
            self.logo(domain, app);
        }
        if self.0.sidecars.register(app, describe) {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        self.say_hello();
        Ok(())
    }

    /// Removes a sidecar from a server (this one if none is named), or with `installed` lets it register there again,
    /// for someone acting for one of the server's owners.
    pub async fn set_sidecar(&self, caller: &Caller, server: Option<&str>, name: &str, installed: bool) -> Result<()> {
        let server = server.map(str::to_string).unwrap_or_else(|| self.server_name());
        let content = self.with(|i| sidecar_entry(&i.r.members, &caller.user, &server, name, installed))?;
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

    /// Refuses a second install: a white label holds one install, and another context is another white label.
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
        let r = Replica::new(self.0.key.clone(), store, Some(Hash::from_bytes(root)))?;
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((Place::from_fields(&f)?, f))).transpose()?;
        let install = Install { r, user: file.user, bucket, peers: HashMap::new(), asked: HashMap::new(), builds: HashMap::new(), status: HashMap::new(), views: HashMap::new(), nearest: HashMap::new(), far: HashMap::new(), read: false };
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

    // Calls.

    /// Runs an operation for this server's local user: here, or on the nearest server that can.
    pub async fn call(&self, req: CallRequest) -> Result<Value> {
        self.call_as(req, Caller { key: self.0.id, user: self.user()?, agent: None, token: None }).await
    }

    /// Runs an operation for `me`, whom this server acts for: here, or on the nearest server that can.
    pub async fn call_as(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let me = Caller { agent: req.agent.clone().or(me.agent), ..me };
        self.route(req, me).await
    }

    /// A call from a kernel sign-in, which is its person's in every way but one: the node's own operations are its
    /// servers' to ask, and a sign-in's caller has this server's key.
    pub fn for_sign_in(&self, req: CallRequest) -> Result<CallRequest> {
        let req = self.resolve(req);
        ensure!(req.app != members::APP, "a sign-in can't ask that");
        Ok(req)
    }

    /// Where the install's kernel is, for nodes to take its sign-ins: (the install's address, the vault's).
    pub fn kernel(&self) -> Result<(String, String)> {
        self.with(|i| {
            let s = &i.r.members.settings;
            let address = s.get("address").context("this install has no address yet, so it takes no sign-ins: an admin sets one with kernel use")?;
            Ok((address.clone(), s.get("kernel_vault").context("this install doesn't know its vault: an admin says with kernel use")?.clone()))
        })
    }

    /// The key of each scope this server holds, with the people who may open it, for an admin handing them to the
    /// install's kernel (`kernel keys`), so a serverless node opens what this one does.
    pub async fn keys_for_kernel(&self, caller: &Caller) -> Result<Vec<Value>> {
        let scopes = self.with(|i| {
            let m = &i.r.members;
            ensure!(members::manages_scope(m, &caller.user, &Scope::Global), "only admins give the kernel the install's keys");
            let people = |s: &Scope| m.users.values().filter(|u| !u.removed && m.may_use(&u.id, s)).map(|u| (u.id.clone(), u.name.clone())).collect::<BTreeMap<_, _>>();
            let title = |s: &Scope| match s {
                Scope::Global => "Everyone".to_string(),
                Scope::User(u) => m.users.get(u).map(|u| u.name.clone()).unwrap_or_default(),
                Scope::Group(g) => m.groups.get(g).map(|g| g.name.clone()).unwrap_or_default(),
            };
            Ok(m.scopes().into_iter().filter(|s| i.r.holds(s)).map(|s| (s.clone(), title(&s), people(&s))).collect::<Vec<_>>())
        })?;
        let mut out = Vec::new();
        for (scope, title, people) in scopes {
            let key = self.scope_key(&scope).await?;
            out.push(json!({ "scope": scope.to_string(), "title": title, "key": data_encoding::BASE64URL_NOPAD.encode(&key), "people": people.keys().collect::<Vec<_>>(), "names": people }));
        }
        Ok(out)
    }

    /// Whether `user` is a person of the install.
    pub fn is_user(&self, user: &str) -> bool {
        self.with(|i| Ok(i.r.members.user(user).is_some())).unwrap_or(false)
    }

    /// Lets in someone the install's kernel signed in who isn't a person of it yet, as a member, `name` as the kernel
    /// knows them: it signs in only those an admin invited, or people at the install's domain signing in with Google.
    /// This server's own person writes it, so it must be an admin.
    pub async fn let_in(&self, user: &str, name: &str) -> Result<()> {
        let me = self.user()?;
        let (admin, removed) = self.with(|i| Ok((i.r.members.is_admin(&me), i.r.members.removed_user(user))))?;
        ensure!(!removed, "you were removed from this install: an admin adds you back");
        ensure!(admin, "you aren't a person of this install yet, and {} can't let you in: its person isn't an admin", self.server_name());
        ensure!(!name.trim().is_empty(), "your sign-in doesn't say your name: sign in again");
        // A key no machine holds, the same on every server, so two letting the same person in at once add them once.
        let key: Key = <sha2::Sha256 as sha2::Digest>::digest(format!("{}/sign-in\0{user}", crate::config::CLI)).into();
        let admit = members::admit(&key, user, &format!("{name}'s sign-in"), false, Some((name.trim(), Role::Member)));
        self.member_as(&Caller { key: self.0.id, user: me, agent: None, token: None }, "admit", admit.data).await
    }

    /// Tries the servers that might run a call, nearest first, one at a time, the next only if one can't be reached
    /// or says to go elsewhere.
    async fn route(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let mut req = self.resolve(req);
        req.id.get_or_insert_with(new_id);
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

    /// Runs a sidecar's call on every server that runs it for the caller, one at a time: `{ <server>: answer }`, or
    /// `{ error }` for one that failed. Only for sidecars: an app's call runs once.
    async fn everywhere(&self, req: CallRequest, me: Caller) -> Result<Value> {
        ensure!(req.sidecar.is_some(), "{} is an app: its calls run once, on one server", req.app);
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

    /// The servers that might run a call, in the order it tries them: those that are up with room first; then the
    /// near before the far (FAR); then by the install's order (src/rank.rs); this server before another as good; then
    /// the nearest. A sidecar about its own machine runs here first.
    fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        if let Some(server) = &req.server {
            return Ok(vec![self.find_server(server)?]);
        }
        let here = self.runs_sidecar_here(&req.app);
        // A sidecar, or one of its operations, that's about its own machine (`"prefer": "here"`: Shell, models_here) runs
        // on the caller's first.
        let about_here = |d: &Value| d["prefer"] == "here" || d["ops"].as_array().is_some_and(|ops| ops.iter().any(|o| o["name"] == req.op.as_str() && o["prefer"] == "here"));
        let here_first = here && req.sidecar.is_some() && self.with(|i| Ok(self.sidecars_here(i).iter().any(|d| d.0 == req.app && about_here(&d.2))))?;
        self.with(|i| {
            let m = &i.r.members;
            let mut servers: Vec<Key> = if req.sidecar.is_none() {
                m.servers().map(|s| s.key).filter(|s| m.serves(s, &me.user)).collect()
            } else {
                let mut s: Vec<Key> = i.peers.iter().filter(|(k, _)| i.online(k)).filter(|(_, s)| s.sidecars.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect();
                if here && (me.user == i.user || (self.0.sidecars.serves_everyone(&req.app) && i.r.members.serves(&self.0.id, &me.user))) {
                    s.push(self.0.id);
                }
                s
            };
            let set = rank::set(&i.r.members.settings);
            let here_load = crate::load::current();
            let rank = |k: &Key| {
                let here = *k == self.0.id;
                let load = if here { Some(&here_load) } else { i.peers.get(k).and_then(|p| p.load.as_ref()) };
                let kind = load.map(|l| l.kind.as_str()).unwrap_or("");
                let far = !here && i.far.get(k).is_some_and(|d| *d > FAR);
                (!here && !i.online(k), here_first && !here, load.is_some_and(full), far, rank::order(&set, &hex(k), kind), !here, i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)))
            };
            servers.sort_by_cached_key(rank);
            Ok(servers)
        })
    }

    /// A call as the node runs it: one that names a sidecar is that sidecar's, whatever app shares its name. As older
    /// machines ask for a sidecar, so is one that names an app with no deploy, or that runs on every server.
    fn resolve(&self, mut req: CallRequest) -> CallRequest {
        if req.sidecar.is_none() {
            req.sidecar = req.driver.take();
        }
        if let Some(sidecar) = &req.sidecar {
            req.app = sidecar.clone();
        } else if req.app != members::APP && (!self.deployed(&req.app) || req.server.as_deref() == Some("all")) {
            req.sidecar = Some(req.app.clone());
        }
        req
    }

    fn deployed(&self, app: &str) -> bool {
        self.with(|i| Ok(i.r.members.apps.get(app).is_some_and(|a| a.bundle.is_some()))).unwrap_or(false)
    }

    /// The sidecars serving on this server, whom each serves, and what each says it is: those built into the node,
    /// then those registered. A sidecar one of its owners removed from this server isn't served here.
    fn sidecars_here(&self, i: &Install) -> Vec<Sidecar> {
        let m = &i.r.members;
        let everyone: Vec<String> = m.users.keys().filter(|u| m.serves(&self.0.id, u)).cloned().collect();
        let removed = |name: &str| m.sidecars.get(&(self.0.id, name.to_string())).is_some_and(|d| d.removed());
        let built_in = |n: &crate::sidecars::BuiltIn| {
            let mut d = (n.describe)();
            d["name"] = json!(n.name);
            d["built_in"] = json!(true);
            d
        };
        let mut sidecars: Vec<Sidecar> = self.0.built_in_sidecars.iter().filter(|n| !removed(n.name)).map(|n| (n.name.to_string(), vec![i.user.clone()], built_in(n))).collect();
        sidecars.extend(self.0.sidecars.describe().into_iter().filter(|d| !removed(d["name"].as_str().unwrap_or(""))).map(|d| {
            let users = if d["serves"] == "everyone" { everyone.clone() } else { vec![i.user.clone()] };
            (d["name"].as_str().unwrap_or("").to_string(), users, d)
        }));
        sidecars
    }

    /// Why a sidecar isn't served here, if one of this server's owners removed it.
    fn removed_here(&self, name: &str) -> Option<String> {
        self.with(|i| {
            let m = &i.r.members;
            Ok(m.sidecars.get(&(self.0.id, name.to_string())).filter(|d| d.removed()).map(|d| {
                let by = m.users.get(&d.by).map(|u| u.name.as_str()).unwrap_or("an owner");
                format!("{name} was removed from {} by {by}; an owner of it installs it again", self.server_name())
            }))
        })
        .ok()
        .flatten()
    }

    fn runs_sidecar_here(&self, app: &str) -> bool {
        (self.0.built_in_sidecars.iter().any(|n| n.name == app) || self.0.sidecars.has(app)) && self.removed_here(app).is_none()
    }

    /// Asks another server to run a call.
    async fn ask_server(&self, server: Key, req: &CallRequest, me: &Caller) -> Result<Reply> {
        let conn = tokio::time::timeout(Duration::from_secs(10), self.0.endpoint.connect(EndpointId::from_bytes(&server)?, CALL_ALPN)).await.context("timed out")??;
        if let Some(rtt) = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt()) {
            let _ = self.with(|i| Ok(i.far.insert(server, rtt)));
        }
        let (mut send, mut recv) = conn.open_bi().await?;
        let request = json!({ "app": req.app, "sidecar": req.sidecar, "driver": req.sidecar, "op": req.op, "input": req.input, "scopes": req.scopes, "depth": req.depth, "from": req.from, "client": req.client, "me": { "key": hex(&me.key), "user": me.user, "agent": me.agent, "token": me.token } });
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

    /// How long a call may run: an app's, CALL_LIMIT; a sidecar's, and that of an app that uses sidecars (which waits
    /// on theirs), SIDECAR_LIMIT.
    async fn limit(&self, req: &CallRequest) -> Duration {
        if req.app == members::APP {
            return CALL_LIMIT;
        }
        if req.sidecar.is_some() || !self.uses_here(&req.app).await.1.is_empty() { SIDECAR_LIMIT } else { CALL_LIMIT }
    }

    async fn run_here(&self, req: &CallRequest, me: &Caller, limit: Duration) -> Result<Reply> {
        let input = if req.input.is_null() { json!({}) } else { req.input.clone() };
        if req.app == members::APP {
            return self.node_op(&req.op, &input, me).await.map(Reply::Ok);
        }
        if req.sidecar.is_some() {
            if !self.runs_sidecar_here(&req.app) {
                return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
            }
            return match self.run_sidecar(&req.app, &req.op, input, me, req.from.clone()).await {
                Err(err) if err.is::<crate::sidecars::Elsewhere>() => Ok(Reply::Elsewhere(err.to_string())),
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
        // The vault is asked once for the keys of every scope a sign-in's call names, which its session keeps here.
        let want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), req.app.as_str(), DATABASE)).collect();
        if let Err(err) = self.session_keys(me, &want).await {
            tracing::debug!("{}'s keys for {}: {err:#}", me.user, req.app);
        }
        // A call that writes, tried on another server before this one, ran there if it took the lease first.
        if let (Some(id), Some(scope)) = (&req.id, scopes.first()) {
            let reads = self.0.apps.describe(&req.app).await?["ops"].as_array().into_iter().flatten().any(|o| o["name"] == req.op.as_str() && o["reads"] == true);
            let lease = format!("{}/{}/calls/{id}", crate::bucket::folder(&scope.to_string()), req.app);
            if !reads && !crate::bucket::Bucket::create(&self.call_bucket(me)?, &lease, hex(&self.0.id).as_bytes()).await? {
                return Ok(Reply::Error("it ran on another server already, whose answer was lost: look before trying again".into()));
            }
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

    // Databases.

    /// The install's bucket, as this server reaches it, with its copy of the databases' steps.
    fn bucket(&self) -> Result<Copied> {
        let place = self.with(|i| i.bucket.as_ref().map(|(p, _)| p.clone()).context("this server can't reach the install's bucket, where apps keep their data: use another server"))?;
        Ok(Copied::moving(place, self.install_dir().join("copy"), self.0.moved.clone()))
    }

    /// A database's key, `app`'s `database` in `scope`, as this server opens it with its own keys: from the app's key,
    /// in the bucket sealed under the scope's. A call that came with a token gets its keys from the vault instead
    /// (`sessions`); this is for this server's own work, and installs with no kernel.
    async fn database_key(&self, scope: &Scope, app: &str, database: &str) -> Result<[u8; 32]> {
        tracing::debug!("opening {scope}'s {app}/{database} with this server's own key");
        let scope_key = self.scope_key(scope).await?;
        let app_key = crate::db::app_key(&self.bucket()?, &scope_key, &scope.to_string(), app, database).await?;
        Ok(crate::kernel::database_key(&app_key, database))
    }

    /// A scope's key, which its databases and backups are sealed with. The first server to need one makes it, and the
    /// bucket says whose is the scope's (its id), so two made at once can't both seal its data. Each server that
    /// holds the scope finds it in the bucket sealed to its own key, as the recovery key does.
    pub(crate) async fn scope_key(&self, scope: &Scope) -> Result<[u8; 32]> {
        use crate::bucket::Bucket;
        if let Some(key) = self.0.scope_keys.lock().unwrap().get(scope) {
            return Ok(*key);
        }
        // One at a time here, so this server never makes two keys for a scope.
        let _making = self.0.making_keys.lock().await;
        if let Some(key) = self.0.scope_keys.lock().unwrap().get(scope) {
            return Ok(*key);
        }
        let bucket = self.bucket()?;
        let folder = crate::bucket::folder(&scope.to_string());
        let given = format!("{folder}/{KEYS}/{}", hex(&self.0.id));
        let key = match Bucket::get(&bucket, &given).await? {
            Some(o) => crate::backups::unseal(self.0.key.as_bytes(), &o.bytes)?,
            None => {
                // One its backups made before the bucket kept keys.
                let earlier = self.with(|i| Ok(i.views.get(&scope.to_string()).and_then(|(_, s)| s.backup_key)))?;
                let key = earlier.unwrap_or_else(rand::random);
                let id = crate::backups::key_id(&key);
                let name = format!("{folder}/{KEY_OBJECT}");
                if !Bucket::create(&bucket, &name, id.as_bytes()).await? && Bucket::get(&bucket, &name).await?.is_none_or(|o| o.bytes != id.as_bytes()) {
                    // Another server made it just now: it gives it to this one next, once it's sealed it.
                    let mut found = None;
                    for _ in 0..50 {
                        found = Bucket::get(&bucket, &given).await?;
                        if found.is_some() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    match found {
                        Some(o) => crate::backups::unseal(self.0.key.as_bytes(), &o.bytes)?,
                        None => bail!("{} doesn't have {scope}'s key yet: a server that has it gives it when it's next on", self.server_name()),
                    }
                } else {
                    key
                }
            }
        };
        self.0.scope_keys.lock().unwrap().insert(scope.clone(), key);
        self.give_key(scope, &key).await?;
        Ok(key)
    }

    /// Seals a scope's key, in the bucket, to each server that holds the scope and to the recovery key, where it isn't
    /// yet.
    async fn give_key(&self, scope: &Scope, key: &[u8; 32]) -> Result<()> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        let folder = crate::bucket::folder(&scope.to_string());
        let to = self.with(|i| {
            let m = &i.r.members;
            let mut to: Vec<(String, Key)> = m.servers().filter(|s| m.holds(&s.key, scope)).map(|s| (hex(&s.key), s.key)).collect();
            to.extend(m.recovery.map(|r| ("recovery".to_string(), r)));
            Ok(to)
        })?;
        for (name, public) in to {
            let name = format!("{folder}/{KEYS}/{name}");
            if Bucket::get(&bucket, &name).await?.is_none() {
                Bucket::create(&bucket, &name, &crate::backups::seal(&public, key)?).await?;
            }
        }
        Ok(())
    }

    /// Gives each scope's key this server has to the servers that hold the scope now.
    fn give_keys(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let keys: Vec<(Scope, [u8; 32])> = node.0.scope_keys.lock().unwrap().iter().map(|(s, k)| (s.clone(), *k)).collect();
            for (scope, key) in keys {
                if let Err(err) = node.give_key(&scope, &key).await {
                    tracing::warn!("giving {scope}'s key: {err:#}");
                }
            }
        });
    }

    /// What another server said it wrote to a database: step `at`, kept in this server's copy if it sent it and holds
    /// the scope, or else the database is behind, and its next read catches up from the bucket.
    fn written_there(&self, by: &Key, scope: &Scope, app: &str, database: &str, at: u64, step: Option<&str>) -> Result<()> {
        let id = (scope.clone(), format!("{app}/{database}"));
        let holds = self.with(|i| Ok(i.r.members.holds(&self.0.id, scope) && i.r.members.holds(by, scope)))?;
        let bucket = self.bucket()?;
        let prefix = format!("{}/{}", crate::bucket::folder(&scope.to_string()), id.1);
        let kept = match step.map(|s| data_encoding::BASE64.decode(s.as_bytes())) {
            Some(Ok(bytes)) if holds && at > 0 => {
                bucket.keep(&crate::db::step_name(&prefix, at), &bytes, &hex(by))?;
                // A step whose one before isn't here may follow one this server wasn't sent.
                at == 1 || bucket.kept(&crate::db::step_name(&prefix, at - 1))?.is_some()
            }
            _ => false,
        };
        if !kept {
            self.0.behind.lock().unwrap().insert(id);
        }
        Ok(())
    }

    /// What a node that sends no pings (the serverless one) said it wrote, as someone who may use `scope`: step `at`
    /// of `app`'s `database`, which this server fetches from the bucket into its copy, if it holds the scope, and
    /// passes on to the others that do as if it wrote it. Only a hint, which each checks against the bucket.
    pub async fn written_elsewhere(&self, me: &Caller, scope: &Scope, app: &str, database: &str, at: u64) -> Result<()> {
        let holds = self.with(|i| {
            ensure!(i.r.members.may_use(&me.user, scope), "you can't use {scope}");
            Ok(i.r.members.holds(&self.0.id, scope))
        })?;
        if holds && at > 0 {
            let bucket = self.bucket()?;
            let prefix = format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()));
            crate::bucket::Bucket::get(&bucket, &crate::db::step_name(&prefix, at)).await?;
            // A step whose one before isn't here may follow one this server hasn't read.
            if at > 1 && bucket.kept(&crate::db::step_name(&prefix, at - 1))?.is_none() {
                self.0.behind.lock().unwrap().insert((scope.clone(), format!("{app}/{database}")));
            }
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        self.ping(scope, Some((app, database, at)));
        Ok(())
    }

    /// Brings a database up to date for a call that reads it, as far as this server's copy goes: other servers send
    /// what they write with their pings, and `fill_copies` keeps up with the bucket. Only when a ping said it changed
    /// without sending what is the bucket asked first, and if it doesn't answer, the copy is read as it is: behind,
    /// never wrong.
    async fn read_ready(&self, id: &(Scope, String), db: &mut Db, bucket: &Copied) -> Result<()> {
        if self.0.behind.lock().unwrap().remove(id) {
            match tokio::time::timeout(BUCKET_WAIT, db.catch_up(bucket)).await {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(err)) => tracing::debug!("{}: reading this server's copy, as the bucket said {err:#}", db.prefix()),
                Err(_) => tracing::debug!("{}: reading this server's copy, as the bucket didn't answer", db.prefix()),
            }
        }
        db.catch_up(&bucket.here_only()).await?;
        Ok(())
    }

    /// Fetches into this server's copy what the bucket has of each database open here that it lacks, every tick: a
    /// ping that was lost, or a serverless write no server was told of. A database a call is using is
    /// left for the next. First, the steps other servers sent are checked against the bucket's: a database that read
    /// one that differs is read again from the bucket's, and the server that sent it is recorded, for Devices.
    async fn fill_copies(&self) {
        let Ok(bucket) = self.bucket() else { return };
        let mut changed = Vec::new();
        match bucket.check().await {
            Ok(wrong) => {
                for (name, from) in wrong {
                    let Some((folder, id)) = name.rsplit_once('/').and_then(|(prefix, _)| prefix.split_once('/')) else { continue };
                    let Ok(scope) = Scope::parse(&crate::bucket::scope_of(folder)) else { continue };
                    tracing::warn!("{from} sent {name}, which isn't the bucket's: reading it again");
                    self.0.dbs.lock().unwrap().remove(&(scope, id.to_string()));
                    changed.extend(id.split('/').next().filter(|app| *app != members::APP).map(str::to_string));
                    let Ok(server) = unhex(&from) else { continue };
                    let name = self.with(|i| Ok(i.r.members.server(&server).map(|s| s.name.clone()))).ok().flatten().unwrap_or_else(|| short(&server));
                    let said = status::Said::Failed { app: members::APP.into(), op: "changed".into(), error: format!("{name} sent a step that isn't the bucket's"), user: from };
                    if let Err(err) = self.say(&Scope::Global, said).await {
                        tracing::warn!("recording that {name} sent a wrong step: {err:#}");
                    }
                }
            }
            Err(err) => tracing::warn!("checking the steps other servers sent: {err:#}"),
        }
        let open: Vec<_> = self.0.dbs.lock().unwrap().iter().map(|(id, db)| (id.clone(), db.clone())).collect();
        for ((_, name), db) in open {
            let Ok((prefix, at)) = db.try_lock().map(|db| (db.prefix().to_string(), db.at())) else { continue };
            match bucket.fill(&prefix, at).await {
                Ok(0) => {}
                Ok(_) => changed.extend(name.split('/').next().filter(|app| *app != members::APP).map(str::to_string)),
                Err(err) => tracing::debug!("{prefix}: {err:#}"),
            }
        }
        if !changed.is_empty() {
            self.emit(Event::Changed { apps: changed });
        }
    }

    /// `app`'s `database` of `scope`, opened from the bucket the first time and brought to `schema`, or else to the
    /// app's deploy's: with this server's own key, or `given`, a key the vault gave a call's session, with the bucket
    /// as that call reaches it.
    async fn database(&self, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>, given: Option<([u8; 32], &Copied)>) -> Result<Arc<tokio::sync::Mutex<Db>>> {
        let id = (scope.clone(), format!("{app}/{database}"));
        let cached = self.0.dbs.lock().unwrap().get(&id).cloned();
        if let Some(db) = cached {
            match given {
                None => return Ok(db),
                Some((key, _)) if db.lock().await.opens_with(&key) => return Ok(db),
                Some(_) => {}
            }
        }
        let (key, bucket) = match given {
            Some((key, bucket)) => (key, bucket.clone()),
            None => (self.database_key(scope, app, database).await?, self.bucket()?),
        };
        let mut db = Db::open(&bucket, &format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string())), key).await?;
        let schema = match schema {
            Some(schema) => schema,
            None => {
                let kind = match scope {
                    Scope::Global => "global",
                    Scope::Group(_) => "group",
                    Scope::User(_) => "user",
                };
                serde_json::from_value(self.0.apps.describe(app).await?["schema"][kind].clone()).unwrap_or_default()
            }
        };
        db.migrate(&bucket, &schema).await?;
        let db = Arc::new(tokio::sync::Mutex::new(db));
        let mut dbs = self.0.dbs.lock().unwrap();
        match dbs.get(&id) {
            Some(open) if open.try_lock().is_ok_and(|d| d.opens_with(&key)) => Ok(open.clone()),
            _ => Ok(dbs.entry(id).insert_entry(db).get().clone()),
        }
    }

    // Insights.

    /// Records, for an app's owners, a call that ran here: the first each day by each user from each client
    /// (`used`), and every failure (`failed`), at most `FAILURES_KEPT` an hour per app on this server. They go in the
    /// owner's scope when the caller may use it and this server holds it, else in `global`; in the background.
    fn record(&self, app: &str, req: &CallRequest, me: &Caller, error: Option<&str>) {
        let now = now_ms();
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let said = match error {
            None => {
                let day = now / 86_400_000;
                if !self.0.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
                    return;
                }
                status::Said::Used { app: app.into(), user: me.user.clone(), client, agent: me.agent.clone(), day }
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
                status::Said::Failed { app: app.into(), op: req.op.clone(), error, user: me.user.clone() }
            }
        };
        self.write_insight(app, me, said);
    }

    /// Writes an insight about `app` for `me`'s call: in the app owner's scope when `me` may use it and this server
    /// holds it, else in `global`; in the background.
    fn write_insight(&self, app: &str, me: &Caller, said: status::Said) {
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
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.say(&scope, said).await {
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
            for (log, status) in &i.status {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = status.insights.get(app).filter(|_| m.may_use(&viewer.user, &scope)) else { continue };
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
                let said = status::Said::Cost {
                    app: to.into(),
                    model: data["model"].as_str().unwrap_or("?").into(),
                    usd: data["usd"].as_f64().unwrap_or(0.0),
                    input: data["input"].as_u64().unwrap_or(0),
                    output: data["output"].as_u64().unwrap_or(0),
                };
                self.write_insight(to, me, said);
            }
        }
        let schedules_changed = appends.iter().any(|a| matches!(a, Append::Install { .. } | Append::Switch { .. }));
        let (installs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Install { .. }));
        let (execs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Exec { .. }));
        // Each scope's statements, in the order the call ran them, and the blobs they keep.
        type Statement = (String, Vec<Value>, Option<String>);
        let mut statements: Vec<(Scope, Vec<Statement>, Vec<String>)> = Vec::new();
        for exec in execs {
            let Append::Exec { scope, sql, params, blobs, check } = exec else { continue };
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blobs.iter().all(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())), "a blob is named by its hash");
            match statements.iter_mut().find(|(s, _, _)| *s == scope) {
                Some((_, list, kept)) => {
                    list.push((sql, params, check));
                    kept.extend(blobs);
                }
                None => statements.push((scope, vec![(sql, params, check)], blobs)),
            }
        }
        let mut writes = Vec::new();
        for append in appends {
            writes.push(self.with(|i| check_append(i, app, append, scopes, me))?);
        }
        let wrote = !statements.is_empty();
        for (scope, list, kept) in statements {
            let (db, bucket) = self.call_database(me, &scope, app, DATABASE, None).await?;
            let mut db = db.lock().await;
            let at = db.at();
            db.write(&bucket, |c| crate::db::statements(c, &list)).await?;
            if db.at() > at {
                self.ping(&scope, Some((app, DATABASE, db.at())));
            }
            drop(db);
            if !kept.is_empty() {
                writes.push((scope, Content { blobs: kept, ..members::node("keep", json!({})) }));
            }
        }
        if wrote {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
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
        if schedules_changed && self.schedules_service().is_some() && me.token.is_some() {
            self.sync_schedules_later(me);
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

    /// Runs a built-in sidecar or sidecar here, for this server's local user.
    async fn run_sidecar(&self, app: &str, op: &str, input: Value, me: &Caller, from: Option<String>) -> Result<Value> {
        let everyone = self.0.sidecars.serves_everyone(app) && self.with(|i| Ok(i.r.members.serves(&self.0.id, &me.user)))?;
        let user = if everyone { me.user.clone() } else { self.user()? };
        ensure!(me.user == user, "{app} runs only for {}'s own user", self.server_name());
        let caller = self.me_for(me)?;
        // An app's call has the fields of the items it's switched on for filled in, a secret one only for a sidecar
        // named among its item's hosts, and blotted out of the answer.
        let mut input = input;
        let used = std::sync::Mutex::new(Vec::<String>::new());
        if let Some(from) = &from {
            vault::fill_value(&mut input, app, &|name| {
                let found = self.vault_for(from, &me.user, name);
                if let Some((hosts, value)) = &found {
                    if !hosts.is_empty() {
                        used.lock().unwrap().push(value.clone());
                    }
                }
                found
            })?;
        }
        let built_in = self.0.built_in_sidecars.iter().find(|n| n.name == app);
        let mut answer = match built_in {
            Some(n) => (n.call)(op.to_string(), input, caller.clone(), if n.records { self.records(app).await? } else { vec![] }).await?,
            None => self.0.sidecars.call(app, op, input, caller.clone(), from).await?,
        };
        vault::blot(&mut answer.ok, &used.into_inner().unwrap());
        let scope = Scope::User(user.clone());
        if !answer.records.is_empty() {
            let (db, bucket) = self.call_database(me, &scope, app, SIDECAR_DATABASE, Some(vec![crate::sidecars::RECORDS.into()])).await?;
            let (time, by, device) = (now_ms(), caller.user_name.clone(), caller.name.clone());
            let mut db = db.lock().await;
            db.write(&bucket, |c| {
                for (kind, data) in &answer.records {
                    let row = [json!(kind), json!(data.to_string()), json!(time), json!(by), json!(device)];
                    crate::db::execute(c, "insert into records (kind, data, time, by, device) values (?, ?, ?, ?, ?)", &row)?;
                }
                Ok(())
            })
            .await?;
            self.ping(&scope, Some((app, SIDECAR_DATABASE, db.at())));
            drop(db);
            if let Some(n) = built_in.filter(|n| n.sync.is_some()) {
                self.sync_built_in(n.name);
            }
        }
        if !answer.blobs.is_empty() {
            let by = (me.key != self.0.id).then_some(me.key);
            self.append_as(&scope, &user, by, Content { blobs: answer.blobs, ..members::node("keep", json!({})) }).await?;
        }
        Ok(answer.ok)
    }

    /// The node's own operations, which other servers ask for.
    async fn node_op(&self, op: &str, input: &Value, me: &Caller) -> Result<Value> {
        match op {
            // Another server's hints, which only a server of the install gives.
            "hello" | "changed" if self.with(|i| Ok(i.r.members.server(&me.key).is_some()))? => {
                if op == "hello" {
                    let sidecars: Vec<Sidecar> = serde_json::from_value(input["sidecars"].clone()).unwrap_or_default();
                    let load = serde_json::from_value(input["load"].clone()).ok();
                    // The sidecars that came, went or changed there, whose apps shells list again.
                    let (new, changed) = self.with(|i| {
                        let before = i.peers.get(&me.key).map(|p| p.sidecars.clone());
                        let was = before.as_deref().unwrap_or_default();
                        let missing = |a: &[Sidecar], b: &[Sidecar]| a.iter().filter(|s| !b.contains(s)).map(|s| s.0.clone()).collect::<Vec<_>>();
                        let mut changed = missing(&sidecars, was);
                        changed.extend(missing(was, &sidecars));
                        changed.sort();
                        changed.dedup();
                        i.peers.insert(me.key, Peer { sidecars, load, seen: Instant::now() });
                        Ok((before.is_none(), changed))
                    })?;
                    if new || !changed.is_empty() {
                        self.emit(Event::Servers);
                    }
                    if !changed.is_empty() {
                        self.emit(Event::Changed { apps: changed });
                    }
                    return Ok(json!(null));
                }
                let scope = Scope::parse(input["scope"].as_str().unwrap_or(""))?;
                let app = input["app"].as_str().map(str::to_string);
                if let (Some(app), Some(database)) = (&app, input["database"].as_str()) {
                    self.written_there(&me.key, &scope, app, database, input["at"].as_u64().unwrap_or(0), input["step"].as_str())?;
                    self.emit(Event::Changed { apps: vec![app.clone()] });
                    return Ok(json!(null));
                }
                let node = self.clone();
                tokio::spawn(async move {
                    let _ = node.read_log(&scope).await;
                    if let Some(app) = app {
                        node.emit(Event::Changed { apps: vec![app] });
                    }
                });
                Ok(json!(null))
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
        self.member_content(caller, content).await?;
        if how == "erased" {
            self.destroy_keys(app).await?;
        }
        Ok(())
    }

    /// Erases an app's data in every scope by deleting its keys: its databases stay in the bucket, which nothing opens
    /// again.
    async fn destroy_keys(&self, app: &str) -> Result<()> {
        let bucket = self.bucket()?;
        self.0.dbs.lock().unwrap().retain(|(_, db), _| db.split('/').next() != Some(app));
        for folder in bucket.folders()? {
            bucket.forget(&format!("{folder}/{app}"))?;
        }
        let (bucket, app) = (bucket.place, app.to_string());
        tokio::task::spawn_blocking(move || {
            for scope in bucket.list("")?.iter().map(|f| crate::bucket::scope_of(f)).filter(|s| Scope::parse(s).is_ok()) {
                bucket.delete(&crate::kernel::app_key_name(&scope, &app))?;
            }
            Ok(())
        })
        .await?
    }

    // Schedules.

    /// Now and then, runs the schedules this server is next in line for.
    async fn schedule_loop(self) {
        for tick in 1u64.. {
            tokio::time::sleep(SCHEDULE_TICK).await;
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
        // An install with a kernel runs them from its schedules service, with only the keys each was let use.
        if self.schedules_service().is_some() {
            return Ok(());
        }
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
                        let ran = i.status.get(log).and_then(|st| st.runs.get(&(app.clone(), s.name.clone()))).is_some_and(|r| r.iter().any(|r| r.slot == slot));
                        if slot < installed || ran || !schedules::due(s, slot, now) {
                            continue;
                        }
                        let line = schedules::line(m, &scope, &user, app, &s.name);
                        let Some(at) = line.iter().position(|k| *k == self.0.id) else { continue };
                        let ahead = line[..at].iter().filter(|k| i.online(k)).count() as u64;
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
        // A slot runs once: the server that takes it in the bucket first, once it's ready to run it.
        let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone())))?;
        if !self.ready() || latest.is_none() || self.0.bundles.lock().unwrap().get(app) != latest.as_ref() {
            return Ok(());
        }
        let lease = format!("{}/{}/slots/{app}/{}/{slot}", crate::bucket::folder(&scope.to_string()), members::APP, schedule.name);
        if !crate::bucket::Bucket::create(&self.bucket()?, &lease, hex(&self.0.id).as_bytes()).await? {
            return Ok(());
        }
        let started = Instant::now();
        let reply = match self.count(app, "schedule").await {
            Err(err) => Reply::Error(format!("{err:#}")),
            Ok(()) => {
                let req = CallRequest { app: app.into(), op: schedule.op.clone(), input: schedule.input.clone(), scopes: Some(vec![scope.to_string()]), client: Some("schedule".into()), ..Default::default() };
                self.run(&req, &Caller { key: self.0.id, user: user.into(), agent: None, token: None }).await
            }
        };
        let (ok, error) = match reply {
            Reply::Ok(_) => (true, None),
            Reply::Error(e) => (false, Some(e.chars().take(500).collect::<String>())),
            // It took the slot, so it says so; the next slot is tried again.
            Reply::Elsewhere(why) => (false, Some(format!("it didn't run: {why}"))),
        };
        let run = status::Run { slot, time: now_ms(), server: self.0.id, user: user.into(), ok, error, ms: started.elapsed().as_millis() as u64, spec: schedule.spec() };
        self.say(scope, status::Said::Ran { app: app.into(), schedule: schedule.name.clone(), run }).await
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
                let status = i.status.get(&scope.to_string()).cloned().unwrap_or_default();
                let scope_name = handles(m, std::slice::from_ref(&scope), &viewer.user).remove(0).name;
                let base = |name: &str| {
                    let key = (app.to_string(), name.to_string());
                    let runs = status.runs.get(&key);
                    let last = runs.and_then(|r| r.last()).map(|r| json!({ "time": r.time, "ok": r.ok, "error": r.error }));
                    let changed = status.changed.get(&key).filter(|(.., t)| now.saturating_sub(*t) < CHANGED_SHOWN).map(|(was, is, t)| json!({ "was": schedules::say(was), "is": schedules::say(is), "time": t }));
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
                for ((a, name), runs) in &status.runs {
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
                for ((a, name), runs) in status.runs.iter().filter(|((a, _), _)| a == app) {
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
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new(), caught_up: Vec::new(), depth, from: ctx.from.clone() };
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

    /// What a placeholder in a request of `app`'s for `user` names, as (hosts, value; no hosts for a plain field):
    /// `<item id>:field` of an item in a scope the user may use, `field` of the oldest item that has it in the user's
    /// own scope, else in `global`, or `<group id>/field` in that group's; always of an item `app` is switched on for.
    pub fn vault_for(&self, app: &str, user: &str, name: &str) -> Option<(Vec<String>, String)> {
        self.with(|i| {
            let m = &i.r.members;
            let items = |scope: &Scope| i.views.get(&scope.to_string()).map(|(_, s)| &s.items);
            let oldest = |scope: &Scope, field: &str| items(scope).and_then(|its| vault::find(its, app, field)).and_then(|(_, it)| vault::filled(it, field));
            Ok(match (name.split_once(':'), name.split_once('/')) {
                (Some((id, field)), _) => i
                    .views
                    .iter()
                    .filter(|(log, _)| Scope::parse(log).is_ok_and(|s| m.may_use(user, &s)))
                    .find_map(|(_, (_, s))| s.items.get(id))
                    .filter(|it| it.apps.contains(app))
                    .and_then(|it| vault::filled(it, field)),
                (None, Some((group, field))) => {
                    let scope = Scope::Group(group.to_string());
                    if m.may_use(user, &scope) { oldest(&scope, field) } else { None }
                }
                (None, None) => oldest(&Scope::User(user.to_string()), name).or_else(|| oldest(&Scope::Global, name)),
            })
        })
        .ok()
        .flatten()
    }

    /// Whether `host` is the name of an app or sidecar, which an item's hosts name to let its fields into calls to
    /// that sidecar: never a site's, so a web request to it gets none.
    pub fn names_app(&self, host: &str) -> bool {
        self.0.built_in_sidecars.iter().any(|n| n.name == host) || self.0.sidecars.has(host) || self.with(|i| Ok(i.r.members.apps.contains_key(host) || i.r.members.sidecars.keys().any(|(_, name)| name == host))).unwrap_or(false)
    }

    /// Fills `text` for a sidecar's call while it waits for the sidecar's answer, as its app's requests are filled: from
    /// the items switched on for the app of the sidecar's name that the call's user may use, a secret field only when
    /// `host` is one of its item's hosts. The sidecar then sends it to `host` itself.
    pub fn sidecar_fill(&self, sidecar: &str, call: u64, text: &str, host: &str) -> Result<String> {
        ensure!(!self.names_app(host), "{host} is an app's name, not a site's");
        let me = self.0.sidecars.caller_of(sidecar, call).with_context(|| format!("no call {call} of {sidecar}'s is waiting"))?;
        vault::fill(text, host, |name| self.vault_for(sidecar, &me.user, name))
    }

    /// Makes an HTTP request for an agent of `caller`'s (`{ url, method?, headers?, body? }`), as the built-in agents
    /// sidecar: the fields of the items a person lets agents use are filled in, a secret one only for its item's
    /// hosts, as an app's requests are. → `{ status, headers, body }`, a secret field's value blotted out wherever the
    /// site echoes it back, so the agent never sees one.
    pub async fn vault_fetch(&self, caller: &Caller, req: &Value) -> Result<Value> {
        let url = req["url"].as_str().context("give url")?;
        let uri: ureq::http::Uri = url.parse().with_context(|| format!("{url} isn't a web address"))?;
        ensure!(matches!(uri.scheme_str(), Some("https" | "http")), "only http and https addresses");
        let host = uri.host().context("the address has no host")?.trim_start_matches('[').trim_end_matches(']').to_lowercase();
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("https") { 443 } else { 80 });
        ensure!(!crate::outbound::own_port(self, &host, port), "that's this server's own port");
        ensure!(!self.names_app(&host), "{host} is an app's name, not a site's");
        let used = std::sync::Mutex::new(Vec::<String>::new());
        let fill = |text: &str| {
            vault::fill(text, &host, |name| {
                let found = self.vault_for(crate::agents::NAME, &caller.user, name);
                if let Some((hosts, value)) = &found {
                    if !hosts.is_empty() {
                        used.lock().unwrap().push(value.clone());
                    }
                }
                found
            })
        };
        let filled = fill(url)?;
        let mut headers = Vec::new();
        for (name, value) in req["headers"].as_object().into_iter().flatten() {
            headers.push((name.clone(), fill(value.as_str().context("a header's value is text")?)?.into_bytes()));
        }
        let body = match &req["body"] {
            Value::Null => Vec::new(),
            Value::String(text) => fill(text)?.into_bytes(),
            other => fill(&other.to_string())?.into_bytes(),
        };
        let method = req["method"].as_str().unwrap_or(if body.is_empty() { "GET" } else { "POST" }).to_uppercase();
        let (status, headers, bytes) = tokio::task::spawn_blocking(move || crate::outbound::send(&method, &filled, headers, body)).await??;
        let secrets = used.into_inner().unwrap();
        let blot = |text: &str| secrets.iter().filter(|s| !s.is_empty()).fold(text.to_string(), |t, s| t.replace(s.as_str(), "••••"));
        let headers: serde_json::Map<String, Value> = headers.iter().map(|(n, v)| (n.clone(), json!(blot(&String::from_utf8_lossy(v))))).collect();
        const SHOWN: usize = 1 << 20;
        let cut = bytes.len() > SHOWN;
        let shown = &bytes[..bytes.len().min(SHOWN)];
        let mut out = json!({ "status": status, "headers": headers });
        match std::str::from_utf8(shown) {
            Ok(text) => out["body"] = json!(blot(text)),
            Err(_) => out["body_base64"] = json!(data_encoding::BASE64.encode(shown)),
        }
        if cut {
            out["cut"] = json!(format!("only the first {SHOWN} of {} bytes", bytes.len()));
        }
        Ok(out)
    }

    /// The items `app` is switched on for in the scopes `user` may use, as its calls see them (`ctx.vault.items()`):
    /// the same ones its placeholders are filled from, the user's own first, then their groups', then `global`'s.
    fn vault_of(&self, app: &str, user: &str) -> Result<Vec<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let mut scopes: Vec<(Scope, &ScopeState)> = i.views.iter().filter_map(|(log, (_, s))| Scope::parse(log).ok().filter(|sc| m.may_use(user, sc)).map(|sc| (sc, s))).collect();
            scopes.sort_by_key(|(sc, _)| match sc {
                Scope::User(_) => 0,
                Scope::Group(_) => 1,
                Scope::Global => 2,
            });
            let mut out = Vec::new();
            for (scope, state) in scopes {
                let mut mine: Vec<(&String, &vault::Item)> = state.items.iter().filter(|(_, it)| it.apps.contains(app)).collect();
                mine.sort_by_key(|(_, it)| it.made);
                out.extend(mine.into_iter().map(|(id, it)| vault::for_app(id, &scope.to_string(), it)));
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
                    let has = |fields: &[String]| fields.iter().all(|f| it.fields.contains_key(f));
                    let wanted: Vec<&String> = needs.iter().filter(|(_, _, fields, _)| has(fields)).map(|(app, ..)| app).collect();
                    // An account: what a sign-in made, by an app that asks for accounts with those fields.
                    let account = needs.iter().any(|(app, _, fields, accounts)| *accounts && it.app.as_ref() == Some(app) && has(fields));
                    let mut wanted_by: Vec<&String> = wanted.into_iter().chain(&it.apps).collect();
                    wanted_by.sort();
                    wanted_by.dedup();
                    out.push(json!({
                        "id": id, "scope": log, "scope_name": scope_name, "title": it.title, "type": it.kind, "domain": it.domain,
                        "hosts": it.hosts, "fields": vault::fields_shown(it), "apps": it.apps, "wanted_by": wanted_by, "app": it.app, "account": account,
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
                if let Some((image, kind)) = self.logo(&domain, &app) {
                    (item["image"], item["image_kind"]) = (json!(image), json!(kind));
                }
            }
        }
        Ok(out)
    }

    /// What the apps installed for `viewer` ask the Vault for: (app, need, its fields, whether it's accounts), from
    /// their `vault`.
    async fn vault_needs(&self, viewer: &Caller) -> Result<Vec<(String, String, Vec<String>, bool)>> {
        let mut out = Vec::new();
        for app in self.apps(viewer).await? {
            let (Some(name), Some(needs)) = (app["name"].as_str(), app["vault"].as_object()) else { continue };
            if app["installed"].as_array().is_none_or(|i| i.is_empty()) {
                continue;
            }
            for (need, d) in needs {
                let fields: Vec<String> = d["fields"].as_object().map(|f| f.keys().cloned().collect()).unwrap_or_default();
                if !fields.is_empty() {
                    out.push((name.to_string(), need.clone(), fields, d["accounts"] == true));
                }
            }
        }
        Ok(out)
    }

    /// Makes (with no `id`) or changes an item in `scope`, for a person or agent (`caller`) → its id.
    pub async fn set_item(&self, caller: &Caller, scope: &Scope, mut data: Value) -> Result<String> {
        let id = data["id"].as_str().map(str::to_string).unwrap_or_else(new_id);
        data["id"] = json!(id);
        vault::generate(&mut data);
        // Only the node says which app's call made an item.
        data.as_object_mut().context("an item is an object")?.remove("app");
        let content = self.with(|i| {
            // Agents use an item (vault_fetch) only once its person lets agents use it, never by themselves.
            let had = i.views.get(&scope.to_string()).and_then(|(_, s)| s.items.get(&id)).is_some_and(|it| it.apps.contains(crate::agents::NAME));
            let wants = data["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == crate::agents::NAME));
            ensure!(caller.agent.is_none() || had || !wants, "only the person lets agents use an item: ask them with vault_request (app agents), and they pick or make it");
            item_entry(i, &caller.user, scope, data)
        })?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        Ok(id)
    }

    /// Asks a person for an item `app` may use, in `scope`, for `caller` (an agent, or an app's page), who never sees
    /// its secret fields: what `app` declares as `need`, or `ask`'s `{ title, fields, hosts }`. → `{ id, link, scope }`.
    pub async fn request_item(&self, caller: &Caller, scope: &Scope, app: &str, need: Option<&str>, ask: &Value, why: &str) -> Result<Value> {
        let mut data = match need {
            Some(need) if app != members::APP => {
                let d = self.0.apps.describe(app).await.ok().map(|d| d["vault"][need].clone()).filter(|d| d.is_object()).with_context(|| format!("{app} asks the Vault for no {need}"))?;
                let fields: serde_json::Map<String, Value> = d["fields"].as_object().into_iter().flatten().map(|(n, f)| (n.clone(), json!({ "secret": f["secret"] == true }))).collect();
                json!({ "title": d["title"], "fields": fields, "hosts": d["hosts"], "need": need })
            }
            _ => json!({ "title": ask["title"], "fields": ask["fields"], "hosts": ask["hosts"], "need": need }),
        };
        let id = new_id();
        data["id"] = json!(id);
        data["app"] = json!(app);
        data["why"] = json!(why);
        data["agent"] = json!(caller.agent);
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, scope), "you can't change the Vault in {scope}");
            ensure!(i.r.members.apps.get(app).is_some_and(|a| !a.erased) || app == members::APP, "no app named {app}");
            Ok(())
        })?;
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, members::node("vault-request", data)).await?;
        // At the install's address (where its kernel signs people in) the web client opens it in any browser; an
        // install without one has only its apps to.
        let web = self.with(|i| Ok(crate::state::settings(&i.r.members).remove("address")))?;
        let link = match web.as_ref().and_then(Value::as_str).filter(|w| w.starts_with("https://") || w.starts_with("http://")) {
            Some(web) => format!("{}/vault?id={id}&scope={scope}", web.trim_end_matches('/')),
            None => format!("{}://vault?id={id}&scope={scope}", crate::config::CLI),
        };
        Ok(json!({ "id": id, "scope": scope.to_string(), "link": link }))
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
                    let mut data = json!({ "id": new_id(), "title": title, "fields": fields, "hosts": r.hosts, "apps": [r.app], "request": id });
                    // What the node asked for itself (the install's storage) is an item of that type.
                    if let Some(need) = r.need.as_ref().filter(|_| r.app == members::APP) {
                        data["type"] = json!(need);
                    }
                    data
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
                        "hosts": r.hosts, "why": r.why, "by": name(&r.user), "agent": r.agent, "time": r.time, "state": state_now,
                        "answered": answered, "matches": matches,
                    }));
                }
            }
            out.sort_by_key(|v| std::cmp::Reverse(v["time"].as_u64().unwrap_or(0)));
            Ok(out)
        })
    }

    /// The Vault's routes, `/v1/vault…`, for the local API and clients alike; `body` is a GET's query or a POST's body.
    /// None for another path.
    pub async fn vault_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        let scope = |default: Option<&str>| body["scope"].as_str().or(default).map(|s| self.scope_for(me, s)).transpose();
        let me = &Caller { agent: body["agent"].as_str().map(str::to_string).or(me.agent.clone()), ..me.clone() };
        Ok(Some(match (method, path) {
            ("GET", "/v1/vault") => json!(self.vault(me, scope(None)?.as_ref()).await?),
            ("POST", "/v1/vault") => {
                let scope = scope(Some("user"))?.unwrap();
                let mut data = body.clone();
                data.as_object_mut().context("an item is an object")?.remove("scope");
                json!({ "id": self.set_item(me, &scope, data).await? })
            }
            ("GET", "/v1/vault/requests") => json!(self.item_requests(me, scope(None)?.as_ref(), body["id"].as_str())?),
            ("POST", "/v1/vault/request") => {
                let scope = scope(Some("user"))?.unwrap();
                self.request_item(me, &scope, body["app"].as_str().unwrap_or(""), body["need"].as_str(), body, body["why"].as_str().unwrap_or("")).await?
            }
            ("POST", "/v1/vault/answer") => {
                self.answer_request(me, &scope(Some("user"))?.unwrap(), body["id"].as_str().unwrap_or(""), body).await?;
                json!({})
            }
            ("POST", "/v1/vault/fetch") => self.vault_fetch(me, body).await?,
            _ => return Ok(None),
        }))
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

    /// Runs an app's hook for an agent harness on this machine. The node remembers, per
    /// session, when the hook last ran, and tells the app as `since` (null on a session's first call).
    pub async fn hook(&self, app: &str, name: &str, event: Value) -> Result<Value> {
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let key = format!("{app}/{name}/{session}");
        let since = seen.get(&key).cloned().unwrap_or(Value::Null);
        let now = now_ms();
        let caller = Caller { key: self.0.id, user: self.user()?, agent: None, token: None };
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
            // A sidecar's screen, for now: its `_screen` operation gives the body, wherever the sidecar runs.
            false => {
                let input = json!({ "name": name, "params": params });
                let req = CallRequest { app: app.into(), sidecar: Some(app.into()), op: "_screen".into(), input, ..Default::default() };
                let body = self.route(req, viewer.clone()).await?;
                body.as_str().context("the screen isn't text")?.to_string()
            }
        };
        self.page(&viewer, json!({ "app": app, "screen": name, "params": params, "scope": scope }), &body)
    }

    /// A client's own screen (`crate::screen::client_screen`) as a whole page, like an app's but no app's.
    pub fn client_screen(&self, viewer: Caller, name: &str, params: Value) -> Result<String> {
        self.page(&viewer, json!({ "screen": name, "params": params }), crate::screen::client_screen(name))
    }

    /// A screen's page: its body with the design kit, the bridge, and `window.__one`, which is `context` and who's
    /// looking.
    fn page(&self, viewer: &Caller, mut context: Value, body: &str) -> Result<String> {
        context["me"] = json!(self.me_for(viewer)?);
        let made_here = self.with(|i| Ok(crate::state::settings(&i.r.members).remove("made_here")))?;
        let api = format!("http://localhost:{}", crate::api::port());
        context["config"] = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "api": api, "made_here": made_here });
        Ok(crate::screen::page(&context, body))
    }

    /// Every app in the install, with what each does for `viewer` where it's installed for them; and the sidecars
    /// that serve them.
    pub async fn apps(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let mut out = self.catalog(viewer, false).await?;
        let mut sidecars = Vec::new();
        if viewer.user == self.user()? {
            sidecars.extend(self.with(|i| Ok(self.sidecars_here(i)))?.into_iter().map(|d| d.2));
        }
        let remote = self.with(|i| {
            let mut d: Vec<Value> = i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.sidecars.iter()).filter(|d| d.1.contains(&viewer.user)).map(|d| d.2.clone()).collect();
            d.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_string());
            Ok(d)
        })?;
        for mut describe in sidecars.into_iter().chain(remote) {
            if out.iter().any(|a| a["name"] == describe["name"]) {
                continue;
            }
            // A sidecar serves its user, so its screens are on their own page.
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
                    match &a.source {
                        Some(link) if !link.is_empty() => info["source"] = json!(link),
                        Some(_) => {}
                        None => info["source"] = json!(self.built_in(a).and(built_in_source(&a.name))),
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
            self.with(|i| Ok(self.0.built_in.iter().filter(|(name, _)| !i.r.members.apps.contains_key(*name)).map(|(name, bundle)| json!({ "name": name, "owner": "admins", "from": null, "forks": [], "deploys": 0, "bundle": null, "history": [], "installed": [], "built_in": bundle, "source": built_in_source(name) })).collect()))?
        } else {
            Vec::new()
        };
        let mut out = Vec::new();
        for (name, mut info, mine) in listed {
            if self.0.apps.has(&name) {
                if let Ok(Value::Object(d)) = self.0.apps.describe(&name).await {
                    // A fork's module may still give its first name; the install's name for it is the one that counts.
                    let declared = Schedule::all(&Value::Object(d.clone()));
                    for (k, v) in d.into_iter().filter(|(k, _)| k != "name" && k != "schedules" && (mine || matches!(k.as_str(), "title" | "icon" | "color" | "domain" | "tagline" | "description" | "uses" | "connector" | "needs"))) {
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
            } else if let Some((image, kind)) = info["name"].as_str().and_then(|n| self.sidecar_image(n)) {
                // An app over a sidecar of its name shows the picture the sidecar gives (a Mac app's icon).
                (info["image"], info["image_kind"]) = (image, kind);
            }
        }
        Ok(out)
    }

    /// The picture a sidecar called `name` gives, here or on a server this one is connected to, and its kind.
    fn sidecar_image(&self, name: &str) -> Option<(Value, Value)> {
        self.with(|i| {
            let here = self.sidecars_here(i).into_iter().map(|d| d.2);
            let there = i.peers.iter().filter(|(k, _)| i.online(k)).flat_map(|(_, s)| s.sidecars.iter().map(|d| d.2.clone()));
            Ok(here.chain(there).find(|d| d["name"] == name && d["image"].is_string()).map(|d| (d["image"].clone(), d["image_kind"].clone())))
        })
        .ok()
        .flatten()
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

    /// What an app's latest deploy says it uses: (apps, sidecars). Waits a while for this server to run that deploy.
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

    /// What the deploy of `app` this server runs says it uses: (apps, sidecars).
    async fn uses_here(&self, app: &str) -> (Vec<String>, Vec<String>) {
        let d = if self.0.apps.has(app) { self.0.apps.describe(app).await.unwrap_or_default() } else { Value::Null };
        let list = |k: &str| d["uses"][k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        (list("apps"), list("sidecars"))
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
        if self.schedules_service().is_some() && caller.token.is_some() {
            self.sync_schedules_later(caller);
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
        let kernel = self.schedules_service().is_some();
        ensure!(!kernel || caller.token.is_some(), "sign in first ({} sign-in): schedules here run from your One App's kernel, with what your sign-in lets them use", crate::config::CLI);
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        if kernel {
            self.sync_schedules(caller).await.context("it's switched, but the schedules service didn't take it")?;
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
    /// operation of an app or sidecar the calling app uses, as its caller; `POST /blob` keeps a blob, `GET
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
                // `ctx.sidecars` says it's a sidecar's; `ctx.apps`, an app's (or, from older shims, either).
                let sidecar = req["sidecar"] == true || req["driver"] == true;
                let (apps, sidecars) = self.uses(&grant.app).await;
                let used = if sidecar { sidecars.iter().any(|u| u == app) } else { apps.iter().chain(&sidecars).any(|u| u == app) };
                ensure!(used, "{} doesn't use {app}", grant.app);
                ensure!(grant.depth < MAX_DEPTH, "{} called {app} {} calls deep, and apps call each other at most {MAX_DEPTH} deep: is it looping?", grant.app, grant.depth + 1);
                self.count(&grant.app, "call").await?;
                // An app may say which server runs it, like the one with a local model.
                let server = req["server"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                let call = CallRequest { app: app.into(), sidecar: sidecar.then(|| app.into()), op: op.into(), input: req["input"].clone(), server, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("POST", "/query") => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let params = req["params"].as_array().cloned().unwrap_or_default();
                let (db, bucket) = self.call_database(&grant.caller, &scope, &grant.app, DATABASE, None).await?;
                let mut db = db.lock().await;
                // Once a call, so it reads one state of the scope throughout.
                if !grant.caught_up.contains(&scope) {
                    self.read_ready(&(scope.clone(), format!("{}/{DATABASE}", grant.app)), &mut db, &bucket).await?;
                    if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                        g.caught_up.push(scope.clone());
                    }
                }
                ok(json!({ "ok": db.query(req["sql"].as_str().unwrap_or(""), &params)? }))
            }
            ("GET", "/vault") => ok(json!(self.vault_of(&grant.app, &grant.caller.user)?)),
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
                // A call this machine's own programs asked may read what its local API would give them: a file put there to
                // attach. A sign-in's asks here too, but reads only what its person may.
                let local = grant.caller.key == self.0.id && grant.caller.token.is_none() && release::blob_path(&self.0.dir, hash).is_ok_and(|p| p.exists());
                ensure!(built_in || local || grant.puts.iter().any(|h| h == hash) || self.may_read_blob(&grant.caller.user, hash)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            ("POST", "/appends") => {
                ensure!(self.0.later.lock().unwrap().contains_key(token), "only work a call left running after it returned hands in appends later");
                let req: Value = serde_json::from_slice(&body)?;
                let appends = crate::call::appends(&req["appends"])?;
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
            // A push to the caller's phones, from this app: the install delivers it through the sidecar its `push`
            // setting names, wherever that runs (the server with Apple's push key); with push off, it isn't sent.
            ("POST", "/notify") => {
                ensure!(!grant.read_only, "this call only reads: it can't push");
                let push = self.with(|i| Ok(crate::state::settings(&i.r.members).remove("push").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()))?;
                if push.is_empty() {
                    return Ok(ok(json!({ "ok": { "sent": false, "why": "push is off in this install" } })));
                }
                let input: Value = serde_json::from_slice(&body)?;
                let call = CallRequest { app: push.clone(), sidecar: Some(push), op: "send".into(), input, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("GET", "/servers") => ok(self.state_for(&grant.caller)["install"]["servers"].take()),
            // Where things run first, as the person the call is for: only an admin changes it.
            ("GET", "/use") => {
                let install = self.state_for(&grant.caller)["install"].take();
                let mut order: Vec<Value> = install["servers"].as_array().cloned().unwrap_or_default().into_iter().map(|s| json!({ "name": s["name"], "key": s["key"], "kind": s["kind"], "use": s["use"], "online": s["online"], "here": s["me"] })).collect();
                if let Some(use_) = install["serverless"]["use"].as_str() {
                    order.push(json!({ "name": "Cloudflare", "key": rank::CLOUDFLARE, "kind": rank::CLOUDFLARE, "use": use_, "online": true, "here": false }));
                }
                let set = self.with(|i| Ok(rank::set(&i.r.members.settings)))?;
                order.sort_by_cached_key(|s| (rank::order(&set, s["key"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase()));
                ok(json!({ "order": order, "admin": install["role"] == "admin" }))
            }
            ("POST", "/use") => {
                ensure!(!grant.read_only, "this call only reads: it can't change where things run");
                let req: Value = serde_json::from_slice(&body)?;
                self.set_use(&grant.caller, req["server"].as_str().unwrap_or(""), req["use"].as_str().unwrap_or("")).await?;
                ok(json!({}))
            }
            _ => bail!("the node has no {method} {path}"),
        })
    }

    /// Sets one of the install's settings, for an admin; `None` puts back the white label's default.
    pub async fn set_setting(&self, caller: &Caller, name: &str, value: Option<&str>) -> Result<()> {
        self.member_as(caller, "setting", json!({ "name": name, "value": value })).await
    }

    /// Puts a server, or the serverless node (`cloudflare`), first, when needed or last in the install's order of where
    /// things run, for an admin: the `use` setting, every other server's place written down as it is now.
    pub async fn set_use(&self, caller: &Caller, server: &str, place: &str) -> Result<()> {
        let place = rank::Use::parse(place).context("a place is first, needed or last")?;
        let id = if server.eq_ignore_ascii_case(rank::CLOUDFLARE) { rank::CLOUDFLARE.to_string() } else { hex(&self.find_server(server)?) };
        let here = crate::load::current().kind;
        let value = self.with(|i| {
            let m = &i.r.members;
            let kind = |k: &Key| if *k == self.0.id { here.clone() } else { i.peers.get(k).and_then(|p| p.load.as_ref()).map(|l| l.kind.clone()).unwrap_or_default() };
            let mut now: Vec<(String, String)> = m.servers().map(|s| (hex(&s.key), kind(&s.key))).collect();
            if m.settings.contains_key("serverless") {
                now.push((rank::CLOUDFLARE.into(), rank::CLOUDFLARE.into()));
            }
            Ok(rank::with(&m.settings, &now, &id, place))
        })?;
        self.set_setting(caller, rank::SETTING, Some(&value)).await
    }

    /// Everything a shell needs to draw this server and its install, for this server's local user.
    pub fn state(&self) -> Value {
        let user = self.user().unwrap_or_default();
        self.state_for(&Caller { key: self.0.id, user, agent: None, token: None })
    }

    /// The state as `viewer`, this server or a client it serves, sees it.
    pub fn state_for(&self, viewer: &Caller) -> Value {
        self.state_as(viewer, None)
    }

    /// The same for someone signed in with the kernel through `client` (its name), which is no device: not the
    /// server it reached, as the serverless node says too.
    pub fn state_for_sign_in(&self, viewer: &Caller, client: &str) -> Value {
        self.state_as(viewer, Some(client))
    }

    fn state_as(&self, viewer: &Caller, sign_in: Option<&str>) -> Value {
        // The device looking, if it's one.
        let device = sign_in.is_none().then_some(viewer.key);
        let ready = self.ready();
        let waiting = self.item_requests(viewer, None, None).map(|r| r.iter().filter(|r| r["state"] == "waiting").count()).unwrap_or(0);
        let arrangements = self.arrangements(&viewer.user);
        let install = self.0.install.lock().unwrap();
        let viewer_name = if viewer.key == self.0.id { self.server_name() } else { install.as_ref().and_then(|i| i.r.members.machine(&viewer.key)).map(|m| m.name.clone()).unwrap_or_default() };
        let backups = install.as_ref().and_then(|i| self.backup_problem(i, &viewer.user));
        let here_sidecars = install.as_ref().map(|i| self.sidecars_here(i)).unwrap_or_default();
        let me = &viewer.user;
        let server_now = |s: &members::Machine| {
            let Some(i) = install.as_ref() else { return serde_json::Map::new() };
            let m = &i.r.members;
            let session = i.peers.get(&s.key).filter(|_| i.online(&s.key));
            // The containers on a server are its owners' business.
            let load = if s.key == self.0.id { Some(crate::load::current()) } else { session.and_then(|s| s.load.clone()) }.map(|mut l| {
                if !m.owners(&s.key).iter().any(|o| m.acts_for(me, o)) {
                    l.containers.clear();
                }
                l
            });
            let sidecars = if s.key == self.0.id { here_sidecars.as_slice() } else { session.map(|s| s.sidecars.as_slice()).unwrap_or_default() };
            let now = json!({
                "kind": load.as_ref().map(|l| l.kind.clone()), "far": i.far.get(&s.key).is_some_and(|d| *d > FAR),
                "me": Some(s.key) == device, "here": s.key == self.0.id, "online": s.key == self.0.id || session.is_some(),
                "build": i.builds.get(&s.key).map(|r| &r.commit), "target": i.builds.get(&s.key).map(|r| &r.target),
                // When it last sent another server a step that wasn't the bucket's.
                "sent_wrong": i.status.get(&Scope::Global.to_string()).and_then(|st| st.insights.get(members::APP)).and_then(|ins| ins.errors.iter().filter(|((op, _), f)| op == "changed" && f.user == hex(&s.key)).map(|(_, f)| f.last).max()),
                "sidecars": sidecars.iter().map(|d| d.0.clone()).collect::<Vec<_>>(),
                "installed_sidecars": installed_sidecars(m, s, sidecars, i.builds.get(&s.key).map(|r| r.commit.as_str())),
                "load": load,
            });
            match now {
                Value::Object(now) => now,
                _ => unreachable!(),
            }
        };
        let client_now = |c: &members::Machine| {
            let asked = install.as_ref().and_then(|i| i.asked.get(&c.key));
            match json!({ "me": Some(c.key) == device, "asked": asked }) {
                Value::Object(now) => now,
                _ => unreachable!(),
            }
        };
        let here = crate::state::Here {
            me: match sign_in {
                None => json!({ "key": hex(&viewer.key), "name": viewer_name, "user": viewer.user }),
                Some(client) => json!({ "key": "", "name": client, "user": viewer.user }),
            },
            client: (sign_in.is_none() && viewer.key != self.0.id).then(|| json!({ "key": hex(&viewer.key), "name": viewer_name })),
            server: json!({ "key": hex(&self.0.id), "name": self.server_name(), "ready": ready, "guide": crate::skills::address() }),
            member: install.as_ref().is_some_and(|i| i.r.members.server(&self.0.id).is_some()),
            entries: install.as_ref().map(|i| i.r.store.count().unwrap_or(0) as u64).unwrap_or(0),
            waiting,
            arrangements: json!(arrangements),
            backups,
            guide: crate::skills::address(),
            server_now: &server_now,
            client_now: &client_now,
        };
        crate::state::state(install.as_ref().map(|i| &i.r.members), me, here)
    }

    // Members.

    /// Writes one of `global`'s node entries as the local user, after checking it would count.
    pub async fn member(&self, kind: &str, data: Value) -> Result<()> {
        self.member_as(&Caller { key: self.0.id, user: self.user()?, agent: None, token: None }, kind, data).await
    }

    /// The same, as `caller`, whose rights it's checked against.
    pub async fn member_as(&self, caller: &Caller, kind: &str, data: Value) -> Result<()> {
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
            let me = Caller { key: caller, user, agent: None, token: None };
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
                // An app's call runs here or the client tries another server; a sidecar's goes where the sidecar is.
                match req.sidecar.is_none() {
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
            (m, p) if p.starts_with("/v1/backups") => self.backups_route(me, m, p, &body).await?.with_context(|| format!("no {m} {p}"))?,
            (m, p) if p.starts_with("/v1/vault") => self.vault_route(me, m, p, &body).await?.with_context(|| format!("no {m} {p}"))?,
            ("POST", "/v1/name") => {
                let name = body["name"].as_str().unwrap_or("").trim();
                self.member_as(me, "rename", json!({ "key": hex(&me.key), "name": name })).await?;
                json!({})
            }
            ("POST", "/v1/pair") => {
                let (code, user) = self.pair_as(me, new_user(&body)?, body["user"].as_str())?;
                json!({ "code": code, "user": user, "qr": qr(&code)? })
            }
            ("POST", "/v1/trust") => {
                let server = self.find_server(body["server"].as_str().unwrap_or(""))?;
                let kind = if body["trust"] == false { "untrust" } else { "trust" };
                self.member_as(me, kind, json!({ "server": hex(&server) })).await?;
                json!({})
            }
            ("POST", "/v1/use") => {
                self.set_use(me, body["server"].as_str().unwrap_or(""), body["use"].as_str().unwrap_or("")).await?;
                json!({})
            }
            ("GET", p) if p.starts_with("/v1/client-screens/") => json!({ "html": self.client_screen(me.clone(), &p["/v1/client-screens/".len()..], body["params"].clone())? }),
            ("GET", p) if p.starts_with("/v1/blobs/") => {
                json!({ "blob": data_encoding::BASE64.encode(&self.blob_for(me, &p["/v1/blobs/".len()..]).await?) })
            }
            _ => bail!("no such request: {method} {path}"),
        })
    }

    // Pairing.

    /// Makes a one-time pairing code for another of the local user's machines, or, for an admin, a new user's.
    pub fn pair(&self, new_user: Option<(String, Role)>) -> Result<String> {
        Ok(self.pair_as(&Caller { key: self.0.id, user: self.user()?, agent: None, token: None }, new_user, None)?.0)
    }

    /// Like `pair`, for `asker`, or for an admin, a code for another machine of `user` (an id or a name). With the id
    /// of the user the code admits.
    pub fn pair_as(&self, asker: &Caller, new_user: Option<(String, Role)>, user: Option<&str>) -> Result<(String, String)> {
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
        // A folder on another machine isn't here: the bucket is this install's only if it says so.
        let bucket = reply.bucket.filter(|f| Place::from_fields(f).ok().and_then(|p| p.get(INSTALL_OBJECT).ok().flatten()).is_some_and(|r| r == reply.root.as_bytes()));
        if bucket.is_none() {
            tracing::warn!("this server can't reach the install's bucket: it keeps no app's data until the install moves to S3-compatible storage");
        }
        self.save_install(&InstallFile { root: reply.root, user: reply.user, bucket })?;
        self.open_install()?;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.read_logs().await {
                tracing::debug!("reading the bucket after joining: {err:#}");
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
            let root = i.r.root.map(|r| r.to_hex()).context("no install")?;
            Ok((root, i.bucket.as_ref().filter(|_| server).map(|(_, f)| f.clone())))
        })?;
        let (root, bucket) = root;
        let by = pairing.by.as_deref().map(unhex).transpose()?;
        let new_user = pairing.new_user.as_ref().map(|(name, role)| (name.as_str(), *role));
        let admit = members::admit(&key, &user, &request.name, server, new_user);
        self.member_as(&Caller { key: by.unwrap_or(self.0.id), user: pairing.user.clone(), agent: None, token: None }, "admit", admit.data).await.context("this server can no longer admit that machine")?;
        Ok(JoinReply { root, user, bucket })
    }

    // Entries in the bucket, and what servers tell each other.

    /// Reads what each scope this server holds has in the bucket after what it last read, `global` first, until it
    /// gains no scope by what it read.
    async fn read_logs(&self) -> Result<()> {
        loop {
            let scopes = self.with(|i| {
                let mut scopes: Vec<Scope> = i.r.members.scopes().into_iter().filter(|s| i.r.holds(s)).collect();
                scopes.sort_by_key(|s| *s != Scope::Global);
                Ok(scopes)
            })?;
            let mut gained = false;
            for scope in scopes {
                gained |= self.read_log(&scope).await?;
            }
            if !gained {
                break;
            }
        }
        self.with(|i| {
            i.read = true;
            Ok(())
        })
    }

    /// Reads a scope's entries in the bucket after the last this server read: whether it now holds a scope it didn't.
    async fn read_log(&self, scope: &Scope) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_log_now(scope).await
    }

    /// `read_log`, while this server holds its writes. A server that has read none of a scope's entries starts from
    /// their newest copy.
    async fn read_log_now(&self, scope: &Scope) -> Result<bool> {
        let (bucket, key, log) = (self.bucket()?, self.log_key(scope).await?, scope.to_string());
        let prefix = crate::log::prefix(&log);
        let from = self.with(|i| i.r.store.position(&log))?;
        let (at, entries) = match from {
            0 => crate::log::open(&bucket, &key, &prefix, &log).await?,
            _ => crate::log::read(&bucket, &key, &prefix, &log, from).await?,
        };
        if at == from {
            self.hint_log(scope, &bucket, &key, at).await;
            return Ok(false);
        }
        let handled = self.with(|i| {
            let handled = i.r.receive(entries, now_ms())?;
            i.r.store.set_position(&log, at)?;
            Ok(handled)
        })?;
        let gained = handled.gained;
        self.after(handled).await?;
        self.hint_log(scope, &bucket, &key, at).await;
        Ok(gained)
    }

    /// Once this server has read a scope's entries up to step `at`, and hasn't said so since it started: a copy of them
    /// in the bucket if it's due, and their newest hint moved on (`src/log.rs`).
    async fn hint_log(&self, scope: &Scope, bucket: &impl crate::bucket::Bucket, key: &[u8; 32], at: u64) {
        let log = scope.to_string();
        if self.0.hinted.lock().unwrap().get(&log) == Some(&at) {
            return;
        }
        let entries = || self.with(|i| i.r.store.scope(&log));
        match crate::log::hint(bucket, key, &crate::log::prefix(&log), at, entries).await {
            Ok(()) => drop(self.0.hinted.lock().unwrap().insert(log, at)),
            Err(err) => tracing::debug!("{log}'s copy: {err:#}"),
        }
    }

    /// Writes an entry this server made to its scope's entries in the bucket, at the first position free.
    async fn put_entry(&self, scope: &Scope, entry: &Entry) -> Result<()> {
        use crate::bucket::Bucket;
        let (bucket, key, prefix) = (self.bucket()?, self.log_key(scope).await?, crate::log::prefix(&scope.to_string()));
        let bytes = crate::entry::pack(&[entry]);
        let written = self.0.written.lock().unwrap().get(scope).copied().unwrap_or(0);
        let mut at = self.with(|i| i.r.store.position(&scope.to_string()))?.max(written);
        for _ in 0..1000 {
            at += 1;
            let name = crate::db::step_name(&prefix, at);
            if Bucket::create(&bucket, &name, &crate::db::seal(&key, &name, &bytes)).await? {
                self.0.written.lock().unwrap().insert(scope.clone(), at);
                return Ok(());
            }
        }
        bail!("{scope} changed too often to write to")
    }

    /// The key a scope's entries are sealed with in the bucket: the node's `log` of the scope.
    async fn log_key(&self, scope: &Scope) -> Result<[u8; 32]> {
        self.database_key(scope, members::APP, crate::log::NAME).await
    }

    /// Tells the servers that hold `scope` it changed, so they read it now rather than in a moment: its entries, or
    /// with `wrote`, step `at` of an app's database, which it sends, so they read it from their copy, not the bucket. In
    /// the background, and only a hint.
    fn ping(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>) {
        let Ok((servers, me)) = self.with(|i| {
            let m = &i.r.members;
            Ok((m.servers().filter(|s| s.key != self.0.id && m.holds(&s.key, scope)).map(|s| s.key).collect::<Vec<_>>(), Caller { key: self.0.id, user: i.user.clone(), agent: None, token: None }))
        }) else {
            return;
        };
        if servers.is_empty() {
            return;
        }
        let mut input = json!({ "scope": scope.to_string() });
        if let Some((app, database, at)) = wrote {
            input = json!({ "scope": scope.to_string(), "app": app, "database": database, "at": at });
            let name = crate::db::step_name(&format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string())), at);
            if let Some(step) = self.bucket().ok().and_then(|b| b.kept(&name).ok().flatten()).filter(|s| s.len() <= PINGED_STEP) {
                input["step"] = json!(data_encoding::BASE64.encode(&step));
            }
        }
        let req = CallRequest { app: members::APP.into(), op: "changed".into(), input, ..Default::default() };
        for server in servers {
            let (node, req, me) = (self.clone(), req.clone(), me.clone());
            tokio::spawn(async move {
                let _ = node.ask_server(server, &req, &me).await;
            });
        }
    }

    /// Tells every other server which sidecars serve its local user here and how loaded it is, now and then.
    async fn presence_loop(self) {
        loop {
            if self.has_install() {
                self.say_hello();
            }
            tokio::time::sleep(PRESENCE_EVERY).await;
        }
    }

    /// Says hello to every other server: as now and then, and when this server's sidecars change.
    fn say_hello(&self) {
        let Ok((servers, me, sidecars)) = self.with(|i| {
            let servers: Vec<Key> = i.r.members.servers().map(|s| s.key).filter(|k| *k != self.0.id).collect();
            Ok((servers, Caller { key: self.0.id, user: i.user.clone(), agent: None, token: None }, self.sidecars_here(i)))
        }) else {
            return;
        };
        let req = CallRequest { app: members::APP.into(), op: "hello".into(), input: json!({ "sidecars": sidecars, "load": crate::load::current() }), ..Default::default() };
        for server in servers {
            let (node, req, me) = (self.clone(), req.clone(), me.clone());
            tokio::spawn(async move {
                let _ = node.ask_server(server, &req, &me).await;
            });
        }
    }

    async fn accept_call(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: Value = serde_json::from_slice(&recv.read_to_end(MESSAGE_LIMIT).await?)?;
        let reply = match self.check_caller(remote, &request) {
            Ok(mut me) => {
                // A sign-in's token goes along with its call: one that isn't the caller's is taken as none.
                if let Some(token) = request["me"]["token"].as_str() {
                    me.token = self.token_of(token, &me.user).await;
                }
                let mut req: CallRequest = serde_json::from_value(request)?;
                // Only a server says an app made a call (whose items fill it in), and how deep it is.
                if !self.with(|i| Ok(i.r.members.trusts(&me.user, &remote)))? {
                    (req.from, req.depth) = (None, 0);
                }
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
        Ok(Caller { key, user, agent, token: None })
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

/// Checks a call may run here, and makes its context; or says why it should run elsewhere.
fn prepare(node: &Node, i: &Install, req: &CallRequest, me: &Caller) -> Result<std::result::Result<(Context, Vec<Scope>), String>> {
    let m = &i.r.members;
    let here = node.0.id;
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

/// Where a built-in app's code is: its folder of `apps/` in the repo releases come from, at this build's commit when
/// it's one (else main); none for a white label with no repo.
fn built_in_source(app: &str) -> Option<String> {
    let repo = crate::config::REPO;
    let commit = crate::config::BUILD.split(['-', '+']).next().filter(|c| !crate::config::BUILD.contains('+') && c.len() >= 7 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    (!repo.is_empty()).then(|| format!("https://github.com/{repo}/tree/{}/apps/{app}", commit.unwrap_or("main")))
}

/// What a server says of each sidecar serving there, as `GET /v1/state` lists it: what it is, its version (a
/// sidecar's own, or for one built into the node, the node's build), whom it serves, and who installed it.
fn installed_sidecars(m: &Members, server: &members::Machine, sidecars: &[Sidecar], build: Option<&str>) -> Vec<Value> {
    let name_of = |u: &String| m.users.get(u).map(|u| u.name.clone());
    sidecars
        .iter()
        .map(|(name, users, d)| {
            let built_in = d["built_in"] == true;
            let installed = m.sidecars.get(&(server.key, name.clone())).filter(|i| i.local);
            // Registered over the local API, a sidecar counts as installed by the server's local user.
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

/// The `sidecar` entry that removes a sidecar from a server, found by name or key prefix, or with `installed` lets it
/// register there again; for someone acting for one of the server's owners.
fn sidecar_entry(m: &Members, user: &str, server: &str, name: &str, installed: bool) -> Result<Content> {
    let found: Vec<&members::Machine> = m.servers().filter(|s| s.name.eq_ignore_ascii_case(server) || (!server.is_empty() && hex(&s.key).starts_with(server))).collect();
    let [s] = found.as_slice() else { bail!("no one server called {server}") };
    ensure!(m.owners(&s.key).iter().any(|o| m.acts_for(user, o)), "only someone acting for one of {}'s owners installs or removes its sidecars", s.name);
    Ok(members::node("sidecar", json!({ "server": hex(&s.key), "name": name, "blob": null, "local": installed })))
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
    vault::generate(&mut data);
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
pub fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
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

    /// Updates this node whenever a newer build for its platform is published, signed with the release key. A download
    /// that drops, or a release that fails to install, is tried again, waiting twice as long each time up to ten minutes.
    /// The first wait is a restart's: the servers it would come from take the release too, and restart at once.
    async fn update_loop(self) {
        let mut wait = Duration::from_secs(2);
        loop {
            let Some((r, author)) = self.wanted_release() else {
                wait = Duration::from_secs(2);
                self.0.released.notified().await;
                continue;
            };
            match self.apply(&r, author).await {
                Ok(exe) => {
                    // The servers taking a release take it from the one that published it first: it gives them a moment.
                    if author == self.0.id {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    let _ = std::fs::write(self.0.dir.join("release"), &r.blob);
                    tracing::info!("updated {} to {}; restarting", exe.display(), r.commit);
                    // Closing says so to every server connected, which would otherwise wait half a minute to notice:
                    // those taking this release from here try again, and get it once this server is back.
                    let _ = tokio::time::timeout(Duration::from_secs(2), self.0.endpoint.close()).await;
                    std::process::exit(0);
                }
                Err(err) => tracing::warn!("couldn't take the release of {}, trying again in {}s: {err:#}", r.commit, wait.as_secs()),
            }
            let _ = tokio::time::timeout(wait, self.0.released.notified()).await;
            wait = (wait * 2).min(Duration::from_secs(600));
        }
    }

    /// The newest release for this node's platform, if it's one this node should take and hasn't yet.
    fn wanted_release(&self) -> Option<(Release, Key)> {
        let (r, author) = self.with(|i| Ok(release::latest(crate::config::RELEASE_KEY, &i.r.store.scope("global")?, crate::config::TARGET))).ok()??;
        let taken = std::fs::read_to_string(self.0.dir.join("release")).is_ok_and(|m| m.trim() == r.blob);
        (release::wanted(&r, crate::config::BUILD, crate::config::COMMITTED) && !taken).then_some((r, author))
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

    /// Keeps `bytes` as a blob.
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

    /// Keeps `bytes` as a blob a sign-in put, which its person may then read back.
    pub fn put_blob_for(&self, caller: &Caller, bytes: &[u8]) -> Result<String> {
        let hash = self.put_blob(bytes)?;
        self.0.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(caller.user.clone());
        Ok(hash)
    }

    /// A blob for someone who isn't this machine's own programs: one they put, or an entry names in a scope they may use.
    pub async fn blob_for(&self, caller: &Caller, hash: &str) -> Result<Vec<u8>> {
        ensure!(self.may_read_blob(&caller.user, hash)?, "no such blob for you");
        self.blob(hash).await
    }

    fn may_read_blob(&self, user: &str, hash: &str) -> Result<bool> {
        Ok(self.0.put_by.lock().unwrap().get(hash).is_some_and(|users| users.contains(user)) || self.blob_named_for(hash, user)?)
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

    /// Every entry a built-in sidecar has in the local user's scope, oldest first.
    /// A built-in sidecar's records, in its database of this server's local user, oldest first.
    async fn records(&self, app: &str) -> Result<Vec<crate::sidecars::Record>> {
        let user = self.user()?;
        let scope = Scope::User(user);
        let db = self.database(&scope, app, SIDECAR_DATABASE, Some(vec![crate::sidecars::RECORDS.into()]), None).await?;
        let mut db = db.lock().await;
        self.read_ready(&(scope, format!("{app}/{SIDECAR_DATABASE}")), &mut db, &self.bucket()?).await?;
        let rows = db.query("select * from records order by n", &[])?;
        Ok(rows
            .into_iter()
            .map(|r| crate::sidecars::Record {
                id: r["n"].to_string(),
                kind: r["kind"].as_str().unwrap_or("").into(),
                data: serde_json::from_str(r["data"].as_str().unwrap_or("null")).unwrap_or_default(),
                time: r["time"].as_u64().unwrap_or(0),
                by: r["by"].as_str().unwrap_or("").into(),
                device: r["device"].as_str().unwrap_or("").into(),
            })
            .collect())
    }

    /// Brings this machine in line with a built-in sidecar's records, in the background.
    fn sync_built_in(&self, app: &'static str) {
        let Some(sync) = self.0.built_in_sidecars.iter().find(|n| n.name == app).and_then(|n| n.sync) else { return };
        let node = self.clone();
        tokio::spawn(async move {
            match node.records(app).await {
                Ok(records) => {
                    let _ = tokio::task::spawn_blocking(move || sync(&records)).await;
                }
                Err(err) => tracing::debug!("{app}'s records: {err:#}"),
            }
        });
    }

    /// The servers this server syncs with now, then the rest.
    fn peers(&self) -> Vec<Key> {
        self.with(|i| {
            let mut peers: Vec<Key> = i.r.members.servers().map(|m| m.key).filter(|d| *d != self.0.id).collect();
            peers.sort_by_key(|k| !i.online(k));
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

    /// Whether an entry in a scope `holds` says yes to names the blob, and that scope still needs it: a blob a scope
    /// keeps stops counting once it forgets it; one another node entry names (a deploy's bundle, a release's binary)
    /// never does.
    fn blob_named(&self, hash: &str, holds: impl Fn(&Scope) -> bool) -> Result<bool> {
        self.with(|i| {
            for log in i.r.store.logs()? {
                let forgotten = i.views.get(&log).is_some_and(|(_, s)| s.forgotten.contains(hash));
                let names = |e: &Entry| e.content.blobs.iter().any(|b| b == hash) && (!forgotten || (e.content.app == members::APP && e.content.kind != "keep"));
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

/// `text` as a QR code, an SVG with no margin, for a screen to draw on white: a pairing code, for a phone's camera.
pub fn qr(text: &str) -> Result<String> {
    Ok(qrcode::QrCode::new(text)?.render::<qrcode::render::svg::Color>().quiet_zone(false).build())
}


