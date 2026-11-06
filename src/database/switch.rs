//! Phase 2's switch (docs/plan-storage-and-permissions.md): the scope database becomes the truth, and the log is
//! frozen, by the ceremony an admin runs once on a network made before it (`one switch`, `Node::switch`); a network
//! made since starts frozen. Once it is, a device that reads less than `GATE` is told to update, by the nodes, the
//! relay and the URL signer. Before it, the devices listed in the network that reads who's who (each person's devices
//! and each machine that runs things, and the Cloudflare node) and don't say they read `GATE` are those not updated
//! yet (`gate`), unless an admin said one's gone for good (`SKIPS`).
//!
//! A device says what it reads as it asks: a client in every request (`HEADER`, from the client crate, which every
//! client is made of: browsers, phones, Macs, Windows and the remote MCP server's devices), and a node of itself, as it says
//! its build. The node asked keeps it in `global`'s node database, where every node reads it (`src/node/status.rs`).

use std::collections::{BTreeSet, HashMap};

use crate::database::scopes::ScopeDb;
use crate::entry::{Key, hex, unhex};

/// What this build reads of who's who: 1, the log, replayed (R1 and before, which say nothing); 2 and on, the scope
/// database, once the log is frozen, with the types, roles, rules and tables of that number. A device that reads less
/// than `GATE` holds the switch back. Once it's switched, a release that changes `scope::SCHEMA`, `permissions::SCHEMA`
/// or `permissions::defaults` reads one more, has `steps::normalize` read the older tables, and its admins' devices
/// move each database on to it once every device reads it (`steps::DEFAULTS`); `steps::tests` holds each number to
/// what it reads.
pub const READS: u32 = 2;

/// What a device must read once the log is frozen: one that reads less is told to update.
pub const GATE: u32 = 2;

const _: () = assert!(READS >= GATE, "this build must read what a frozen network asks of every device: READS >= GATE");

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

/// Where the gate is: the devices that don't read what it asks, those an admin said are gone, and whether it's open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// Each listed device that hasn't said it reads what the gate asks: its key, and what it said it reads (None for
    /// nothing).
    pub waiting: Vec<(Key, Option<Reads>)>,
    /// Devices an admin said are gone for good, still listed.
    pub skipped: Vec<Key>,
}

impl Gate {
    pub fn open(&self) -> bool {
        self.waiting.is_empty()
    }
}

/// The devices whose builds the gate asks about: every listed client (but a passkey, which runs nothing) and machine
/// that runs things, and the Cloudflare node. A service endpoint reads no who's who: its device's node asks for it.
pub fn readers(db: &ScopeDb) -> Vec<Key> {
    let mut keys: Vec<Key> = db.machines().into_iter().filter(|m| !m.removed && m.passkey.is_none() && (m.is(crate::members::Kind::Client) || m.is(crate::members::Kind::Compute))).map(|m| m.key).collect();
    keys.extend(db.cloudflare_node());
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
    gate_at(db, reads, GATE)
}

/// The gate for what reads `at_least`: the switch's (`GATE`), or a release's that moves the database on to it.
pub fn gate_at(db: &ScopeDb, reads: &HashMap<Key, Reads>, at_least: u32) -> Gate {
    let skips = skips(db);
    let mut gate = Gate::default();
    for key in readers(db) {
        let said = reads.get(&key);
        if said.is_some_and(|r| r.reads >= at_least) {
            continue;
        }
        // Only a client is skipped: a machine that runs things writes to the bucket itself, under epochs it no longer
        // knows once its install switched, so one gone for good is removed, which the bucket's URLs stop for; and the
        // Cloudflare node never is, which no one removes.
        if skips.contains(&key) && db.compute(&key).is_none() && db.cloudflare_node() != Some(key) {
            gate.skipped.push(key);
        } else {
            gate.waiting.push((key, said.cloned()));
        }
    }
    gate
}

/// The gate as `GET /v1/state` says it, with each device's name and person: before the freeze, the devices not updated
/// yet, which are refused once it's frozen.
pub fn gate_json(db: &ScopeDb, reads: &HashMap<Key, Reads>) -> serde_json::Value {
    let g = gate(db, reads);
    let named = |k: &Key| {
        let m = db.machine_ever(k);
        serde_json::json!({ "key": hex(k), "name": m.as_ref().map(|m| m.name.clone()).or_else(|| (db.cloudflare_node() == Some(*k)).then(|| "the Cloudflare node".into())), "user": m.map(|m| m.user) })
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
    serde_json::json!({ "open": g.open(), "gate": GATE, "waiting": waiting, "skipped": g.skipped.iter().map(named).collect::<Vec<_>>() })
}
