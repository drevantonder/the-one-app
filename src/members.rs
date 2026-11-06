//! Members: users, groups, the endpoints of each (devices people use, compute that runs calls, services), epochs,
//! app schedules, apps and deploys, made by replaying `global`'s node entries in causal order; and which entries count: a
//! member entry only when one of its user's own devices signed it, a scope's other entries also when a compute
//! endpoint that runs that scope's calls did. Pure: entries in, members out.
//!
//! Where a scope's calls run is who its devices are: a node runs (and holds) a person's own scope if it's listed as one
//! of their devices, a group's if it's a device of the group's admins or the install's, and `global` if it's an admin's.
//! No one reads another person's data. Each person and group has a handle, unique among them, that every app calls
//! them by (`ctx.handles`): picked as they're replayed, from the name they joined or were made with, so no entry
//! carries it, and changed only by a `rename` that gives one. An entry, once appended, is never lost.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Hash, Key, Scope, hex, is_id, unhex};

/// The node's own entries. Apps never see them.
pub const APP: &str = "node";
/// The app service, as a part of a device's oned its `service` entries switch: running apps' calls.
pub const APPS_PART: &str = "apps";
/// The kinds of node entry that say a release of the node's or a service's program (`release`).
pub const RELEASES: [&str; 2] = ["node-release", "service-release"];
/// Who agents act as when the Vault fills their requests (`vault_fetch`): here too for the Cloudflare node.
pub const AGENTS: &str = "agents";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

/// Who owns an app.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    User(String),
    Group(String),
    /// The install's admins.
    Admins,
}

impl Principal {
    pub fn parse(s: &str) -> Result<Principal> {
        Ok(match s.split_once(':') {
            None if s == "admins" => Principal::Admins,
            Some(("user", id)) if is_id(id) => Principal::User(id.into()),
            Some(("group", id)) if is_id(id) => Principal::Group(id.into()),
            _ => bail!("{s} isn't user:<id>, group:<id> or admins"),
        })
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Principal::User(id) => write!(f, "user:{id}"),
            Principal::Group(id) => write!(f, "group:{id}"),
            Principal::Admins => write!(f, "admins"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: String,
    pub name: String,
    /// What every app calls them by in addresses and lists (`valid_handle`), unique among people and groups.
    pub handle: String,
    pub role: Role,
    pub removed: bool,
}

/// What an endpoint does: a client is what a person and their agents use, and signs for them; compute runs apps'
/// calls; a service offers a device's abilities.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Client,
    Compute,
    Service,
}

/// An endpoint of the install, listed by a `device` entry (or the genesis): its key, the key things are sealed to,
/// whose it is, and what it does.
#[derive(Clone, Debug)]
pub struct Machine {
    pub key: Key,
    pub seal: Key,
    pub user: String,
    pub name: String,
    pub roles: BTreeSet<Kind>,
    /// A passkey listed as a device: its credential id.
    pub passkey: Option<String>,
    /// For a service endpoint, the service it offers (`whatsapp`): what calls naming it reach.
    pub service: Option<String>,
    /// For a service endpoint, whether it serves everyone its person's listing lets it (a push sender, say), not only
    /// its own person.
    pub shared: bool,
    /// The device it's part of, when it's one's: for a service, its device's oned; for a native app's sign-in, the
    /// device it's on (that device's oned, or an id of the device's own).
    pub device: Option<Key>,
    /// Who listed it: its person, from one of their own devices, or an admin.
    pub by: String,
    /// When it was listed, as its entry says (ms).
    pub since: u64,
    /// What it signed in with, when that was checked as it was let in (`sign_in_proof`): one of its person's accounts,
    /// which their next device's sign-in is checked against. None for a shape this release doesn't know.
    pub sign_in: Option<crate::sign_in_proof::Account>,
    /// The city it asked to join from, as its request said.
    pub place: Option<String>,
    pub removed: bool,
}

impl Machine {
    pub fn is(&self, kind: Kind) -> bool {
        self.roles.contains(&kind)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// As a person's: unique among people and groups.
    pub handle: String,
    pub members: BTreeMap<String, Role>,
    pub invited: BTreeMap<String, Role>,
}

#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    pub owner: Principal,
    pub from: Option<String>,
    /// Its latest deploy's bundle, a blob.
    pub bundle: Option<String>,
    pub deploys: u32,
    /// Every deploy: (bundle, time, user), oldest first.
    pub history: Vec<(String, u64, String)>,
    /// When it was deleted, and by whom: it's in Recently Deleted until it's restored or erased.
    pub deleted: Option<(u64, String)>,
    /// Erased for good: gone from every list, and its name never used again.
    pub erased: bool,
    /// Where its code is kept, as its owners last said: a web address or `<cli>://<repo>`; empty once they cleared it.
    /// None if they never did, so a built-in app links to its folder in the release's repo.
    pub source: Option<String>,
}

/// The latest `service` entry for a part of a device's oned (a service, or `apps`, the app service): on or off there,
/// as the device's person switched it.
#[derive(Clone, Debug)]
pub struct Switch {
    pub on: bool,
    pub by: String,
    pub time: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Members {
    pub root: Option<Hash>,
    pub name: String,
    pub recovery: Option<Key>,
    pub users: BTreeMap<String, User>,
    pub machines: BTreeMap<Key, Machine>,
    pub groups: BTreeMap<String, Group>,
    /// The install's settings its admins set: each name's latest value. The rest are the custom build's defaults.
    pub settings: BTreeMap<String, String>,
    pub apps: BTreeMap<String, App>,
    /// The latest `service` entry of each part on each device: (device, part).
    pub services: BTreeMap<(Key, String), Switch>,
    /// When the install's devices last changed, by the newest genesis, device or remove: the relay's list is as of then.
    pub devices_at: u64,
    /// Each scope's current epoch, by its newest `epoch` entry that counts (src/keys.rs).
    pub epochs: BTreeMap<Scope, Epoch>,
    /// Each epoch's hash (`keys::epoch_hash`), by scope and number, as its `epoch` entry (or the genesis, for
    /// `global`'s first) named it: a device takes a key from the bucket as epoch n only if it has this hash.
    pub epoch_hashes: BTreeMap<(Scope, u32), String>,
    /// App schedules: until when (milliseconds) each endpoint may hold an app's keys in a scope, by (endpoint, scope,
    /// app), as the latest `grant` of each said; one with `until: 0` is gone.
    pub app_schedules: BTreeMap<(Key, Scope, String), AppSchedules>,
    /// Each scope's Vault key, by its newest `vault` entry that counts: items' secrets are sealed to its public half,
    /// which only the machines it lists open.
    pub vaults: BTreeMap<Scope, VaultKey>,
    /// Each Vault key's public half, by scope and number: a machine takes a secret key from the bucket as key n only
    /// if it's this one's.
    pub vault_publics: BTreeMap<(Scope, u32), [u8; 32]>,
    /// Scopes that owe a new epoch: since their newest, someone who held their key lost the right to (a client device
    /// removed by another, a person removed, a member taken out of a group, app schedules stopped, or a machine
    /// running them removed). Any of the scope's admins' devices starts it (`Node::start_owed_keys`), so a removal whose own new
    /// epoch failed, or whose device went off first, still gets one. A device signing itself out owes none.
    pub owed: BTreeSet<Scope>,
    /// Where each person's own things run besides their own machines, as their latest `runs` said from one of their
    /// own devices: the install's other compute endpoints that run their own scope (`runs_for`).
    pub runs: BTreeMap<String, BTreeSet<Key>>,
    /// When the install's recovery key last let a device in (a `device` entry it signed), and whose.
    pub recovery_used: Option<(u64, String)>,
    /// When the install's recovery key was set: by the genesis, or by a `recovery` entry the one before it signed.
    pub recovery_set: u64,
    /// The install's recovery keys before this one, which the relay lets in no more.
    pub recovery_was: Vec<Key>,
    /// Who opened the gate to the scope database, each from an admin's own device as it sorts (`gate`).
    gates: BTreeSet<String>,
    /// Where each endpoint's entries stop counting, by (endpoint, log): the first seq that doesn't, wherever it sorts.
    /// A removed device's in `global` after the last its remover had seen (`remove`'s `seq`), and a device's in a log
    /// it forked from the fork on (`Store::forks`).
    pub cuts: Cuts,
    /// Devices that forked `global` (two entries at one seq): removed once it's replayed.
    forked: BTreeMap<Key, Fork>,
    /// How many of each forked device's entries before its fork replay has passed so far.
    passed: BTreeMap<Key, usize>,
}

/// Where a device forked `global`: the seq with two entries, and how many of its member entries came before it.
#[derive(Clone, Copy, Debug, Default)]
struct Fork {
    seq: u32,
    before: usize,
}

/// Where endpoints' entries stop counting: (endpoint, log) → the first seq that doesn't.
pub type Cuts = BTreeMap<(Key, String), u32>;

/// Adds `more` to `cuts`, each the lower where both have one: → whether that changed anything.
fn merge(cuts: &mut Cuts, more: &Cuts) -> bool {
    let mut changed = false;
    for (k, seq) in more {
        let at = cuts.entry(k.clone()).or_insert(u32::MAX);
        if *seq < *at {
            *at = *seq;
            changed = true;
        }
    }
    changed
}

/// The first seq of a device's in `global` that doesn't count, from the last its remover had seen (null for none).
fn first_not(seq: &Value) -> Result<u32> {
    Ok(match seq {
        Value::Null => 0,
        seq => u32::try_from(seq.as_u64().context("a seq is a device's last in global, or null")?)?.checked_add(1).context("seq is too big")?,
    })
}

/// App schedules: who set them, until when, and the hash (`keys::epoch_hash`) of the app's key in each epoch
/// it sealed to the endpoint (`keys::app_schedules_key_name`), so the endpoint takes from the bucket only those.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSchedules {
    pub by: String,
    pub until: u64,
    pub hashes: BTreeMap<u32, String>,
}

/// A scope's Vault key: its number, its X25519 public half, and the machines its secret half is sealed to
/// (`keys::vault_key_name`), besides the recovery key. A new one whenever who's granted changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultKey {
    pub n: u32,
    pub public: [u8; 32],
    pub to: Vec<Key>,
}

/// A scope's epoch: its number, the endpoints it was first sealed to (`keys::epoch_name`), besides the recovery key,
/// and its key's hash.
/// A new one starts only when a person or device is removed; one added is sealed the current one by the device that
/// approves it, so who may open a scope comes from roles and devices, not from `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Epoch {
    pub n: u32,
    pub to: Vec<Key>,
    pub hash: String,
}

/// Node entries that change only their own scope's state, never the members, so no other scope replays them.
const SCOPE_ONLY: &[&str] = &["schedule", "vault-request", "keep", "forget"];

/// Whether an entry is one of `global`'s node entries, which every scope's replay replays.
pub fn is_member_entry(e: &Entry) -> bool {
    e.content.app == APP && e.log() == "global" && !SCOPE_ONLY.contains(&e.content.kind.as_str())
}

impl Members {
    pub fn new(root: Option<Hash>) -> Members {
        Members { root, ..Default::default() }
    }

    /// Replays `global`'s node entries, sorted in replay order, with the forks `Store::forks` found. A removal's cut
    /// applies to entries that sort before it too, so where one stops entries that counted, it's replayed again with the
    /// cuts known from the start, until they don't change.
    pub fn replay(root: Option<Hash>, member_entries: &[Entry], forks: &Cuts) -> Members {
        let forked: BTreeMap<Key, Fork> = forks
            .iter()
            .filter(|((_, log), _)| log == "global")
            .map(|((k, _), seq)| (*k, Fork { seq: *seq, before: member_entries.iter().filter(|e| e.author() == *k && e.seq() < *seq).count() }))
            .collect();
        let mut known = forks.clone();
        loop {
            let m = Members::replay_with(root, member_entries, &known, &forked);
            if !merge(&mut known, &m.cuts) {
                return m;
            }
        }
    }

    /// Replays with cuts known from the start.
    fn replay_with(root: Option<Hash>, member_entries: &[Entry], cuts: &Cuts, forked: &BTreeMap<Key, Fork>) -> Members {
        let mut m = Members::with_cuts(root, cuts, forked);
        for e in member_entries {
            m.step(e);
        }
        m.end();
        m
    }

    /// Members before any entry, with the cuts a full replay found, and the devices that forked `global`.
    fn with_cuts(root: Option<Hash>, cuts: &Cuts, forked: &BTreeMap<Key, Fork>) -> Members {
        Members { root, cuts: cuts.clone(), forked: forked.clone(), ..Default::default() }
    }

    /// Once every member entry is replayed: a device that forked `global` is removed, and a recovery key that did is
    /// no recovery key: whoever forked it could fork away its own replacement, so an admin's own device names the
    /// next (`recovery_lost`).
    pub fn end(&mut self) {
        for key in self.forked.keys() {
            if let Some(m) = self.machines.get_mut(key) {
                m.removed = true;
            }
        }
        if self.recovery_lost() {
            self.recovery_was.extend(self.recovery.take());
        }
    }

    /// Whether an admin's own device said the gate to the scope database is open (`GATE`), before the log's freeze:
    /// only a freeze of the log from there on counts (`database::steps::claimed`). Only if one who's still an admin
    /// did, so a device of someone demoted can't open it with an entry that sorts before their demotion; worked out
    /// as it's asked, so it's the same stepped one entry at a time as replayed whole.
    pub fn gate(&self) -> bool {
        self.gates.iter().any(|u| self.is_admin(u))
    }

    /// Whether the install has no recovery key, or one that forked `global` whose every entry from before its fork
    /// replay has passed: only then does an admin's own device name the next, and once replayed, `end` takes it away.
    /// Only those entries of a forked key's count, its handover to the next among them, so no admin's naming that
    /// sorts before that handover, from a device that hadn't seen it, ever replaces the key it handed over to. Not
    /// "has the namer seen the fork": a node holds only the first of two entries at one seq, so which it holds, and
    /// where it sorts, isn't the same on every reader; the entries before it are.
    pub fn recovery_lost(&self) -> bool {
        self.recovery.is_none_or(|k| self.forked.get(&k).is_some_and(|f| self.passed.get(&k).copied().unwrap_or(0) >= f.before))
    }

    /// Applies one of `global`'s node entries, if it counts. Returns whether it did.
    pub fn step(&mut self, e: &Entry) -> bool {
        let counted = self.try_step(e).is_ok();
        if !counted {
            tracing::debug!("{} entry {} doesn't count", e.content.kind, e.id());
        }
        counted
    }

    /// As `step`, saying why an entry doesn't count.
    pub fn try_step(&mut self, e: &Entry) -> Result<()> {
        if let Some(f) = self.forked.get(&e.author())
            && e.log() == "global"
            && e.seq() < f.seq
        {
            *self.passed.entry(e.author()).or_default() += 1;
        }
        ensure!(e.content.kind == "genesis" || self.counts(e), "that changes nothing: you may not {} that", e.content.kind);
        self.apply(e)
    }

    /// Whether an entry is at or after where its device's entries in its log stop counting.
    pub fn cut(&self, e: &Entry) -> bool {
        self.cuts.get(&(e.author(), e.log().to_string())).is_some_and(|c| e.seq() >= *c)
    }

    /// The hash of a scope's epoch `n`, if an entry that counts made it.
    pub fn epoch_hash(&self, scope: &Scope, n: u32) -> Option<&str> {
        self.epoch_hashes.get(&(scope.clone(), n)).map(String::as_str)
    }

    /// A scope's current epoch: 0 while it has none yet.
    pub fn epoch(&self, scope: &Scope) -> u32 {
        self.epochs.get(scope).map_or(0, |e| e.n)
    }

    /// That `scope` owes a new epoch, if it has one to follow.
    fn owe(&mut self, scope: Scope) {
        if self.epoch(&scope) > 0 {
            self.owed.insert(scope);
        }
    }

    /// The scopes whose keys `keys` held, which their removal makes owe a new epoch: each a client of them opens, but
    /// `except` (a person's own, which removing them keeps as it is), and each one of them runs app schedules in.
    fn held_by(&self, keys: &[Key], except: Option<&Scope>) -> Vec<Scope> {
        let opens = |s: &Scope| keys.iter().filter_map(|k| self.machine(k)).any(|d| d.is(Kind::Client) && self.may_use(&d.user, s));
        let scheduled = self.app_schedules.keys().filter(|(k, _, _)| keys.contains(k)).map(|(_, s, _)| s.clone());
        self.scopes().into_iter().filter(opens).chain(scheduled).filter(|s| Some(s) != except).collect()
    }

    /// The key things are sealed to for an endpoint of the install, as it's listed.
    pub fn seal_key(&self, key: &Key) -> Option<[u8; 32]> {
        self.machine(key).map(|m| m.seal)
    }

    /// The app schedules in force at `now`, each by (endpoint, scope, app): not expired, on an endpoint still listed,
    /// set by someone who may still set them there.
    pub fn app_schedules_at(&self, now: u64) -> impl Iterator<Item = (&(Key, Scope, String), &AppSchedules)> {
        self.app_schedules.iter().filter(move |((k, s, _), g)| g.until > now && self.machine(k).is_some() && self.is_scope_admin(&g.by, s))
    }

    /// Whether `endpoint` is granted `scope`'s Vault now: its newest `vault` entry lists it, and it's still listed.
    pub fn opens_vault(&self, endpoint: &Key, scope: &Scope) -> bool {
        self.vaults.get(scope).is_some_and(|v| v.to.contains(endpoint)) && self.machine(endpoint).is_some()
    }

    /// Whether `user` is a scope's admin, who sets its app schedules and Vault: their own, a group's they're an admin of, or `global` as an
    /// admin of the install.
    pub fn is_scope_admin(&self, user: &str, scope: &Scope) -> bool {
        match scope {
            Scope::User(u) => u == user && self.user(user).is_some(),
            Scope::Group(g) => self.group_role(user, g) == Some(Role::Admin),
            Scope::Global => self.is_admin(user),
        }
    }

    // Who is who.

    pub fn user(&self, id: &str) -> Option<&User> {
        self.users.get(id).filter(|u| !u.removed)
    }

    /// Whether `id` was a person of the install, and was removed.
    pub fn removed_user(&self, id: &str) -> bool {
        self.users.get(id).is_some_and(|u| u.removed)
    }

    /// Whose `handle` is: a person's (removed ones keep theirs) or a group's.
    pub fn by_handle(&self, handle: &str) -> Option<Principal> {
        let user = self.users.values().find(|u| u.handle == handle).map(|u| Principal::User(u.id.clone()));
        user.or_else(|| self.groups.values().find(|g| g.handle == handle).map(|g| Principal::Group(g.id.clone())))
    }

    /// Everyone's and every group's handle, as apps get them (`ctx.handles`): `{ handle, id, name }`, `id` being
    /// `user:<id>` or `group:<id>`; a removed person's with `removed`, and a group's with its `members`' roles by id.
    pub fn handles(&self) -> Value {
        let users = self.users.values().map(|u| {
            let mut v = json!({ "handle": u.handle, "id": format!("user:{}", u.id), "name": u.name });
            if u.removed {
                v["removed"] = json!(true);
            }
            v
        });
        let groups = self.groups.values().map(|g| json!({ "handle": g.handle, "id": format!("group:{}", g.id), "name": g.name, "members": g.members }));
        Value::Array(users.chain(groups).collect())
    }

    /// A person's or group's handle, picked from the name they joined or were made with: `handle_from`, with -2, -3…
    /// after it while that's taken.
    fn pick_handle(&self, name: &str, fallback: &str) -> String {
        let base = handle_from(name, fallback);
        let numbered = |n: u32| {
            let tail = format!("-{n}");
            format!("{}{tail}", base[..base.len().min(39 - tail.len())].trim_end_matches('-'))
        };
        (1..).map(|n| if n == 1 { base.clone() } else { numbered(n) }).find(|h| self.by_handle(h).is_none()).unwrap()
    }

    pub fn is_admin(&self, user: &str) -> bool {
        self.user(user).is_some_and(|u| u.role == Role::Admin)
    }

    /// An endpoint of the install: listed, not removed, and its person not removed.
    pub fn machine(&self, key: &Key) -> Option<&Machine> {
        self.machines.get(key).filter(|m| !m.removed && self.user(&m.user).is_some())
    }

    /// One of the person's own devices, which signs for them.
    pub fn client(&self, key: &Key) -> Option<&Machine> {
        self.machine(key).filter(|m| m.is(Kind::Client))
    }

    /// An endpoint that runs apps' calls: a node.
    pub fn compute(&self, key: &Key) -> Option<&Machine> {
        self.machine(key).filter(|m| m.is(Kind::Compute))
    }

    pub fn computes(&self) -> impl Iterator<Item = &Machine> {
        self.machines.values().filter(|m| m.is(Kind::Compute) && self.machine(&m.key).is_some())
    }

    pub fn clients(&self) -> impl Iterator<Item = &Machine> {
        self.machines.values().filter(|m| m.is(Kind::Client) && self.machine(&m.key).is_some())
    }

    /// A service endpoint: one that offers a device's abilities, checking each caller itself.
    pub fn service(&self, key: &Key) -> Option<&Machine> {
        self.machine(key).filter(|m| m.is(Kind::Service) && m.service.is_some())
    }

    /// Whom a service endpoint serves, as its listing says: its own person, or everyone of the install if it's shared.
    pub fn service_serves(&self, key: &Key, user: &str) -> bool {
        self.service(key).is_some_and(|s| s.user == user || (s.shared && self.user(user).is_some()))
    }

    /// The service endpoints offering `name`.
    pub fn offering<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Machine> {
        self.machines.values().filter(move |m| m.service.as_deref() == Some(name) && self.service(&m.key).is_some())
    }

    /// A user's role in a group, if they're in it.
    pub fn group_role(&self, user: &str, group: &str) -> Option<Role> {
        self.groups.get(group)?.members.get(user).copied().filter(|_| self.user(user).is_some())
    }

    /// The groups a user is in, with their role in each.
    pub fn groups_of(&self, user: &str) -> BTreeMap<String, Role> {
        self.groups.values().filter_map(|g| Some((g.id.clone(), self.group_role(user, &g.id)?))).collect()
    }

    /// A user's role in a scope: in the install for `global`, in the group, and admin in their own.
    pub fn role_in(&self, user: &str, scope: &Scope) -> Option<Role> {
        match scope {
            Scope::Global => self.user(user).map(|u| u.role),
            Scope::Group(g) => self.group_role(user, g),
            Scope::User(u) => (u == user && self.user(user).is_some()).then_some(Role::Admin),
        }
    }

    /// The scopes a user may use: their own, their groups', and `global`.
    pub fn may_use(&self, user: &str, scope: &Scope) -> bool {
        self.role_in(user, scope).is_some()
    }

    /// Everyone's role in each scope they may use, and in each group they're invited to: (role, invited).
    pub fn roles(&self) -> BTreeMap<(String, Scope), (Role, bool)> {
        let mut out = BTreeMap::new();
        for user in self.users.keys() {
            for scope in [Scope::Global, Scope::User(user.clone())].into_iter().chain(self.groups.keys().map(|g| Scope::Group(g.clone()))) {
                if let Some(role) = self.role_in(user, &scope) {
                    out.insert((user.clone(), scope), (role, false));
                }
            }
        }
        for g in self.groups.values() {
            for (user, role) in g.invited.iter().filter(|(u, _)| self.user(u).is_some()) {
                out.entry((user.clone(), Scope::Group(g.id.clone()))).or_insert((*role, true));
            }
        }
        out
    }

    /// Whether `user` acts for `owner`: a user for themselves, a group's admins for the group, the install's
    /// admins for `admins`.
    pub fn acts_for(&self, user: &str, owner: &Principal) -> bool {
        match owner {
            Principal::User(u) => u == user && self.user(user).is_some(),
            Principal::Group(g) => self.group_role(user, g) == Some(Role::Admin),
            Principal::Admins => self.is_admin(user),
        }
    }

    /// Whether `user` manages `group`: its admins, and the install's (so a group whose last admin left isn't stuck).
    pub fn manages(&self, user: &str, group: &str) -> bool {
        self.groups.contains_key(group) && (self.group_role(user, group) == Some(Role::Admin) || self.is_admin(user))
    }

    // Endpoints.

    /// Whether `user` looks after an endpoint: its person, or an admin.
    pub fn looks_after(&self, user: &str, key: &Key) -> bool {
        self.machine(key).is_some_and(|m| m.user == user) || self.is_admin(user)
    }

    /// Whether a scope's calls run on `endpoint`: a compute endpoint its owners trust, which for a user's own scope is
    /// one of their own machines they listed themselves (one an admin listed for them runs only what that admin's
    /// could), or another of the install's their own device chose (`runs`); of an admin of the group or of the install, for a group's; of an admin of
    /// the install, for `global`.
    pub fn runs_for(&self, endpoint: &Key, scope: &Scope) -> bool {
        let Some(m) = self.compute(endpoint) else {
            // The Cloudflare node, as an admin's setting names it: `global` and groups as any admin's machine, and a
            // person's own scope only once their own device chose it (`runs`), as any other.
            return self.cloudflare_node() == Some(*endpoint)
                && match scope {
                    Scope::User(u) => self.runs.get(u).is_some_and(|on| on.contains(endpoint)),
                    Scope::Group(_) | Scope::Global => true,
                };
        };
        match scope {
            Scope::User(u) => (m.user == *u && m.by == *u) || self.runs.get(u).is_some_and(|on| on.contains(endpoint)),
            Scope::Group(g) => self.manages(&m.user, g),
            Scope::Global => self.is_admin(&m.user),
        }
    }

    /// The Cloudflare node's endpoint, as an admin's `serverless` setting names it (the setting keeps its old name: every device reads it from the signed entries).
    pub fn cloudflare_node(&self) -> Option<Key> {
        self.settings.get("serverless").and_then(|k| unhex(k).ok())
    }

    /// Whether `endpoint` runs any of the calls of `user`: a compute endpoint that runs some scope they use.
    pub fn serves(&self, endpoint: &Key, user: &str) -> bool {
        self.user(user).is_some() && self.scopes().iter().any(|s| self.may_use(user, s) && self.runs_for(endpoint, s))
    }

    /// The compute endpoints a call of `user`'s that reaches `scopes` may be passed on to: each runs every one of those
    /// scopes, all of which `user` may use, so none is handed a call, or its keys, that isn't theirs to run.
    pub fn runners(&self, user: &str, scopes: &[Scope]) -> Vec<&Machine> {
        if self.user(user).is_none() || !scopes.iter().all(|s| self.may_use(user, s)) {
            return Vec::new();
        }
        self.computes().filter(|m| self.serves(&m.key, user) && scopes.iter().all(|s| self.runs_for(&m.key, s))).collect()
    }

    /// Every scope there is: `global`, each user's (removed ones too: removing deletes nothing) and each group's.
    pub fn scopes(&self) -> Vec<Scope> {
        let mut out = vec![Scope::Global];
        out.extend(self.groups.keys().map(|g| Scope::Group(g.clone())));
        out.extend(self.users.keys().map(|u| Scope::User(u.clone())));
        out
    }

    /// Whether an entry counts at this point in replay, by who signed it: one of its user's own devices, or for any
    /// but a member entry, a compute endpoint that runs the scope's calls (the call it ran for was the user's); its
    /// user may use the scope, and it names no epoch the scope hasn't reached. A node entry's own kind rules are
    /// checked as it's applied (`apply`), or, for one in a user or group scope, by `apply_in_scope`.
    pub fn counts(&self, e: &Entry) -> bool {
        let scope = e.scope();
        if !self.may_use(e.user(), &scope) || e.epoch() > self.epoch(&scope) || self.cut(e) {
            return false;
        }
        let author = e.author();
        // The install's recovery key, kept offline by an admin, only lets a device of someone's in, as an admin's would,
        // and hands over to the next recovery key.
        if self.recovery == Some(author) {
            return is_member_entry(e) && ["device", "recovery"].contains(&e.content.kind.as_str());
        }
        self.client(&author).is_some_and(|c| c.user == e.user()) || (!is_member_entry(e) && self.runs_for(&author, &scope))
    }

    /// Applies a node entry of `global`.
    fn apply(&mut self, e: &Entry) -> Result<()> {
        ensure!(e.content.app == APP && e.log() == "global", "not a member entry");
        let w = e.user().to_string();
        let author = e.author();
        let data = &e.content.data;
        let text = |key: &str| data[key].as_str().map(str::to_string).with_context(|| format!("no {key}"));
        let key_of = |key: &str| -> Result<Key> { unhex(&text(key)?) };
        let id_of = |key: &str| -> Result<String> {
            let id = text(key)?;
            ensure!(is_id(&id), "{key} isn't an id");
            Ok(id)
        };
        let role_of = |key: &str| -> Result<Role> { Ok(serde_json::from_value(data[key].clone())?) };
        match e.content.kind.as_str() {
            "genesis" => {
                ensure!(self.root == Some(e.hash()) && self.users.is_empty(), "not this install's genesis");
                let user = id_of("user")?;
                ensure!(user == w, "genesis is written as its first user");
                let device = &data["device"];
                let machine = listed(device, &user, e.time())?;
                ensure!(machine.key == author && machine.is(Kind::Client), "genesis is signed by the first device it lists, a client");
                let hash = epoch_hash_of(&data["epoch"])?;
                self.name = text("name")?;
                self.recovery = Some(key_of("recovery")?);
                self.recovery_set = e.time();
                let name = text("user_name")?;
                let handle = self.pick_handle(&name, "person");
                self.users.insert(user.clone(), User { id: user.clone(), name, handle, role: Role::Admin, removed: false });
                self.machines.insert(author, machine);
                self.devices_at = e.time();
                // `global`'s first epoch, made with the install, before anything is sealed under it.
                self.epoch_hashes.insert((Scope::Global, 1), hash.clone());
                self.epochs.insert(Scope::Global, Epoch { n: 1, to: vec![author], hash });
            }
            "device" if self.recovery == Some(author) => {
                // A person who lost every device let back in with the recovery key: a new client of someone already in.
                let user = id_of("user")?;
                let machine = listed(data, &user, e.time())?;
                ensure!(user == w && self.user(&user).is_some(), "the recovery key lets in a device of someone already in");
                ensure!(machine.roles == BTreeSet::from([Kind::Client]) && machine.passkey.is_none(), "the recovery key lets in a device people use");
                ensure!(!self.machines.contains_key(&machine.key), "that key is listed already");
                self.machines.insert(machine.key, machine);
                self.devices_at = self.devices_at.max(e.time());
                self.recovery_used = Some((e.time(), user));
            }
            "device" => {
                let user = id_of("user")?;
                let mut machine = listed(data, &user, e.time())?;
                machine.by = w.clone();
                // A service answers its person's calls as them: only their own device lists one, never an admin's.
                ensure!(!machine.is(Kind::Service) || w == user, "only a person's own device lists their services");
                // A machine an admin listed for someone is listed again by that person's own device to be theirs: then it
                // runs their own scope too (`runs_for`).
                if let Some(was) = self.machines.get(&machine.key) {
                    // Or a service of theirs listed again by their own device, to name the device it's part of; or a sign-in
                    // that named no device, naming the one it's on.
                    let part = was.is(Kind::Service) && was.roles == machine.roles && was.service == machine.service;
                    let part = part || (was.is(Kind::Client) && was.roles == machine.roles && was.device.is_none() && machine.device.is_some() && was.passkey == machine.passkey);
                    ensure!(!was.removed && was.user == user && w == user && (was.by != user || part) && was.seal == machine.seal, "that key is listed already");
                    self.machines.insert(machine.key, machine);
                    self.devices_at = self.devices_at.max(e.time());
                    return Ok(());
                }
                match self.users.get(&user) {
                    Some(u) => ensure!(!u.removed && (w == user || (self.is_admin(&w) && !machine.is(Kind::Client))), "only a person's own device lists their devices; an admin's lists compute and services"),
                    None => {
                        ensure!(self.is_admin(&w), "only admins add people");
                        let role = role_of("role")?;
                        let name = text("user_name")?;
                        let handle = self.pick_handle(&name, "person");
                        self.users.insert(user.clone(), User { id: user.clone(), name, handle, role, removed: false });
                    }
                }
                self.machines.insert(machine.key, machine);
                self.devices_at = self.devices_at.max(e.time());
            }
            "refuse" => {
                // A device its person said isn't theirs ("Not Me"), before it was ever listed: its key is taken out now, so
                // no device of theirs lets it in, by its sign-in or its code, and nothing it writes counts. An older
                // device skips this, and goes on as before.
                let key = key_of("key")?;
                ensure!(self.user(&w).is_some(), "only someone in the install turns down a device of theirs");
                ensure!(!self.machines.contains_key(&key), "that key is listed already: take it out instead");
                let mut machine = listed(&json!({ "key": data["key"], "seal": data["seal"], "name": data["name"], "roles": [Kind::Client] }), &w, e.time())?;
                machine.by = w.clone();
                machine.removed = true;
                merge(&mut self.cuts, &BTreeMap::from([((key, "global".to_string()), 0)]));
                self.machines.insert(key, machine);
                self.devices_at = self.devices_at.max(e.time());
            }
            "forget-sign-in" => {
                // What one of their devices signed in with, forgotten by its person's own device: a new device's sign-in
                // is no longer checked against it. An older device skips this.
                let key = key_of("key")?;
                let it = self.machine(&key).context("no such device")?;
                ensure!(it.user == w, "only its person's own device forgets what it signed in with");
                self.machines.get_mut(&key).unwrap().sign_in = None;
                self.devices_at = self.devices_at.max(e.time());
            }
            "role" => {
                ensure!(self.is_admin(&w), "admins only");
                let user = id_of("user")?;
                let role = role_of("role")?;
                self.users.get_mut(&user).filter(|u| !u.removed).context("no such user")?.role = role;
            }
            "remove" => {
                if let Ok(user) = id_of("user") {
                    ensure!(self.is_admin(&w), "admins only");
                    // The last of each of their devices' entries in `global` its remover had seen: those after never
                    // count, wherever they sort, so a device of theirs that hadn't read its removal can't write one that
                    // sorts before it. A device its remover hadn't seen at all keeps none.
                    let seqs = data["seqs"].as_object().context("a person's removal says the last of each of their devices' entries its remover had seen (seqs)")?;
                    let mut cut = Cuts::new();
                    for d in self.machines.values().filter(|d| d.user == user) {
                        cut.insert((d.key, "global".to_string()), first_not(seqs.get(&hex(&d.key)).unwrap_or(&Value::Null))?);
                    }
                    merge(&mut self.cuts, &cut);
                    let theirs: Vec<Key> = self.machines.values().filter(|d| d.user == user).map(|d| d.key).collect();
                    let owed = self.held_by(&theirs, Some(&Scope::User(user.clone())));
                    self.users.get_mut(&user).context("no such user")?.removed = true;
                    owed.into_iter().for_each(|s| self.owe(s));
                } else {
                    let key = key_of("key")?;
                    ensure!(self.machine(&key).is_some(), "no such key");
                    ensure!(self.looks_after(&w, &key), "only its person or an admin removes a device");
                    // The last of its entries in `global` its remover had seen: those after never count, wherever they sort.
                    ensure!(data.get("seq").is_some(), "a device's removal says the last of its entries its remover had seen (seq)");
                    merge(&mut self.cuts, &BTreeMap::from([((key, "global".to_string()), first_not(&data["seq"])?)]));
                    // A device signing itself out deletes its own keys: signed by it, or by a node of its person's it
                    // asked (`itself`). So does a sign-in before on a device signed in again (`Node::approve_request`).
                    // One removed otherwise may still hold them.
                    let gone = &self.machines[&key];
                    let itself = author == key || (data["itself"] == true && self.machines.get(&author).is_some_and(|a| a.user == gone.user));
                    let again = gone.device.is_some() && gone.passkey.is_none() && self.clients().any(|c| c.key != key && c.user == gone.user && c.device == gone.device && c.passkey.is_none());
                    let owed = if itself || again { Vec::new() } else { self.held_by(&[key], None) };
                    self.machines.get_mut(&key).unwrap().removed = true;
                    owed.into_iter().for_each(|s| self.owe(s));
                }
                self.devices_at = self.devices_at.max(e.time());
            }
            "epoch" => {
                let scope = Scope::parse(&text("scope")?)?;
                let allowed = match &scope {
                    Scope::Global => self.is_admin(&w),
                    Scope::Group(g) => self.manages(&w, g),
                    Scope::User(u) => *u == w,
                };
                ensure!(allowed, "only the scope's admins start its new epoch");
                let n = data["n"].as_u64().context("no n")?;
                ensure!(n == self.epoch(&scope) as u64 + 1, "an epoch follows the one before it");
                let to = data["to"].as_array().context("no to")?.iter().map(|k| unhex(k.as_str().unwrap_or(""))).collect::<Result<Vec<_>>>()?;
                ensure!(to.iter().all(|k| self.machine(k).is_some()), "an epoch is sealed only to the install's endpoints");
                let hash = epoch_hash_of(&data["hash"])?;
                self.epoch_hashes.insert((scope.clone(), n as u32), hash.clone());
                self.owed.remove(&scope);
                self.epochs.insert(scope, Epoch { n: n as u32, to, hash });
            }
            "runs" => {
                // Only from the person's own device (`counts`), for their own scope: an admin can't say where it runs.
                let on = data["on"].as_array().context("on is a list of endpoints' keys")?.iter().map(|k| unhex(k.as_str().unwrap_or(""))).collect::<Result<BTreeSet<_>>>()?;
                ensure!(on.iter().all(|k| self.compute(k).is_some() || self.cloudflare_node() == Some(*k)), "only the install's machines that run things, and its Cloudflare node, run someone's own");
                self.runs.insert(w, on);
            }
            // App schedules, by the name older devices replay.
            "grant" => {
                let endpoint = key_of("endpoint")?;
                let scope = Scope::parse(&text("scope")?)?;
                let app = text("app")?;
                // The node's own databases of a scope (its log, its Vault) are never an app's to schedule.
                ensure!(valid_app_name(&app), "app schedules are an app's");
                let until = data["until"].as_u64().context("no until")?;
                let mut hashes = BTreeMap::new();
                for (n, h) in data["keys"].as_object().into_iter().flatten() {
                    hashes.insert(n.parse::<u32>().context("an epoch is a number")?, epoch_hash_of(h)?);
                }
                ensure!(self.is_scope_admin(&w, &scope), "only the scope's person, or its group's admins, set app schedules in it");
                ensure!(until == 0 || self.machine(&endpoint).is_some(), "no such endpoint");
                match until {
                    0 => {
                        if self.app_schedules.remove(&(endpoint, scope.clone(), app)).is_some() {
                            self.owe(scope);
                        }
                    }
                    until => drop(self.app_schedules.insert((endpoint, scope, app), AppSchedules { by: w, until, hashes })),
                }
            }
            "vault" => {
                // Who opens a scope's Vault is its admins' to say, never a member's who can only add to it.
                let scope = Scope::parse(&text("scope")?)?;
                ensure!(self.is_scope_admin(&w, &scope), "only the scope's person, or its admins, say which machines open its Vault");
                let n = data["n"].as_u64().context("no n")?;
                ensure!(n == self.vaults.get(&scope).map_or(0, |v| v.n) as u64 + 1, "a Vault key follows the one before it");
                let public = key_of("public")?;
                let to = data["to"].as_array().context("no to")?.iter().map(|k| unhex(k.as_str().unwrap_or(""))).collect::<Result<Vec<_>>>()?;
                ensure!(to.iter().all(|k| self.compute(k).is_some() || self.cloudflare_node() == Some(*k)), "a Vault opens only on the install's machines that run things");
                self.vault_publics.insert((scope.clone(), n as u32), public);
                self.vaults.insert(scope, VaultKey { n: n as u32, public, to });
            }
            "group" => {
                let group = id_of("group")?;
                ensure!(!self.groups.contains_key(&group), "that group exists");
                let members = BTreeMap::from([(w, Role::Admin)]);
                let name = text("name")?;
                let handle = self.pick_handle(&name, "group");
                self.groups.insert(group.clone(), Group { id: group, name, handle, members, invited: BTreeMap::new() });
            }
            "invite" => {
                let (group, user, role) = (id_of("group")?, id_of("user")?, role_of("role")?);
                ensure!(self.manages(&w, &group), "only the group's admins invite");
                ensure!(self.user(&user).is_some(), "no such user");
                self.groups.get_mut(&group).unwrap().invited.insert(user, role);
            }
            "accept" => {
                let group = id_of("group")?;
                let g = self.groups.get_mut(&group).context("no such group")?;
                let role = g.invited.remove(&w).context("not invited")?;
                g.members.insert(w, role);
            }
            "member" => {
                let (group, user) = (id_of("group")?, id_of("user")?);
                let role: Option<Role> = serde_json::from_value(data["role"].clone())?;
                ensure!(self.manages(&w, &group) || (role.is_none() && user == w), "only the group's admins change its members");
                let g = self.groups.get_mut(&group).context("no such group")?;
                ensure!(g.members.contains_key(&user) || g.invited.contains_key(&user), "not in the group");
                match role {
                    Some(role) => {
                        ensure!(g.members.contains_key(&user), "not in the group");
                        g.members.insert(user, role);
                    }
                    None => {
                        g.invited.remove(&user);
                        if g.members.remove(&user).is_some() {
                            self.owe(Scope::Group(group));
                        }
                    }
                }
            }
            "rename" => {
                // A person's or group's handle: one that's taken (wherever it sorts) or malformed doesn't count. An older
                // device skips one, which has no name.
                if let Some(handle) = data.get("handle") {
                    let handle = handle.as_str().context("a handle is text")?;
                    ensure!(valid_handle(handle), "a handle is lowercase letters, digits and single hyphens, up to 39, like {}", handle_from(handle, "name"));
                    let mine = match (id_of("user"), id_of("group")) {
                        (Ok(user), _) => {
                            ensure!(user == w || self.is_admin(&w), "not allowed to change that person's handle");
                            ensure!(self.user(&user).is_some(), "no such user");
                            Principal::User(user)
                        }
                        (_, Ok(group)) => {
                            ensure!(self.manages(&w, &group), "not allowed to change that group's handle");
                            Principal::Group(group)
                        }
                        _ => bail!("a handle is a person's or a group's"),
                    };
                    ensure!(self.by_handle(handle).is_none_or(|p| p == mine), "{handle} is taken");
                    match mine {
                        Principal::User(u) => self.users.get_mut(&u).unwrap().handle = handle.to_string(),
                        Principal::Group(g) => self.groups.get_mut(&g).unwrap().handle = handle.to_string(),
                        Principal::Admins => unreachable!(),
                    }
                    return Ok(());
                }
                let name = text("name")?;
                ensure!(!name.trim().is_empty(), "no name");
                if let Ok(key) = key_of("key") {
                    ensure!(self.machine(&key).is_some(), "no such key");
                    ensure!(self.looks_after(&w, &key), "not allowed to rename that");
                    self.machines.get_mut(&key).unwrap().name = name;
                } else if let Ok(user) = id_of("user") {
                    ensure!(user == w || self.is_admin(&w), "not allowed to rename that user");
                    self.users.get_mut(&user).filter(|u| !u.removed).context("no such user")?.name = name;
                } else {
                    let group = id_of("group")?;
                    ensure!(self.manages(&w, &group), "not allowed to rename that group");
                    self.groups.get_mut(&group).unwrap().name = name;
                }
            }
            "setting" => {
                ensure!(self.is_admin(&w), "admins only");
                let name = text("name")?;
                ensure!(crate::labs::valid_setting(&name), "setting names are lowercase words joined by _, or a lab's (lab.…)");
                match &data["value"] {
                    Value::Null => self.settings.remove(&name),
                    value => self.settings.insert(name, value.as_str().context("a setting is text")?.to_string()),
                };
            }
            "recovery" => {
                // Only the recovery key itself hands over to a new one: no admin's device can, since every epoch is
                // sealed to it, own scopes too. Unless there's none, since the one there was forked `global`: then an
                // admin's own device names the next.
                let named = self.recovery_lost() && self.is_admin(&w) && self.client(&author).is_some_and(|c| c.user == w);
                ensure!(self.recovery == Some(author) || named, "only the install's recovery key changes it");
                let key = key_of("key")?;
                ensure!(Some(key) != self.recovery && !self.recovery_was.contains(&key) && !self.machines.contains_key(&key), "a new recovery key is a key of its own");
                self.recovery_was.extend(self.recovery.replace(key));
                self.recovery_set = e.time();
            }
            "gate" => {
                // The gate to the scope database is open: said by an admin's own device in the ceremony that freezes the
                // log (`Node::switch`), before it. An older device skips it.
                ensure!(self.is_admin(&w), "admins only");
                self.gates.insert(w);
            }
            "app" => {
                let name = text("name")?;
                ensure!(valid_app_name(&name), "app names are lowercase words joined by -");
                let owner = Principal::parse(&text("owner")?)?;
                ensure!(self.acts_for(&w, &owner), "only someone acting for an app's owner makes it");
                match self.apps.get_mut(&name) {
                    Some(app) => {
                        let current = app.owner.clone();
                        ensure!(!app.erased, "that app was erased");
                        ensure!(self.acts_for(&w, &current), "only someone acting for the app's owner gives it away");
                        let app = self.apps.get_mut(&name).unwrap();
                        app.owner = owner;
                        // Deleting moves it to Recently Deleted; `deleted: false` restores it; erasing is for good.
                        if data["erased"] == true {
                            ensure!(app.deleted.is_some(), "delete it first");
                            app.erased = true;
                        } else if let Some(deleted) = data["deleted"].as_bool() {
                            app.deleted = deleted.then(|| (e.time(), w.clone()));
                        }
                        if let Some(source) = data.get("source") {
                            app.source = Some(source_link(source)?);
                        }
                    }
                    None => {
                        let from = data["from"].as_str().map(str::to_string);
                        self.apps.insert(name.clone(), App { name, owner, from, bundle: None, deploys: 0, history: Vec::new(), deleted: None, erased: false, source: None });
                    }
                }
            }
            "deploy" => {
                let name = text("app")?;
                let bundle = text("bundle")?;
                let owner = self.apps.get(&name).context("no such app")?.owner.clone();
                ensure!(self.acts_for(&w, &owner), "only someone acting for the app's owner deploys it");
                ensure!(self.apps[&name].deleted.is_none(), "it was deleted: restore it first");
                ensure!(e.content.blobs.contains(&bundle), "a deploy names its bundle among its blobs");
                let app = self.apps.get_mut(&name).unwrap();
                app.bundle = Some(bundle.clone());
                app.deploys += 1;
                app.history.push((bundle, e.time(), w.clone()));
            }
            "service" => {
                let device = key_of("device")?;
                let name = text("name")?;
                ensure!(valid_app_name(&name), "service names are lowercase words joined by -");
                // The Cloudflare node is a node of the install's, not a person's device: its admins switch its parts.
                if self.cloudflare_node() == Some(device) {
                    ensure!(self.is_admin(&w), "only an admin switches what the Cloudflare node runs");
                } else {
                    let owner = self.machine(&device).context("no such device")?.user.clone();
                    ensure!(w == owner, "only a device's person switches what it runs");
                }
                self.services.insert((device, name), Switch { on: data["on"] == true, by: w, time: e.time() });
            }
            "install" | "uninstall" | "node-release" | "service-release" => {}
            other => bail!("unknown kind {other}"),
        }
        Ok(())
    }
}

/// The `hash` an `epoch` entry (or `epoch`, the genesis) names its key by: `keys::epoch_hash`, 64 hex characters.
fn epoch_hash_of(hash: &Value) -> Result<String> {
    let hash = hash.as_str().context("an epoch names its key's hash")?;
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "an epoch's hash is 64 hex characters");
    Ok(hash.to_string())
}

/// An endpoint as a `device` entry (or the genesis's `device`) lists it, for `user`.
fn listed(data: &Value, user: &str, since: u64) -> Result<Machine> {
    let text = |key: &str| data[key].as_str().with_context(|| format!("no {key}"));
    let roles: BTreeSet<Kind> = serde_json::from_value(data["roles"].clone()).context("roles are client, compute or service")?;
    ensure!(!roles.is_empty(), "an endpoint does something: client, compute or service");
    let name = text("name")?.trim().to_string();
    ensure!(!name.is_empty(), "an endpoint has a name");
    let service = data["service"].as_str().map(str::to_string);
    ensure!(service.as_deref().is_none_or(valid_app_name), "a service's name is lowercase words joined by -");
    ensure!(service.is_some() == roles.contains(&Kind::Service), "a service endpoint names the service it offers, and only it does");
    Ok(Machine {
        key: unhex(text("key")?)?,
        seal: unhex(text("seal")?)?,
        user: user.to_string(),
        name,
        roles,
        passkey: data["passkey"].as_str().map(str::to_string),
        shared: service.is_some() && data["shared"] == true,
        service,
        device: data["device"].as_str().map(unhex).transpose().context("device is the key of the device it's part of")?,
        by: user.to_string(),
        since,
        sign_in: serde_json::from_value(data["sign_in"].clone()).ok(),
        place: data["place"].as_str().map(|p| p.chars().take(64).collect()),
        removed: false,
    })
}

/// An app's source link as an `app` entry gives it: a web address, or empty (or null) for none.
pub fn source_link(v: &Value) -> Result<String> {
    let link = match v {
        Value::Null => "",
        v => v.as_str().context("a source link is text")?.trim(),
    };
    let scheme = link.split_once("://").map(|(s, _)| s);
    ensure!(link.is_empty() || (link.len() <= 500 && matches!(scheme, Some("https" | "http"))), "a source link is a web address (https://…)");
    ensure!(!link.chars().any(char::is_whitespace), "a source link has no spaces");
    Ok(link.to_string())
}

/// A person's or group's handle: lowercase letters, digits and single hyphens, neither first nor last, up to 39
/// characters, like GitHub's logins.
pub fn valid_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 39 && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !h.starts_with('-') && !h.ends_with('-') && !h.contains("--")
}

/// A handle made from a name: its ASCII letters and digits, lowercased, words joined by single hyphens, cut to 39;
/// `fallback` for a name with none.
pub fn handle_from(name: &str, fallback: &str) -> String {
    let words: Vec<String> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    let handle = words.join("-");
    let handle = handle[..handle.len().min(39)].trim_end_matches('-');
    if handle.is_empty() { fallback.to_string() } else { handle.to_string() }
}

pub fn valid_app_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && name != APP
}

/// What a scope's own node entries add up to: the apps installed in it, every `open` of it, requests for Vault items
/// (the items themselves are in its `vault` database, `src/vault.rs`), and its schedules' switches.
#[derive(Clone, Debug, Default)]
pub struct ScopeState {
    pub installed: BTreeSet<String>,
    /// Who last installed each app here, and when: its schedules run as them, unless someone switched one on since.
    pub installers: BTreeMap<String, (String, u64)>,
    /// Each schedule switched on or off here, by (app, schedule): (on, by whom, when).
    pub switches: BTreeMap<(String, String), (bool, String, u64)>,
    /// Apps' and agents' requests for a person's item here, by id.
    pub requests: BTreeMap<String, VaultRequest>,
    /// The blobs this scope's apps said they no longer need, though their entries name them.
    pub forgotten: BTreeSet<String>,
    /// In a user's own scope: how they arranged each page, by page (`user`, `group:<id>` or `global`):
    /// `{ items, hidden }`, as their latest `arrange` of it said.
    pub arrangements: BTreeMap<String, Value>,
}

/// An app's or agent's request for a person's item that an app may use: the fields it wants (name, and whether
/// secret), what for, who asked, and how it was answered: (`set` or `declined`, by whom, when).
#[derive(Clone, Debug)]
pub struct VaultRequest {
    pub app: String,
    pub title: String,
    pub fields: Vec<(String, bool)>,
    pub hosts: Vec<String>,
    pub why: String,
    pub need: Option<String>,
    pub user: String,
    pub agent: Option<String>,
    pub time: u64,
    pub answer: Option<(String, String, u64)>,
}

/// Whether `user` installs apps and manages the Vault's items in `scope`: their own, a group's they're an admin of, and `global`
/// if they're an admin.
pub fn manages_scope(m: &Members, user: &str, scope: &Scope) -> bool {
    match scope {
        Scope::User(u) => u == user,
        Scope::Group(g) => m.group_role(user, g) == Some(Role::Admin),
        Scope::Global => m.is_admin(user),
    }
}

/// How people's roles changed from `before` to `after` (`Members::roles`), as the install's vault hears it: (user,
/// role, invited, scopes) each. None, in every scope they may no longer use or were invited to; otherwise their role
/// in each they're newly in, invited to, or an admin of no longer or now. Not someone joining the install (their
/// invite link opens Everyone's to them), their own scope, nor a group just made (the vault made its key with it).
pub fn roles_changed(before: &BTreeMap<(String, Scope), (Role, bool)>, after: &BTreeMap<(String, Scope), (Role, bool)>) -> Vec<(String, Option<Role>, bool, Vec<Scope>)> {
    let mut out: BTreeMap<(String, Option<Role>, bool), Vec<Scope>> = BTreeMap::new();
    for key in before.keys().chain(after.keys().filter(|k| !before.contains_key(*k))) {
        let (user, scope) = key;
        let (role, invited) = match (before.get(key), after.get(key)) {
            (Some(_), None) => (None, false),
            (b, Some(a)) if b == Some(a) => continue,
            (_, Some(_)) if matches!(scope, Scope::User(_)) || !before.keys().any(|(_, s)| s == scope) => continue,
            (None, Some((Role::Member, false))) if *scope == Scope::Global => continue,
            (_, Some((role, invited))) => (Some(*role), *invited),
            (None, None) => continue,
        };
        out.entry((user.clone(), role, invited)).or_default().push(scope.clone());
    }
    out.into_iter().map(|((user, role, invited), scopes)| (user, role, invited, scopes)).collect()
}

/// A page a person arranges: their own (`user`), a group's (`group:<id>`) or everyone's (`global`).
pub fn valid_page(page: &str) -> bool {
    page == "user" || page == "global" || page.strip_prefix("group:").is_some_and(is_id)
}

/// A Vault field's name (once a secret's): letters, digits, `_`, `-` and `.`.
pub fn valid_secret_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// A host an item's secret fields are sent to: a lowercase host name or IP address, without scheme or port.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

/// Whether a node entry in a user or group scope counts, by its kind's rules, and what it changes.
/// Whether one of a scope's node entries would count, after what its earlier ones add up to: why not, if it wouldn't.
pub fn applies_in_scope(m: &Members, state: &ScopeState, e: &Entry) -> Result<()> {
    apply_in_scope(m, &mut state.clone(), e)
}

/// Applies one of a scope's own node entries to its state, by replay's rules: whether it counted.
pub fn apply_in_scope(m: &Members, state: &mut ScopeState, e: &Entry) -> Result<()> {
    let scope = e.scope();
    let w = e.user();
    match e.content.kind.as_str() {
        "install" | "uninstall" => {
            ensure!(manages_scope(m, w, &scope), "not allowed to install there");
            let app = e.content.data["app"].as_str().context("no app")?.to_string();
            if e.content.kind == "install" {
                ensure!(m.apps.contains_key(&app), "no such app");
                state.installers.insert(app.clone(), (w.to_string(), e.time()));
                state.installed.insert(app);
            } else {
                state.installed.remove(&app);
            }
        }
        "schedule" => {
            ensure!(manages_scope(m, w, &scope), "not allowed to switch schedules there");
            let data = &e.content.data;
            let app = data["app"].as_str().context("no app")?.to_string();
            let name = data["schedule"].as_str().context("no schedule")?.to_string();
            let on = data["on"].as_bool().context("on or off?")?;
            state.switches.insert((app, name), (on, w.to_string(), e.time()));
        }
        // A blob the scope keeps, which its `blobs` names.
        "keep" => {}
        // Only someone who may write there says a blob is no longer needed: its data goes from the bucket.
        "forget" => {
            ensure!(m.may_use(w, &scope), "not allowed to forget blobs there");
            let blob = e.content.data["blob"].as_str().filter(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())).context("no blob")?;
            state.forgotten.insert(blob.to_string());
        }
        "vault-request" => {
            ensure!(manages_scope(m, w, &scope), "not allowed to change the Vault there");
            let data = &e.content.data;
            let id = data["id"].as_str().filter(|id| is_id(id)).context("no id")?.to_string();
            // Answered: declined, or set once the item it asked for is in the scope's Vault (`src/node/vault.rs`).
            if let Some(how) = ["declined", "set"].into_iter().find(|how| data[*how] == true) {
                let r = state.requests.get_mut(&id).context("no such request")?;
                ensure!(r.answer.is_none(), "already answered");
                r.answer = Some((how.into(), w.to_string(), e.time()));
                return Ok(());
            }
            ensure!(!state.requests.contains_key(&id), "that request is already made");
            let app = data["app"].as_str().context("no app")?.to_string();
            // An item the node itself uses (a backup place's) is asked for as the node's, and one agents use (vault_fetch)
            // as theirs.
            ensure!(m.apps.contains_key(&app) || app == APP || app == AGENTS, "no such app");
            let fields: Vec<(String, bool)> = data["fields"].as_object().context("no fields")?.iter().map(|(n, f)| (n.clone(), f["secret"] == true)).collect();
            let title = data["title"].as_str().unwrap_or("").trim().to_string();
            ensure!(!title.is_empty(), "a request has a title");
            ensure!(!fields.is_empty(), "a request asks for one field or more");
            if let Some((name, _)) = fields.iter().find(|(n, _)| !valid_secret_name(n)) {
                bail!("“{name}” can't be a field's name: use letters, digits, _, - and . with no spaces, like {}", name.trim().replace(|c: char| !c.is_ascii_alphanumeric() && !"_-.".contains(c), "_"));
            }
            let hosts: Vec<String> = data["hosts"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect();
            ensure!(hosts.iter().all(|h| valid_host(h)), "a host is a host name, like api.example.com");
            ensure!(!hosts.is_empty() || !fields.iter().any(|(_, secret)| *secret), "a secret field goes to one host or more");
            let why = data["why"].as_str().unwrap_or("").to_string();
            let agent = data["agent"].as_str().map(str::to_string);
            let need = data["need"].as_str().map(str::to_string);
            state.requests.insert(id, VaultRequest { app, title, fields, hosts, why, need, user: w.to_string(), agent, time: e.time(), answer: None });
        }
        "arrange" => {
            ensure!(scope == Scope::User(w.to_string()), "each person arranges pages in their own scope");
            let data = &e.content.data;
            let page = data["page"].as_str().context("no page")?;
            ensure!(valid_page(page), "a page is user, group:<id> or global");
            let items = data["items"].as_array().context("no items")?;
            ensure!(items.len() <= 500, "too many items on one page");
            for item in items {
                ensure!(item["app"].as_str().is_some_and(valid_app_name), "each item names an app");
                if !item["widget"].is_null() {
                    ensure!(item["widget"].is_string() && item["id"].as_str().is_some_and(|id| !id.is_empty() && id.len() <= 64), "a widget on a page has an id");
                    ensure!(item["options"].is_null() || item["options"].is_object(), "a widget's options are an object");
                }
            }
            let hidden = data["hidden"].as_array().cloned().unwrap_or_default();
            ensure!(hidden.iter().all(|a| a.as_str().is_some_and(valid_app_name)), "hidden lists apps");
            let mut kept = json!({ "items": items, "hidden": hidden, "time": e.time() });
            // With your own page: the order of your groups' pages, and those you hid (Edit Pages…).
            if page == "user" && !data["pages"].is_null() {
                let groups = |k: &str| data["pages"][k].as_array().is_some_and(|l| l.len() <= 500 && l.iter().all(|g| g.as_str().is_some_and(|g| g.starts_with("group:") && valid_page(g))));
                ensure!(groups("order") && groups("hidden"), "pages orders and hides groups' pages");
                kept["pages"] = json!({ "order": data["pages"]["order"], "hidden": data["pages"]["hidden"] });
            }
            state.arrangements.insert(page.to_string(), kept);
        }
        _ if scope == Scope::Global => {}
        other => bail!("{other} isn't a node entry of a user or group scope"),
    }
    Ok(())
}

/// Replays a scope: `global`'s member entries and the scope's own, merged in replay order (both sorted). Calls `f`
/// with each of the scope's entries that counts, the members as they were then, and the scope's node state.
/// For `global`, pass its entries as both.
/// `known` are the members a full replay of `global` made, whose cuts (and forks) apply from the start.
pub fn replay(known: &Members, member_entries: &[Entry], scope_entries: &[Entry], f: impl FnMut(&Entry, &Members, &ScopeState)) -> (Members, ScopeState) {
    replay_from(known, member_entries, scope_entries, ScopeState::default(), f)
}

/// Replays more of a scope's entries onto `state`, what its earlier ones added up to.
pub fn replay_from(known: &Members, member_entries: &[Entry], scope_entries: &[Entry], mut state: ScopeState, mut f: impl FnMut(&Entry, &Members, &ScopeState)) -> (Members, ScopeState) {
    let mut m = Members::with_cuts(known.root, &known.cuts, &known.forked);
    let mut members = member_entries.iter().peekable();
    for e in scope_entries {
        let order = e.order();
        while let Some(me) = members.next_if(|me| me.order() < order) {
            m.step(me);
        }
        let counted = if is_member_entry(e) {
            members.next_if(|me| me.order() == order);
            m.step(e) && (!matches!(e.content.kind.as_str(), "install" | "uninstall") || apply_in_scope(&m, &mut state, e).is_ok())
        } else if e.content.app == APP && e.log() == "global" {
            // A scope-only node entry of `global`'s.
            m.counts(e) && apply_in_scope(&m, &mut state, e).is_ok()
        } else if m.counts(e) {
            e.content.app != APP || apply_in_scope(&m, &mut state, e).is_ok()
        } else {
            false
        };
        if counted {
            f(e, &m, &state);
        }
    }
    for me in members {
        m.step(me);
    }
    m.end();
    (m, state)
}

// Building node entries.

pub fn node(kind: &str, data: Value) -> Content {
    Content::new(APP, kind, data)
}

/// The install's first entry: its name, its first person, the first device (signing it), the recovery key, and the
/// hash of `global`'s first epoch (`keys::epoch_hash`), made with it.
pub fn genesis(name: &str, user: &str, user_name: &str, device: &crate::keys::Endpoint, device_name: &str, roles: &[Kind], recovery: &Key, epoch: &str) -> Content {
    let device = json!({ "key": hex(&device.key), "seal": hex(&device.seal), "name": device_name, "roles": roles });
    node("genesis", json!({ "name": name, "user": user, "user_name": user_name, "device": device, "recovery": hex(recovery), "epoch": epoch }))
}

/// Lists an endpoint as `user`'s, and with `new_user` lets them in, by an admin's device.
pub fn device(endpoint: &crate::keys::Endpoint, user: &str, name: &str, roles: &[Kind], new_user: Option<(&str, Role)>) -> Content {
    let mut data = json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "user": user, "name": name, "roles": roles });
    if let Some((user_name, role)) = new_user {
        data["user_name"] = json!(user_name);
        data["role"] = json!(role);
    }
    node("device", data)
}

/// Lists a passkey as `user`'s device, by the credential it is: what's sealed to its keys (from its PRF) lets them back
/// in on a new device.
pub fn passkey(endpoint: &crate::keys::Endpoint, user: &str, name: &str, credential: &str) -> Content {
    let mut content = device(endpoint, user, name, &[Kind::Client], None);
    content.data["passkey"] = json!(credential);
    content
}

// The rest of `global`'s member entries, as people's devices write them: the web client builds the same content
// (cloudflare/web/members.js), and tests/contracts.rs checks it does.

pub fn role(user: &str, role: Role) -> Content {
    node("role", json!({ "user": user, "role": role }))
}

/// Takes a person out: `seqs` is the last entry in `global` the remover holds of each of their devices (none if it has
/// none), and their later ones don't count.
pub fn remove_user(user: &str, seqs: &BTreeMap<Key, Option<u32>>) -> Content {
    let seqs: serde_json::Map<String, Value> = seqs.iter().map(|(k, seq)| (hex(k), json!(seq))).collect();
    node("remove", json!({ "user": user, "seqs": seqs }))
}

/// Takes a device out: `seq` is its last entry in `global` that the remover holds (none if it has none), and its later
/// ones don't count.
pub fn remove_device(key: &Key, seq: Option<u32>) -> Content {
    node("remove", json!({ "key": hex(key), "seq": seq }))
}

/// One of the install's settings; `None` puts back the custom build's default.
pub fn setting(name: &str, value: Option<&str>) -> Content {
    node("setting", json!({ "name": name, "value": value }))
}

/// The gate to the scope database is open, as an admin's own device says in the ceremony that freezes the log.
pub fn gate() -> Content {
    node("gate", json!({}))
}

pub fn group(id: &str, name: &str) -> Content {
    node("group", json!({ "group": id, "name": name }))
}

pub fn invite(group: &str, user: &str, role: Role) -> Content {
    node("invite", json!({ "group": group, "user": user, "role": role }))
}

pub fn accept(group: &str) -> Content {
    node("accept", json!({ "group": group }))
}

/// A group member's role, or `None` to take them out.
pub fn member(group: &str, user: &str, role: Option<Role>) -> Content {
    node("member", json!({ "group": group, "user": user, "role": role }))
}

/// Renames a device (`key`), a person (`user`) or a group (`group`): `what` is which, `id` its key (hex) or id.
/// Turns down a device that asked to join as this person's, before it's listed ("Not Me"): every device of theirs
/// then refuses it.
pub fn refuse(endpoint: &crate::keys::Endpoint, name: &str) -> Content {
    node("refuse", json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "name": name }))
}

/// Forgets what one of this person's devices signed in with (`Machine::sign_in`).
pub fn forget_sign_in(key: &Key) -> Content {
    node("forget-sign-in", json!({ "key": hex(key) }))
}

pub fn rename(what: &str, id: &str, name: &str) -> Content {
    let mut data = json!({ "name": name });
    data[what] = json!(id);
    node("rename", data)
}

/// Changes a person's (`what` user) or a group's (`group`) handle.
pub fn handle(what: &str, id: &str, handle: &str) -> Content {
    let mut data = json!({ "handle": handle });
    data[what] = json!(id);
    node("rename", data)
}

/// A scope's new epoch, sealed to `to`, its key's `keys::epoch_hash` as `hash`.
pub fn epoch(scope: &Scope, n: u32, to: &[Key], hash: &str) -> Content {
    node("epoch", json!({ "scope": scope.to_string(), "n": n, "to": to.iter().map(hex).collect::<Vec<_>>(), "hash": hash }))
}

/// A scope's Vault key `n`, its public half `public`, sealed to the machines `to`.
pub fn vault(scope: &Scope, n: u32, public: &[u8; 32], to: &[Key]) -> Content {
    node("vault", json!({ "scope": scope.to_string(), "n": n, "public": hex(public), "to": to.iter().map(hex).collect::<Vec<_>>() }))
}

/// A part of a device's oned (a service, or `apps`, the app service) switched on or off there.
pub fn service(device: &Key, name: &str, on: bool) -> Content {
    node("service", json!({ "device": hex(device), "name": name, "on": on }))
}

/// Where its writer's own things run besides their own machines: the install's others that run their own scope.
pub fn runs(on: &[Key]) -> Content {
    node("runs", json!({ "on": on.iter().map(hex).collect::<Vec<_>>() }))
}

/// App schedules: `endpoint` runs `app`'s schedules in `scope` until `until`, or no longer with 0. The entry is
/// `grant`, as older devices replay it.
pub fn app_schedules(endpoint: &Key, scope: &Scope, app: &str, until: u64, keys: &BTreeMap<u32, String>) -> Content {
    let keys: serde_json::Map<String, Value> = keys.iter().map(|(n, h)| (n.to_string(), json!(h))).collect();
    node("grant", json!({ "endpoint": hex(endpoint), "scope": scope.to_string(), "app": app, "until": until, "keys": keys }))
}

/// Deletes an app (`deleted`), restores it (`restored`) or erases it (`erased`), as its owner (`owner`) says.
pub fn retire(app: &str, owner: &str, how: &str) -> Result<Content> {
    let mut data = json!({ "name": app, "owner": owner });
    match how {
        "deleted" => data["deleted"] = json!(true),
        "restored" => data["deleted"] = json!(false),
        "erased" => data["erased"] = json!(true),
        other => bail!("an app is deleted, restored or erased, not {other}"),
    }
    Ok(node("app", data))
}

/// Puts an app on a scope's page, or takes it off.
pub fn install(app: &str, installed: bool) -> Content {
    node(if installed { "install" } else { "uninstall" }, json!({ "app": app }))
}

/// Lists a service endpoint offering `service` as `user`'s, a part of `device`, by that device: `shared` if it serves
/// everyone it may, not only them.
pub fn service_endpoint(endpoint: &crate::keys::Endpoint, user: &str, name: &str, service: &str, shared: bool, device: &Key) -> Content {
    let mut data = json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "user": user, "name": name, "roles": [Kind::Service], "service": service, "device": hex(device) });
    if shared {
        data["shared"] = json!(true);
    }
    node("device", data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Ext, Head, causal};
    use crate::keys::Endpoint;
    use ed25519_dalek::SigningKey;

    /// An endpoint writing entries, each after everything `read` holds.
    struct Dev {
        key: SigningKey,
        last: BTreeMap<String, Head>,
    }

    impl Dev {
        fn new(seed: u8) -> Self {
            Dev { key: SigningKey::from_bytes(&[seed; 32]), last: BTreeMap::new() }
        }
        fn id(&self) -> Key {
            self.key.verifying_key().to_bytes()
        }
        fn endpoint(&self) -> Endpoint {
            Endpoint::of(&self.key)
        }
        fn write(&mut self, log: &str, user: &str, content: Content, read: &[Entry]) -> Entry {
            let previous = read.iter().filter(|e| e.author() != self.id()).map(Entry::hash).collect();
            let e = Entry::create(&self.key, Ext { log: log.into(), time: 0, user: user.into(), epoch: 0, previous }, self.last.get(log).copied(), content);
            self.last.insert(log.into(), e.head());
            e
        }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const BOTH: &[Kind] = &[Kind::Client, Kind::Compute];
    const H: &str = "e";

    fn setup() -> (Dev, Vec<Entry>) {
        let mut mac = Dev::new(1);
        let g = mac.write("global", ANDRE, genesis("Home", ANDRE, "Andre", &mac.endpoint(), "mac", BOTH, &[7; 32], &H.repeat(64)), &[]);
        (mac, vec![g])
    }

    fn members(entries: &[Entry]) -> Members {
        let sorted = causal(entries.to_vec(), &Default::default());
        let m = Members::replay(Some(entries[0].hash()), &sorted, &Cuts::new());
        // Phase 1 changes nothing: the scope database says what replay does.
        #[cfg(feature = "db")]
        crate::database::scope::tests::agrees(&m);
        // Phase 2 changes nothing either: the same changes as steps, in that order, leave the database replay made.
        #[cfg(feature = "db")]
        crate::database::steps::tests::agrees(&sorted, &m);
        m
    }

    /// Writes `content` as `user` from `dev` after everything in `entries`, and keeps it there.
    fn add(entries: &mut Vec<Entry>, dev: &mut Dev, user: &str, content: Content) -> Entry {
        let e = dev.write("global", user, content, &entries.clone());
        entries.push(e.clone());
        e
    }

    /// What a device signed in with is recorded with its listing, for its person's next device to be checked against;
    /// a device taken out takes its record with it, and a shape this release doesn't know records nothing.
    #[test]
    fn a_devices_sign_in_is_its_persons_while_it_is_listed() {
        let (mut mac, mut entries) = setup();
        let google = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
        let listed = |d: &Dev, sign_in: Value| {
            let mut c = device(&d.endpoint(), ANDRE, "phone", &[Kind::Client], None);
            c.data["sign_in"] = sign_in;
            c
        };
        let (phone, laptop) = (Dev::new(3), Dev::new(4));
        add(&mut entries, &mut mac, ANDRE, listed(&phone, google.clone()));
        add(&mut entries, &mut mac, ANDRE, listed(&laptop, json!({ "password": "hunter2" })));
        let m = members(&entries);
        assert_eq!(m.machines[&phone.id()].sign_in, serde_json::from_value(google).ok());
        assert_eq!(m.machines[&laptop.id()].sign_in, None);
        add(&mut entries, &mut mac, ANDRE, remove_device(&phone.id(), None));
        let m = members(&entries);
        assert!(m.machine(&phone.id()).is_none(), "taken out, and the scope database's sign_ins agree it's gone (agrees)");
    }

    /// What a device signed in with is forgotten only by its person's own device; and a device they said isn't theirs
    /// ("Not Me") is never listed after, by them or anyone, and nothing it writes counts.
    #[test]
    fn a_sign_in_is_forgotten_and_a_device_turned_down_stays_out() {
        let (mut mac, mut entries) = setup();
        let google = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
        let (mut phone, mut bobs) = (Dev::new(3), Dev::new(5));
        let mut c = device(&phone.endpoint(), ANDRE, "phone", &[Kind::Client], None);
        c.data["sign_in"] = google;
        add(&mut entries, &mut mac, ANDRE, c);
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        add(&mut entries, &mut bobs, BOB, forget_sign_in(&phone.id()));
        assert!(members(&entries).machines[&phone.id()].sign_in.is_some(), "only its person forgets it");
        let forget = forget_sign_in(&phone.id());
        add(&mut entries, &mut phone, ANDRE, forget);
        assert_eq!(members(&entries).machines[&phone.id()].sign_in, None);

        let mut stranger = Dev::new(9);
        add(&mut entries, &mut mac, ANDRE, refuse(&stranger.endpoint(), "MacBook Pro"));
        add(&mut entries, &mut phone, ANDRE, device(&stranger.endpoint(), ANDRE, "MacBook Pro", &[Kind::Client], None));
        add(&mut entries, &mut stranger, ANDRE, setting("sneaky", Some("1")));
        let m = members(&entries);
        assert!(m.machine(&stranger.id()).is_none() && m.machines[&stranger.id()].removed, "turned down, it's never listed");
        assert!(!m.settings.contains_key("sneaky"), "and what it writes doesn't count");
        // One listed already is taken out instead, as Remove does.
        add(&mut entries, &mut mac, ANDRE, refuse(&phone.endpoint(), "phone"));
        assert!(members(&entries).machine(&phone.id()).is_some());
    }

    /// A new device's sign-in is checked only against what a person's own devices recorded of their own clients: not a
    /// machine an admin listed for them, nor their first device, which the inviting admin let in.
    #[cfg(feature = "db")]
    #[test]
    fn only_a_persons_own_clients_say_what_they_sign_in_with() {
        let (mut mac, mut entries) = setup();
        let account = |s: &str| json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": s } });
        let with = |c: Content, s: &str| {
            let mut c = c;
            c.data["sign_in"] = account(s);
            c
        };
        let (mut bobs, box_, laptop) = (Dev::new(5), Dev::new(6), Dev::new(7));
        add(&mut entries, &mut mac, ANDRE, with(device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))), "by-admin"));
        add(&mut entries, &mut mac, ANDRE, with(device(&box_.endpoint(), BOB, "box", &[Kind::Compute], None), "admins-machine"));
        add(&mut entries, &mut bobs, BOB, with(device(&laptop.endpoint(), BOB, "laptop", &[Kind::Client], None), "bobs-own"));
        let m = members(&entries);
        assert!(m.machine(&box_.id()).is_some(), "the admin listed it");
        let db = crate::database::scopes::ScopeDb::build(&m, &Scope::Global, &Default::default(), 0).unwrap();
        assert_eq!(db.sign_ins(BOB), vec![serde_json::from_value(account("bobs-own")).unwrap()]);
    }

    #[test]
    fn only_this_installs_genesis_counts() {
        let mut other = Dev::new(2);
        let fake = other.write("global", ANDRE, genesis("Home", ANDRE, "x", &other.endpoint(), "y", BOTH, &[7; 32], &H.repeat(64)), &[]);
        let (_, entries) = setup();
        let m = Members::replay(Some(entries[0].hash()), &[fake], &Cuts::new());
        assert!(m.users.is_empty());
        let m = members(&entries);
        assert_eq!(m.users.len(), 1);
        assert_eq!(m.seal_key(&entries[0].author()), Some(Dev::new(1).endpoint().seal), "the seal key it was listed with");
    }

    #[test]
    fn only_a_devices_person_switches_its_parts_and_lists_them_as_its_own() {
        let (mut mac, mut entries) = setup();
        let mut bobs = Dev::new(3);
        let mut linux = Dev::new(4);
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        add(&mut entries, &mut mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
        // Bob, even with Andre's listing, switches nothing on Andre's linux; Andre's own linux does.
        let (linux_id, bobs_id, mac_id) = (linux.id(), bobs.id(), mac.id());
        add(&mut entries, &mut bobs, BOB, service(&linux_id, "whatsapp", true));
        assert!(members(&entries).services.is_empty());
        add(&mut entries, &mut linux, ANDRE, service(&linux_id, "whatsapp", true));
        add(&mut entries, &mut mac, ANDRE, service(&linux_id, APPS_PART, false));
        let m = members(&entries);
        assert!(m.services[&(linux.id(), "whatsapp".into())].on && !m.services[&(linux.id(), APPS_PART.into())].on);
        // linux lists its part; it lists it again naming itself, which an old listing lacked; Bob's device can't.
        let part = Dev::new(8);
        let listed = |device: &Key| service_endpoint(&part.endpoint(), ANDRE, "linux", "whatsapp", false, device);
        add(&mut entries, &mut linux, ANDRE, listed(&mac_id));
        add(&mut entries, &mut linux, ANDRE, listed(&linux_id));
        add(&mut entries, &mut bobs, BOB, service_endpoint(&part.endpoint(), BOB, "linux", "whatsapp", false, &bobs_id));
        let m = members(&entries);
        assert_eq!((m.machines[&part.id()].user.as_str(), m.machines[&part.id()].device), (ANDRE, Some(linux_id)));
        // Nor is anything but a service listed again by its person.
        add(&mut entries, &mut mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        assert_eq!(members(&entries).machines[&linux_id].roles.len(), 2);
    }

    #[test]
    fn a_member_entry_counts_only_when_signed_by_a_device_of_its_user() {
        let (mut mac, mut entries) = setup();
        let mut bobs = Dev::new(3);
        let mut linux = Dev::new(4);
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        add(&mut entries, &mut mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        // Andre's device can't sign as Bob, Bob's can't as Andre, and a compute endpoint signs no member entry.
        add(&mut entries, &mut mac, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" })));
        add(&mut entries, &mut bobs, ANDRE, node("setting", json!({ "name": "a", "value": "1" })));
        add(&mut entries, &mut linux, ANDRE, node("setting", json!({ "name": "b", "value": "1" })));
        let m = members(&entries);
        assert_eq!(m.users[BOB].name, "Bob");
        assert!(m.settings.is_empty());
        // Bob's own device renames him; it can't list a device for Andre, nor Andre's list a client for Bob.
        add(&mut entries, &mut bobs, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" })));
        add(&mut entries, &mut bobs, BOB, device(&Dev::new(5).endpoint(), ANDRE, "x", &[Kind::Client], None));
        add(&mut entries, &mut mac, ANDRE, device(&Dev::new(6).endpoint(), BOB, "y", &[Kind::Client], None));
        let m = members(&entries);
        assert_eq!(m.users[BOB].name, "Bobby");
        assert!(m.machine(&Dev::new(5).id()).is_none() && m.machine(&Dev::new(6).id()).is_none());
        // The compute endpoint runs Andre's scope and the install's, not Bob's.
        assert!(m.runs_for(&linux.id(), &Scope::User(ANDRE.into())) && m.runs_for(&linux.id(), &Scope::Global));
        assert!(!m.runs_for(&linux.id(), &Scope::User(BOB.into())) && !m.runs_for(&bobs.id(), &Scope::User(BOB.into())));
        // A machine an admin lists as Bob's runs what Bob may run elsewhere, never Bob's own scope, until Bob's own
        // device lists it.
        let server = Dev::new(7);
        add(&mut entries, &mut mac, ANDRE, device(&server.endpoint(), BOB, "bob's server", &[Kind::Compute], None));
        assert!(!members(&entries).runs_for(&server.id(), &Scope::User(BOB.into())), "not by an admin's say");
        add(&mut entries, &mut bobs, BOB, device(&server.endpoint(), BOB, "bob's server", &[Kind::Compute], None));
        assert!(members(&entries).runs_for(&server.id(), &Scope::User(BOB.into())));
        add(&mut entries, &mut mac, ANDRE, device(&server.endpoint(), ANDRE, "mine now", &[Kind::Compute], None));
        assert_eq!(members(&entries).machines[&server.id()].user, BOB, "an admin can't take it back");
    }

    #[test]
    fn a_removed_devices_later_entries_dont_count_wherever_they_sort() {
        let (mut mac, mut entries) = setup();
        let mut laptop = Dev::new(3);
        add(&mut entries, &mut mac, ANDRE, device(&laptop.endpoint(), ANDRE, "laptop", &[Kind::Client], None));
        let before = add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "before", "value": "1" })));
        let read = entries.clone();
        add(&mut entries, &mut mac, ANDRE, node("setting", json!({ "name": "busy", "value": "1" })));
        let removed = add(&mut entries, &mut mac, ANDRE, node("remove", json!({ "key": hex(&laptop.id()), "seq": before.seq() })));
        // The laptop, never having read its removal, writes after what it had read: it sorts before the removal.
        let sneaky = laptop.write("global", ANDRE, node("setting", json!({ "name": "sneaky", "value": "1" })), &read);
        entries.push(sneaky.clone());
        add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "after", "value": "1" })));
        let sorted = causal(entries.clone(), &Default::default());
        let at = |h: Hash| sorted.iter().position(|e| e.hash() == h).unwrap();
        assert!(at(sneaky.hash()) < at(removed.hash()), "it sorts before its removal");
        let m = members(&entries);
        assert!(m.settings.contains_key("before"), "what its remover had seen still counts");
        assert!(!m.settings.contains_key("sneaky") && !m.settings.contains_key("after"));
        assert!(m.machine(&laptop.id()).is_none());
        // A removal that doesn't say what its remover had seen doesn't count.
        let (mut mac, mut entries) = setup();
        add(&mut entries, &mut mac, ANDRE, device(&Dev::new(4).endpoint(), ANDRE, "phone", &[Kind::Client], None));
        add(&mut entries, &mut mac, ANDRE, node("remove", json!({ "key": hex(&Dev::new(4).id()) })));
        assert!(members(&entries).machine(&Dev::new(4).id()).is_some());
    }

    #[test]
    fn a_device_that_forks_its_log_counts_no_more_from_the_fork_on_wherever_its_sides_arrive() {
        let (mut mac, mut entries) = setup();
        let mut laptop = Dev::new(3);
        add(&mut entries, &mut mac, ANDRE, device(&laptop.endpoint(), ANDRE, "laptop", &[Kind::Client], None));
        add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "before", "value": "1" })));
        let fork_at = laptop.last.get("global").copied();
        let one = add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "one", "value": "1" })));
        laptop.last.insert("global".into(), fork_at.unwrap());
        let two = add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "two", "value": "1" })));
        add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "after", "value": "1" })));
        add(&mut entries, &mut mac, ANDRE, node("setting", json!({ "name": "mac", "value": "1" })));
        assert_eq!(one.seq(), two.seq());
        // Two devices that took the sides in opposite orders count the same.
        let replayed = |list: Vec<Entry>| {
            let store = crate::store::Store::memory().unwrap();
            store.take(list).unwrap();
            let member_entries: Vec<Entry> = store.scope("global").unwrap().into_iter().filter(is_member_entry).collect();
            Members::replay(Some(entries[0].hash()), &member_entries, &store.forks().unwrap())
        };
        let a = replayed(entries.clone());
        let b = replayed(entries.iter().rev().cloned().collect());
        for m in [&a, &b] {
            assert_eq!(m.settings.keys().collect::<Vec<_>>(), ["before", "mac"], "nothing from the fork on");
            assert!(m.machine(&laptop.id()).is_none(), "it's removed");
            #[cfg(feature = "db")]
            crate::database::scope::tests::agrees(m);
        }
    }

    #[test]
    fn a_new_epoch_counts_only_from_the_scopes_admins_and_names_its_key() {
        let (mut mac, mut entries) = setup();
        let mut bobs = Dev::new(3);
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        let m = members(&entries);
        assert_eq!(m.epochs[&Scope::Global], Epoch { n: 1, to: vec![mac.id()], hash: H.repeat(64) }, "global's first is the genesis's");
        let to = json!([hex(&mac.id())]);
        let epoch = |scope: &str, n: u32, hash: Option<String>| node("epoch", json!({ "scope": scope, "n": n, "to": to, "hash": hash }));
        add(&mut entries, &mut bobs, BOB, epoch("global", 2, Some(H.repeat(64))));
        add(&mut entries, &mut bobs, BOB, epoch(&format!("user:{ANDRE}"), 1, Some(H.repeat(64))));
        assert_eq!(members(&entries).epochs.len(), 1, "not Bob's to start");
        add(&mut entries, &mut mac, ANDRE, epoch("global", 3, Some(H.repeat(64))));
        add(&mut entries, &mut mac, ANDRE, epoch("global", 2, None));
        assert_eq!(members(&entries).epoch(&Scope::Global), 1, "epochs go one at a time, each naming its key");
        add(&mut entries, &mut mac, ANDRE, epoch("global", 2, Some("f".repeat(64))));
        let bob = hex(&bobs.id());
        add(&mut entries, &mut bobs, BOB, node("epoch", json!({ "scope": format!("user:{BOB}"), "n": 1, "to": [bob], "hash": H.repeat(64) })));
        let m = members(&entries);
        assert_eq!(m.epoch(&Scope::Global), 2);
        assert_eq!(m.epoch_hash(&Scope::Global, 2), Some("f".repeat(64).as_str()));
        assert_eq!(m.epoch_hash(&Scope::Global, 1), Some(H.repeat(64).as_str()), "each epoch's hash is kept");
        assert_eq!(m.epoch(&Scope::User(BOB.into())), 1, "a person's own scope is theirs");
        assert_eq!(m.epoch(&Scope::User(ANDRE.into())), 0);
    }

    /// An install whose recovery key is `recovery`'s, with Andre's mac and linux (compute), and Bob in with his phone.
    fn with_bob(recovery: &Dev) -> (Dev, Dev, Dev, Vec<Entry>) {
        let (mut mac, mut linux, bobs) = (Dev::new(1), Dev::new(4), Dev::new(3));
        let g = mac.write("global", ANDRE, genesis("Home", ANDRE, "Andre", &mac.endpoint(), "mac", BOTH, &recovery.id(), &H.repeat(64)), &[]);
        let mut entries = vec![g];
        add(&mut entries, &mut mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        let _ = &mut linux;
        (mac, linux, bobs, entries)
    }

    #[test]
    fn only_a_persons_own_device_makes_their_scopes_keys_and_says_where_it_runs() {
        let recovery = Dev::new(9);
        let (mut mac, linux, mut bobs, mut entries) = with_bob(&recovery);
        let bob = Scope::User(BOB.into());
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        // Nothing runs Bob's own things until his device says: then the install's machine he chose does.
        assert!(!members(&entries).runs_for(&linux.id(), &bob));
        add(&mut entries, &mut mac, ANDRE, runs(&[linux.id()]));
        add(&mut entries, &mut mac, BOB, runs(&[linux.id()]));
        assert!(!members(&entries).runs_for(&linux.id(), &bob), "an admin's device can't say where Bob's things run");
        add(&mut entries, &mut bobs, BOB, runs(&[linux.id()]));
        let m = members(&entries);
        assert!(m.runs_for(&linux.id(), &bob) && m.serves(&linux.id(), BOB));
        assert!(m.runs_for(&linux.id(), &Scope::User(ANDRE.into())), "Andre's says nothing of his own: his machines run it");
        // Only machines that run things: his phone isn't one.
        add(&mut entries, &mut bobs, BOB, runs(&[linux.id(), bob_id]));
        assert!(members(&entries).runs_for(&linux.id(), &bob), "a list naming what can't run things doesn't count");
        add(&mut entries, &mut bobs, BOB, runs(&[]));
        assert!(!members(&entries).runs_for(&linux.id(), &bob), "nothing runs them now");
        // His scope's keys: an admin's device can't make them, his own can, sealed only to his devices.
        add(&mut entries, &mut mac, ANDRE, epoch(&bob, 1, &[mac_id], &H.repeat(64)));
        add(&mut entries, &mut mac, BOB, epoch(&bob, 1, &[mac_id], &H.repeat(64)));
        assert_eq!(members(&entries).epoch(&bob), 0);
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 1, &[bob_id], &"b".repeat(64)));
        let m = members(&entries);
        assert_eq!(m.epoch_hash(&bob, 1), Some("b".repeat(64).as_str()));
        let opens: Vec<Key> = m.clients().filter(|d| m.may_use(&d.user, &bob)).map(|d| d.key).collect();
        assert_eq!(opens, vec![bobs.id()], "only Bob's own devices open it");
    }

    #[test]
    fn a_removed_persons_devices_get_no_new_epoch_and_their_entries_stop() {
        let recovery = Dev::new(9);
        let (mut mac, _, mut bobs, mut entries) = with_bob(&recovery);
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        add(&mut entries, &mut mac, ANDRE, remove_user(BOB, &[(bob_id, None)].into()));
        // A new epoch of everyone's can't be sealed to Bob's phone, and what his phone writes after doesn't count.
        add(&mut entries, &mut mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id, bob_id], &"c".repeat(64)));
        add(&mut entries, &mut bobs, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" })));
        let m = members(&entries);
        assert_eq!(m.epoch(&Scope::Global), 1, "an epoch sealed to a removed person's device doesn't count");
        assert!(m.client(&bobs.id()).is_none() && m.clients().all(|c| c.user != BOB));
        assert_eq!(m.users[BOB].name, "Bob");
        add(&mut entries, &mut mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id], &"c".repeat(64)));
        assert_eq!(members(&entries).epoch(&Scope::Global), 2, "sealed to those left, it does");
        assert!(members(&entries).scopes().contains(&Scope::User(BOB.into())), "their own scope is kept");
    }

    #[test]
    fn a_scope_owes_a_new_epoch_once_someone_holding_its_keys_loses_the_right_until_one_counts() {
        let recovery = Dev::new(9);
        let (mut mac, linux, mut bobs, mut entries) = with_bob(&recovery);
        let (mac_id, bob_id, linux_id) = (mac.id(), bobs.id(), linux.id());
        let (bob, work) = (Scope::User(BOB.into()), Scope::Group("c000000000000003".into()));
        let owed = |entries: &[Entry]| members(entries).owed.into_iter().collect::<Vec<_>>();
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 1, &[bob_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, group("c000000000000003", "Work"));
        add(&mut entries, &mut mac, ANDRE, epoch(&work, 1, &[mac_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, invite("c000000000000003", BOB, Role::Member));
        add(&mut entries, &mut bobs, BOB, accept("c000000000000003"));
        // Apps of the group's, the admins' and Bob's, and a Cloudflare node, which change nothing owed.
        add(&mut entries, &mut mac, ANDRE, node("app", json!({ "name": "notes", "owner": "group:c000000000000003" })));
        add(&mut entries, &mut mac, ANDRE, node("app", json!({ "name": "tally", "owner": "admins" })));
        add(&mut entries, &mut bobs, BOB, node("app", json!({ "name": "bobs", "owner": format!("user:{BOB}") })));
        add(&mut entries, &mut mac, ANDRE, setting("serverless", Some(&hex(&Dev::new(14).id()))));
        let (mut laptop, tablet) = (Dev::new(5), Dev::new(6));
        add(&mut entries, &mut bobs, BOB, device(&laptop.endpoint(), BOB, "laptop", &[Kind::Client], None));
        add(&mut entries, &mut bobs, BOB, device(&tablet.endpoint(), BOB, "tablet", &[Kind::Client], None));
        assert!(owed(&entries).is_empty());
        // Signing itself out, a device deletes its own keys: nothing's owed.
        let laptop_id = laptop.id();
        add(&mut entries, &mut laptop, BOB, remove_device(&laptop_id, Some(0)));
        assert!(owed(&entries).is_empty() && members(&entries).machine(&laptop_id).is_none(), "signing out owes nothing");
        // Nor one a node of its person's wrote for it, saying it signed itself out; an admin's saying so for another's
        // device is taken as removing it.
        let (asked, said) = (Dev::new(12), Dev::new(13));
        add(&mut entries, &mut bobs, BOB, device(&asked.endpoint(), BOB, "app", &[Kind::Client], None));
        add(&mut entries, &mut bobs, BOB, device(&said.endpoint(), BOB, "app", &[Kind::Client], None));
        let gone = |k: &Dev| node("remove", json!({ "key": hex(&k.id()), "seq": null, "itself": true }));
        add(&mut entries, &mut bobs, BOB, gone(&asked));
        assert!(owed(&entries).is_empty() && members(&entries).machine(&asked.id()).is_none(), "its own node signing it out owes nothing");
        add(&mut entries, &mut mac, ANDRE, gone(&said));
        assert_eq!(owed(&entries).len(), 3, "an admin's saying it signed itself out is a removal");
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 2, &[bob_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id, bob_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, epoch(&work, 2, &[mac_id, bob_id], &H.repeat(64)));
        // Nor does a sign-in before on a device signed in again, which goes as signing out would have taken it.
        let on = |d: &Dev| {
            let mut c = device(&d.endpoint(), BOB, "phone app", &[Kind::Client], None);
            c.data["device"] = json!(hex(&[8; 32]));
            c
        };
        let (before, again) = (Dev::new(10), Dev::new(11));
        add(&mut entries, &mut bobs, BOB, on(&before));
        add(&mut entries, &mut bobs, BOB, on(&again));
        add(&mut entries, &mut bobs, BOB, remove_device(&before.id(), None));
        assert!(owed(&entries).is_empty() && members(&entries).machine(&before.id()).is_none(), "signing in again owes nothing");
        // Taken out by another, it may still hold every key it opened, until each scope's next epoch counts.
        add(&mut entries, &mut mac, ANDRE, remove_device(&tablet.id(), None));
        assert_eq!(owed(&entries), vec![Scope::Global, work.clone(), bob.clone()]);
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 3, &[bob_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, epoch(&Scope::Global, 3, &[tablet.id()], &H.repeat(64)));
        assert_eq!(owed(&entries), vec![Scope::Global, work.clone()], "one sealed to the device taken out doesn't count");
        add(&mut entries, &mut mac, ANDRE, epoch(&Scope::Global, 3, &[mac_id, bob_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, epoch(&work, 3, &[mac_id, bob_id], &H.repeat(64)));
        assert!(owed(&entries).is_empty());
        // App schedules stopped, and a machine running them taken out.
        add(&mut entries, &mut bobs, BOB, app_schedules(&linux_id, &bob, "notes", 1, &BTreeMap::new()));
        add(&mut entries, &mut bobs, BOB, app_schedules(&linux_id, &bob, "notes", 0, &BTreeMap::new()));
        assert_eq!(owed(&entries), vec![bob.clone()]);
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 4, &[bob_id], &H.repeat(64)));
        add(&mut entries, &mut bobs, BOB, app_schedules(&linux_id, &bob, "notes", 1, &BTreeMap::new()));
        add(&mut entries, &mut mac, ANDRE, remove_device(&linux_id, None));
        assert_eq!(owed(&entries), vec![bob.clone()], "a machine holds no epoch, only the keys of the app schedules it runs");
        add(&mut entries, &mut bobs, BOB, epoch(&bob, 5, &[bob_id], &H.repeat(64)));
        // Taken out of a group, then out of the install: their own scope is kept as it is.
        add(&mut entries, &mut mac, ANDRE, member("c000000000000003", BOB, None));
        assert_eq!(owed(&entries), vec![work.clone()]);
        add(&mut entries, &mut mac, ANDRE, epoch(&work, 4, &[mac_id], &H.repeat(64)));
        add(&mut entries, &mut mac, ANDRE, remove_user(BOB, &[(bob_id, None)].into()));
        assert_eq!(owed(&entries), vec![Scope::Global]);
    }

    #[test]
    fn a_removed_admins_devices_write_nothing_that_counts_after_what_their_remover_had_seen() {
        const MAL: &str = "c000000000000003";
        let (mut mac, mut entries) = setup();
        let (mut laptop, eve, stranger) = (Dev::new(5), Dev::new(6), Dev::new(8));
        add(&mut entries, &mut mac, ANDRE, device(&laptop.endpoint(), MAL, "mal's laptop", &[Kind::Client], Some(("Mal", Role::Admin))));
        let before = add(&mut entries, &mut laptop, MAL, node("setting", json!({ "name": "before", "value": "1" })));
        let read = entries.clone();
        let removed = add(&mut entries, &mut mac, ANDRE, remove_user(MAL, &[(laptop.id(), Some(before.seq()))].into()));
        // Mal's laptop, never having read its removal, writes after only what it had read: it sorts before the removal.
        let admin = laptop.write("global", MAL, device(&eve.endpoint(), "e000000000000005", "eve's", &[Kind::Client], Some(("Eve", Role::Admin))), &read);
        let recovery = laptop.write("global", MAL, node("recovery", json!({ "key": hex(&stranger.id()) })), &read);
        entries.extend([admin.clone(), recovery.clone()]);
        let sorted = causal(entries.clone(), &Default::default());
        let at = |h: Hash| sorted.iter().position(|e| e.hash() == h).unwrap();
        assert!(at(admin.hash()) < at(removed.hash()) || at(recovery.hash()) < at(removed.hash()), "one sorts before the removal");
        let m = members(&entries);
        assert!(m.settings.contains_key("before"), "what its remover had seen still counts");
        assert!(m.machine(&eve.id()).is_none() && m.user("e000000000000005").is_none(), "the admin device it wrote doesn't count");
        assert_eq!(m.recovery, Some([7; 32]), "nor its recovery key");
        assert!(m.machine(&laptop.id()).is_none() && m.user(MAL).is_none());
        // A removal that doesn't say what its remover had seen of their devices doesn't count.
        add(&mut entries, &mut mac, ANDRE, device(&Dev::new(10).endpoint(), "d000000000000004", "dan's", &[Kind::Client], Some(("Dan", Role::Member))));
        add(&mut entries, &mut mac, ANDRE, node("remove", json!({ "user": "d000000000000004" })));
        assert!(members(&entries).user("d000000000000004").is_some());
    }

    #[test]
    fn the_recovery_key_lets_someones_device_in_as_an_admins_approval_and_is_listed() {
        let mut recovery = Dev::new(9);
        let (mut mac, _, _, mut entries) = with_bob(&recovery);
        let laptop = Dev::new(5);
        assert!(members(&entries).recovery_used.is_none());
        let e = recovery.write("global", BOB, device(&laptop.endpoint(), BOB, "bob's laptop", &[Kind::Client], None), &entries);
        entries.push(e.clone());
        let m = members(&entries);
        assert_eq!(m.client(&laptop.id()).map(|c| c.user.as_str()), Some(BOB), "it counts as an admin's approval");
        assert_eq!(m.recovery_used, Some((e.time(), BOB.to_string())), "and it's listed");
        // Only a device people use, of someone already in; and nothing else.
        for (user, content) in [
            (BOB, device(&Dev::new(6).endpoint(), BOB, "box", &[Kind::Compute], None)),
            ("c000000000000003", device(&Dev::new(7).endpoint(), "c000000000000003", "new", &[Kind::Client], Some(("Cy", Role::Admin)))),
            (ANDRE, node("setting", json!({ "name": "a", "value": "1" }))),
            (ANDRE, role(BOB, Role::Admin)),
        ] {
            let e = recovery.write("global", user, content, &entries);
            entries.push(e);
        }
        let m = members(&entries);
        assert!(m.machine(&Dev::new(6).id()).is_none() && m.machine(&Dev::new(7).id()).is_none());
        assert!(m.settings.is_empty() && m.users[BOB].role == Role::Member && m.user("c000000000000003").is_none());
        // A key that isn't the install's recovery key lets nobody in.
        let mut other = Dev::new(8);
        entries.push(other.write("global", BOB, device(&Dev::new(10).endpoint(), BOB, "x", &[Kind::Client], None), &entries));
        assert!(members(&entries).machine(&Dev::new(10).id()).is_none());
        // Only the recovery key hands over to a new one: an admin's device can't, and the old key counts no more.
        add(&mut entries, &mut mac, ANDRE, node("recovery", json!({ "key": hex(&other.id()) })));
        assert_eq!(members(&entries).recovery, Some(recovery.id()), "not by an admin's device");
        let e = recovery.write("global", ANDRE, node("recovery", json!({ "key": hex(&other.id()) })), &entries);
        entries.push(e.clone());
        let m = members(&entries);
        assert_eq!((m.recovery, m.recovery_set, m.recovery_was.clone()), (Some(other.id()), e.time(), vec![recovery.id()]));
        entries.push(recovery.write("global", BOB, device(&Dev::new(11).endpoint(), BOB, "y", &[Kind::Client], None), &entries));
        assert!(members(&entries).machine(&Dev::new(11).id()).is_none(), "the old recovery key lets nobody in");
    }

}
