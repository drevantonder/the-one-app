//! The Vault, as an app runtime runs it: its items in each scope, the runtime's database `vault` there (`vault`),
//! requests for one, which machines open each scope's (`vault grant`), and filling them into apps' and agents'
//! requests. A caller's items open as its other data does: with the device's own keys for its own user, else with the
//! keys the caller hands (`x-one-keys`), that database's alone; and their secrets, hosts and apps only on a machine
//! its admins granted the scope's Vault key, which nobody hands.

mod fetch;
mod items;
mod requests;
mod routes;
mod tools;

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use api::Caller;
use api::runtime::Runs;
use api::runtime::prelude::*;
use scope::scopes::Scope;
use vault::Items;

pub use fetch::names_app;
pub use items::{call_item, items, items_of, lookup, set_item};
pub use requests::{answer_request, request_item, waiting};

/// How an agent is told to ask the user for an item it may use: a request with no app is for agents, the user's own
/// items, which they use with `vault_fetch`.
pub const ASK_FOR_AGENTS: &str = "vault_request, leaving out app,";

/// How often it reads `global`'s items with the device's own keys, for what the runtime itself keeps there.
const OWN_EVERY: Duration = Duration::from_secs(10);

pub fn runs() -> Runs {
    Runs {
        tools: tools::served(),
        routes: routes::routes(),
        state: Box::new(State::default()),
        every: Some((OWN_EVERY, |rt| Box::pin(items::refresh_own(rt)))),
        ..Runs::named("vault")
    }
}

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// `global`'s items, as this device last read them with its own keys: what the runtime itself keeps there.
    own: Mutex<Items>,
    /// The Vault secret keys this device's key opened, by scope and number.
    secrets: Mutex<HashMap<(Scope, u32), [u8; 32]>>,
    /// Those the bucket hadn't, as of who's who at that position: asked for again only once it moves on.
    no_secrets: Mutex<HashMap<(Scope, u32), Option<u64>>>,
    /// What follows items saved here (`watch`).
    watchers: Mutex<Vec<Watch>>,
}

/// What another feature follows of the Vault: each item of `kind` saved in `scope` on this device, by `saved`, with
/// who saved it and the item's id. The Vault names no feature that watches it.
#[derive(Clone)]
pub struct Watch {
    pub scope: Scope,
    pub kind: &'static str,
    pub saved: fn(Rt, Caller, String) -> Fut<'static, ()>,
}

/// Follows items saved here, from as the runtime starts (`Runs.start`).
pub fn watch(rt: &Rt, watch: Watch) {
    rt.state::<State>().watchers.lock().unwrap().push(watch);
}

/// `global`'s items as the device's own keys last read them: what the runtime itself keeps there (none on a device that
/// doesn't open `global`).
pub fn own_items(rt: &Rt) -> Items {
    rt.state::<State>().own.lock().unwrap().clone()
}

/// The Vault secret keys of `scope` this device holds, by number.
type Secrets = BTreeMap<u32, [u8; 32]>;
