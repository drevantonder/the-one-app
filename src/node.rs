//! A node: this device's network (a custom build holds one), its replica and views, its socket to the relay, and where
//! calls run.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
use crate::protocol::{CallRequest, Event};
use crate::bucket::{Copied, Place};
use crate::database::stored::Db;
use crate::database::content::Content;
use crate::database::steps::Step;
use crate::keys::{Hash, Key, hex, short, unhex};
use crate::database::scopes::{Scope, new_id};
use crate::database::scopes::{self, ScopeDb};
use crate::database::content;
use crate::database::scopes::{Principal, ScopeState};
use crate::config::{CLI, YOURS};
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
mod versions;
pub use versions::heard;
#[path = "onboarding/pairing.rs"]
mod pairing;
pub(crate) mod push;
pub(crate) mod releases;
mod scheduled;
pub(crate) mod screens;
mod sessions;
#[path = "insights/node.rs"]
mod insights;
pub use databases::NotGiven;
pub use net::{Endpoints, alpn, relay, service_alpn};
pub use calls::{Elsewhere, Grant};
use calls::{Granted, Reply};
use apps::built_in_bundle;
use scheduled::switch_entry;
use services::added_scope;
pub use crate::keys::Needs;
pub mod services;
mod app_schedules;
mod settings;
pub(crate) use settings::routes as settings_routes;
pub(crate) use holds::routes as holds_routes;
mod state;
mod status;
mod steps;
pub mod storage;
pub mod trace;
pub mod vault;
pub mod widgets;
mod caller;
mod exclusive;
mod holds;
mod home;
mod network;

pub use caller::{Caller, Credential};
pub use holds::{HeldShare, HeldSign, HeldWrite, Holder, sign_request};
use exclusive::*;
pub use home::{device_here, device_key, load_local_token, runtime_endpoint, runtime_folder};
use home::*;
use network::*;
pub use network::NetworkFile;
pub use pairing::{Joined, join_with};

/// The folder in a node's home its app runtime's workerd keeps its files in.
const RUNTIME_DIR: &str = "runtime";

const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A device whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

const TICK: Duration = Duration::from_secs(10);
/// How often a device reads what other devices wrote that no notice told it of, for a notice that was lost; and as it
/// wakes from sleep, when it may have missed many.
const REFILL: Duration = Duration::from_secs(120);
/// How often a refill reads every scope from the bucket whatever the relay says moved (`Node::moved`), for a write it
/// wasn't told of.
const WHOLE_REFILL: Duration = Duration::from_secs(1800);
/// How old what the relay said of writes (`moved`) may be for a refill to go by it.
const MOVED_FRESH: Duration = Duration::from_secs(60);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this device's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// How often a device says hello to the others: how loaded it is.
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
    /// What each computer runs as processes of its oned, and each one's state, as its onet last said (`oned::keep_parts`).
    parts: HashMap<Key, Value>,
    /// The devices connected to the relay, as it last said: `Network::online`.
    connected: std::collections::HashSet<Key>,
    /// What each device told the relay it's on, as it last said: its key → `protocol::version`.
    versions: Value,
    /// What the relay says the Workers are on, and the device they're deployed from (`workers` in its answer).
    workers: Value,
    /// How many writes the relay has counted to each scope (`moved` in its answer), and when it said so.
    moved: Option<(Instant, Value)>,
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
    /// What a device runs, as it told the relay (`versions`): for an app runtime, what its device does.
    fn running(&self, key: &Key) -> Option<crate::release::Version> {
        let (commit, committed, target) = crate::protocol::told(self.versions[hex(&self.r.db().device_of(key))].as_str()?)?;
        Some(crate::release::Version { commit: commit.into(), target: target.into(), committed })
    }

    /// Whether a device is on, the same answer every node gives: what the relay says.
    fn online(&self, device: &Key) -> bool {
        self.connected.contains(device)
    }

    /// Whether this device can count on reaching that device or service endpoint: it said hello here lately.
    fn heard(&self, device: &Key) -> bool {
        self.peers.get(device).is_some_and(|p| p.seen.elapsed() < 3 * PRESENCE_EVERY)
    }
}

/// What a machine's node is: the device its user's CLI, MCP and Mac app sign with, and compute.
pub const MACHINE: &[scopes::Kind] = &[scopes::Kind::Client, scopes::Kind::Compute];


thread_local!(static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) });

pub struct Inner {
    dir: PathBuf,
    /// What this machine's own programs send to say they are: `api::LOCAL_TOKEN`, a file only its user reads.
    local_token: String,
    /// This node's own key, its endpoint's: what it signs and is handed keys under as itself.
    key: SigningKey,
    id: Key,
    /// What its device's key opens and signs.
    holds: holds::Holds,
    name: Mutex<String>,
    apps: Apps,
    network: Exclusive<Option<Network>>,
    pairings: Mutex<HashMap<[u8; 16], pairing::Pairing>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this device.
    changed: tokio::sync::Notify,
    /// Woken by each service endpoint's hello, for a call waiting for one that's starting.
    heard: tokio::sync::Notify,
    /// What the record at the network's address last said (`crate::latest`), and why it couldn't take it, if so.
    latest: Mutex<crate::latest::Latest>,
    bundles: Mutex<BTreeMap<String, String>>,
    /// The bundle of each app this release has built in, kept as a blob: any network may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png`, its finished app icon, as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The logos of the sites apps and services connect to.
    logos: crate::logos::Logos,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The port apps' requests go out through.
    outbound: u16,
    calls: calls::State,
    insights: insights::State,
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
    /// Apps' keys the bucket hadn't, by scope, app and epoch, as of who's who at that position (`Node::sealed_keys`).
    no_app_keys: Mutex<HashMap<(Scope, String, u32), Option<u64>>>,
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
    /// The guide's address on the tailnet, `http://<address>:<port>/llms.txt`, while this node serves it there (`api::serve_guide`).
    guide: Mutex<Option<String>>,
    /// Whether it was asked to erase what this device keeps, which its oned does (`oned::keep_parts`).
    erasing: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub struct Node(Arc<Inner>);

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({})", short(&self.0.id))
    }
}


/// What a node stopping waits at most for its last insights to be written: they may be lost, and stopping may not wait.
const LAST_SAY: Duration = Duration::from_secs(3);

/// `oned apps --listen=<address>`: this device's app runtime, an endpoint of its own key (`Node::runtime`), answering on
/// `address` only its device's own programs, with the token in its folder, and asking its device's key, held by the
/// computer's onet at the API's address, for what that opens and signs. It runs its own watchdog.
#[tokio::main]
pub async fn serve(listen: &str) -> Result<()> {
    crate::logs::keep(crate::logs::NODE);
    tracing_subscriber::fmt()
        .with_writer(crate::logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into()))
        .init();
    crate::watchdog::started();
    // Asked to stop, even while it starts: its workerd first, then at once, rather than once every blocking task
    // ends, which a stuck request would hold up. Its endpoint is closed, so other devices connect at once to the one
    // oned starts next.
    let stop = crate::os::stop_asked()?;
    let (ready_node, node_ready) = tokio::sync::oneshot::channel::<Node>();
    tokio::spawn(async move {
        stop.await;
        crate::insights::Ended::Signal.write(&crate::home());
        if let Ok(Ok(node)) = tokio::time::timeout(LAST_SAY, node_ready).await {
            let _ = tokio::time::timeout(LAST_SAY, async { tokio::join!(node.close_net(), node.flush_insights()) }).await;
        }
        crate::app::runtime::stop_all();
        std::process::exit(0)
    });
    let dir = crate::home();
    let token = crate::api::local_token(&dir).with_context(|| format!("{} has no token for this device's own programs: its onet makes it as it starts", dir.display()))?;
    let node = Node::runtime(&dir, Holder { at: crate::api::addr(), token }).await?;
    let _ = ready_node.send(node.clone());
    node.node_started(crate::insights::Ended::read(&dir));
    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("listen on {listen}"))?;
    tracing::info!("{} app runtime {} answering {listen} from {}", crate::config::NAME, crate::keys::short(&node.id()), dir.display());
    tokio::spawn(crate::api::serve_guide(node.clone()));
    crate::watchdog::watch();
    let served = crate::api::serve(listener, crate::api::guarded(crate::api::routes(), node)).await;
    crate::app::runtime::stop_all();
    Ok(served?)
}

impl Node {
    /// Starts a node of the home `dir` that is its own device, holding the device's key: the tests' devices. A
    /// computer's is its app runtime (`runtime`), whose device is its onet.
    pub async fn start(dir: &Path) -> Result<Node> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let key = SigningKey::from(unhex(&device.secret)?);
        Node::open(dir, key.clone(), device.name, load_local_token(dir)?, holds::Holds::Key(key)).await
    }

    /// Starts the app runtime of the device whose home is `dir`, an endpoint of its own key, listed as a compute
    /// naming the device: its key, and the token only its device's own programs ask it with, are in its folder
    /// (`runtime_folder`); what the device's key opens and signs, it asks `holder`. Its data is the device's home's.
    pub async fn runtime(dir: &Path, holder: holds::Holder) -> Result<Node> {
        let folder = runtime_folder(dir);
        std::fs::create_dir_all(&folder)?;
        let holds = holds::Holds::asking(holder).await?;
        Node::open(dir, load_runtime_key(&folder)?, hostname(), load_local_token(&folder)?, holds).await
    }

    async fn open(dir: &Path, key: SigningKey, name: String, local_token: String, holds: holds::Holds) -> Result<Node> {
        let id = *key.verifying_key().as_bytes();
        let outbound = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let apps = Apps::start(dir.join(RUNTIME_DIR), outbound.local_addr()?.port()).await?;
        let (events, _) = broadcast::channel(256);
        let mut built_in = BTreeMap::new();
        for (app, files) in crate::config::BUILT_IN {
            built_in.insert(app.to_string(), release::put_blob(dir, &built_in_bundle(files)?)?);
        }
        let node = Node(Arc::new(Inner {
            dir: dir.to_path_buf(),
            local_token,
            holds,
            key,
            id,
            name: Mutex::new(name),
            apps,
            network: Exclusive::new(None),
            pairings: Mutex::new(pairing::load(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            heard: tokio::sync::Notify::new(),
            latest: Mutex::new(Default::default()),
            bundles: Mutex::new(BTreeMap::new()),
            built_in,
            images: Mutex::new(HashMap::new()),
            logos: crate::logos::Logos::open(dir),
            grants: Mutex::new(HashMap::new()),
            outbound: outbound.local_addr()?.port(),
            calls: Default::default(),
            insights: Default::default(),
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
            no_app_keys: Default::default(),
            issuer_keys: Default::default(),
            net: Default::default(),
            settled: tokio::sync::watch::channel(0).0,
            settling: tokio::sync::Mutex::new(()),
            unfetched: Default::default(),
            erased: Default::default(),
            guide: Default::default(),
            erasing: Default::default(),
        }));
        let outbound_router = crate::app::outbound::router(node.clone());
        tokio::spawn(async move {
            if let Err(err) = axum::serve(outbound, outbound_router).await {
                tracing::warn!("apps' outbound requests stopped: {err:#}");
            }
        });
        if dir.join(NETWORK_DIR).join(NETWORK_FILE).exists() {
            node.take_network()?;
        }
        tokio::spawn(node.clone().pull_loop());
        tokio::spawn(node.clone().invites_loop());
        tokio::spawn(node.clone().presence_loop());
        tokio::spawn(node.clone().tick_loop());
        tokio::spawn(node.clone().backup_loop());
        tokio::spawn(node.clone().schedule_loop());
        tokio::spawn(node.clone().insights_loop());
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

    /// Holds this device's writes for one that it signs itself.
    pub(crate) async fn own_writes(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.writes.lock().await
    }

    pub fn dir(&self) -> &Path {
        &self.0.dir
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

    #[track_caller]
    fn with<R>(&self, f: impl FnOnce(&mut Network) -> Result<R>) -> Result<R> {
        let mut network = self.0.network.lock()?;
        let _held = crate::watchdog::Held::new();
        f(network.as_mut().with_context(|| not_joined(&self.device_name()))?)
    }

    /// `with`, for what answers a plain yes or no, or a list, and can't fail: `default` when it's in no network yet, or
    /// when its who's who couldn't be read, which the log says.
    #[track_caller]
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
        let listed = network.as_ref().and_then(|i| i.as_ref()).and_then(|i| i.r.db().machine(&self.0.holds.device()).inspect_err(|err| tracing::warn!("reading this device's name: {err:#}")).ok().flatten().map(|m| m.name));
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
    /// The network this device's home names, opened and read, as it starts, or as its onet joins one while it runs.
    fn take_network(&self) -> Result<()> {
        self.open_network().context("open the network")?;
        if let Err(err) = self.bucket().and_then(|b| b.forget_sent()) {
            tracing::warn!("forgetting steps other devices sent: {err:#}");
        }
        self.with(update_views)?;
        self.keep_alone_here()?;
        self.read_own_items();
        self.follow_net();
        self.read_all_status(false);
        let reader = self.clone();
        tokio::spawn(async move {
            if let Err(err) = reader.read_scopes(None).await {
                tracing::warn!("reading the bucket: {err:#}");
            }
            reader.share_blobs().await;
        });
        Ok(())
    }

    async fn tick_loop(self) {
        let (mut refilled, mut whole): (Option<SystemTime>, Option<SystemTime>) = (None, None);
        // What the relay said of writes at the last refill.
        let mut seen = Value::Null;
        for tick in 1u64.. {
            tokio::time::sleep(TICK).await;
            // A network its device's onet joined since it started, which writes it to their home.
            if !self.has_network() && self.0.dir.join(NETWORK_DIR).join(NETWORK_FILE).exists() {
                match self.take_network() {
                    Ok(()) => self.emit(Event::Network),
                    Err(err) => tracing::warn!("{err:#}"),
                }
            }
            let mut read = false;
            if self.has_network() {
                // The wall clock, which runs on while a machine sleeps, as `Instant` doesn't everywhere.
                // A runtime its device hasn't listed yet reads who's who each tick, to run things from when it has.
                let due = |at: Option<SystemTime>, every| at.is_none_or(|at| at.elapsed().unwrap_or_default() >= every) || !self.ready();
                if due(refilled, REFILL) {
                    refilled = Some(SystemTime::now());
                    // Only the scopes the relay counted writes to since the last refill, while it answers; all of them now
                    // and then, and whenever it doesn't.
                    let moved = self.with_or(None, |i| Ok(i.moved.as_ref().filter(|(at, _)| at.elapsed() < MOVED_FRESH).map(|(_, m)| m.clone())));
                    let only = match moved {
                        Some(now) if !due(whole, WHOLE_REFILL) => Some(moved_since(&seen, &now)),
                        _ => {
                            whole = refilled;
                            None
                        }
                    };
                    seen = self.with_or(Value::Null, |i| Ok(i.moved.as_ref().map(|(_, m)| m.clone()).unwrap_or_default()));
                    read = only.as_ref().is_none_or(|o| !o.is_empty());
                    if read {
                        if let Err(err) = self.read_scopes(only.as_ref()).await {
                            tracing::debug!("reading the bucket: {err:#}");
                        }
                        self.fill_copies(only.as_ref()).await;
                    }
                }
                self.forget_sessions();
                self.read_all_status(!read);
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

/// The scopes whose count of writes the relay said changed between `was` and `now` (`moved`, each scope's name to a
/// count).
fn moved_since(was: &Value, now: &Value) -> HashSet<Scope> {
    let names = now.as_object().into_iter().chain(was.as_object()).flat_map(|m| m.keys());
    names.filter(|s| was.get(s.as_str()) != now.get(s.as_str())).filter_map(|s| Scope::parse(s).ok()).collect()
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

/// What a computer says when its node isn't in a network yet, whatever else is signed in on it (the Mac app's own
/// sign-in is a client's): what the command line, `status` and a call that needs one all tell to do.
pub fn not_joined(device: &str) -> String {
    format!("{device} isn't in {YOURS} yet: {CLI} join <code> joins it with a pairing code, or {CLI} create <name> --as <your name> starts a new one")
}
