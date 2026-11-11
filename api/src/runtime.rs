//! The app runtime as a feature's runtime crate sees it (`AppRuntime`, behind `Rt`), and how a feature registers with
//! it (`Runs`): a feature's runtime half is its own crate, `<feature>/runtime/`, which names no runtime type, only these.
//! Each method is one a feature, or two, needs of its runtime; a feature that needs another adds it here and to the
//! runtime's impl.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};

use database::stored::Db;
use key::Key;
use scope::content::Content;
use scope::read::Read;
use scope::scopes::Scope;

use crate::call::{CallRequest, Event};
use crate::{Caller, Routes};

/// What a method that waits answers with.
pub type Fut<'a, T> = BoxFuture<'a, Result<T>>;

/// The runtime a feature runs in.
pub type Rt = Arc<dyn AppRuntime>;

/// Everything a feature reaches of its runtime. A function needing less names less (`&dyn Holds`).
pub trait AppRuntime: Here + ScopeDatabase + Databases + Holds + Apps + Peers + Features {}
impl<T: Here + ScopeDatabase + Databases + Holds + Apps + Peers + Features + ?Sized> AppRuntime for T {}

/// This app runtime, on its device.
pub trait Here: Any + Send + Sync {
    /// Its own endpoint: what its rows and lines name.
    fn id(&self) -> Key;
    fn device_name(&self) -> String;
    /// Its home: a feature keeps its own files in `home()/<feature>/`.
    fn home(&self) -> &Path;
    /// In a network, and has read it.
    fn ready(&self) -> bool;
    /// Its device's user, as its own calls ask.
    fn caller(&self) -> Result<Caller>;
    fn is_this_machine(&self, caller: &Caller) -> bool;
    /// The ports of its own, which a request it makes for someone never reaches.
    fn own_ports(&self) -> Vec<u16>;
    /// The network's address, where its users sign in: none until an admin sets one.
    fn address(&self) -> Result<String>;
    /// Tells whoever watches (`/api/events`).
    fn emit(&self, event: Event);
    /// Its own key, never its device's: what signs its asks and the notes it sends.
    fn signing_key(&self) -> key::SigningKey;
    /// The route of the API's that `path` asks, as its list names it (`/api/apps/{app}`): none for none of them.
    fn route_of(&self, path: &str) -> Option<&'static str>;
    /// The names of the client's own screens, as this program carries them.
    fn client_screens(&self) -> Vec<&'static str>;
    /// The source map of one of the client's own screens, by its name.
    fn client_map(&self, screen: &str) -> Option<&'static str>;
}

/// Who's who: the scope database of each scope it holds, as it last read them; and steps written to it.
pub trait ScopeDatabase: Send + Sync {
    fn read_dyn(&self, f: &mut dyn FnMut(&Read<'_>) -> Result<()>) -> Result<()>;
    /// Signs a step of `scope`'s as `user`, and waits for it to be taken.
    fn append<'a>(&'a self, scope: &'a Scope, user: &'a str, content: Content) -> Fut<'a, ()>;
    /// Signs a step of `global`'s for `caller` with this device, which counts only if the device is theirs, and runs
    /// here what it changed (a deploy, an app added) before it's done.
    fn as_member<'a>(&'a self, caller: &'a Caller, content: Content) -> Fut<'a, ()>;
    /// Reads the scope databases of the scopes `caller` uses that this device runs but doesn't open, with the keys they
    /// hand (428, `handed::Needs`, when they didn't): what their calls are checked by, and written after.
    fn read_callers_scopes<'a>(&'a self, caller: &'a Caller) -> Fut<'a, ()>;
}

/// `read` and `read_or`, for every `ScopeDatabase`: its closure is sync, so nothing waits while who's who is held.
pub trait ReadExt {
    fn read<R>(&self, f: impl FnOnce(&Read<'_>) -> Result<R>) -> Result<R>;

    /// `read`, for what answers a plain yes or no, or a list, and can't fail: `default` when it's in no network yet, or
    /// when who's who couldn't be read, which the log says.
    fn read_or<R>(&self, default: R, f: impl FnOnce(&Read<'_>) -> Result<R>) -> R {
        self.read(f).unwrap_or_else(|err| {
            tracing::debug!("reading who's who: {err:#}");
            default
        })
    }
}

impl<T: ScopeDatabase + ?Sized> ReadExt for T {
    fn read<R>(&self, f: impl FnOnce(&Read<'_>) -> Result<R>) -> Result<R> {
        let (mut f, mut out) = (Some(f), None);
        self.read_dyn(&mut |r| {
            if let Some(f) = f.take() {
                out = Some(f(r)?);
            }
            Ok(())
        })?;
        out.ok_or_else(|| anyhow::anyhow!("who's who wasn't read"))
    }
}

/// Copies of apps' and the runtime's own databases in each scope, opened for a caller as their keys open them: one
/// this device can't open yet fails `NotGiven`.
pub trait Databases: Send + Sync {
    /// `app`'s `database` in `scope`, for `caller`, read up to date: made with `schema` if it's new.
    fn open<'a>(&'a self, caller: &'a Caller, scope: &'a Scope, app: &'a str, database: &'a str, schema: &'a [&'a str]) -> Fut<'a, Arc<tokio::sync::Mutex<Db>>>;
    /// Changes it: `change` runs in its write, again if another device wrote first, and the devices that hold `scope`
    /// are told.
    fn write<'a>(&'a self, caller: &'a Caller, scope: &'a Scope, app: &'a str, database: &'a str, schema: &'a [&'a str], change: &'a mut (dyn FnMut(&rusqlite::Connection) -> Result<()> + Send)) -> Fut<'a, ()>;
    /// Refuses (428, `handed::Needs`) a caller that wasn't handed the keys of all of `wants` (scope, app, database).
    fn needs<'a>(&'a self, caller: &'a Caller, wants: &'a [(Scope, &'a str, &'a str)]) -> Fut<'a, ()>;
    /// An object of the network's bucket, by its name (`database::keys` names them): none if it has none.
    fn object<'a>(&'a self, name: &'a str) -> Fut<'a, Option<Vec<u8>>>;
    /// The network's bucket, as a place: where everything is kept, which a feature that copies it reads.
    fn bucket(&self) -> Result<bucket::Place>;
    /// Where the network's bucket is, as this device keeps it (`api::home::NetworkFile`'s `bucket`): what a device it
    /// admits starts from. None for a folder of its own.
    fn bucket_fields(&self) -> Option<BTreeMap<String, String>>;
    /// Whether this runtime is an admin's, whose endpoint alone the URL signer lets remove from the bucket.
    fn removes_from_bucket(&self) -> bool;
    /// The network's bucket from `fields` (`bucket_fields`'s): a folder here, or S3 through the URL signer at the
    /// network's address, signing as its device.
    fn place(&self, fields: &BTreeMap<String, String>) -> Result<bucket::Place>;
    /// Keeps the network's data in `to` (from `fields`) from now on, once `copy` is done, which copies there what it
    /// reaches of the bucket it had: its writes wait until then, and it reads every database again from there.
    fn keep_in<'a>(&'a self, to: bucket::Place, fields: BTreeMap<String, String>, copy: Fut<'a, ()>) -> Fut<'a, ()>;
}

/// A scope this device wasn't given the newest key of: a device of someone who opens it seals it to this one. Its
/// device's name, and whose the scope is as its users read it (`Read::scope_name`: a user's or group's name, or Everyone).
#[derive(Debug)]
pub struct NotGiven(pub Scope, pub String, pub String);

impl std::fmt::Display for NotGiven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} wasn't given the newest key to {}'s things yet: a device of someone who opens them gives it when it's next on", self.1, self.2)
    }
}

impl std::error::Error for NotGiven {}

/// A device that couldn't be reached to ask it something: what its caller is told.
#[derive(Debug)]
pub struct NotThere(pub String);

impl std::fmt::Display for NotThere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotThere {}

/// No device asked ran a call: how the last one didn't (an `error.type` of Insights'), and what each said.
#[derive(Debug)]
pub struct NoneCould {
    pub kind: &'static str,
    pub why: Vec<String>,
}

impl std::fmt::Display for NoneCould {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "None of your devices can do this now. Try again soon. ({})", self.why.join("; "))
    }
}

impl std::error::Error for NoneCould {}

/// What its device's key opens, asked of its holder.
pub trait Holds: Send + Sync {
    fn device(&self) -> Key;
    /// Every epoch's key of `scope` the device's key opens, by number: none if it opens none yet.
    fn epochs<'a>(&'a self, scope: &'a Scope) -> Fut<'a, database::stored::Keys>;
    /// The Vault secret keys of `scope` the device's key opens, of those `wanted` (number, public key).
    fn vault_keys<'a>(&'a self, scope: &'a Scope, wanted: &'a [(u32, [u8; 32])]) -> Fut<'a, BTreeMap<u32, [u8; 32]>>;
    /// A request of who can do what (`settings::answered`), asked by this machine's own programs and answered as its
    /// device, which signs what it changes.
    fn answer_as_device<'a>(&'a self, method: &'a str, path: &'a str, body: &'a [u8]) -> Fut<'a, Value>;
    /// The app schedules in force for this device to run `app`'s schedules in `scope`, their keys opened: → until when
    /// they last. Fails when this device doesn't run them.
    fn app_schedules_keys<'a>(&'a self, scope: &'a Scope, app: &'a str) -> Fut<'a, u64>;
    /// The keys of `app`'s `database` in `scope`, as its device opens them.
    fn database_keys<'a>(&'a self, scope: &'a Scope, app: &'a str, database: &'a str) -> Fut<'a, database::stored::Keys>;
    /// Tells the relay something (`scope::settings::tell`), signed by its device, or by this runtime's own key.
    fn tell_relay(&self, fields: Value, by_device: bool) -> Fut<'_, ()>;
    /// Asks the network's auth something as its device (`scope::settings::ask_auth`).
    fn ask_auth<'a>(&'a self, method: &'a str, path: &'a str, fields: Value) -> Fut<'a, Value>;
}

/// The apps it runs.
pub trait Apps: Send + Sync {
    /// What `app` says of itself, as its deploy runs here.
    fn describe<'a>(&'a self, app: &'a str) -> Fut<'a, Value>;
    /// The apps as `viewer` lists them (`GET /api/apps`).
    fn apps_for<'a>(&'a self, viewer: &'a Caller) -> Fut<'a, Vec<Value>>;
    /// Runs a tool for `caller`, whom this device acts for: here, or on the nearest device that can.
    fn call<'a>(&'a self, req: CallRequest, caller: Caller) -> Fut<'a, Value>;
    /// The apps a page shows for `viewer`: with `scope`, those added there, each reaching only it; without, every app,
    /// reaching all it may.
    fn page_apps(&self, viewer: &Caller, scope: Option<&str>) -> Result<Vec<PageApp>>;
    /// What `app` says of itself: one added here, or a built-in app no one has added, which touches no data.
    fn describe_any<'a>(&'a self, app: &'a str) -> Fut<'a, Value>;
    /// What users call `app`: its title, or its service's, else its name.
    fn title<'a>(&'a self, app: &'a str) -> Fut<'a, String>;
    /// The bundle this release has of the built-in app `app`, if it's one.
    fn built_in(&self, app: &str) -> Option<String>;
    /// Whether this release carries the blob `hash` as a built-in app's bundle.
    fn carries(&self, hash: &str) -> bool;
    /// Adds the built-in app `app` if the network doesn't have it yet, as this release has it, and runs it here.
    /// Admins only.
    fn bring_in<'a>(&'a self, caller: &'a Caller, app: &'a str) -> Fut<'a, ()>;
    /// What `app`'s latest deploy says it uses, (apps, services), once this device runs it, or a while.
    fn uses<'a>(&'a self, app: &'a str) -> Fut<'a, (Vec<String>, Vec<String>)>;
    /// The deploy of `app` this device runs (its bundle), if it runs one.
    fn bundle_here(&self, app: &str) -> Option<String>;
    /// Whether this device runs `app`'s latest deploy, waiting up to `within` for it.
    fn runs_latest<'a>(&'a self, app: &'a str, within: Duration) -> Fut<'a, bool>;
    /// Where an app that `described` itself can run its service for `viewer`, if it has one: their own computers' names.
    fn computers_for(&self, viewer: &Caller, described: &Value) -> Option<Vec<String>>;
    /// The apps it runs, but those in Recently Deleted.
    fn names(&self) -> Vec<String>;
    /// Runs one of `app`'s tools here in `scope`, as `user`, with the keys of the app schedules this device holds there
    /// and no others, counted as a schedule's call.
    fn call_scheduled<'a>(&'a self, scope: &'a Scope, app: &'a str, user: &'a str, tool: &'a str, input: Value) -> Fut<'a, Value>;
}

/// An app a page shows, as it draws for who's looking.
pub struct PageApp {
    pub app: String,
    /// What it reaches for them, that this device holds.
    pub scopes: Vec<Scope>,
    pub draws: Arc<dyn Draws>,
}

/// What an app draws for who's looking: its requests are theirs for as long as this is held.
pub trait Draws: Send + Sync {
    fn widget<'a>(&'a self, name: &'a str, size: &'a str, options: &'a Value) -> Fut<'a, Value>;
    fn widget_options<'a>(&'a self, name: &'a str) -> Fut<'a, Value>;
    /// The count on its icon.
    fn badge(&self) -> Fut<'_, Value>;
}

/// The network's other devices, as this runtime reaches them and the relay says who's connected.
pub trait Peers: Send + Sync {
    /// Whether `device` answers now: connected to it already, or once it connects.
    fn reaches<'a>(&'a self, device: &'a Key) -> BoxFuture<'a, bool>;
    /// That `device` is off, for whoever asked it something: its name, and when it was last seen.
    fn away(&self, device: &Key) -> String;
    /// What each device and the Workers are on, as each last told the relay, and whether a newer build is published for
    /// each.
    fn versions(&self) -> Fut<'_, Value>;
}

/// The features it runs.
pub trait Features: Send + Sync {
    /// The state a feature registered (`Runs.state`), by its type.
    fn state_dyn(&self, of: TypeId) -> Option<&(dyn Any + Send + Sync)>;
    /// Runs `feature`'s loop (`Runs.every`) now.
    fn wake(&self, feature: &str);
}

/// `rt.state::<T>()`: a feature's own state, which it registered, so only a bug fails to find it.
pub trait StateExt {
    fn state<T: Any>(&self) -> &T;
}

impl<F: Features + ?Sized> StateExt for F {
    fn state<T: Any>(&self) -> &T {
        self.state_dyn(TypeId::of::<T>()).and_then(|s| (s as &dyn Any).downcast_ref::<T>()).unwrap_or_else(|| panic!("no state {} registered", std::any::type_name::<T>()))
    }
}

/// What serves a tool, as the caller, with the input its schema let through. Its answer is held to its `output`
/// schema. `serves!` makes one of an `async fn`.
pub type Serves = fn(Rt, Caller, Map<String, Value>) -> Fut<'static, Value>;

/// A `Serves` from its `async fn(Rt, Caller, Map<String, Value>) -> Result<Value>`.
#[macro_export]
macro_rules! serves {
    ($f:path) => {
        |rt, me, input| Box::pin($f(rt, me, input))
    };
}

/// A feature, as its runtime runs it: one line in the runtime's `features()`.
pub struct Runs {
    /// As its tools are named: `vault` of `vault_list`.
    pub feature: &'static str,
    /// What serves each tool its `Feature` lists, by name.
    pub tools: Vec<(&'static str, Serves)>,
    /// Its routes that aren't tools.
    pub routes: Routes<Rt>,
    /// What it keeps while the runtime runs (`StateExt::state`).
    pub state: Box<dyn Any + Send + Sync>,
    /// Once, as the runtime starts, before its loop: what it follows of another feature (the Vault's `watch`).
    pub start: Option<fn(&Rt)>,
    /// Its loop: this often, at once as the runtime starts and on `wake`.
    pub every: Option<(Duration, fn(Rt) -> Fut<'static, ()>)>,
    /// What needs the viewer's attention, if anything: `state.features.<feature>` of `GET /api/state`, which badges a
    /// Settings item (`scope::state::settings_items`).
    pub problem: Option<fn(&Rt, &Caller) -> Result<Option<Value>>>,
    /// What it follows of who's who, as it happens: quick, and never reading who's who itself.
    pub on: Option<fn(&Rt, &Happened)>,
}

/// What happened in who's who, which a feature follows (`Runs.on`).
pub enum Happened {
    /// A scope's database took steps (for `global`, its members changed).
    Steps { scope: Scope },
}

impl Runs {
    /// One that does nothing yet: `Runs { tools, .., ..Runs::named("vault") }`.
    pub fn named(feature: &'static str) -> Runs {
        Runs { feature, tools: Vec::new(), routes: Routes::default(), state: Box::new(()), start: None, every: None, problem: None, on: None }
    }
}

/// What a feature's runtime crate uses to reach its runtime: `use api::runtime::prelude::*;`.
pub mod prelude {
    pub use super::{AppRuntime, Apps, Databases, Draws, Features, Fut, Here, Holds, PageApp, Peers, ReadExt, Rt, ScopeDatabase, StateExt};
}
