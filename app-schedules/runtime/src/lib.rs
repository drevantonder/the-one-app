//! App schedules, as an app runtime runs them: an app's schedules run on the machine its user chose for them (its app
//! schedules, whose keys the runtime holds), with those keys alone: each slot once, whichever such machine takes it
//! first, and none while every one is off. How each ran is its rows (`rows`); what a user sees and switches, `view`.

mod routes;
mod rows;
mod run;
mod view;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use api::runtime::Runs;
use scope::scopes::Scope;

pub use view::{failing, schedules_of, switch_entry};

/// How often it looks for slots that came.
const TICK: Duration = Duration::from_secs(10);

pub fn runs() -> Runs {
    Runs { routes: routes::routes(), state: Box::new(State::default()), every: Some((TICK, |rt| Box::pin(run::tick(rt)))), ..Runs::named("app-schedules") }
}

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// Each schedule's next slot, by (scope, app, schedule), with what it was worked out from.
    next: Mutex<HashMap<(Scope, String, String), (Value, u64)>>,
    /// Each scope's runs, as this device last read them.
    rows: Mutex<HashMap<Scope, rows::Runs>>,
    /// When it last read every scope's runs.
    read: Mutex<u64>,
    /// The scopes whose runs builds before kept in the app runtime's `main` it copied (`rows::copy_once`).
    copied: Mutex<HashSet<Scope>>,
}
