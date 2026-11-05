//! A node: this device's install (a white label holds one), its replica and views, its socket to the relay, and where
//! calls run.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::apps::{Apps, Context, Me, ScopeHandle};
use crate::call::Append;
use crate::bucket::{Copied, Place};
use crate::db::Db;
use crate::entry::{Content, Entry, Hash, Key, Order, Scope, hex, new_id, short, unhex};
use crate::database::scopes::{self, ScopeDb};
use crate::members::{self, Principal, Role, ScopeState};
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
mod devices;
mod logs;
mod net;
mod pairing;
mod push;
mod releases;
mod scheduled;
mod screens;
mod sessions;
pub use databases::NotGiven;
pub use devices::{DeviceRequest, Invited};
pub use net::{alpn, relay, service_alpn};
pub use calls::Elsewhere;
pub use sessions::{Needs, client_token, passed_token};
mod services;
mod standing;
pub use standing::GRANT_DAYS;
mod state;
mod status;
mod steps;
mod storage;
pub mod trace;
mod vault;

/// How long a call may run before it's stopped.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A service's call may take longer, as a browser or a desktop starts: as long as a service is waited for.
const SERVICE_LIMIT: Duration = Duration::from_secs(600);
/// How many app-to-app calls deep a call may be: a person's call of A, A's of B, B's of C, C's of D.
pub const MAX_DEPTH: u32 = 3;
/// How many calls an app may make of other apps, or runs of its schedules, a minute on one device.
pub const RATE_LIMIT: u32 = 600;
/// How long an app that looped stays stopped on a device, unless it's deployed again sooner.
/// A device whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

/// Whether a device has no room for more: its memory, or its CPU of late, over 90% used.
fn full(load: &crate::load::Load) -> bool {
    let busy = load.cpu.iter().rev().take(3).map(|c| *c as u32).sum::<u32>() / 3;
    load.memory.is_some_and(|m| m.total > 0 && m.used * 10 > m.total * 9) || busy > 90
}

const STOPPED_FOR: u64 = 15 * 60_000;
/// How often a device reads what other devices wrote and said.
const TICK: Duration = Duration::from_secs(10);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this device's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// The largest step a notice carries: a bigger one, the devices it tells read from the bucket.
const SENT_STEP: usize = 1 << 20;
/// How long a blob a scope forgot stays in the bucket, in case an entry another device wrote names it again.
const FORGOTTEN_KEPT: Duration = Duration::from_secs(600);
/// How long a call's work may carry on after it returns (`ctx.waitUntil`).
const LATER_LIMIT: Duration = Duration::from_secs(600);
/// How many of an app's failures a device records an hour; the rest are only counted on the next hour's.
const FAILURES_KEPT: u32 = 30;
/// How long a request for an item waits for its answer.
const REQUEST_LIFE: u64 = 86_400_000;
/// How long an app's page says a deploy changed or removed one of its schedules.
const CHANGED_SHOWN: u64 = 14 * 86_400_000;
/// The object in an install's bucket that says whose it is: the install's genesis.
const INSTALL_OBJECT: &str = "install";
/// How often a device says hello to the others: which services serve there, and how loaded it is.
const PRESENCE_EVERY: Duration = Duration::from_secs(15);
/// How soon an app's deploy that couldn't be fetched is tried again.
const REFETCH_AFTER: Duration = Duration::from_secs(30);
/// What an app's own database is called: SQLite's name for a connection's first.
const DATABASE: &str = "main";
/// What a built-in service's database of its records is called, under its name.
// Named before services were called services: it holds sealed records (AGENTS.md's history among them), which only
// clients could copy under a new name.
const RECORDS_DATABASE: &str = "sidecar";

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Serialize, Deserialize)]
struct DeviceFile {
    secret: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct InstallFile {
    /// The hash of the install's genesis.
    root: String,
    /// This device's local user.
    user: String,
    /// The install's bucket, as a place's fields: a folder's `path`, or S3's. None if this device can't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
    /// The folders on this device it backs up to, by their place's id: a folder is only ever this device's own choice.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    folders: BTreeMap<String, String>,
}

/// A service serving on a device: its name, the users it serves, and what it does (its operations), which callers
/// list until apps wrap services.
pub type Service = (String, Vec<String>, Value);

/// Another device, as it last said hello: the services serving there, and how loaded it is.
struct Peer {
    services: Vec<Service>,
    load: Option<crate::load::Load>,
    /// What it can run, and each part's state (`Node::parts_here`), if it says.
    parts: Option<Value>,
    seen: Instant,
}

struct Install {
    r: Replica,
    user: String,
    /// The install's bucket, and its fields, which a device joining is given.
    bucket: Option<(Place, BTreeMap<String, String>)>,
    /// Each other device, as it last said hello.
    peers: HashMap<Key, Peer>,
    /// The build each device last said it runs.
    builds: HashMap<Key, release::Running>,
    /// What each device last said it reads of who's who (`database::switch`).
    reads: HashMap<Key, crate::database::switch::Reads>,
    /// Once `global`'s log is frozen, each other scope's own database as its steps leave it (`database::steps`): the
    /// last position read, and the database, once its anchor checked out here.
    scope_dbs: HashMap<Scope, (u64, ScopeDb)>,
    /// Each scope's rows of how things went, as this device last read them.
    status: HashMap<String, status::Status>,
    /// Each scope's views: how far they've replayed, and what the scope's node entries add up to.
    views: HashMap<String, (Order, ScopeState)>,
    /// `global`'s Vault items, as this device last read them with its own keys: what the node itself keeps there.
    vault: crate::vault::Items,
    /// The folders on this device it backs up to, by their place's id (`InstallFile::folders`).
    folders: BTreeMap<String, String>,
    /// How long each device took to answer a call lately.
    nearest: HashMap<Key, Duration>,
    /// How far each device is: how long it took to answer this device's last hello.
    far: HashMap<Key, Duration>,
    /// Whether this device has read every scope it holds from the bucket since it woke.
    read: bool,
}

impl Install {
    /// Whether another device said hello lately.
    fn online(&self, device: &Key) -> bool {
        self.peers.get(device).is_some_and(|p| p.seen.elapsed() < 3 * PRESENCE_EVERY)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    Changed { apps: Vec<String> },
    Devices,
    /// This device created or joined an install.
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
    /// The machine whose join with it didn't finish (hex), which alone may use it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tried: Option<String>,
}

/// What a machine's node is: the device its person's CLI, MCP and Mac app sign with, and compute.
pub const MACHINE: &[members::Kind] = &[members::Kind::Client, members::Kind::Compute];

const PAIRING_LIFE: u64 = 7 * 86_400_000;

#[derive(Serialize, Deserialize)]
struct PairingCode {
    /// The install's address, whose relay reaches the device that made it.
    address: String,
    from: Key,
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
    /// For a device: the install's bucket, as a place's fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
}

/// Who asked for something: this device or another, or a client, as one user.
#[derive(Clone, Debug)]
pub struct Caller {
    pub key: Key,
    pub user: String,
    /// The agent that asked through it, by name ("Claude Code"), when one did.
    pub agent: Option<String>,
    /// The kernel access token it came with, when it's a sign-in's: a call
    /// gets its keys and bucket URLs from the vault with it, and passes it on with the call to another device.
    pub token: Option<String>,
}

/// An operation to run: of an app, or of a service.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallRequest {
    /// Names this call wherever it's tried, so a call that writes runs on one device only: the first to take its lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The app, or for a service's call, the service.
    #[serde(default)]
    pub app: String,
    /// The service whose operation this is, when it is one: an app and the service it wraps may share a name.
    #[serde(default)]
    pub service: Option<String>,
    pub op: String,
    #[serde(default)]
    pub input: Value,
    /// The scopes the call touches, named before it runs; by default every scope it may reach.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// The device to run it on, by name or key prefix; by default the nearest that can.
    #[serde(default)]
    pub device: Option<String>,
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

/// What a device says to a call.
enum Reply {
    Ok(Value),
    Error(String),
    /// It can't run here; try another device.
    Elsewhere(String),
    /// It lacks keys its caller hands: said as the API says it (`{ error, need, to, seal }`), so the device that passed
    /// it on can ask for them in turn (428). An older device reads only its `error`.
    Needs(Needs),
}

impl Reply {
    fn json(self) -> Value {
        match self {
            Reply::Ok(v) => json!({ "ok": v }),
            Reply::Error(e) => json!({ "error": e }),
            Reply::Elsewhere(w) => json!({ "elsewhere": w }),
            Reply::Needs(n) => json!({ "error": n.to_string(), "need": n.need, "to": n.to, "seal": n.seal }),
        }
    }

    fn from_json(v: Value) -> Reply {
        if let Some(w) = v.get("elsewhere") {
            return Reply::Elsewhere(w.as_str().unwrap_or("").into());
        }
        if let Ok(need) = serde_json::from_value::<Vec<crate::keys::Need>>(v["need"].clone())
            && !need.is_empty()
        {
            let said = |k: &str| v[k].as_str().unwrap_or("").to_string();
            return Reply::Needs(Needs { need, to: said("to"), seal: said("seal") });
        }
        match v.get("error") {
            Some(e) => Reply::Error(e.as_str().unwrap_or("failed").into()),
            None => Reply::Ok(v["ok"].clone()),
        }
    }
}

/// A lock that refuses the thread already holding it. A std `Mutex` asked for again on the thread holding it waits on
/// itself for good, and every thread asking after that waits on it too: once those are all the runtime's workers, the
/// node takes no connection and answers nothing, though it still listens. Refused instead, and said where.
struct Exclusive<T>(Mutex<T>);

thread_local!(static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) });

/// What `Exclusive::lock` gives: the value, held until it's dropped.
struct Held<'a, T>(std::sync::MutexGuard<'a, T>, usize);

impl<T> Exclusive<T> {
    const fn new(value: T) -> Self {
        Exclusive(Mutex::new(value))
    }

    fn lock(&self) -> Result<Held<'_, T>> {
        let me = self as *const Self as usize;
        if HOLDING.with_borrow(|h| h.contains(&me)) {
            let at = std::backtrace::Backtrace::force_capture();
            tracing::error!("the install was asked for on a thread that holds it, which would stop this node answering: {at}");
            debug_assert!(false, "the install asked for on a thread that holds it: {at}");
            bail!("this device couldn't answer that: a bug in it, which its log shows");
        }
        let held = Held(self.0.lock().unwrap(), me);
        HOLDING.with_borrow_mut(|h| h.push(me));
        Ok(held)
    }
}

impl<T> std::ops::Deref for Held<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for Held<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Drop for Held<'_, T> {
    fn drop(&mut self) {
        HOLDING.with_borrow_mut(|h| h.retain(|l| *l != self.1));
    }
}

pub struct Inner {
    dir: PathBuf,
    /// What this machine's own programs send to say they are: `api::LOCAL_TOKEN`, a file only its user reads.
    local_token: String,
    key: SigningKey,
    id: Key,
    name: Mutex<String>,
    apps: Apps,
    built_in_services: Vec<crate::services::BuiltIn>,
    pub services: crate::services::Services,
    install: Exclusive<Option<Install>>,
    pairings: Mutex<HashMap<[u8; 16], Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this device.
    changed: tokio::sync::Notify,
    /// Wakes the task that updates this node to a newer release.
    released: tokio::sync::Notify,
    /// The bundle each app runs here.
    bundles: Mutex<BTreeMap<String, String>>,
    /// The bundle of each app this release has built in, kept as a blob: any install may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png`, its finished app icon, as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The pictures windows' app lists named (`/v1/images/<name>`), by name: each a data URL.
    named_images: Mutex<HashMap<String, String>>,
    /// The logos of the sites apps and services connect to.
    logos: crate::logos::Logos,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The blobs sign-ins put here (their files in an MCP answer, say), by hash: for whom, so they may read them back.
    put_by: Mutex<HashMap<String, std::collections::HashSet<String>>>,
    /// The files put from elsewhere this last hour, which count against their person's room (`put_blob_from`).
    remote_puts: Mutex<Vec<blobs::RemotePut>>,
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
    /// When this device last tried telling admins about each place's problem and reached none, so it tries hourly.
    alert_tried: Mutex<HashMap<String, u64>>,
    /// The newest snapshot this device knows of each (place, scope), with its key's id.
    backed: Mutex<HashMap<(String, String), (crate::backups::Snapshot, String)>>,
    /// When each place's last run here failed, so it's tried again an hour later, not every minute.
    backup_failed: Mutex<HashMap<String, u64>>,
    /// When this device last pruned each place, by its id.
    pruned: Mutex<HashMap<String, u64>>,
    /// When this device last copied each (place, scope)'s databases from the bucket.
    backup_looked: Mutex<HashMap<(String, String), u64>>,
    /// Each database of each scope, by its app and name (`board/main`), as this device last read it.
    dbs: Mutex<HashMap<(Scope, String), Arc<tokio::sync::Mutex<Db>>>>,
    /// The databases another device said it wrote whose step isn't in this device's copy yet, which a read catches up
    /// from the bucket.
    behind: Mutex<std::collections::HashSet<(Scope, String)>>,
    /// Each scope's epochs this device opens, by number, once it has looked.
    epochs: Mutex<HashMap<Scope, crate::db::Keys>>,
    /// Each database's keys by epoch as this device opens them (`database_keys`), with the newest epoch they're as of.
    database_keys: Mutex<HashMap<(Scope, String), (u32, crate::db::Keys)>>,
    /// Steps other devices sent, waiting for the bucket to confirm them, while one ask of it is under way: none then.
    sent: Mutex<Option<Vec<logs::Sent>>>,
    /// Whether this device is moving to the install's new storage now.
    moving: Mutex<bool>,
    /// The install's recovery keys this device told the relay about since it started: the old ones taken out, the
    /// current one let in as a client (`follow_recovery`).
    recovery_told: Mutex<std::collections::BTreeSet<Key>>,
    /// When it last read `global` for a client device it didn't list (`Node::catch_up`).
    caught_up: tokio::sync::Mutex<Option<std::time::Instant>>,
    /// Where the install's data went: every bucket this device has used writes there, after a move (`Copied`).
    moved: Arc<tokio::sync::RwLock<Option<Place>>>,
    /// Held while this device makes a scope's first epoch.
    making_keys: tokio::sync::Mutex<()>,
    /// Positions of each database (by prefix) holding a step under an epoch no entry made, which every read passes over.
    junk: Mutex<HashMap<String, std::collections::BTreeSet<u64>>>,
    /// When this device last tidied each database (by prefix), ms.
    tidied: Mutex<HashMap<String, u64>>,
    /// The last position this device wrote an entry of each scope at.
    written: Mutex<HashMap<Scope, u64>>,
    /// The step of each scope's entries this device last moved their newest hint on to, by scope.
    hinted: Mutex<HashMap<String, u64>>,
    /// The blobs this device has put in the bucket, as `<scope> <hash>`, kept in `install/shared` too.
    shared: Mutex<std::collections::HashSet<String>>,
    /// The blobs of entries this device wrote that the bucket didn't take then, tried again each minute.
    unshared: Mutex<Vec<(Scope, String)>>,
    /// Requests to join, by their device's key, that their person said weren't theirs, not shown again while it runs.
    refused: Mutex<std::collections::HashSet<String>>,
    /// Held while `install/forgotten` changes.
    forgotten: Mutex<()>,
    /// What the vault gave this device for the sign-ins its calls came with.
    sessions: sessions::Sessions,
    /// Apps whose deploy doesn't start here, by the bundle that didn't: left out until they're deployed again.
    broken: Mutex<BTreeMap<String, String>>,
    /// The keys of the apps this device keeps running for their people.
    standing: standing::Standing,
    /// What `Node::start_owed_keys` knows between its looks.
    owed: Mutex<devices::Owed>,
    /// What `Node::sign_in_loop` knows between its looks.
    sign_ins: Mutex<devices::SignIns>,
    /// This device's iroh endpoint.
    net: net::Net,
    /// How many times its deploys were settled (`settle`), the first once its apps are loaded.
    settled: tokio::sync::watch::Sender<u64>,
    /// Held while settling, so one that read the members before a change can't load its apps after one that read it.
    settling: tokio::sync::Mutex<()>,
    /// Whether the last settling couldn't fetch an app's deploy, so it's tried again soon, not only at the next change.
    unfetched: std::sync::atomic::AtomicBool,
}

/// Set once this process's node is handing over to the next: it opens nothing only one of them may hold.
static HANDING_OVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Loop guards: an app that calls other apps, or runs its schedules, more than `RATE_LIMIT` times a minute on this
/// device is stopped here for `STOPPED_FOR`, or until it's deployed again.
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
    /// The trace of the request that made the call, if it asked for one: what its app's reads add to.
    pub trace: Option<trace::Trace>,
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
    /// Starts this device's node, with the built-in services this build has.
    pub async fn start(dir: &Path, built_in_services: Vec<crate::services::BuiltIn>) -> Result<Node> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let secret = unhex(&device.secret)?;
        let key = SigningKey::from(secret);
        let id = *key.verifying_key().as_bytes();
        let outbound = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let apps = Apps::start(dir.join(crate::handoff::runtime()), outbound.local_addr()?.port()).await?;
        let (events, _) = broadcast::channel(256);
        let mut built_in = BTreeMap::new();
        for (app, files) in crate::config::BUILT_IN {
            built_in.insert(app.to_string(), release::put_blob(dir, &built_in_bundle(files)?)?);
        }
        let node = Node(Arc::new(Inner {
            dir: dir.to_path_buf(),
            local_token: load_local_token(dir)?,
            key,
            id,
            name: Mutex::new(device.name),
            apps,
            built_in_services,
            services: Default::default(),
            install: Exclusive::new(None),
            pairings: Mutex::new(load_pairings(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
            bundles: Mutex::new(BTreeMap::new()),
            built_in,
            images: Mutex::new(HashMap::new()),
            named_images: Mutex::new(HashMap::new()),
            logos: crate::logos::Logos::open(dir),
            grants: Mutex::new(HashMap::new()),
            put_by: Mutex::new(HashMap::new()),
            remote_puts: Mutex::new(Vec::new()),
            outbound: outbound.local_addr()?.port(),
            guard: Mutex::new(Guard::default()),
            later: Mutex::new(HashMap::new()),
            seen: Mutex::new(Default::default()),
            failures: Mutex::new(HashMap::new()),
            backup_now: tokio::sync::Notify::new(),
            backup_forced: Mutex::new(false),
            alert_tried: Mutex::new(HashMap::new()),
            backed: Mutex::new(HashMap::new()),
            backup_failed: Mutex::new(HashMap::new()),
            pruned: Mutex::new(HashMap::new()),
            backup_looked: Mutex::new(HashMap::new()),
            dbs: Mutex::new(HashMap::new()),
            behind: Mutex::new(Default::default()),
            epochs: Mutex::new(HashMap::new()),
            database_keys: Mutex::new(HashMap::new()),
            sent: Mutex::new(None),
            moving: Mutex::new(false),
            recovery_told: Mutex::new(Default::default()),
            caught_up: tokio::sync::Mutex::new(None),
            moved: Arc::default(),
            making_keys: tokio::sync::Mutex::new(()),
            junk: Mutex::new(HashMap::new()),
            tidied: Mutex::new(HashMap::new()),
            written: Mutex::new(HashMap::new()),
            hinted: Mutex::new(HashMap::new()),
            shared: Mutex::new(Default::default()),
            unshared: Mutex::new(Vec::new()),
            refused: Mutex::default(),
            forgotten: Mutex::new(()),
            sessions: Default::default(),
            broken: Default::default(),
            standing: Default::default(),
            owed: Default::default(),
            sign_ins: Default::default(),
            net: Default::default(),
            settled: tokio::sync::watch::channel(0).0,
            settling: tokio::sync::Mutex::new(()),
            unfetched: Default::default(),
        }));
        let outbound_router = crate::outbound::router(node.clone());
        tokio::spawn(async move {
            if let Err(err) = axum::serve(outbound, outbound_router).await {
                tracing::warn!("apps' outbound requests stopped: {err:#}");
            }
        });
        if dir.join("install").join("install.json").exists() {
            node.open_install().context("open the install")?;
            if let Err(err) = node.bucket().and_then(|b| b.forget_sent()) {
                tracing::warn!("forgetting steps other devices sent: {err:#}");
            }
            node.replay_all().await?;
            node.keep_alone_here()?;
            node.read_own_items();
            node.follow_net();
            node.read_all_status();
            let reader = node.clone();
            tokio::spawn(async move {
                if let Err(err) = reader.read_logs().await {
                    tracing::warn!("reading the bucket: {err:#}");
                }
                reader.share_blobs().await;
            });
        }
        tokio::spawn(node.clone().presence_loop());
        tokio::spawn(node.clone().tick_loop());
        tokio::spawn(node.clone().backup_loop());
        tokio::spawn(node.clone().schedule_loop());
        tokio::spawn(node.clone().update_loop());
        tokio::spawn(node.clone().sign_in_loop());
        let worker = node.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = worker.settle().await {
                    tracing::warn!("{err:#}");
                }
                worker.0.settled.send_modify(|n| *n += 1);
                match worker.0.unfetched.load(std::sync::atomic::Ordering::Relaxed) {
                    true => drop(tokio::time::timeout(REFETCH_AFTER, worker.0.changed.notified()).await),
                    false => worker.0.changed.notified().await,
                }
            }
        });
        if node.has_install() {
            for service in node.0.built_in_services.iter().filter(|n| n.sync.is_some()) {
                node.sync_built_in(service.name);
            }
        }
        Ok(node)
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    /// Waits until its apps are loaded and each has answered once, so a node taking over answers its first calls warm.
    pub async fn warm(&self) {
        let mut settled = self.0.settled.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(60), settled.wait_for(|n| *n > 0)).await;
        let mut asked = tokio::task::JoinSet::new();
        for app in self.0.apps.names() {
            let apps = self.0.apps.clone();
            asked.spawn(async move { apps.describe(&app).await });
        }
        let _ = tokio::time::timeout(Duration::from_secs(30), asked.join_all()).await;
    }

    /// Whether this node is handing over to the next: then it keeps its endpoint only until the next one is ready
    /// (`handed_over`), never binding it again; and if the next one didn't start, it keeps it as before.
    pub fn handing_over(&self, handing: bool) {
        HANDING_OVER.store(handing, std::sync::atomic::Ordering::SeqCst);
        if !handing {
            self.follow_net();
        }
    }

    /// The next node is ready, with an endpoint of this device's key of its own: this one closes its own, so the relay
    /// and other devices reach only that one.
    pub fn handed_over(&self) {
        self.close_net();
    }

    /// What this machine's own programs send to say they are (`api::LOCAL_TOKEN`).
    pub fn local_token(&self) -> &str {
        &self.0.local_token
    }

    /// Runs `f` on this device's install.
    fn with<R>(&self, f: impl FnOnce(&mut Install) -> Result<R>) -> Result<R> {
        let mut install = self.0.install.lock()?;
        f(install.as_mut().context("this device is in no install yet: create one, or join with a pairing code")?)
    }

    pub fn has_install(&self) -> bool {
        self.0.install.lock().is_ok_and(|i| i.is_some())
    }

    /// This device's local user.
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

    /// What `user` is told of `event`: of changed apps, only those they reach.
    pub fn event_for(&self, event: Event, user: &str) -> Event {
        match event {
            Event::Changed { apps } => {
                let apps = self.with(|i| Ok(apps.into_iter().filter(|a| !reachable(i, a, user).is_empty()).collect())).unwrap_or_default();
                Event::Changed { apps }
            }
            other => other,
        }
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
        let r = Replica::new(self.0.key.clone(), store, Some(Hash::from_bytes(root)), now_ms())?;
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((self.install_place(&f)?, f))).transpose()?;
        let install = Install { r, user: file.user, bucket, folders: file.folders, peers: HashMap::new(), builds: HashMap::new(), reads: HashMap::new(), scope_dbs: HashMap::new(), status: HashMap::new(), views: HashMap::new(), vault: Default::default(), nearest: HashMap::new(), far: HashMap::new(), read: false };
        *self.0.install.lock()? = Some(install);
        self.start_kept();
        Ok(())
    }

    /// An install made before it kept its data in a bucket, with no device but this one: it keeps it in a folder here.
    fn keep_alone_here(&self) -> Result<()> {
        let alone = self.with(|i| Ok(i.bucket.is_none() && i.r.db().computes().iter().all(|s| s.key == self.0.id)))?;
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

    /// Keeps (or with `None` forgets) the folder on this device that the backup place `id` is.
    fn keep_folder(&self, id: &str, path: Option<&std::path::Path>) -> Result<()> {
        let mut file: InstallFile = serde_json::from_slice(&std::fs::read(self.install_dir().join("install.json"))?)?;
        match path {
            Some(path) => drop(file.folders.insert(id.to_string(), path.display().to_string())),
            None => drop(file.folders.remove(id)),
        }
        self.save_install(&file)?;
        self.with(|i| {
            i.folders = file.folders;
            Ok(())
        })
    }

    fn save_install(&self, file: &InstallFile) -> Result<()> {
        let dir = self.install_dir();
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join("install.json"), &serde_json::to_vec_pretty(file)?)?;
        Ok(())
    }

    /// This device's name: as the install lists it, which another of its person's devices may have changed, when the
    /// install isn't being read (never waiting on it, so it's asked for anywhere); else as it last was.
    pub fn device_name(&self) -> String {
        let install = self.0.install.0.try_lock().ok();
        let listed = install.as_ref().and_then(|i| i.as_ref()).and_then(|i| i.r.db().machine(&self.0.id).map(|m| m.name));
        drop(install);
        let mut name = self.0.name.lock().unwrap();
        if let Some(listed) = listed {
            *name = listed;
        }
        name.clone()
    }

    /// Creates this device's install, with its local user as the first admin, and installs the config's apps in
    /// `global`. Returns the recovery key's secret, given only this once.
    pub async fn create(&self, name: &str, user_name: &str) -> Result<String> {
        self.create_with(name, user_name, None, crate::config::STARTERS).await
    }

    /// Creates the install, with every built-in app added (owned by admins), so anyone may add one to their own page,
    /// and these installed in `global`. Its first person is `user` if given (who they were in an install this one
    /// replaces, so their account at its kernel is still theirs), else someone new.
    pub async fn create_with(&self, name: &str, user_name: &str, user: Option<&str>, apps: &[&str]) -> Result<String> {
        self.ensure_no_install()?;
        ensure!(!name.trim().is_empty() && !user_name.trim().is_empty(), "give the install a name, and yourself one");
        ensure!(user.is_none_or(|u| u.len() == 16 && u.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))), "a person's id is 16 hex digits, like 976091fe8c23a057");
        let user = user.map_or_else(new_id, str::to_string);
        let recovery = SigningKey::from_bytes(&rand::random());
        let me = crate::keys::Endpoint::of(&self.0.key);
        // `global`'s first epoch, named by the genesis before anything is sealed under it.
        let global: [u8; 32] = rand::random();
        let genesis = members::genesis(name.trim(), &user, user_name.trim(), &me, &self.device_name(), MACHINE, recovery.verifying_key().as_bytes(), &crate::keys::epoch_hash(&global));
        let ext = crate::entry::Ext { log: "global".into(), time: now_ms(), user: user.clone(), epoch: 0, previous: Vec::new() };
        let entry = Entry::create(&self.0.key, ext, None, genesis);
        // Until it's moved to S3-compatible storage, the install's bucket is a folder here.
        let bucket: BTreeMap<String, String> = [("path".to_string(), self.install_dir().join("bucket").display().to_string())].into();
        Place::from_fields(&bucket)?.create(INSTALL_OBJECT, entry.hash().to_hex().as_bytes())?;
        self.save_install(&InstallFile { root: entry.hash().to_hex(), user: user.clone(), bucket: Some(bucket), folders: BTreeMap::new() })?;
        Store::open(&self.install_dir().join("entries.db"))?.insert_own(&entry, None)?;
        self.open_install()?;
        self.replay_all().await?;
        let bucket = self.bucket()?;
        let recovery_seal = crate::kernel::twin_public(recovery.verifying_key().as_bytes())?;
        for (to, seal) in [("recovery".to_string(), recovery_seal), (hex(&me.key), me.seal)] {
            crate::db::created(&bucket, &crate::keys::epoch_name("global", 1, &to), &crate::kernel::seal_to(&seal, &global)?).await?;
        }
        self.0.epochs.lock().unwrap().insert(Scope::Global, crate::db::Keys::from([(1, global)]));
        self.put_entry(&Scope::Global, &entry).await?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })?;
        // Its Vault and its first person's own open on this machine, so a new install fills items with no other step.
        for scope in [Scope::Global, Scope::User(user.clone())] {
            self.new_vault_key(&scope, 1, &[self.0.id], recovery.verifying_key().as_bytes()).await?;
        }
        let me = Caller { key: self.0.id, user, agent: None, token: None };
        for (app, _) in crate::config::BUILT_IN {
            self.bring_in(&me, app).await?;
        }
        for app in apps {
            self.bring_in(&me, app).await?;
            self.append(&Scope::Global, members::node("install", json!({ "app": app }))).await?;
        }
        self.settle().await?;
        self.emit(Event::Install);
        Ok(hex(&recovery.to_bytes()))
    }

    /// Writes an entry as this device's local user.
    pub async fn append(&self, scope: &Scope, content: Content) -> Result<Entry> {
        let user = self.user()?;
        self.append_as(scope, &user, content).await
    }

    /// Writes an entry to this device's log of `scope`, signed by this device, as `user`.
    async fn append_as(&self, scope: &Scope, user: &str, content: Content) -> Result<Entry> {
        // Once `global`'s log is frozen, what it said is a step of its scope's database (`database::steps`).
        if self.frozen() {
            return match scope {
                Scope::Global => self.write_step(user, content).await,
                _ => self.write_scope_step(scope, user, content).await,
            };
        }
        self.first_epoch(scope).await?;
        let _write = self.0.writes.lock().await;
        // What's in the bucket first, so the entry comes after all of it.
        self.read_log_now(scope).await?;
        // Its blobs first, so a device that reads the entry finds them; one the bucket doesn't take now is tried again
        // each minute.
        for blob in &content.blobs {
            if let Err(err) = self.share_blob(scope, blob, false).await {
                tracing::warn!("putting blob {} in the bucket, to try again: {err:#}", &blob[..12.min(blob.len())]);
                self.0.unshared.lock().unwrap().push((scope.clone(), blob.clone()));
            }
        }
        let entry = self.with(|i| i.r.make(scope, user, content, now_ms()))?;
        // An entry that wouldn't count is never written: it would say it was done.
        self.with(|i| i.r.check(&entry))?;
        self.check_in_scope(&entry)?;
        let at = self.put_entry(scope, &entry).await?;
        let handled = self.with(|i| i.r.add_own(&entry, now_ms()))?;
        self.after(handled).await?;
        self.ping(scope, None);
        // The log froze as it was written, without it: a step, then.
        if self.left_out(&entry, at).await? {
            drop(_write);
            return Box::pin(self.append_as(scope, user, entry.content.clone())).await;
        }
        Ok(entry)
    }

    /// A scope's first epoch, before anything is written there, made outside this device's writes, since its entry is
    /// one: a scope with none has nothing to read or write until it has.
    async fn first_epoch(&self, scope: &Scope) -> Result<()> {
        if *scope != Scope::Global && self.with(|i| Ok(i.r.db().epoch(scope)))? == 0 {
            self.epoch_keys(scope, true).await?;
        }
        Ok(())
    }

    /// One of a scope's node entries that its replay would skip is never written: it would say it was done. In
    /// `global` too, for those only a scope's replay checks.
    fn check_in_scope(&self, entry: &Entry) -> Result<()> {
        if entry.content.app == members::APP && !members::is_member_entry(entry) {
            self.with(|i| members::applies_in_scope(&i.r.members, &i.views.get(entry.log()).map(|(_, s)| s.clone()).unwrap_or_default(), entry))?;
        }
        Ok(())
    }

    /// What a device signing an entry in `scope` for `caller` needs (`src/entry.rs`): the scope's `epoch`, the
    /// `previous` it follows (every log's end there, and in `global`), and each log's end there by author (`logs`, its
    /// own log's the backlink).
    pub async fn entry_heads(&self, caller: &Caller, scope: &Scope) -> Result<Value> {
        self.read_callers_logs(caller).await?;
        self.read_log(scope).await?;
        self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "use", &scopes::object(scope)) && i.r.holds(scope), "this device doesn't hold {scope} for you");
            let log = scope.to_string();
            let mut previous = i.r.store.heads(&log, None)?;
            if *scope != Scope::Global {
                previous.extend(i.r.store.heads("global", None)?);
            }
            let mut logs = serde_json::Map::new();
            for (author, _, next) in i.r.store.frontier()?.into_iter().filter(|(_, l, _)| *l == log) {
                if let Some(e) = i.r.store.last(&author, &log)? {
                    logs.insert(hex(&author), json!({ "seq": e.seq(), "hash": e.hash().to_hex(), "next": next }));
                }
            }
            Ok(json!({ "epoch": i.r.db().epoch(scope), "previous": previous.iter().map(Hash::to_hex).collect::<Vec<_>>(), "logs": logs }))
        })
    }

    /// `global`'s entries and its forks' evidence, packed, but those of each author's log below the seq `have` says the
    /// caller holds: what a client device replays itself to know who's who (one-client's `members`), never taking it
    /// from a node. None if there's nothing new.
    pub async fn global_entries(&self, caller: &Caller, have: &HashMap<Key, u32>) -> Result<Option<Vec<u8>>> {
        // Those it holds while its newest epoch is on its way: the device that just made it asks before sealing it.
        if let Err(err) = self.read_log(&Scope::Global).await
            && !err.is::<databases::NotGiven>()
        {
            return Err(err);
        }
        self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "use", scopes::INSTALL), "you aren't one of this install's people");
            let new: Vec<Entry> = i.r.store.scope("global")?.into_iter().filter(|e| have.get(&e.author()).is_none_or(|n| e.seq() >= *n)).collect();
            if new.is_empty() {
                return Ok(None);
            }
            let forked = i.r.store.forked("global")?;
            Ok(Some(crate::entry::pack(&new.iter().chain(&forked).collect::<Vec<_>>())))
        })
    }

    /// Writes entries `caller`'s own device signed (a browser's, say), each once it's checked: it follows what this
    /// device holds, and counts.
    pub async fn put_signed(&self, caller: &Caller, entries: Vec<Entry>) -> Result<()> {
        for entry in &entries {
            ensure!(entry.user() == caller.user, "entry {} isn't written as you", entry.id());
        }
        self.read_callers_logs(caller).await?;
        self.put_checked(entries).await
    }

    /// Writes entries another key signed, each once it's checked: it follows what this device holds, and counts.
    pub(super) async fn put_checked(&self, entries: Vec<Entry>) -> Result<()> {
        for entry in entries {
            let scope = entry.scope();
            // An older device's, which writes to the frozen logs, where it changes nothing.
            ensure!(!self.frozen(), "update this device to change who's in: your One App's other devices have moved on");
            self.first_epoch(&scope).await?;
            {
                let _write = self.0.writes.lock().await;
                self.read_log_now(&scope).await?;
                for blob in &entry.content.blobs {
                    self.share_blob(&scope, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
                }
                self.with(|i| i.r.check_signed(&entry))?;
                self.check_in_scope(&entry)?;
                let at = self.put_entry(&scope, &entry).await?;
                let handled = self.with(|i| i.r.receive(vec![entry.clone()], now_ms()))?;
                self.after(handled).await?;
                ensure!(!self.left_out(&entry, at).await?, "update this device to change who's in: your One App's other devices moved on as it wrote that");
            }
            self.ping(&scope, None);
        }
        Ok(())
    }

    /// Passes on new entries, follows what changed in the members, and brings each scope's state up to date.
    async fn after(&self, handled: Handled) -> Result<()> {
        if handled.new.is_empty() && handled.dropped.is_empty() && !handled.members {
            return Ok(());
        }
        self.with(|i| {
            if handled.members {
                let db = i.r.db();
                i.peers.retain(|peer, _| db.compute(peer).is_some() || db.service(peer).is_some());
            }
            for log in &handled.dropped {
                i.views.remove(log);
            }
            if handled.recount {
                i.views.clear();
                let all = i.r.store.ordered()?;
                return update_views(i, &all);
            }
            update_views(i, &handled.new)
        })?;
        // A scope this device no longer holds: it forgets what it read of it.
        self.close_dbs(|scope, _| handled.dropped.contains(&scope.to_string()));
        if let Ok(bucket) = self.bucket() {
            for log in &handled.dropped {
                if let Err(err) = bucket.forget(&crate::bucket::folder(log)) {
                    tracing::warn!("forgetting this device's copy of {log}: {err:#}");
                }
            }
        }
        if handled.new.iter().any(|e| e.content.app == release::APP && e.content.kind == "node-release") {
            self.0.released.notify_one();
        }
        if !handled.lost.is_empty() {
            self.forget_sessions_of(&handled.lost);
        }
        if handled.members {
            // A device removed keeps nothing it handed this one, nor its connections.
            self.forget_removed_clients();
            self.close_removed();
            self.0.changed.notify_one();
            self.emit(Event::Devices);
            self.seal_epochs();
            self.settle_own();
            self.follow_recovery();
        }
        if handled.members || handled.new.iter().any(|e| e.log() == "global" && e.content.app == members::APP) {
            self.follow_storage();
            self.follow_net();
        }
        let forgotten: Vec<(String, String)> = handled.new.iter().filter(|e| e.content.app == members::APP && e.content.kind == "forget").filter_map(|e| Some((e.log().to_string(), e.content.data["blob"].as_str()?.to_string()))).collect();
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

    /// Brings this device in line with the members: runs each app's latest deploy, fetching bundles it lacks, and
    /// says which build it runs.
    async fn settle(&self) -> Result<()> {
        let _settling = self.0.settling.lock().await;
        let Ok((wanted, author)) = self.with(|i| {
            let wanted: BTreeMap<String, String> = i.r.db().apps().into_iter().filter(|a| !a.erased).filter_map(|a| Some((a.name, a.bundle?))).collect();
            Ok((wanted, i.r.db().compute(&self.0.id).is_some()))
        }) else {
            return Ok(());
        };
        if !author {
            return Ok(());
        }
        // Each app's deploy, and the bundle it's of: its newest, or while that can't be fetched yet, the one it ran.
        let (mut modules, mut used) = (Vec::new(), BTreeMap::new());
        let mut unfetched = false;
        let broken = self.0.broken.lock().unwrap().clone();
        let ran = self.0.bundles.lock().unwrap().clone();
        for (app, bundle) in wanted.iter().filter(|(a, b)| broken.get(*a) != Some(*b)) {
            let mut fetched = self.blob(bundle).await.and_then(|b| crate::bundle::module(&b)).map(|m| (m, bundle));
            if let Err(err) = &fetched {
                tracing::warn!("{app}'s deploy: {err:#}: tried again soon");
                unfetched = true;
                if let Some(old) = ran.get(app).filter(|old| *old != bundle)
                    && let Ok(module) = self.blob(old).await.and_then(|b| crate::bundle::module(&b))
                {
                    fetched = Ok((module, old));
                }
            }
            if let Ok((module, bundle)) = fetched {
                modules.push((app.clone(), module));
                used.insert(app.clone(), bundle.clone());
            }
        }
        self.0.unfetched.store(unfetched, std::sync::atomic::Ordering::Relaxed);
        let changed: Vec<String> = modules.iter().map(|(a, _)| a.clone()).filter(|a| ran.get(a) != used.get(a)).collect();
        if !changed.is_empty() || modules.len() != self.0.apps.names().len() {
            // Writes wait for the new deploys, so what's written next runs on them.
            let _write = self.0.writes.lock().await;
            // An app whose code doesn't start is left out, so the rest run.
            while let Err(err) = self.0.apps.load(modules.clone()).await {
                let Some(broken) = err.downcast_ref::<crate::apps::Broken>() else { return Err(err) };
                tracing::warn!("{broken}: it's left out until it's deployed again");
                if let Some(bundle) = used.get(&broken.app) {
                    self.0.broken.lock().unwrap().insert(broken.app.clone(), bundle.clone());
                }
                let before = modules.len();
                modules.retain(|(a, _)| *a != broken.app);
                ensure!(modules.len() < before, "{err:#}");
            }
            *self.0.bundles.lock().unwrap() = modules.iter().map(|(a, _)| (a.clone(), used[a].clone())).collect();
            // Read again, so each is brought to its new deploy's schema.
            self.close_dbs(|_, db| changed.iter().any(|app| db.starts_with(&format!("{app}/"))));
            self.emit(Event::Changed { apps: changed });
        }
        let running = self.with(|i| Ok(i.builds.get(&self.0.id).map(|r| r.commit.clone())))?;
        if running.as_deref() != Some(crate::config::BUILD) {
            self.say(&Scope::Global, status::Said::Build { device: self.0.id, build: crate::config::BUILD.into(), target: crate::config::TARGET.into() }).await?;
        }
        self.said_reads(self.0.id, crate::database::switch::READS, crate::config::BUILD).await;
        // An admin's device freezes the log once every device reads the scope database (`database::switch`).
        if crate::database::switch::SWITCH && !self.frozen() {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(err) = node.freeze(false).await {
                    tracing::warn!("freezing the log: {err:#}");
                }
            });
        }
        // And once it's frozen, a release's new default roles as a step.
        if self.frozen() {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(err) = node.say_defaults().await {
                    tracing::warn!("saying the release's roles: {err:#}");
                }
            });
        }
        Ok(())
    }

    /// Whether this device is ready to run calls: it's a device of the install, and has read every scope it holds
    /// from the bucket since it woke.
    pub fn ready(&self) -> bool {
        self.with(|i| {
            let alone = i.r.db().computes().iter().all(|s| s.key == self.0.id);
            Ok(i.r.db().compute(&self.0.id).is_some() && (i.read || alone))
        })
        .unwrap_or(false)
    }

    /// Now and then, reads what other devices wrote and said, and syncs built-in services' records.
    async fn tick_loop(self) {
        for tick in 1u64.. {
            tokio::time::sleep(TICK).await;
            // What other devices wrote and said, every tick; and wrote of a built-in service's records, every minute.
            if self.has_install() {
                if let Err(err) = self.read_logs().await {
                    tracing::debug!("reading the bucket: {err:#}");
                }
                self.fill_copies().await;
                self.forget_sessions();
                self.read_all_status();
                self.read_own_items();
            }
            if tick % 6 == 0 && self.has_install() {
                self.share_unshared().await;
                self.take_forgotten().await;
                self.tidy_databases().await;
                self.sweep_remote_puts();
                for service in self.0.built_in_services.iter().filter(|n| n.sync.is_some()) {
                    self.sync_built_in(service.name);
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
    let earliest_member = new.iter().filter(|e| members::is_member_entry(e)).map(Entry::order).min();
    let mut logs: Vec<String> = new.iter().map(|e| e.log().to_string()).collect();
    if let Some(m) = &earliest_member {
        logs.extend(i.views.iter().filter(|(_, (pos, _))| m <= pos).map(|(log, _)| log.clone()));
    }
    logs.sort();
    logs.dedup();
    for log in logs {
        let Ok(scope) = Scope::parse(&log) else { continue };
        // Once its own database's steps say its state, its entries change nothing (`Node::read_scope_steps_now`).
        if !i.r.holds(&scope) || i.scope_dbs.contains_key(&scope) {
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
        let (_, state) = members::replay_from(&i.r.members, &i.r.member_entries, &entries, state, |_, _, _| {});
        let position = match view {
            Some((pos, _)) if !rebuild => pos.clone().max(last),
            _ => last,
        };
        i.views.insert(log.clone(), (position, state));
    }
    Ok(())
}

/// The scopes a call of `app` as `user` may reach: their own if it's installed for them at all, each of their groups
/// it's installed for (in the group or in `global`; a group whose installs this device can't see counts), and `global`
/// if it's installed there.
fn reachable(i: &Install, app: &str, user: &str) -> Vec<Scope> {
    let db = i.r.db();
    let installed = |s: &Scope| i.views.get(&s.to_string()).is_some_and(|(_, st)| st.installed.contains(app));
    let global = installed(&Scope::Global);
    let mut out = Vec::new();
    for g in db.groups_of(user).keys() {
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

fn handles(db: &ScopeDb, scopes: &[Scope], user: &str) -> Vec<ScopeHandle> {
    scopes.iter().map(|s| ScopeHandle { id: s.to_string(), name: whose(db, s), role: db.role_in(user, s) }).collect()
}

/// A scope's name: the install's, a group's, or a person's, removed people's too.
fn whose(db: &ScopeDb, scope: &Scope) -> String {
    match scope {
        Scope::Global => db.name(),
        Scope::Group(g) => db.group(g).map(|g| g.name).unwrap_or_default(),
        Scope::User(u) => db.user_ever(u).map(|u| u.name).unwrap_or_default(),
    }
}

fn node_me(node: &Node, db: &ScopeDb, caller: &Caller) -> Me {
    let name = if caller.key == node.0.id { node.device_name() } else { db.machine(&caller.key).map(|k| k.name).unwrap_or_default() };
    let user = db.user(&caller.user);
    Me {
        key: hex(&caller.key),
        user: caller.user.clone(),
        name,
        role: user.as_ref().map(|u| u.role),
        user_name: user.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
        handle: user.map(|u| u.handle).unwrap_or_default(),
        agent: caller.agent.clone(),
    }
}

/// Checks one of a call's appends: into a scope it named, which the caller may write. `title` is what people call
/// the app a switch, retire or source names.
fn check_append(i: &Install, append: Append, scopes: &[Scope], me: &Caller, title: &str) -> Result<(Scope, Content)> {
    Ok(match append {
        Append::Switch { app, schedule, scope, on } => {
            let scope = if scope == "user" { Scope::User(me.user.clone()) } else { Scope::parse(&scope)? };
            (scope.clone(), switch_entry(i, &me.user, &scope, &app, title, &schedule, on)?)
        }
        Append::Forget { scope, blob } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blob.len() == 64 && blob.bytes().all(|b| b.is_ascii_hexdigit()), "a blob is named by its hash");
            (scope, members::node("forget", json!({ "blob": blob.to_lowercase() })))
        }
        Append::Retire { app: name, how } => {
            let db = i.r.db();
            let a = db.app(&name).with_context(|| format!("no app named {name}"))?;
            ensure!(db.may(&me.user, "delete", &scopes::app(&name)), "Only {title}'s owners can delete, restore or erase it.");
            let mut data = json!({ "name": name, "owner": a.owner.to_string() });
            match how.as_str() {
                "deleted" => data["deleted"] = json!(true),
                "restored" => data["deleted"] = json!(false),
                "erased" if a.deleted.is_some() => data["erased"] = json!(true),
                "erased" => bail!("Delete {title} before erasing it."),
                other => bail!("an app is deleted, restored or erased, not {other}"),
            }
            (Scope::Global, members::node("app", data))
        }
        Append::Source { app: name, link } => {
            let db = i.r.db();
            let a = db.app(&name).filter(|a| !a.erased).with_context(|| format!("no app named {name}"))?;
            ensure!(db.may(&me.user, "deploy", &scopes::app(&name)), "Only {title}'s owners can set where its code is kept.");
            let link = members::source_link(&json!(link))?;
            (Scope::Global, members::node("app", json!({ "name": name, "owner": a.owner.to_string(), "source": link })))
        }
        Append::Service { device, name, on } => (Scope::Global, service_entry(i.r.db(), &me.user, &device, &name, on)?),
        Append::Install { .. } | Append::Cost { .. } | Append::Exec { .. } | Append::Vault { .. } => bail!("installs, costs, statements and items are written on their own"),
    })
}

/// The `service` entry that switches a part of a device's oned on or off, the device found by name or key prefix; for
/// the device's person.
fn service_entry(db: &ScopeDb, user: &str, device: &str, name: &str, on: bool) -> Result<Content> {
    let mut found: Vec<members::Machine> = db.machines().into_iter().filter(|s| !s.removed).filter(|s| s.name.eq_ignore_ascii_case(device) || (!device.is_empty() && hex(&s.key).starts_with(device))).collect();
    // A machine's services are named for it: by its name, it's its node.
    if found.len() > 1 {
        found.retain(|s| s.is(members::Kind::Compute));
    }
    let [s] = found.as_slice() else { bail!("no one device called {device}") };
    ensure!(db.may(user, "switch", &scopes::device(&s.key)), "only {}'s person switches what it runs", s.name);
    Ok(members::service(&s.key, name, on))
}

/// A scope as people call it: Everyone, a person's name, or a group's.
fn scope_name(i: &Install, scope: &str) -> String {
    let db = i.r.db();
    match Scope::parse(scope) {
        Ok(Scope::Global) => "Everyone".to_string(),
        Ok(Scope::User(u)) => db.user(&u).map(|u| u.name).unwrap_or(u),
        Ok(Scope::Group(g)) => db.group(&g).map(|g| g.name).unwrap_or(g),
        Err(_) => scope.to_string(),
    }
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
fn switch_entry(i: &Install, user: &str, scope: &Scope, app: &str, title: &str, schedule: &str, on: bool) -> Result<Content> {
    let name = scope_name(i, &scope.to_string());
    ensure!(i.r.db().may(user, "add-apps", &scopes::object(scope)), "You can't switch schedules in {name}.");
    ensure!(i.views.get(&scope.to_string()).is_some_and(|(_, s)| s.installed.contains(app)), "{title} isn't on {name}.");
    ensure!(i.r.holds(scope), "This device can't open {name} yet.");
    Ok(members::node("schedule", json!({ "app": app, "schedule": schedule, "on": on })))
}

/// A built-in app's bundle: its `app.js` as the module, and every file of its folder as its source. One with npm
/// packages (a `package.json`) or React screens (a `dist/`, built when the app was committed) is built as `<cli> deploy`
/// builds any app, its screens as they are.
pub fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.iter().any(|(p, _)| *p == "package.json" || p.starts_with("dist/")) {
        return crate::deploy::build_built_in(files);
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

/// Writes a file only this account can read: device keys, pairing secrets.
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

/// The key of the device whose home is `dir`, for what this machine signs as itself without its node (`kernel invite`).
pub fn device_key(dir: &Path) -> Result<ed25519_dalek::SigningKey> {
    let bytes = std::fs::read(dir.join("device.json")).with_context(|| format!("{} has no device: {} serve starts one", dir.display(), crate::config::CLI))?;
    let file: DeviceFile = serde_json::from_slice(&bytes)?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&unhex(&file.secret)?))
}

fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    // This device's key, from before devices were called devices: a new key would quietly make it a new device.
    let was = dir.join("server.json");
    ensure!(!was.exists(), "{} holds this device's key: rename it to device.json, then start again", was.display());
    let file = DeviceFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
    Ok(file)
}

/// This machine's own programs' token, made once: only its user reads the file, which a program in a VM or emulator
/// here, or a page, can't.
fn load_local_token(dir: &Path) -> Result<String> {
    let path = dir.join(crate::api::LOCAL_TOKEN);
    match std::fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_string()),
        _ => {
            let token = hex(&rand::random::<[u8; 32]>());
            write_private(&path, token.as_bytes())?;
            Ok(token)
        }
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".into())
}
