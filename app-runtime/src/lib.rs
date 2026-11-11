//! The app runtime (`Host`): this device's network (a custom build holds one), its replica and views, its socket to the
//! relay, and where calls run. What a feature's runtime half reaches of it is `api::AppRuntime` (`runtime.rs`).

use custom_build::now_ms;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::app::runtime::{Apps, Context};
use ::scope::state::{Me, handles};
use ::api::call::Append;
use ::api::call::{CallRequest, Event};
use crate::copied::Copied;
use ::bucket::Place;
use ::database::stored::Db;
use ::scope::content::Content;
use ::scope::steps::Step;
use ::key::{Hash, Key, hex, short, unhex};
use ::scope::scopes::{Scope, new_id};
use ::scope::scopes::{self, ScopeDb};
use ::scope::content;
use ::scope::scopes::{Principal, ScopeState};
use ::custom_build::config::YOURS;
use ::scope::rank;
use ::app_schedules::Schedule;
use ::scope::replica::{Handled, Replica};

/// The built-in apps this build carries, and those a new network starts with (build.rs).
pub mod built_in {
    include!(concat!(env!("OUT_DIR"), "/built_in.rs"));
}

pub mod api;
pub mod app;
pub mod apps;
pub mod copied;
mod calls;
mod databases;
mod keys;
mod sync;
mod net;
mod versions;
pub(crate) mod releases;
pub(crate) mod screens;
mod sessions;
use ::api::runtime::NotGiven;
pub use calls::Grant;
use calls::{Granted, Reply};
use ::deploy::build::built_in_bundle;
pub use ::api::handed::Needs;
pub mod services;
mod app_schedules;
mod settings;
pub(crate) use settings::routes as settings_routes;
pub(crate) use holds::routes as holds_routes;
mod state;
mod steps;
use ::api::trace;
mod caller;
pub mod runtime;
mod exclusive;
mod watchdog;
mod holds;
mod network;

use ::api::{Caller, Credential};
pub use holds::Holder;
use exclusive::*;
use ::api::home::*;

/// The folder in an app runtime's home its app runtime's workerd keeps its files in.
const RUNTIME_DIR: &str = "runtime";

const CALL_LIMIT: Duration = Duration::from_secs(30);
/// A device whose round trip is longer than this is far: a nearer one runs what it would have, whatever their order.
const FAR: Duration = Duration::from_millis(250);

const TICK: Duration = Duration::from_secs(10);
/// How often a device reads what other devices wrote that no notice told it of, for a notice that was lost; and as it
/// wakes from sleep, when it may have missed many.
const REFILL: Duration = Duration::from_secs(120);
/// How often a refill reads every scope from the bucket whatever the relay says moved (`Host::moved`), for a write it
/// wasn't told of.
const WHOLE_REFILL: Duration = Duration::from_secs(1800);
/// How old what the relay said of writes (`moved`) may be for a refill to go by it: two of its answers.
const MOVED_FRESH: Duration = Duration::from_secs(2 * 60);
/// How long a read waits for the bucket, when a ping said its database changed, before it reads this device's copy.
const BUCKET_WAIT: Duration = Duration::from_secs(5);
/// How often this device asks the relay which devices are on and when each was seen.
const RELAY_EVERY: Duration = Duration::from_secs(60);
/// How long what another device said of what it runs (`Host::ask_state`) stands before it's asked again.
const STATE_FRESH: Duration = Duration::from_secs(60);
const REFETCH_AFTER: Duration = Duration::from_secs(30);

/// A service serving on a device: its name, the users it serves, and what it describes of itself.
pub type Service = (String, Vec<String>, Value);

/// What another device said of itself in its last answer here: how loaded it is, and why it hasn't taken the latest
/// update, if it says.
struct Said {
    load: Option<::scope::state::Load>,
    updating: Option<String>,
}

struct Network {
    r: Replica,
    user: String,
    /// The network's bucket, and its fields, which a device joining is given.
    bucket: Option<(Place, BTreeMap<String, String>)>,
    /// What each device said of itself in its last answer here.
    said: HashMap<Key, Said>,
    /// When this device last asked or answered each other one.
    exchanged: HashMap<Key, Instant>,
    /// What each service of another device describes of itself, by its key, as that device's runtime last said when
    /// asked (`Host::ask_state`).
    described: HashMap<Key, Value>,
    /// When each other runtime was last asked what its device runs.
    stated: HashMap<Key, Instant>,
    /// What each computer runs as processes of its oned, and each one's state: this one's as its onet said
    /// (`oned::keep_parts`), another's as its runtime said when asked.
    parts: HashMap<Key, Value>,
    /// The devices connected to the relay, as it last said: `Network::online`.
    connected: std::collections::HashSet<Key>,
    /// When the relay last saw each key (ms): its socket opening or closing, or a Worker asked of it.
    last_seen: HashMap<Key, u64>,
    /// What each device told the relay it's on, as it last said: its key → `protocol::version`.
    versions: Value,
    /// What the relay says the Workers are on, and the device they're deployed from (`workers` in its answer).
    workers: Value,
    /// How many writes the relay has counted to each scope (`moved` in its answer), and when it said so.
    moved: Option<(Instant, Value)>,
    /// Once `global`'s log is frozen, each other scope's own database as its steps leave it (`database::steps`): the
    /// last position read, and the database, once its anchor checked out here.
    scope_dbs: HashMap<Scope, ::scope::steps::ScopeReader>,
    /// Each scope's own state, as its database says it.
    views: HashMap<String, ScopeState>,
    /// How long each device took to answer a call lately.
    nearest: HashMap<Key, Duration>,
    /// How far each device is: the round trip iroh measured on the connection of its last answer here.
    far: HashMap<Key, Duration>,
    /// Whether this device has read every scope it holds from the bucket since it woke.
    read: bool,
}

impl Network {
    /// What a device runs, as it told the relay (`versions`): for an app runtime, what its device does.
    fn running(&self, key: &Key) -> Option<::oned_update::release::Version> {
        let (commit, committed, target) = ::custom_build::protocol::told(self.versions[hex(&self.r.db().device_of(key))].as_str()?)?;
        Some(::oned_update::release::Version { commit: commit.into(), target: target.into(), committed })
    }

    /// Whether a device is on, the same answer every app runtime gives: what the relay says.
    fn online(&self, device: &Key) -> bool {
        self.connected.contains(device)
    }

    /// Whether this device can count on reaching that device: its socket is open at the relay, or the two talked
    /// within a connection's idle time.
    fn reachable(&self, device: &Key) -> bool {
        self.online(device) || self.exchanged.get(device).is_some_and(|at| at.elapsed() < net::IDLE)
    }

    /// When a device was last seen (ms): now if it's online and answering (not `down`, as `Host::down` says), else the
    /// latest of what the relay saw of it or its runtime and when this device last talked to either.
    fn seen(&self, device: &Key, down: impl Fn(&Key) -> bool) -> Option<u64> {
        let m = self.r.db();
        let device = m.device_of(device);
        let keys = [Some(device), m.runtime_for(&device).ok().flatten()];
        let now = now_ms();
        let mut seen = None;
        for k in keys.into_iter().flatten() {
            if self.online(&k) && !down(&k) {
                return Some(now);
            }
            let talked = self.exchanged.get(&k).map(|at| now.saturating_sub(at.elapsed().as_millis() as u64));
            seen = seen.max(self.last_seen.get(&k).copied()).max(talked);
        }
        seen
    }
}


thread_local!(static HOLDING: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) });

pub struct Inner {
    dir: PathBuf,
    /// What this machine's own programs send to say they are: `api::LOCAL_TOKEN`, a file only its user reads.
    local_token: String,
    /// This app runtime's own key, its endpoint's: what it signs and is handed keys under as itself.
    key: SigningKey,
    id: Key,
    /// What its device's key opens and signs.
    holds: holds::Holds,
    name: Mutex<String>,
    apps: Apps,
    network: Exclusive<Option<Network>>,
    events: broadcast::Sender<Event>,
    /// Held while writing or replaying, so views replay in order.
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that loads deploys and announces this device.
    changed: tokio::sync::Notify,
    /// Woken by each answer that says what a device's services are, and each part registering here, for a call waiting
    /// for a service that's starting.
    heard: tokio::sync::Notify,
    /// This device's parts, registered at its app runtime, which carries their asks to them.
    sockets: ::services::carried::Parts,
    /// The asks this device's parts made of it, so none is taken twice.
    part_asks: ::services::carried::Once,
    /// What the record at the network's address last said (`oned_update::latest`), and why it couldn't take it, if so.
    latest: Mutex<::oned_update::latest::Latest>,
    bundles: Mutex<BTreeMap<String, String>>,
    /// The bundle of each app this release has built in, kept as a blob: any network may add them.
    built_in: BTreeMap<String, String>,
    /// Each bundle's `icon.png`, its finished app icon, as a data URL, once read, or none.
    images: Mutex<HashMap<String, Option<String>>>,
    /// The calls, widgets and hooks running now, by the token their requests carry.
    grants: Mutex<HashMap<String, Grant>>,
    /// The port apps' requests go out through.
    outbound: u16,
    calls: calls::State,
    databases: databases::State,
    /// Steps other devices sent, waiting for the bucket to confirm them, while one ask of it is under way: none then.
    sent: Mutex<Option<Vec<sync::Sent>>>,
    /// The network's recovery keys this device told the relay about since it started: the old ones taken out, the
    /// current one let in as a client (`follow_recovery`).
    recovery_told: Mutex<std::collections::BTreeSet<Key>>,
    /// When it last read `global` for a client device it didn't list (`Host::catch_up`).
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
    owed: Mutex<keys::Owed>,
    /// Apps' keys the bucket hadn't, by scope, app and epoch, as of who's who at that position (`Host::sealed_keys`).
    no_app_keys: Mutex<HashMap<(Scope, String, u32), Option<u64>>>,
    issuer_keys: Mutex<settings::IssuerKeys>,
    net: net::Net,
    /// How many times its deploys were settled (`settle`), the first once its apps are loaded.
    settled: tokio::sync::watch::Sender<u64>,
    /// Held while settling, so one that read the members before a change can't load its apps after one that read it.
    settling: tokio::sync::Mutex<()>,
    /// The erased apps this app runtime erased here since it started (`follow_erased`).
    erased: Mutex<BTreeSet<String>>,
    /// Whether the last settling couldn't fetch an app's deploy, so it's tried again soon, not only at the next change.
    unfetched: std::sync::atomic::AtomicBool,
    /// The guide's address on the tailnet, `http://<address>:<port>/llms.txt`, while this app runtime serves it there (`api::serve_guide`).
    guide: Mutex<Option<String>>,
    /// What its features keep (`runtime::features`).
    kept: runtime::Kept,
}

#[derive(Clone)]
pub struct Host(Arc<Inner>);

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Host({})", short(&self.0.id))
    }
}

/// What an app runtime stopping waits at most for its last insights to be written: they may be lost, and stopping may not wait.
const LAST_SAY: Duration = Duration::from_secs(3);

/// `<cli>-apps --listen-socket=<socket>`: this device's app runtime, an endpoint of its own key (`Host::runtime`), answering on
/// `listener`, which oned keeps, only its device's own programs, with the token in its folder, and asking its device's key, held by the
/// computer's onet at the API's address, for what that opens and signs. It runs its own watchdog.
#[tokio::main]
pub async fn serve(listener: std::net::TcpListener) -> Result<()> {
    ::device_logs::keep(::device_logs::NODE);
    tracing_subscriber::fmt()
        .with_writer(::device_logs::Tee)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "app_runtime=info,one=info,warn".into()))
        .init();
    crate::watchdog::started();
    // Asked to stop, even while it starts: its workerd first, then at once, rather than once every blocking task
    // ends, which a stuck request would hold up. Its endpoint is closed, so other devices connect at once to the one
    // oned starts next.
    let stop = platform::os::stop_asked()?;
    let (ready_host, host_ready) = tokio::sync::oneshot::channel::<Host>();
    tokio::spawn(async move {
        stop.await;
        ::insights::Ended::Signal.write(&platform::home());
        if let Ok(Ok(host)) = tokio::time::timeout(LAST_SAY, host_ready).await {
            let rt = host.rt();
            let _ = tokio::time::timeout(LAST_SAY, async { tokio::join!(host.close_net(), insights_runtime::flush(&rt)) }).await;
        }
        crate::app::runtime::stop_all();
        std::process::exit(0)
    });
    let dir = platform::home();
    let token = ::api::local::local_token(&dir).with_context(|| format!("{} has no token for this device's own programs: its onet makes it as it starts", dir.display()))?;
    let host = Host::runtime(&dir, Holder { at: ::api::local::addr(), token }).await?;
    let _ = ready_host.send(host.clone());
    // This runtime starting, and why the last one ended: what it said when it stopped, or `abort` if it didn't (panics
    // abort, so a crash can't say), or `first` if there was none before it. An update's new one says so too.
    let last = ::insights::Ended::read(&dir);
    insights_runtime::device_signal(&host.rt(), "app-runtime.start", ::insights::Attrs::default().reason(last.reason()), ::insights::Status::Unset);
    if last == ::insights::Ended::Update {
        insights_runtime::device_signal(&host.rt(), "update.running", ::insights::Attrs::default().part(&::oned_update::latest::Thing::program_name(custom_build::config::TARGET)), ::insights::Status::Unset);
    }
    let listener = tokio::net::TcpListener::from_std(listener)?;
    tracing::info!("{} app runtime {} answering {} from {}", ::custom_build::config::NAME, ::key::short(&host.id()), listener.local_addr()?, dir.display());
    tokio::spawn(crate::api::serve_guide(host.clone()));
    crate::watchdog::watch();
    let served = ::api::serve(listener, crate::api::guarded(crate::api::routes(), host)).await;
    crate::app::runtime::stop_all();
    Ok(served?)
}

impl Host {
    /// Starts an app runtime of the home `dir` that is its own device, holding the device's key: the tests' devices. A
    /// computer's is its app runtime (`runtime`), whose device is its onet.
    pub async fn start(dir: &Path) -> Result<Host> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let key = SigningKey::from(unhex(&device.secret)?);
        Host::open(dir, key.clone(), device.name, load_local_token(dir)?, holds::Holds::Key(key)).await
    }

    /// Starts the app runtime of the device whose home is `dir`, an endpoint of its own key, listed as a compute
    /// naming the device: its key, and the token only its device's own programs ask it with, are in its folder
    /// (`runtime_folder`); what the device's key opens and signs, it asks `holder`. Its data is the device's home's.
    pub async fn runtime(dir: &Path, holder: holds::Holder) -> Result<Host> {
        let folder = runtime_folder(dir);
        std::fs::create_dir_all(&folder)?;
        let holds = holds::Holds::asking(holder).await?;
        Host::open(dir, load_runtime_key(&folder)?, hostname(), load_local_token(&folder)?, holds).await
    }

    async fn open(dir: &Path, key: SigningKey, name: String, local_token: String, holds: holds::Holds) -> Result<Host> {
        let id = *key.verifying_key().as_bytes();
        let outbound = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let apps = Apps::start(dir.join(RUNTIME_DIR), outbound.local_addr()?.port()).await?;
        let (events, _) = broadcast::channel(256);
        let mut built_in = BTreeMap::new();
        for (app, files) in crate::built_in::BUILT_IN {
            built_in.insert(app.to_string(), ::database::put_blob(dir, &built_in_bundle(files)?)?);
        }
        let (kept, starts) = runtime::Kept::of(runtime::features());
        let host = Host(Arc::new(Inner {
            dir: dir.to_path_buf(),
            local_token,
            holds,
            key,
            id,
            name: Mutex::new(name),
            apps,
            network: Exclusive::new(None),
            events,
            writes: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            heard: tokio::sync::Notify::new(),
            sockets: Default::default(),
            part_asks: Default::default(),
            latest: Mutex::new(Default::default()),
            bundles: Mutex::new(BTreeMap::new()),
            built_in,
            images: Mutex::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
            outbound: outbound.local_addr()?.port(),
            calls: Default::default(),
            databases: Default::default(),
            sent: Mutex::new(None),
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
            kept,
        }));
        let outbound_router = crate::app::outbound::router(host.clone());
        tokio::spawn(async move {
            if let Err(err) = axum::serve(outbound, outbound_router).await {
                tracing::warn!("apps' outbound requests stopped: {err:#}");
            }
        });
        if dir.join(NETWORK_DIR).join(NETWORK_FILE).exists() {
            host.take_network()?;
        }
        tokio::spawn(host.clone().pull_loop());
        tokio::spawn(host.clone().invites_loop());
        tokio::spawn(host.clone().tick_loop());
        host.start_features(starts);
        let worker = host.clone();
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
        Ok(host)
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

    /// The guide's address on the tailnet, if this app runtime serves it there.
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
        f(network.as_mut().with_context(|| ::onboarding::not_joined(&self.device_name()))?)
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
            return Err(::custom_build::protocol::Coded::error(::custom_build::protocol::code::ALREADY_IN_NETWORK, format!("this device already belongs to a network, and {} holds one; for another, network that context's own app", ::custom_build::config::NAME)));
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
        self.wake_all();
        self.follow_net();
        let reader = self.clone();
        tokio::spawn(async move {
            if let Err(err) = reader.read_scopes(None).await {
                tracing::warn!("reading the bucket: {err:#}");
            }
            database_runtime::share_all(&reader.rt()).await;
        });
        Ok(())
    }

    async fn tick_loop(self) {
        let (mut refilled, mut whole): (Option<SystemTime>, Option<SystemTime>) = (None, None);
        // What the relay said of writes at the last refill.
        let mut seen = Value::Null;
        let (mut relayed, mut paths_said): (Option<Instant>, Instant) = (None, Instant::now());
        for tick in 1u64.. {
            tokio::time::sleep(TICK).await;
            // A network its device's onet joined since it started, which writes it to their home.
            if !self.has_network() && self.0.dir.join(NETWORK_DIR).join(NETWORK_FILE).exists() {
                match self.take_network() {
                    Ok(()) => self.emit(Event::Network),
                    Err(err) => tracing::warn!("{err:#}"),
                }
            }
            if self.has_network() {
                // A computer's runtime is given what the relay says as its onet hears it.
                if matches!(self.0.holds, holds::Holds::Key(_)) && relayed.is_none_or(|at| at.elapsed() >= RELAY_EVERY) {
                    relayed = Some(Instant::now());
                    self.read_connected();
                }
                if paths_said.elapsed() >= sync::PATHS_EVERY {
                    paths_said = Instant::now();
                    self.log_paths();
                }
                self.close_idle();
                self.renew_app_schedules("");
                self.forget_app_schedules();
                self.start_owed_keys();
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
                    if only.as_ref().is_none_or(|o| !o.is_empty()) {
                        if let Err(err) = self.read_scopes(only.as_ref()).await {
                            tracing::debug!("reading the bucket: {err:#}");
                        }
                        self.fill_copies(only.as_ref()).await;
                    }
                }
                self.forget_sessions();
            }
            if tick % 6 == 0 && self.has_network() {
                self.tidy_databases().await;
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
/// (`Host::read_scope_steps_now`).
fn update_views(i: &mut Network) -> Result<()> {
    if i.r.freeze().is_some() {
        i.views.insert("global".to_string(), i.r.db().state()?);
    }
    Ok(())
}

/// The scopes a call of `app` as `user` reaches (`state::reachable`), as this device can tell: of each scope it holds,
/// by what it read.
fn reachable(i: &Network, app: &str, user: &str) -> Result<Vec<Scope>> {
    ::scope::state::reachable(i.r.db(), user, |s| i.r.holds(s).then(|| i.views.get(&s.to_string()).is_some_and(|st| st.added.contains(app))))
}

fn host_me(host: &Host, db: &ScopeDb, caller: &Caller) -> Result<Me> {
    let name = if caller.key == host.0.id { host.device_name() } else { db.machine(&caller.key)?.map(|k| k.name).unwrap_or_default() };
    ::scope::state::me(db, &caller.user, hex(&caller.key), name, caller.agent.clone())
}

fn scope_name(i: &Network, scope: &str) -> Result<String> {
    let db = i.r.db();
    Ok(match Scope::parse(scope)? {
        Scope::Global => "Everyone".to_string(),
        Scope::User(u) => db.user(&u)?.map(|u| u.name).unwrap_or(u),
        Scope::Group(g) => db.group(&g)?.map(|g| g.name).unwrap_or(g),
    })
}
