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
    /// A blob, by its hash: from this device, else from the bucket, in a scope whose database names it.
    fn blob<'a>(&'a self, hash: &'a str) -> Fut<'a, Vec<u8>>;
    /// Keeps `bytes` as a blob `caller` put, which they may then read back: → its hash.
    fn put_blob_for(&self, caller: &Caller, bytes: &[u8]) -> Result<String>;
    /// Whether `caller` may read the blob `hash`: this machine's own programs, whoever put it, or someone a scope they
    /// may use names it for.
    fn may_read_blob(&self, caller: &Caller, hash: &str) -> Result<bool>;
}

/// A scope this device wasn't given the newest key of: a device of someone who opens it seals it to this one.
#[derive(Debug)]
pub struct NotGiven(pub Scope, pub String);

impl std::fmt::Display for NotGiven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} wasn't given {}'s newest key: a device of someone who opens it gives it when it's next on", self.1, self.0)
    }
}

impl std::error::Error for NotGiven {}

/// What its device's key opens, asked of its holder.
pub trait Holds: Send + Sync {
    fn device(&self) -> Key;
    /// The Vault secret keys of `scope` the device's key opens, of those `wanted` (number, public key).
    fn vault_keys<'a>(&'a self, scope: &'a Scope, wanted: &'a [(u32, [u8; 32])]) -> Fut<'a, BTreeMap<u32, [u8; 32]>>;
    /// A request of who can do what (`settings::answered`), asked by this machine's own programs and answered as its
    /// device, which signs what it changes.
    fn answer_as_device<'a>(&'a self, method: &'a str, path: &'a str, body: &'a [u8]) -> Fut<'a, Value>;
}

/// The apps it runs.
pub trait Apps: Send + Sync {
    /// What `app` says of itself, as its deploy runs here.
    fn describe<'a>(&'a self, app: &'a str) -> Fut<'a, Value>;
    /// The apps as `viewer` lists them (`GET /api/apps`).
    fn apps_for<'a>(&'a self, viewer: &'a Caller) -> Fut<'a, Vec<Value>>;
    /// The logo of a site an app connects to, as a data URL and its kind.
    fn logo(&self, domain: &str, app: &str) -> Option<(String, String)>;
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
    /// Adds the built-in app `app` if the network doesn't have it yet, as this release has it, and runs it here.
    /// Admins only.
    fn bring_in<'a>(&'a self, caller: &'a Caller, app: &'a str) -> Fut<'a, ()>;
    /// What `app`'s latest deploy says it uses, (apps, services), once this device runs it, or a while.
    fn uses<'a>(&'a self, app: &'a str) -> Fut<'a, (Vec<String>, Vec<String>)>;
    /// Whether this device runs `app`'s latest deploy, waiting up to `within` for it.
    fn runs_latest<'a>(&'a self, app: &'a str, within: Duration) -> Fut<'a, bool>;
    /// Where an app that `described` itself can run its service for `viewer`, if it has one: their own devices'
    /// names, and `cloudflare` for an admin when it's built for that.
    fn nodes_for(&self, viewer: &Caller, described: &Value) -> Option<Vec<String>>;
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
}

impl Runs {
    /// One that does nothing yet: `Runs { tools, .., ..Runs::named("vault") }`.
    pub fn named(feature: &'static str) -> Runs {
        Runs { feature, tools: Vec::new(), routes: Routes::default(), state: Box::new(()), start: None, every: None, problem: None }
    }
}

/// What a feature's runtime crate uses to reach its runtime: `use api::runtime::prelude::*;`.
pub mod prelude {
    pub use super::{AppRuntime, Apps, Databases, Draws, Features, Fut, Here, Holds, PageApp, Peers, ReadExt, Rt, ScopeDatabase, StateExt};
}
