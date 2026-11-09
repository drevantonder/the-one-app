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

use crate::app::runtime::{Apps, Context};
use crate::state::{Me, handles, whose};
use crate::call::Append;
use crate::bucket::{Copied, Place};
use crate::database::stored::Db;
use crate::database::content::Content;
use crate::database::steps::Step;
use crate::keys::{Hash, Key, hex, short, unhex};
use crate::database::scopes::{Scope, new_id};
use crate::database::scopes::{self, ScopeDb};
use crate::database::content;
use crate::database::scopes::{Principal, ScopeState};
use crate::rank;
use crate::release;
use crate::schedules::{self, Schedule};
use crate::replica::{Handled, Replica};

pub mod apps;
pub mod backups;
pub mod blobs;
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
pub use net::{alpn, relay, service_alpn};
pub use calls::{CallRequest, Elsewhere, Grant};
use calls::{Granted, Reply};
use apps::built_in_bundle;
use scheduled::switch_entry;
use services::added_scope;
pub use releases::Build;
pub use sessions::Needs;
pub mod services;
mod app_schedules;
mod settings;
pub(crate) use settings::routes as settings_routes;
mod state;
mod status;
mod steps;
pub mod storage;
pub mod trace;
pub mod vault;
pub mod widgets;
mod caller;
mod exclusive;
mod home;
mod network;

pub use caller::{Caller, Credential};
use exclusive::*;
pub use home::{device_here, device_key};
use home::*;
use network::*;

const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A device whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

const TICK: Duration = Duration::from_secs(10);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this device's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// How often a device says hello to the others: how loaded it is, and what its parts are doing.
const PRESENCE_EVERY: Duration = Duration::from_secs(15);
const REFETCH_AFTER: Duration = Duration::from_secs(30);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A service serving on a device: its name, the users it serves, and what it describes of itself.
pub type Service = (String, Vec<String>, Value);

/// Another device, as it last said hello (a service endpoint: the service it offers), and how loaded it is.
struct Peer {
    services: Vec<Service>,
    load: Option<crate::load::Load>,
    /// What it can run, and each part's state (`Node::parts_here`), if it says.
    parts: Option<Value>,
    /// Why it hasn't taken the latest update, if it says.
    updating: Option<String>,
    seen: Instant,
}

struct Network {
    r: Replica,
    user: String,
    /// The network's bucket, and its fields, which a device joining is given.
    bucket: Option<(Place, BTreeMap<String, String>)>,
    peers: HashMap<Key, Peer>,
    /// The devices connected to the relay, as it last said: `Network::online`.
    connected: std::collections::HashSet<Key>,
    builds: HashMap<Key, release::Running>,
    /// Once `global`'s log is frozen, each other scope's own database as its steps leave it (`database::steps`): the
    /// last position read, and the database, once its anchor checked out here.
    scope_dbs: HashMap<Scope, crate::database::steps::ScopeReader>,
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
    /// Whether a device is on, the same answer every node gives: what the relay says.
    fn online(&self, device: &Key) -> bool {
        self.connected.contains(device)
    }

    /// Whether this device can count on reaching that device or service endpoint: it said hello here lately.
    fn heard(&self, device: &Key) -> bool {
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

/// What a machine's node is: the device its user's CLI, MCP and Mac app sign with, and compute.
pub const MACHINE: &[scopes::Kind] = &[scopes::Kind::Client, scopes::Kind::Compute];


thread_local!(static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) });

pub struct Inner {
    dir: PathBuf,
    /// What this machine's own programs send to say they are: `api::LOCAL_TOKEN`, a file only its user reads.
    local_token: String,
    key: SigningKey,
    id: Key,
    name: Mutex<String>,
    apps: Apps,
    network: Exclusive<Option<Network>>,
    pairings: Mutex<HashMap<[u8; 16], pairing::Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this device.
    changed: tokio::sync::Notify,
    /// What the record at the network's address last said (`crate::latest`), and why it couldn't take it, if so.
    latest: Mutex<crate::latest::Latest>,
    updating: Mutex<Option<String>>,
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
    /// What the vault gave this device for the sign-ins its calls came with.
    sessions: sessions::Sessions,
    /// Apps whose deploy doesn't start here, by the bundle that didn't: left out until they're deployed again.
    broken: Mutex<BTreeMap<String, String>>,
    /// The keys of the apps whose schedules this device runs for their users.
    app_schedules: app_schedules::State,
    owed: Mutex<devices::Owed>,
    issuer_keys: Mutex<settings::IssuerKeys>,
    net: net::Net,
    /// How many times its deploys were settled (`settle`), the first once its apps are loaded.
    settled: tokio::sync::watch::Sender<u64>,
    /// Held while settling, so one that read the members before a change can't load its apps after one that read it.
    settling: tokio::sync::Mutex<()>,
    /// The erased apps this node erased here since it started (`follow_erased`).
    erased: Mutex<BTreeSet<String>>,
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
            network: Exclusive::new(None),
            pairings: Mutex::new(pairing::load(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            latest: Mutex::new(Default::default()),
            updating: Mutex::new(None),
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
            sessions: Default::default(),
            broken: Default::default(),
            app_schedules: Default::default(),
            owed: Default::default(),
            issuer_keys: Default::default(),
            net: Default::default(),
            settled: tokio::sync::watch::channel(0).0,
            settling: tokio::sync::Mutex::new(()),
            unfetched: Default::default(),
            erased: Default::default(),
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
            node.with(update_views)?;
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
        tokio::spawn(node.clone().pull_loop());
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
        let mut settled = self.0.settled.subscribe();
        let _ = tokio::time::timeout_at(until, settled.wait_for(|n| *n > 0 && self.ready())).await;
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

    /// `with`, for what answers a plain yes or no, or a list, and can't fail: `default` when it's in no network yet, or
    /// when its who's who couldn't be read, which the log says.
    fn with_or<R>(&self, default: R, f: impl FnOnce(&mut Network) -> Result<R>) -> R {
        self.with(f).unwrap_or_else(|err| {
            if self.has_network() {
                tracing::warn!("reading who's who: {err:#}");
            }
            default
        })
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
            return Err(crate::protocol::Coded::error(crate::protocol::code::ALREADY_IN_NETWORK, format!("this device already belongs to a network, and {} holds one; for another, network that context's own app", crate::config::NAME)));
        }
        Ok(())
    }

    /// What `user` is told of `event`: of changed apps, only those they reach.
    pub fn event_for(&self, event: Event, user: &str) -> Event {
        match event {
            Event::Changed { apps } => {
                // An event that can't be checked is told to no one.
                let apps = self.with_or(Vec::new(), |i| {
                    let mut told = Vec::new();
                    for a in apps {
                        if !reachable(i, &a, user)?.is_empty() {
                            told.push(a);
                        }
                    }
                    Ok(told)
                });
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

    /// This device's name: as the network lists it, which another of its user's devices may have changed, when the
    /// network isn't being read (never waiting on it, so it's asked for anywhere); else as it last was.
    pub fn device_name(&self) -> String {
        let network = self.0.network.0.try_lock().ok();
        // A name that can't be read from the network now is the last one, so the error is the log's.
        let listed = network.as_ref().and_then(|i| i.as_ref()).and_then(|i| i.r.db().machine(&self.0.id).inspect_err(|err| tracing::warn!("reading this device's name: {err:#}")).ok().flatten().map(|m| m.name));
        drop(network);
        let mut name = self.0.name.lock().unwrap();
        if let Some(listed) = listed {
            *name = listed;
        }
        name.clone()
    }

    pub fn ready(&self) -> bool {
        self.with(|i| {
            let alone = i.r.db().computes()?.iter().all(|s| s.key == self.0.id);
            Ok(i.r.db().compute(&self.0.id)?.is_some() && (i.read || alone))
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
fn update_views(i: &mut Network) -> Result<()> {
    if i.r.freeze().is_some() {
        i.views.insert("global".to_string(), i.r.db().state()?);
    }
    Ok(())
}

fn note_change(i: &mut Network, scope: &Scope) {
    let now = now_ms();
    i.changed.entry(scope.clone()).and_modify(|c| c.1 = now).or_insert((now, now));
}

/// The scopes a call of `app` as `user` reaches (`state::reachable`), as this device can tell: of each scope it holds,
/// by what it read.
fn reachable(i: &Network, app: &str, user: &str) -> Result<Vec<Scope>> {
    crate::state::reachable(i.r.db(), user, |s| i.r.holds(s).then(|| i.views.get(&s.to_string()).is_some_and(|st| st.added.contains(app))))
}

/// A user's name in the network, removed users' too; none if it never knew them.
fn user_name(db: &ScopeDb, user: &str) -> Result<String> {
    Ok(db.user_ever(user)?.map(|u| u.name).unwrap_or_default())
}

fn node_me(node: &Node, db: &ScopeDb, caller: &Caller) -> Result<Me> {
    let name = if caller.key == node.0.id { node.device_name() } else { db.machine(&caller.key)?.map(|k| k.name).unwrap_or_default() };
    crate::state::me(db, &caller.user, hex(&caller.key), name, caller.agent.clone())
}

fn scope_name(i: &Network, scope: &str) -> Result<String> {
    let db = i.r.db();
    Ok(match Scope::parse(scope)? {
        Scope::Global => "Everyone".to_string(),
        Scope::User(u) => db.user(&u)?.map(|u| u.name).unwrap_or(u),
        Scope::Group(g) => db.group(&g)?.map(|g| g.name).unwrap_or(g),
    })
}

