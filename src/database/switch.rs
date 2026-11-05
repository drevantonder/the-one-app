//! Phase 2's switch (docs/plan-storage-and-permissions.md): the scope database becomes the truth, and the log is
//! frozen. It's one constant, `SWITCH`, and it waits for its gate: every device listed in the install that reads who's
//! who (each person's devices and each machine that runs things, and the serverless node) says it reads the scope
//! database (`READS`), or an admin said it's gone for good (`SKIPS`).
//!
//! A device says what it reads as it asks: a client in every request (`HEADER`, from the client crate, which every
//! client is made of: browsers, phones, Macs, Windows and the MCP Worker's devices), and a node of itself, as it says
//! its build. The node asked keeps it in `global`'s node database, where every node reads it (`src/node/status.rs`).

use std::collections::{BTreeSet, HashMap};

use crate::database::scopes::ScopeDb;
use crate::entry::{Key, hex, unhex};

/// Whether an admin's device freezes the log once the gate is open, and the scope database becomes the truth. Off
/// until every device runs a build that reads it and the switch is decided: the one place to switch it on.
pub const SWITCH: bool = false;

/// What this build reads of who's who: 1, the log, replayed (R1 and before, which say nothing); 2, the scope database,
/// once the log is frozen. A device that reads less than `GATE` holds the switch back.
pub const READS: u32 = 1;

/// What every listed device must read before the log is frozen.
pub const GATE: u32 = 2;

/// The header a client says what it reads in, with its build: `<reads> <build>`. A node before the gate ignores it.
pub const HEADER: &str = "x-one-reads";

/// The setting an admin lists devices in (their keys, hex, separated by spaces) that are gone for good, which the
/// gate doesn't wait for.
pub const SKIPS: &str = "switch_skips";

/// What a device said it reads, its build, and when (ms).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub reads: u32,
    pub build: String,
    pub time: u64,
}

/// The header's value this build sends.
pub fn header() -> String {
    format!("{READS} {}", crate::config::BUILD)
}

/// A header's value, as a device said it: None for one that isn't.
pub fn parse(value: &str) -> Option<(u32, String)> {
    let (reads, build) = value.trim().split_once(' ').unwrap_or((value.trim(), ""));
    let build = build.trim();
    (build.len() <= 64 && build.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))).then_some(())?;
    Some((reads.parse().ok()?, build.to_string()))
}

/// Where the gate is: the devices it waits for, those an admin said are gone, and whether it's open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// Each listed device that hasn't said it reads `GATE`: its key, and what it said it reads (None for nothing).
    pub waiting: Vec<(Key, Option<Reads>)>,
    /// Devices an admin said are gone for good, still listed.
    pub skipped: Vec<Key>,
}

impl Gate {
    pub fn open(&self) -> bool {
        self.waiting.is_empty()
    }
}

/// The devices whose builds the gate waits for: every listed client (but a passkey, which runs nothing) and machine
/// that runs things, and the serverless node. A service endpoint reads no who's who: its device's node asks for it.
pub fn readers(db: &ScopeDb) -> Vec<Key> {
    let mut keys: Vec<Key> = db.machines().into_iter().filter(|m| !m.removed && m.passkey.is_none() && (m.is(crate::members::Kind::Client) || m.is(crate::members::Kind::Compute))).map(|m| m.key).collect();
    keys.extend(db.serverless());
    keys.sort();
    keys.dedup();
    keys
}

/// The devices an admin said are gone for good (`SKIPS`).
pub fn skips(db: &ScopeDb) -> BTreeSet<Key> {
    db.setting(SKIPS).unwrap_or_default().split_whitespace().filter_map(|k| unhex(k).ok()).collect()
}

/// The gate, as `global`'s scope database lists devices and `reads` says what each reads.
pub fn gate(db: &ScopeDb, reads: &HashMap<Key, Reads>) -> Gate {
    let skips = skips(db);
    let mut gate = Gate::default();
    for key in readers(db) {
        let said = reads.get(&key);
        if said.is_some_and(|r| r.reads >= GATE) {
            continue;
        }
        if skips.contains(&key) {
            gate.skipped.push(key);
        } else {
            gate.waiting.push((key, said.cloned()));
        }
    }
    gate
}

/// The gate as `GET /v1/state` says it, with each device's name and person.
pub fn gate_json(db: &ScopeDb, reads: &HashMap<Key, Reads>) -> serde_json::Value {
    let g = gate(db, reads);
    let named = |k: &Key| {
        let m = db.machine_ever(k);
        serde_json::json!({ "key": hex(k), "name": m.as_ref().map(|m| m.name.clone()).or_else(|| (db.serverless() == Some(*k)).then(|| "the serverless node".into())), "user": m.map(|m| m.user) })
    };
    let waiting: Vec<serde_json::Value> = g
        .waiting
        .iter()
        .map(|(k, r)| {
            let mut v = named(k);
            v["reads"] = serde_json::json!(r.as_ref().map(|r| r.reads));
            v["build"] = serde_json::json!(r.as_ref().map(|r| r.build.clone()));
            v
        })
        .collect();
    serde_json::json!({ "switch": SWITCH, "open": g.open(), "gate": GATE, "waiting": waiting, "skipped": g.skipped.iter().map(named).collect::<Vec<_>>() })
}
