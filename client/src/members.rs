//! Who's who as the install's own signed entries say it, checked on the device with the nodes' own Rust (`one`'s
//! entry checks and member replay), never as a node says it: a node holds no keys, and a hacked one could list a seal
//! key of its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device
//! seals, and to whom it hands keys, it takes only from here.
//!
//! A device keeps `global`'s entries it has checked and the install's root, the genesis it saw when it joined, and
//! refuses entries of an install with another; then it asks a node only for entries it doesn't have
//! (`GET /v1/entries/global?have=`, `have`).

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};

use one::entry::{Entry, Hash, Key, Scope, causal, hex, pack, unpack};
use one::members::{Cuts, Members, is_member_entry};

/// `global`'s entries a device holds, checked, and the install they're of.
pub struct Global {
    root: Option<Hash>,
    entries: BTreeMap<Hash, Entry>,
}

impl Global {
    /// For the install whose genesis is `root`, or, for a device that's only now joined, whichever one genesis the
    /// first entries it's given hold.
    pub fn new(root: Option<Hash>) -> Global {
        Global { root, entries: BTreeMap::new() }
    }

    /// The install's genesis, once known.
    pub fn root(&self) -> Option<Hash> {
        self.root
    }

    /// Adds a pack's entries (`entry::pack`), each checked as from anyone: signature, body and content.
    pub fn add(&mut self, packed: &[u8]) -> Result<()> {
        self.add_entries(unpack(packed)?)
    }

    /// Adds entries already checked as from anyone (`Entry::decode`): refused, all of them, if one isn't of `global`,
    /// or starts another install than this one.
    pub fn add_entries(&mut self, entries: Vec<Entry>) -> Result<()> {
        for e in &entries {
            ensure!(e.scope() == Scope::Global, "a node gave an entry of {} as one of everyone's", e.log());
        }
        let geneses: Vec<Hash> = entries.iter().chain(self.entries.values()).filter(|e| e.content.app == one::members::APP && e.content.kind == "genesis").map(Entry::hash).collect();
        let root = match self.root {
            Some(root) => root,
            None => match geneses.as_slice() {
                [only, rest @ ..] if rest.iter().all(|g| g == only) => *only,
                [] => bail!("those aren't any install's entries: none starts it"),
                _ => bail!("those entries start more than one install"),
            },
        };
        ensure!(geneses.iter().all(|g| *g == root), "those entries are another install's, not your One App's");
        self.root = Some(root);
        for e in entries {
            self.entries.entry(e.hash()).or_insert(e);
        }
        Ok(())
    }

    /// Every entry held, packed, to keep.
    pub fn pack(&self) -> Vec<u8> {
        pack(&self.entries.values().collect::<Vec<_>>())
    }

    /// What to ask a node for only what's new: `<key>.<seq>,…`, the next seq of each author's log held.
    pub fn have(&self) -> String {
        let mut next: BTreeMap<Key, u32> = BTreeMap::new();
        for e in self.entries.values() {
            let n = next.entry(e.author()).or_default();
            *n = (*n).max(e.seq() + 1);
        }
        next.iter().map(|(k, n)| format!("{}.{n}", hex(k))).collect::<Vec<_>>().join(",")
    }

    /// The members, replayed as a node replays them: in causal order, each entry counting only as `one::members`
    /// says, a device that signed two entries at one seq cut from there and removed. Refused if they aren't the
    /// install's, or its genesis isn't among them.
    pub fn members(&self) -> Result<Members> {
        let Some(root) = self.root else { bail!("no entries of your One App yet") };
        let mut seen: HashMap<(Key, u32), Hash> = HashMap::new();
        let mut forks = Cuts::new();
        for e in self.entries.values() {
            if seen.insert((e.author(), e.seq()), e.hash()).is_some_and(|h| h != e.hash()) {
                let at = forks.entry((e.author(), "global".into())).or_insert(u32::MAX);
                *at = (*at).min(e.seq());
            }
        }
        let ordered: Vec<Entry> = causal(self.entries.values().cloned().collect(), &HashMap::new()).into_iter().filter(is_member_entry).collect();
        let m = Members::replay(Some(root), &ordered, &forks);
        ensure!(!m.users.is_empty(), "your One App's first entry isn't among those");
        Ok(m)
    }
}

/// What a device takes from the members to seal and hand keys by, as JSON (the web client's, `kernel/web/device.js`):
///
/// - `root`, `name`, `recovery` (hex, what each epoch is sealed to besides devices)
/// - `devices`: each endpoint listed now, `{ key, seal, user, name, roles, passkey }`, and the serverless node, as an
///   admin's `serverless` setting names it (its seal key its key's twin), as `{ key, seal, roles: ["compute"] }`
/// - `openers`: each scope's client devices of the people who may use it, `{ key, seal }`: what its epochs are sealed to
/// - `roles`: each scope's people and their role in it (`admin` in their own): who may use it, and start its epochs
/// - `runs_for`: each compute endpoint's scopes whose calls it runs, the serverless node's too (a person's own only as
///   their own device chose): the only nodes handed a scope's keys
/// - `runs`: where each person's own things run besides their own machines, as their own device chose
/// - `epochs`: each scope's newest epoch `n` and every epoch's hash, which a key must match to be used
pub fn view(m: &Members) -> Value {
    let scopes: Vec<Scope> = m.scopes();
    let mut devices: Vec<Value> = m
        .machines
        .values()
        .filter(|d| m.machine(&d.key).is_some())
        .map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal), "user": d.user, "name": d.name, "roles": d.roles, "passkey": d.passkey }))
        .collect();
    let mut openers = Map::new();
    let mut roles = Map::new();
    for s in &scopes {
        let people: Map<String, Value> = m.users.keys().filter_map(|u| Some((u.clone(), json!(m.role_in(u, s)?)))).collect();
        if people.is_empty() {
            continue;
        }
        let to: Vec<Value> = m.clients().filter(|c| m.may_use(&c.user, s)).map(|c| json!({ "key": hex(&c.key), "seal": hex(&c.seal) })).collect();
        openers.insert(s.to_string(), json!(to));
        roles.insert(s.to_string(), json!(people));
    }
    let mut runs_for: Map<String, Value> = m.computes().map(|c| (hex(&c.key), json!(scopes.iter().filter(|s| m.runs_for(&c.key, s)).map(Scope::to_string).collect::<Vec<_>>()))).collect();
    if let Some(key) = m.serverless()
        && let Ok(seal) = one::kernel::twin_public(&key)
    {
        devices.push(json!({ "key": hex(&key), "seal": hex(&seal), "roles": ["compute"] }));
        runs_for.insert(hex(&key), json!(scopes.iter().filter(|s| m.runs_for(&key, s)).map(Scope::to_string).collect::<Vec<_>>()));
    }
    let epochs: Map<String, Value> = m
        .epochs
        .iter()
        .map(|(s, e)| {
            let hashes: Map<String, Value> = m.epoch_hashes.range((s.clone(), 0)..=(s.clone(), u32::MAX)).map(|((_, n), h)| (n.to_string(), json!(h))).collect();
            (s.to_string(), json!({ "n": e.n, "hashes": hashes }))
        })
        .collect();
    let runs: Map<String, Value> = m.runs.iter().map(|(u, on)| (u.clone(), json!(on.iter().map(hex).collect::<Vec<_>>()))).collect();
    json!({
        "root": m.root.map(|r| r.to_hex()), "name": m.name, "recovery": m.recovery.as_ref().map(hex),
        "devices": devices, "openers": openers, "roles": roles, "runs_for": runs_for, "runs": runs, "epochs": epochs,
    })
}
