//! Backups, as an app runtime runs them: which scopes it backs up to which places, hourly when anything changed, and
//! how each run went, in `global`'s rows so every device shows it (`rows`). The format, and opening a backup with the
//! recovery key alone, are `engine`; the places, `places`.

mod cli;
pub mod engine;
mod places;
mod routes;
mod rows;
mod run;
mod tools;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;

use api::runtime::prelude::*;
use api::runtime::{Happened, Runs};
use key::Key;
use scope::scopes::Scope;

pub use cli::commands;
pub use run::{add_place, back_up_now, check_backups, request_place, status};

/// How often it looks at what's due.
const TICK: Duration = Duration::from_secs(60);

pub fn runs() -> Runs {
    Runs {
        tools: tools::served(),
        routes: routes::routes(),
        state: Box::new(State::default()),
        start: Some(|rt| vault_runtime::watch(rt, vault_runtime::Watch { scope: Scope::Global, kind: backups::KIND, saved: |rt, caller, id, answered| Box::pin(async move { run::saved_place(&rt, &caller, &id, answered).await }) })),
        every: Some((TICK, |rt| Box::pin(run::tick(rt)))),
        problem: Some(places::problem),
        on: Some(|rt, happened| match happened {
            Happened::Steps { scope } => rt.state::<State>().note_change(scope),
        }),
        ..Runs::named("backups")
    }
}

/// What it keeps while its runtime runs.
#[derive(Default)]
pub struct State {
    /// When this runtime first looked at what's due: its checks wait a while after.
    started: OnceLock<u64>,
    /// Whether the next run backs up whether or not an hour passed.
    forced: AtomicBool,
    /// When this device last tried telling admins about each place's problem without it being marked as told (it
    /// reached none, or the mark didn't save), so it tries hourly rather than every tick.
    alert_tried: Mutex<HashMap<String, u64>>,
    /// The newest snapshot this device knows of each (place, scope), with its key's id.
    backed: Mutex<HashMap<(String, String), (engine::Snapshot, String)>>,
    /// When each place's last run here failed, so it's tried again an hour later, not every minute.
    failed: Mutex<HashMap<String, u64>>,
    /// When this device last pruned each place, by its id.
    pruned: Mutex<HashMap<String, u64>>,
    /// When this device last copied each (place, scope)'s databases from the bucket.
    looked: Mutex<HashMap<(String, String), u64>>,
    /// When each scope's database first and last took steps since this runtime started.
    changed: Mutex<HashMap<Scope, (u64, u64)>>,
    /// How backups went, as the rows last said.
    rows: Mutex<rows::Rows>,
    /// Whether it has copied the rows builds before kept in the app runtime's `main` (`rows::copy_once`).
    copied: AtomicBool,
}

impl State {
    fn note_change(&self, scope: &Scope) {
        let now = custom_build::now_ms();
        self.changed.lock().unwrap().entry(scope.clone()).and_modify(|c| c.1 = now).or_insert((now, now));
    }
}

/// What the places and their problems are read with, beside who's who: the rows, this device's folders and its own
/// view of `global`'s Vault, as they are now.
pub(crate) struct Seen {
    rows: rows::Rows,
    folders: BTreeMap<String, String>,
    changed: HashMap<Scope, (u64, u64)>,
    own: vault::Items,
    device: Key,
}

impl Seen {
    /// When a scope's database first changed after `after` (ms), as this device read it since it started: none if it
    /// hasn't, or this device hasn't seen it change since then.
    fn first_change(&self, scope: &Scope, after: u64) -> Option<u64> {
        self.changed.get(scope).filter(|(_, last)| *last > after).map(|(first, _)| (*first).max(after))
    }
}

pub(crate) fn seen(rt: &Rt) -> Seen {
    let state = rt.state::<State>();
    Seen { rows: state.rows.lock().unwrap().clone(), folders: folders(rt), changed: state.changed.lock().unwrap().clone(), own: vault_runtime::own_items(rt), device: rt.device() }
}

/// The folders on this device it backs up to, by their place's id (`<home>/backups/folders.json`): a folder is only ever
/// this device's own choice, never an item's, which anyone may write.
fn folders(rt: &Rt) -> BTreeMap<String, String> {
    let read = |path: PathBuf| -> Option<BTreeMap<String, String>> { serde_json::from_slice(&std::fs::read(path).ok()?).ok() };
    if let Some(folders) = read(folders_file(rt)) {
        return folders;
    }
    // Builds before this file kept them in the network's own file: taken from there once.
    let before: Option<serde_json::Value> = std::fs::read(rt.home().join("network/network.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let folders: BTreeMap<String, String> = before.and_then(|v| serde_json::from_value(v["folders"].clone()).ok()).unwrap_or_default();
    if !folders.is_empty()
        && let Err(err) = keep_folders(rt, &folders)
    {
        tracing::warn!("keeping backups' folders: {err:#}");
    }
    folders
}

fn folders_file(rt: &Rt) -> PathBuf {
    rt.home().join("backups").join("folders.json")
}

fn keep_folders(rt: &Rt, folders: &BTreeMap<String, String>) -> Result<()> {
    let path = folders_file(rt);
    std::fs::create_dir_all(path.parent().expect("a file in a folder"))?;
    Ok(std::fs::write(&path, serde_json::to_vec_pretty(folders)?)?)
}

/// Keeps the folder of place `id` on this device (`None`: forgets it).
fn keep_folder(rt: &Rt, id: &str, path: Option<&std::path::Path>) -> Result<()> {
    let mut folders = folders(rt);
    match path {
        Some(path) => drop(folders.insert(id.to_string(), path.display().to_string())),
        None => drop(folders.remove(id)),
    }
    keep_folders(rt, &folders)
}
