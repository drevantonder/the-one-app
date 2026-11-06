//! A node: this device's network (a custom build holds one), its replica and views, its socket to the relay, and where
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

use crate::app::runtime::{Apps, Context, Me, ScopeHandle};
use crate::call::Append;
use crate::bucket::{Copied, Place};
use crate::database::stored::Db;
use crate::database::content::Content;
use crate::database::steps::Step;
use crate::keys::{Hash, Key, hex, short, unhex};
use crate::database::scopes::{Scope, new_id};
use crate::database::scopes::{self, ScopeDb};
use crate::database::content;
use crate::database::scopes::{Principal, Role, ScopeState};
use crate::rank;
use crate::release::{self, Release};
use crate::schedules::{self, Schedule};
use crate::replica::{Handled, Replica};

pub(crate) mod apps;
pub mod backups;
pub(crate) mod blobs;
mod calls;
mod databases;
pub mod devices;
mod sync;
mod net;
mod pairing;
pub(crate) mod push;
pub(crate) mod releases;
mod scheduled;
pub(crate) mod screens;
mod sessions;
pub use databases::NotGiven;
pub use devices::{DeviceRequest, Invited};
pub use net::{alpn, relay, service_alpn};
pub use calls::Elsewhere;
pub use releases::Build;
pub use sessions::Needs;
pub(crate) mod services;
mod app_schedules;
pub use app_schedules::APP_SCHEDULES_DAYS;
mod state;
mod status;
mod steps;
pub mod storage;
pub mod trace;
pub mod vault;
pub mod own;
pub mod groups;
pub mod widgets;

const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A device whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

const TICK: Duration = Duration::from_secs(10);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this device's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// The object in a network's bucket that says whose it is: the network's root.
const NETWORK_OBJECT: &str = "network";

/// Whose the bucket at `place` is, as its `NETWORK_OBJECT` says: or as `install` says, the name a build from before it
/// was called a network gave it (a bucket then, or a backup of one put back), which it puts under the new name too.
fn bucket_root(place: &Place) -> Result<Option<Vec<u8>>> {
    if let Some(root) = place.get(NETWORK_OBJECT)? {
        return Ok(Some(root));
    }
    let old = place.get("install")?;
    if let Some(root) = &old {
        place.create(NETWORK_OBJECT, root)?;
    }
    Ok(old)
}
const NETWORK_DIR: &str = "network";
const NETWORK_FILE: &str = "network.json";
/// How often a device says hello to the others: which services serve there, and how loaded it is.
const PRESENCE_EVERY: Duration = Duration::from_secs(15);
const REFETCH_AFTER: Duration = Duration::from_secs(30);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Serialize, Deserialize)]
struct DeviceFile {
    secret: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct NetworkFile {
    root: String,
    user: String,
    /// The network's bucket, as a place's fields: a folder's `path`, or S3's. None if this device can't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bucket: Option<BTreeMap<String, String>>,
    /// The folders on this device it backs up to, by their place's id: a folder is only ever this device's own choice.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    folders: BTreeMap<String, String>,
}

/// A service serving on a device: its name, the users it serves, and what it describes of itself.
pub type Service = (String, Vec<String>, Value);

/// Another device, as it last said hello: the services serving there, and how loaded it is.
struct Peer {
    services: Vec<Service>,
    load: Option<crate::load::Load>,
    /// What it can run, and each part's state (`Node::parts_here`), if it says.
    parts: Option<Value>,
    seen: Instant,
}

struct Network {
    r: Replica,
    user: String,
    /// The network's bucket, and its fields, which a device joining is given.
    bucket: Option<(Place, BTreeMap<String, String>)>,
    peers: HashMap<Key, Peer>,
    builds: HashMap<Key, release::Running>,
    /// Once `global`'s log is frozen, each other scope's own database as its steps leave it (`database::steps`): the
    /// last position read, and the database, once its anchor checked out here.
    scope_dbs: HashMap<Scope, (u64, ScopeDb)>,
    /// Each scope's rows of how things went, as this device last read them.
    status: HashMap<String, status::Status>,
    /// Each scope's own state, as its database says it.
    views: HashMap<String, ScopeState>,
    /// When this device first and last saw each scope's database change since it started (ms): what backups wait on.
    changed: HashMap<Scope, (u64, u64)>,
    /// `global`'s Vault items, as this device last read them with its own keys: what the node itself keeps there.
    vault: crate::vault::Items,
    /// The folders on this device it backs up to, by their place's id (`NetworkFile::folders`).
    folders: BTreeMap<String, String>,
    /// How long each device took to answer a call lately.
    nearest: HashMap<Key, Duration>,
    /// How far each device is: how long it took to answer this device's last hello.
    far: HashMap<Key, Duration>,
    /// Whether this device has read every scope it holds from the bucket since it woke.
    read: bool,
}

impl Network {
    fn online(&self, device: &Key) -> bool {
        self.peers.get(device).is_some_and(|p| p.seen.elapsed() < 3 * PRESENCE_EVERY)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    Changed { apps: Vec<String> },
    Devices,
    /// This device created or joined a network.
    Network,
}

/// What a machine's node is: the device its person's CLI, MCP and Mac app sign with, and compute.
pub const MACHINE: &[scopes::Kind] = &[scopes::Kind::Client, scopes::Kind::Compute];

/// Who asked for something: this device or another, or a client, as one user.
#[derive(Clone, Debug)]
pub struct Caller {
    pub key: Key,
    pub user: String,
    /// The agent that asked through it, by name ("Claude Code"), when one did.
    pub agent: Option<String>,
    /// What it came with, which says whose keys its calls open data with.
    pub credential: Credential,
}

/// What a caller came with: this device's own keys, or the keys it was handed for one of these, kept here by it
/// (sessions.rs) and never this device's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    /// None: this device's own programs, or a device or service of its own person asking as itself. Its calls use this
    /// device's own keys, for its own person alone.
    Own,
    /// An access token the network's sign-in signed, passed on with its calls to another device, with the keys it was
    /// handed.
    SignIn(String),
    /// A client device's calls over iroh, by its key, proven by QUIC: never a token anyone sends, so no request reaches
    /// what a device was handed by naming it.
    Client(Key),
    /// A call another node (`node`, its endpoint key, proven by QUIC) passed on for `user` with no sign-in: it runs only
    /// on the keys it was handed, sealed here for that node's key.
    Passed { node: Key, user: String },
    /// App schedules' calls: `app`'s in `scope`, with the keys sealed to this device for them alone. Never passed on.
    AppSchedules { scope: Scope, app: String },
}

impl Caller {
    pub fn own(key: Key, user: impl Into<String>) -> Caller {
        Caller { key, user: user.into(), agent: None, credential: Credential::Own }
    }

    pub fn local(node: &Node) -> Result<Caller> {
        Ok(Caller::own(node.0.id, node.user()?))
    }
}

impl Node {
    pub fn this_machine(&self, caller: &Caller) -> bool {
        caller.credential == Credential::Own && caller.key == self.0.id
    }
}

/// A call to run: an app's tool, or a service's function.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallRequest {
    /// Names this call wherever it's tried, so a call that writes runs on one device only: the first to take its lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The app, or for a service's call, the service.
    #[serde(default)]
    pub app: String,
    /// The service whose function this is, when it is one: an app and the service it wraps may share a name.
    #[serde(default)]
    pub service: Option<String>,
    /// The tool it runs: `op` on the wire.
    #[serde(rename = "op")]
    pub tool: String,
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
    /// Files put on this device for this call alone (an app's `_http` body), which go with it to the device that runs
    /// it (`Node::carried`): never a client's to name.
    #[serde(skip)]
    pub puts: Vec<String>,
    /// This node named the call (`Node::identified`), no client did.
    #[serde(skip)]
    pub named_here: bool,
    /// It runs where it was named, first, so no other device has it or the id: it takes no lease.
    #[serde(skip)]
    pub unleased: bool,
}

enum Reply {
    Ok(Value),
    /// It failed, saying why and, for the kinds callers act on, which kind (`protocol::code`).
    Error(String, Option<String>),
    /// It can't run here; try another device.
    Elsewhere(String),
    /// It lacks keys its caller hands: said as the API says it (`{ error, need, to, seal }`), so the device that passed
    /// it on can ask for them in turn (428).
    Needs(Needs),
}

impl Reply {
    fn error(why: impl Into<String>) -> Reply {
        Reply::Error(why.into(), None)
    }

    fn failed(err: &anyhow::Error) -> Reply {
        Reply::Error(format!("{err:#}"), crate::protocol::Coded::of(err))
    }

    fn json(self) -> Value {
        match self {
            Reply::Ok(v) => json!({ "ok": v }),
            Reply::Error(e, None) => json!({ "error": e }),
            Reply::Error(e, Some(code)) => json!({ "error": e, "code": code }),
            Reply::Elsewhere(w) => json!({ "elsewhere": w }),
            Reply::Needs(n) => n.json(),
        }
    }

    fn from_json(v: Value) -> Reply {
        if let Some(w) = v.get("elsewhere") {
            return Reply::Elsewhere(w.as_str().unwrap_or("").into());
        }
        if let Some(needs) = Needs::from_json(&v) {
            return Reply::Needs(needs);
        }
        match v.get("error") {
            Some(e) => Reply::Error(e.as_str().unwrap_or("failed").into(), v["code"].as_str().map(str::to_string)),
            None => Reply::Ok(v["ok"].clone()),
        }
    }
}

/// A lock that refuses the thread already holding it. A std `Mutex` asked for again on the thread holding it waits on
/// itself for good, and every thread asking after that waits on it too: once those are all the runtime's workers, the
/// node takes no connection and answers nothing, though it still listens. Refused instead, and said where.
struct Exclusive<T>(Mutex<T>);

thread_local!(static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) });

struct Held<'a, T>(std::sync::MutexGuard<'a, T>, usize);

impl<T> Exclusive<T> {
    const fn new(value: T) -> Self {
        Exclusive(Mutex::new(value))
    }

    fn lock(&self) -> Result<Held<'_, T>> {
        let me = self as *const Self as usize;
        if HOLDING.with_borrow(|h| h.contains(&me)) {
            let at = std::backtrace::Backtrace::force_capture();
            tracing::error!("the network was asked for on a thread that holds it, which would stop this node answering: {at}");
            debug_assert!(false, "the network asked for on a thread that holds it: {at}");
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
    pub services: crate::services::Services,
    network: Exclusive<Option<Network>>,
    pairings: Mutex<HashMap<[u8; 16], pairing::Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this device.
    changed: tokio::sync::Notify,
    /// Wakes the task that updates this node to a newer release.
    released: tokio::sync::Notify,
    bundles: Mutex<BTreeMap<String, String>>,
    /// The bundle of each app this release has built in, kept as a blob: any network may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png`, its finished app icon, as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The pictures `apps_drawn` named (`/v1/images/<name>`), by name: each a data URL.
    named_images: Mutex<HashMap<String, String>>,
    /// The logos of the sites apps and services connect to.
    logos: crate::logos::Logos,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The port apps' requests go out through.
    outbound: u16,
    calls: calls::State,
    backups: backups::State,
    blobs: blobs::State,
    databases: databases::State,
    /// Steps other devices sent, waiting for the bucket to confirm them, while one ask of it is under way: none then.
    sent: Mutex<Option<Vec<sync::Sent>>>,
    /// Whether this device is moving to the network's new storage now.
    moving: Mutex<bool>,
    /// The network's recovery keys this device told the relay about since it started: the old ones taken out, the
    /// current one let in as a client (`follow_recovery`).
    recovery_told: Mutex<std::collections::BTreeSet<Key>>,
    /// When it last read `global` for a client device it didn't list (`Node::catch_up`).
    caught_up: tokio::sync::Mutex<Option<std::time::Instant>>,
    /// Where the network's data went: every bucket this device has used writes there, after a move (`Copied`).
    moved: Arc<tokio::sync::RwLock<Option<Place>>>,
    /// Held while this device makes a scope's first epoch.
    making_keys: tokio::sync::Mutex<()>,
    /// Requests to join, by their device's key, that their person said weren't theirs, not shown again while it runs.
    refused: Mutex<std::collections::HashSet<String>>,
    /// What the vault gave this device for the sign-ins its calls came with.
    sessions: sessions::Sessions,
    /// Apps whose deploy doesn't start here, by the bundle that didn't: left out until they're deployed again.
    broken: Mutex<BTreeMap<String, String>>,
    /// The keys of the apps whose schedules this device runs for their people.
    app_schedules: app_schedules::State,
    owed: Mutex<devices::Owed>,
    sign_ins: Mutex<devices::SignIns>,
    net: net::Net,
    /// How many times its deploys were settled (`settle`), the first once its apps are loaded.
    settled: tokio::sync::watch::Sender<u64>,
    /// Held while settling, so one that read the members before a change can't load its apps after one that read it.
    settling: tokio::sync::Mutex<()>,
    /// Whether the last settling couldn't fetch an app's deploy, so it's tried again soon, not only at the next change.
    unfetched: std::sync::atomic::AtomicBool,
    /// Set once this node is handing over to the next: it opens nothing only one of them may hold.
    handing_over: std::sync::atomic::AtomicBool,
    /// The guide's address on the tailnet, `http://<address>:<port>/llms.txt`, while this node serves it there (`api::serve_guide`).
    guide: Mutex<Option<String>>,
    signed_ins: crate::api::SignedIns,
    /// What oned's thread for its parts tells it about them.
    parts: crate::oned::Parts,
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
    /// Starts the node of the home `dir`. `apart`: its app runtime is a program of its own, which oned starts
    /// (`Apps::program`), not a thread of this process.
    pub async fn start(dir: &Path, apart: bool) -> Result<Node> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let secret = unhex(&device.secret)?;
        let key = SigningKey::from(secret);
        let id = *key.verifying_key().as_bytes();
        let outbound = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let apps = Apps::start(dir.join(crate::handoff::runtime()), outbound.local_addr()?.port(), apart).await?;
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
            services: Default::default(),
            network: Exclusive::new(None),
            pairings: Mutex::new(pairing::load(dir)),
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
            outbound: outbound.local_addr()?.port(),
            calls: Default::default(),
            backups: Default::default(),
            blobs: Default::default(),
            databases: Default::default(),
            sent: Mutex::new(None),
            moving: Mutex::new(false),
            recovery_told: Mutex::new(Default::default()),
            caught_up: tokio::sync::Mutex::new(None),
            moved: Arc::default(),
            making_keys: tokio::sync::Mutex::new(()),
            refused: Mutex::default(),
            sessions: Default::default(),
            broken: Default::default(),
            app_schedules: Default::default(),
            owed: Default::default(),
            sign_ins: Default::default(),
            net: Default::default(),
            settled: tokio::sync::watch::channel(0).0,
            settling: tokio::sync::Mutex::new(()),
            unfetched: Default::default(),
            handing_over: Default::default(),
            guide: Default::default(),
            signed_ins: Default::default(),
            parts: Default::default(),
        }));
        let outbound_router = crate::app::outbound::router(node.clone());
        tokio::spawn(async move {
            if let Err(err) = axum::serve(outbound, outbound_router).await {
                tracing::warn!("apps' outbound requests stopped: {err:#}");
            }
        });
        if dir.join(NETWORK_DIR).join(NETWORK_FILE).exists() {
            node.open_network().context("open the network")?;
            if let Err(err) = node.bucket().and_then(|b| b.forget_sent()) {
                tracing::warn!("forgetting steps other devices sent: {err:#}");
            }
            node.with(|i| Ok(update_views(i)))?;
            node.keep_alone_here()?;
            node.read_own_items();
            node.follow_net();
            node.read_all_status();
            let reader = node.clone();
            tokio::spawn(async move {
                if let Err(err) = reader.read_scopes().await {
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
        Ok(node)
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    /// Waits until it has read every scope it holds and its apps are loaded and each has answered once, so a node
    /// taking over answers its first calls warm: before it's read them, what's added where reads as nothing, and an
    /// app's tools as missing.
    pub async fn warm(&self) {
        let until = tokio::time::Instant::now() + Duration::from_secs(60);
        while !self.ready() && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut settled = self.0.settled.subscribe();
        let _ = tokio::time::timeout_at(until, settled.wait_for(|n| *n > 0)).await;
        let mut asked = tokio::task::JoinSet::new();
        for app in self.0.apps.names() {
            let apps = self.0.apps.clone();
            asked.spawn(async move { apps.describe(&app).await });
        }
        let _ = tokio::time::timeout(Duration::from_secs(30), asked.join_all()).await;
    }

    /// Whether this node is handing over to the next: then it keeps its endpoint only until the next one is ready
    /// (`handed_over`), never binding it again; and if the next one didn't start, it keeps it as before.
    /// Handing over, it signs nothing more of its own once what it's writing is written: the next one, with the same
    /// key, writes from where it reads the logs, so two writing at once would fork them, and its device would count as
    /// removed.
    pub async fn handing_over(&self, handing: bool) {
        self.0.handing_over.store(handing, std::sync::atomic::Ordering::SeqCst);
        if handing {
            drop(self.0.writes.lock().await);
        } else {
            self.follow_net();
        }
    }

    /// Whether this node is handing over to the next, or has: then it starts and lists no parts, and writes nothing.
    pub fn is_handing_over(&self) -> bool {
        self.0.handing_over.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Holds this device's writes for one that it signs itself: none once it's handing over to the next node.
    pub(crate) async fn own_writes(&self) -> Result<tokio::sync::MutexGuard<'_, ()>> {
        let write = self.0.writes.lock().await;
        ensure!(!self.is_handing_over(), "this device is starting its new build: try again in a moment");
        Ok(write)
    }

    /// The next node is ready, with an endpoint of this device's key of its own: this one closes its own, so the relay
    /// and other devices reach only that one.
    pub fn handed_over(&self) {
        self.close_net();
    }

    pub fn dir(&self) -> &Path {
        &self.0.dir
    }

    pub fn parts(&self) -> &crate::oned::Parts {
        &self.0.parts
    }

    pub(crate) fn signed_ins(&self) -> &crate::api::SignedIns {
        &self.0.signed_ins
    }

    /// The guide's address on the tailnet, if this node serves it there.
    pub fn guide(&self) -> Option<String> {
        self.0.guide.lock().unwrap().clone()
    }

    pub fn set_guide(&self, address: Option<String>) {
        *self.0.guide.lock().unwrap() = address;
    }

    pub fn local_token(&self) -> &str {
        &self.0.local_token
    }

    fn with<R>(&self, f: impl FnOnce(&mut Network) -> Result<R>) -> Result<R> {
        let mut network = self.0.network.lock()?;
        f(network.as_mut().context("this device is in no network yet: create one, or join with a pairing code")?)
    }

    pub fn has_network(&self) -> bool {
        self.0.network.lock().is_ok_and(|i| i.is_some())
    }

    pub fn user(&self) -> Result<String> {
        self.with(|i| Ok(i.user.clone()))
    }

    /// Refuses a second network: a custom build holds one network, and another context is another custom build.
    fn ensure_no_network(&self) -> Result<()> {
        if self.has_network() {
            bail!("this device already belongs to a network, and {} holds one; for another, network that context's own app", crate::config::NAME);
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

    fn network_dir(&self) -> PathBuf {
        self.0.dir.join(NETWORK_DIR)
    }

    fn open_network(&self) -> Result<()> {
        let dir = self.network_dir();
        let file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        let root: [u8; 32] = data_encoding::HEXLOWER.decode(file.root.as_bytes())?.try_into().map_err(|_| anyhow!("bad root"))?;
        let r = Replica::new(self.0.key.clone(), Some(Hash::from_bytes(root)), now_ms());
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((self.network_place(&f)?, f))).transpose()?;
        let network = Network { r, user: file.user, bucket, folders: file.folders, peers: HashMap::new(), builds: HashMap::new(), scope_dbs: HashMap::new(), status: HashMap::new(), views: HashMap::new(), changed: HashMap::new(), vault: Default::default(), nearest: HashMap::new(), far: HashMap::new(), read: false };
        *self.0.network.lock()? = Some(network);
        self.start_kept();
        Ok(())
    }

    /// A network made before it kept its data in a bucket, with no device but this one: it keeps it in a folder here.
    fn keep_alone_here(&self) -> Result<()> {
        let alone = self.with(|i| Ok(i.bucket.is_none() && i.r.db().computes().iter().all(|s| s.key == self.0.id)))?;
        if !alone {
            return Ok(());
        }
        let dir = self.network_dir();
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        let fields: BTreeMap<String, String> = [("path".to_string(), dir.join("bucket").display().to_string())].into();
        let place = Place::from_fields(&fields)?;
        place.create(NETWORK_OBJECT, file.root.as_bytes())?;
        file.bucket = Some(fields.clone());
        self.save_network(&file)?;
        self.with(|i| {
            i.bucket = Some((place, fields));
            Ok(())
        })
    }

    fn keep_folder(&self, id: &str, path: Option<&std::path::Path>) -> Result<()> {
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(self.network_dir().join(NETWORK_FILE))?)?;
        match path {
            Some(path) => drop(file.folders.insert(id.to_string(), path.display().to_string())),
            None => drop(file.folders.remove(id)),
        }
        self.save_network(&file)?;
        self.with(|i| {
            i.folders = file.folders;
            Ok(())
        })
    }

    fn save_network(&self, file: &NetworkFile) -> Result<()> {
        let dir = self.network_dir();
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join(NETWORK_FILE), &serde_json::to_vec_pretty(file)?)?;
        Ok(())
    }

    /// This device's name: as the network lists it, which another of its person's devices may have changed, when the
    /// network isn't being read (never waiting on it, so it's asked for anywhere); else as it last was.
    pub fn device_name(&self) -> String {
        let network = self.0.network.0.try_lock().ok();
        let listed = network.as_ref().and_then(|i| i.as_ref()).and_then(|i| i.r.db().machine(&self.0.id).map(|m| m.name));
        drop(network);
        let mut name = self.0.name.lock().unwrap();
        if let Some(listed) = listed {
            *name = listed;
        }
        name.clone()
    }

    /// Creates the network, with every built-in app added (owned by admins), so anyone may add one to their own page,
    /// and these installed in `global`. Its first person is `user` if given (who they were in a network this one
    /// replaces, so their account at its sign-in is still theirs), else someone new. Who's who is its scope database from
    /// its first freeze on, which its new recovery key signs as it starts (`steps::first_freeze`).
    pub async fn create_with(&self, name: &str, user_name: &str, user: Option<&str>, apps: &[&str]) -> Result<String> {
        self.ensure_no_network()?;
        ensure!(!name.trim().is_empty() && !user_name.trim().is_empty(), "give the network a name, and yourself one");
        ensure!(user.is_none_or(|u| u.len() == 16 && u.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))), "a person's id is 16 hex digits, like 976091fe8c23a057");
        let user = user.map_or_else(new_id, str::to_string);
        let recovery = SigningKey::from_bytes(&rand::random());
        let me = crate::keys::Endpoint::of(&self.0.key);
        // The network's id, which every device of it pins, and `global`'s first epoch, named by its first freeze before
        // anything is sealed under it.
        let root = Hash::from_bytes(rand::random());
        let global: [u8; 32] = rand::random();
        let start = content::first(name.trim(), &user, user_name.trim(), &me, &self.device_name(), MACHINE, recovery.verifying_key().as_bytes(), &crate::keys::epoch_hash(&global));
        // Until it's moved to S3-compatible storage, the network's bucket is a folder here.
        let bucket: BTreeMap<String, String> = [("path".to_string(), self.network_dir().join("bucket").display().to_string())].into();
        Place::from_fields(&bucket)?.create(NETWORK_OBJECT, root.to_hex().as_bytes())?;
        self.save_network(&NetworkFile { root: root.to_hex(), user: user.clone(), bucket: Some(bucket), folders: BTreeMap::new() })?;
        self.open_network()?;
        let bucket = self.bucket()?;
        let recovery_seal = crate::keys::twin_public(recovery.verifying_key().as_bytes())?;
        for (to, seal) in [("recovery".to_string(), recovery_seal), (hex(&me.key), me.seal)] {
            crate::database::stored::created(&bucket, &crate::keys::epoch_name("global", 1, &to), &crate::keys::seal_to(&seal, &global)?).await?;
        }
        self.0.databases.epochs.lock().unwrap().insert(Scope::Global, crate::database::stored::Keys::from([(1, global)]));
        // Who's who, as the freeze its recovery key signs at step 1 of its scope database.
        let keys = self.step_keys(&Scope::Global).await?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })?;
        let first = crate::database::steps::first_freeze(&recovery, root, &start, &user, now_ms())?;
        let name = crate::database::stored::step_name(&crate::database::steps::prefix("global"), 1);
        let sealed = crate::database::steps::seal(&keys, &name, &crate::database::steps::encode(&first.step, first.copy.as_deref()))?;
        ensure!(crate::bucket::Bucket::create(&bucket, &name, &sealed).await?, "the new network's bucket isn't empty");
        self.read_steps().await?;
        ensure!(self.frozen(), "the new network's freeze didn't count");
        let me = Caller::own(self.0.id, user);
        // Its Vault and its first person's own open on this machine, so a new network fills items with no other step.
        for scope in [Scope::Global, Scope::User(me.user.clone())] {
            self.new_vault_key(&scope, 1, &[self.0.id], recovery.verifying_key().as_bytes()).await?;
        }
        for (app, _) in crate::config::BUILT_IN {
            self.bring_in(&me, app).await?;
        }
        for app in apps {
            self.bring_in(&me, app).await?;
            self.append(&Scope::Global, content::node("install", json!({ "app": app }))).await?;
        }
        self.settle().await?;
        self.emit(Event::Network);
        Ok(hex(&recovery.to_bytes()))
    }

    pub async fn append(&self, scope: &Scope, content: Content) -> Result<Step> {
        let user = self.user()?;
        self.append_as(scope, &user, content).await
    }

    async fn append_as(&self, scope: &Scope, user: &str, content: Content) -> Result<Step> {
        if !self.frozen() {
            self.read_steps().await?;
        }
        match scope {
            Scope::Global => self.write_step(user, content).await,
            _ => self.write_scope_step(scope, user, content).await,
        }
    }

    /// A scope's first epoch, before anything is written there, made outside this device's writes, since its entry is
    /// one: a scope with none has nothing to read or write until it has.
    async fn first_epoch(&self, scope: &Scope) -> Result<()> {
        if *scope != Scope::Global && self.with(|i| Ok(i.r.db().epoch(scope)))? == 0 {
            self.epoch_keys(scope, true).await?;
        }
        Ok(())
    }

    /// Follows what changed in who's who, and brings each scope's state up to date.
    async fn after(&self, handled: Handled) -> Result<()> {
        if handled.dropped.is_empty() && !handled.members {
            return Ok(());
        }
        // What `global`'s scope database forgets in a step, once its log is frozen, as its `forget` entries said before.
        let forgotten_before = self.with(|i| Ok(i.views.get("global").map(|s| s.forgotten.clone()).unwrap_or_default()))?;
        let forgotten_now = self.with(|i| {
            if handled.members {
                let db = i.r.db();
                i.peers.retain(|peer, _| db.compute(peer).is_some() || db.service(peer).is_some());
            }
            for log in &handled.dropped {
                i.views.remove(log);
            }
            if handled.members {
                note_change(i, &Scope::Global);
            }
            update_views(i);
            Ok(i.views.get("global").map(|s| s.forgotten.clone()).unwrap_or_default())
        })?;
        self.close_dbs(|scope, _| handled.dropped.contains(&scope.to_string()));
        if let Ok(bucket) = self.bucket() {
            for log in &handled.dropped {
                if let Err(err) = bucket.forget(&crate::bucket::folder(log)) {
                    tracing::warn!("forgetting this device's copy of {log}: {err:#}");
                }
            }
        }
        // A release is a step, which changes who's who: the updater looks again.
        if handled.members {
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
        if handled.members {
            self.follow_storage();
            self.follow_net();
        }
        let forgotten: Vec<(String, String)> = forgotten_now.into_iter().filter(|b| !forgotten_before.contains(b)).map(|b| ("global".to_string(), b)).collect();
        self.emit(Event::Changed { apps: Vec::new() });
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
        Ok(())
    }

    /// Brings this device in line with the members: runs each app's latest deploy, fetching bundles it lacks, and
    /// says which build it runs.
    async fn settle(&self) -> Result<()> {
        let _settling = self.0.settling.lock().await;
        let Ok((wanted, unadded, author)) = self.with(|i| {
            let wanted: BTreeMap<String, String> = i.r.db().apps().into_iter().filter(|a| !a.erased).filter_map(|a| Some((a.name, a.bundle?))).collect();
            let unadded: BTreeMap<String, String> = self.0.built_in.iter().filter(|(name, _)| i.r.db().app(name).is_none()).map(|(n, b)| (n.clone(), b.clone())).collect();
            Ok((wanted, unadded, i.r.db().compute(&self.0.id).is_some()))
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
            let mut fetched = self.blob(bundle).await.and_then(|b| crate::app::bundle::module(&b)).map(|m| (m, bundle));
            if let Err(err) = &fetched {
                tracing::warn!("{app}'s deploy: {err:#}: tried again soon");
                unfetched = true;
                if let Some(old) = ran.get(app).filter(|old| *old != bundle)
                    && let Ok(module) = self.blob(old).await.and_then(|b| crate::app::bundle::module(&b))
                {
                    fetched = Ok((module, old));
                }
            }
            if let Ok((module, bundle)) = fetched {
                modules.push((app.clone(), module));
                used.insert(app.clone(), bundle.clone());
            }
        }
        // The built-in apps no one has added run no calls: they're there to be described, for the App Store to list.
        let mut waiting = Vec::new();
        for (app, bundle) in unadded.iter().filter(|(a, b)| broken.get(*a) != Some(*b)) {
            if let Ok(module) = self.blob(bundle).await.and_then(|b| crate::app::bundle::module(&b)) {
                waiting.push((app.clone(), module));
            }
        }
        self.0.unfetched.store(unfetched, std::sync::atomic::Ordering::Relaxed);
        let changed: Vec<String> = modules.iter().map(|(a, _)| a.clone()).filter(|a| ran.get(a) != used.get(a)).collect();
        if !changed.is_empty() || modules.len() != self.0.apps.names().len() || waiting.iter().map(|(a, _)| a).ne(self.0.apps.unadded().iter()) {
            // Writes wait for the new deploys, so what's written next runs on them.
            let _write = self.0.writes.lock().await;
            // An app whose code doesn't start is left out, so the rest run.
            while let Err(err) = self.0.apps.load(modules.clone(), waiting.clone()).await {
                let Some(broken) = err.downcast_ref::<crate::app::runtime::Broken>() else { return Err(err) };
                tracing::warn!("{broken}: it's left out until it's deployed again");
                if let Some(bundle) = used.get(&broken.app).or(unadded.get(&broken.app)) {
                    self.0.broken.lock().unwrap().insert(broken.app.clone(), bundle.clone());
                }
                let before = modules.len() + waiting.len();
                modules.retain(|(a, _)| *a != broken.app);
                waiting.retain(|(a, _)| *a != broken.app);
                ensure!(modules.len() + waiting.len() < before, "{err:#}");
            }
            *self.0.bundles.lock().unwrap() = modules.iter().map(|(a, _)| (a.clone(), used[a].clone())).collect();
            // Read again, so each is brought to its new deploy's schema.
            self.close_dbs(|_, db| changed.iter().any(|app| db.starts_with(&format!("{app}/"))));
            self.emit(Event::Changed { apps: changed });
            // The services of apps that came or changed are listed as their apps now say.
            for app in self.0.services.offered() {
                self.learn_app(&app).await;
            }
            self.say_hello();
        }
        let running = self.with(|i| Ok(i.builds.get(&self.0.id).map(|r| r.commit.clone())))?;
        if running.as_deref() != Some(crate::config::BUILD) {
            self.say(&Scope::Global, status::Said::Build { device: self.0.id, build: crate::config::BUILD.into(), target: crate::config::TARGET.into() }).await?;
        }
        Ok(())
    }

    pub fn ready(&self) -> bool {
        self.with(|i| {
            let alone = i.r.db().computes().iter().all(|s| s.key == self.0.id);
            Ok(i.r.db().compute(&self.0.id).is_some() && (i.read || alone))
        })
        .unwrap_or(false)
    }

    /// Now and then, reads what other devices wrote and said.
    async fn tick_loop(self) {
        for tick in 1u64.. {
            tokio::time::sleep(TICK).await;
            if self.has_network() {
                if let Err(err) = self.read_scopes().await {
                    tracing::debug!("reading the bucket: {err:#}");
                }
                self.fill_copies().await;
                self.forget_sessions();
                self.read_all_status();
                self.read_own_items();
            }
            if tick % 6 == 0 && self.has_network() {
                self.share_unshared().await;
                self.take_forgotten().await;
                self.tidy_databases().await;
                self.sweep_remote_puts();
            }
        }
    }
}

/// `global`'s state, as its scope database says it: each other scope's is its own database's
/// (`Node::read_scope_steps_now`).
fn update_views(i: &mut Network) {
    if i.r.freeze().is_some() {
        i.views.insert("global".to_string(), i.r.db().state());
    }
}

fn note_change(i: &mut Network, scope: &Scope) {
    let now = now_ms();
    i.changed.entry(scope.clone()).and_modify(|c| c.1 = now).or_insert((now, now));
}

/// The scopes a call of `app` as `user` may reach: their own if it's installed for them at all, each of their groups
/// it's installed for (in the group or in `global`; a group whose installs this device can't see counts), and `global`
/// if it's installed there.
fn reachable(i: &Network, app: &str, user: &str) -> Vec<Scope> {
    let db = i.r.db();
    let installed = |s: &Scope| i.views.get(&s.to_string()).is_some_and(|st| st.installed.contains(app));
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

/// A scope's name: the network's, a group's, or a person's, removed people's too.
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
fn check_append(i: &Network, append: Append, scopes: &[Scope], me: &Caller, title: &str) -> Result<(Scope, Content)> {
    Ok(match append {
        Append::Switch { app, schedule, scope, on } => {
            let scope = Scope::parse_for(&scope, &me.user)?;
            (scope.clone(), switch_entry(i, &me.user, &scope, &app, title, &schedule, on)?)
        }
        Append::Forget { scope, blob } => {
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blob.len() == 64 && blob.bytes().all(|b| b.is_ascii_hexdigit()), "a blob is named by its hash");
            (scope, content::node("forget", json!({ "blob": blob.to_lowercase() })))
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
            (Scope::Global, content::node("app", data))
        }
        Append::Source { app: name, link } => {
            let db = i.r.db();
            let a = db.app(&name).filter(|a| !a.erased).with_context(|| format!("no app named {name}"))?;
            ensure!(db.may(&me.user, "deploy", &scopes::app(&name)), "Only {title}'s owners can set where its code is kept.");
            let link = scopes::source_link(&json!(link))?;
            (Scope::Global, content::node("app", json!({ "name": name, "owner": a.owner.to_string(), "source": link })))
        }
        Append::Service { device, name, on, scope } => {
            let scope = match scope {
                Some(s) => Some(Scope::parse_for(&s, &me.user)?),
                None if on => added_scope(i, &me.user, &name),
                None => None,
            };
            (Scope::Global, service_entry(i.r.db(), &me.user, &device, &name, on, scope.as_ref())?)
        }
        Append::Install { .. } | Append::Cost { .. } | Append::Exec { .. } | Append::Vault { .. } => bail!("installs, costs, statements and items are written on their own"),
    })
}

/// The `service` entry that switches a part of a device's oned on or off, the device found by name or key prefix; for
/// the device's person, or to switch it off, for who may add apps where it ran. A service runs for the app added in
/// `scope`, which its writer may add apps to.
fn service_entry(db: &ScopeDb, user: &str, device: &str, name: &str, on: bool, scope: Option<&Scope>) -> Result<Content> {
    if let Some(s) = scope.filter(|_| on) {
        ensure!(db.may(user, "add-apps", &scopes::object(s)), "only who may add apps there runs a service for them");
    }
    let managed = |k: &Key| !on && db.switch(k, name).and_then(|s| s.scope).and_then(|s| Scope::parse(&s).ok()).is_some_and(|s| db.may(user, "add-apps", &scopes::object(&s)));
    // The Cloudflare node, by its name in run priority or its key: its admins switch its parts.
    if let Some(k) = db.cloudflare_node().filter(|k| device.eq_ignore_ascii_case(crate::rank::CLOUDFLARE) || (!device.is_empty() && hex(k).starts_with(device))) {
        ensure!(managed(&k) || db.may(user, "change-settings", scopes::NETWORK), "only an admin switches what the Cloudflare node runs");
        // It always runs apps; what else it can run is the services built for it that it carries.
        ensure!(!on || crate::config::CLOUDFLARE_SERVICES.iter().any(|(n, _)| *n == name), "the Cloudflare node can't run {name}: it runs apps, and only the services built for Cloudflare");
        return Ok(content::service(&k, name, on, scope));
    }
    let mut found: Vec<scopes::Machine> = db.machines().into_iter().filter(|s| !s.removed).filter(|s| s.name.eq_ignore_ascii_case(device) || (!device.is_empty() && hex(&s.key).starts_with(device))).collect();
    // A machine's services are named for it: by its name, it's its node.
    if found.len() > 1 {
        found.retain(|s| s.is(scopes::Kind::Compute));
    }
    let key = match found.as_slice() {
        [s] => s.key,
        // A node that was removed still has its records, which who added the app may switch off.
        _ => match unhex(device).ok().filter(|k| managed(k)) {
            Some(k) => k,
            None => bail!("no one device called {device}"),
        },
    };
    ensure!(managed(&key) || db.may(user, "switch", &scopes::device(&key)), "only {device}'s person switches what it runs");
    Ok(content::service(&key, name, on, scope))
}

/// The scope whose added app `name` is, as `user` sees it, for a service switched on with none said: their own, if
/// it's added there, then everyone's, then one of their groups'.
fn added_scope(i: &Network, user: &str, name: &str) -> Option<Scope> {
    let added = |s: &Scope| i.views.get(&s.to_string()).is_some_and(|v| v.installed.contains(name));
    let own = Scope::User(user.to_string());
    let db = i.r.db();
    let groups = db.groups().into_iter().filter(|g| db.group_role(user, &g.id).is_some()).map(|g| Scope::Group(g.id));
    [own, Scope::Global].into_iter().chain(groups).find(|s| added(s) && db.may(user, "add-apps", &scopes::object(s)))
}

fn scope_name(i: &Network, scope: &str) -> String {
    let db = i.r.db();
    match Scope::parse(scope) {
        Ok(Scope::Global) => "Everyone".to_string(),
        Ok(Scope::User(u)) => db.user(&u).map(|u| u.name).unwrap_or(u),
        Ok(Scope::Group(g)) => db.group(&g).map(|g| g.name).unwrap_or(g),
        Err(_) => scope.to_string(),
    }
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
fn switch_entry(i: &Network, user: &str, scope: &Scope, app: &str, title: &str, schedule: &str, on: bool) -> Result<Content> {
    let name = scope_name(i, &scope.to_string());
    ensure!(i.r.db().may(user, "add-apps", &scopes::object(scope)), "You can't switch schedules in {name}.");
    ensure!(i.views.get(&scope.to_string()).is_some_and(|s| s.installed.contains(app)), "{title} isn't on {name}.");
    ensure!(i.r.holds(scope), "This device can't open {name} yet.");
    Ok(content::node("schedule", json!({ "app": app, "schedule": schedule, "on": on })))
}

/// A built-in app's bundle: its `app.js` as the module, and every file of its folder as its source. One with npm
/// packages (a `package.json`) or React screens (a `dist/`, built when the app was committed) is built as `<cli> deploy`
/// builds any app, its screens as they are.
pub fn built_in_bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if files.iter().any(|(p, _)| *p == "package.json" || p.starts_with("dist/")) {
        return crate::app::deploy::build_built_in(files);
    }
    let module = files.iter().find(|(p, _)| *p == "app.js").context("a built-in app has no app.js")?.1;
    let mut bundle = vec![("app.js".to_string(), module.to_vec())];
    bundle.extend(files.iter().map(|(p, b)| (format!("src/{p}"), b.to_vec())));
    crate::app::bundle::pack(&bundle)
}

/// A deploy of `app`: the bundle at blob `bundle`, and the builds of its service it names (`bundle::SERVICES`), each a
/// blob too, so a node that runs it finds it.
pub fn deploy(app: &str, bundle: &str, builds: impl IntoIterator<Item = String>) -> Content {
    let blobs = std::iter::once(bundle.to_string()).chain(builds).collect();
    Content { blobs, ..content::node("deploy", json!({ "app": app, "bundle": bundle })) }
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

/// The key of the device whose home is `dir`, for what this machine signs as itself without its node (`cloudflare invite`).
pub fn device_key(dir: &Path) -> Result<ed25519_dalek::SigningKey> {
    let bytes = std::fs::read(dir.join("device.json")).with_context(|| format!("{} has no device: {} serve starts one", dir.display(), crate::config::CLI))?;
    let file: DeviceFile = serde_json::from_slice(&bytes)?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&unhex(&file.secret)?))
}

/// The key (hex) and name of the device whose home is `dir`, as it last was.
pub fn device_here(dir: &Path) -> Result<(String, String)> {
    let file = load_device(dir)?;
    let key = ed25519_dalek::SigningKey::from_bytes(&unhex(&file.secret)?);
    Ok((hex(&key.verifying_key().to_bytes()), file.name))
}

fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
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

