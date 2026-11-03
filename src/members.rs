//! Members: users, groups, the endpoints of each (devices people use, compute that runs calls, services), epochs,
//! grants, apps and deploys, made by replaying `global`'s node entries in causal order; and which entries count: a
//! member entry only when one of its user's own devices signed it, a scope's other entries also when a compute
//! endpoint that runs that scope's calls did. Pure: entries in, members out.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Hash, Key, Scope, hex, is_id, unhex};

/// The node's own entries. Apps never see them.
pub const APP: &str = "node";
/// Agents' built-in sidecar (`src/agents.rs`), whose Vault requests are theirs: here too for the serverless node.
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

/// The latest `sidecar` entry for a sidecar on a device: removed there, or let register there again.
#[derive(Clone, Debug)]
pub struct SidecarInstall {
    /// Its executable, for its device's node to start *(not built yet)*.
    pub blob: Option<String>,
    /// Installed by registering over the local API, as `sidecars/install.sh` does.
    pub local: bool,
    pub by: String,
    pub time: u64,
}

impl SidecarInstall {
    /// Removed from its device: it isn't served there until it's installed again.
    pub fn removed(&self) -> bool {
        self.blob.is_none() && !self.local
    }
}

#[derive(Clone, Debug, Default)]
pub struct Members {
    pub root: Option<Hash>,
    pub name: String,
    pub recovery: Option<Key>,
    pub users: BTreeMap<String, User>,
    pub machines: BTreeMap<Key, Machine>,
    pub groups: BTreeMap<String, Group>,
    /// The install's settings its admins set: each name's latest value. The rest are the white label's defaults.
    pub settings: BTreeMap<String, String>,
    pub apps: BTreeMap<String, App>,
    /// The latest `sidecar` entry of each sidecar on each device: (device, sidecar).
    pub sidecars: BTreeMap<(Key, String), SidecarInstall>,
    /// When the install's devices last changed, by the newest genesis, device or remove: the relay's list is as of then.
    pub devices_at: u64,
    /// Each scope's current epoch, by its newest `epoch` entry that counts (src/keys.rs).
    pub epochs: BTreeMap<Scope, Epoch>,
    /// Standing work: until when (milliseconds) each endpoint may hold an app's keys in a scope, by (endpoint, scope,
    /// app), as the latest `grant` of each said; one with `until: 0` is gone.
    pub grants: BTreeMap<(Key, Scope, String), Grant>,
}

/// A grant of standing work: who gave it, and until when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub by: String,
    pub until: u64,
}

/// A scope's epoch: its number, and the endpoints it was first sealed to (`keys::epoch_name`), besides the recovery key.
/// A new one starts only when a person or device is removed; one added is sealed the current one by the device that
/// approves it, so who may open a scope comes from roles and devices, not from `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Epoch {
    pub n: u32,
    pub to: Vec<Key>,
}

/// Node entries that change only their own scope's state, never the members, so no other scope replays them.
const SCOPE_ONLY: &[&str] = &["schedule", "secret-request", "vault-request", "item", "keep", "forget", "backup-key"];

/// Whether an entry is one of `global`'s node entries, which every scope's replay replays.
pub fn is_member_entry(e: &Entry) -> bool {
    e.content.app == APP && e.log() == "global" && !SCOPE_ONLY.contains(&e.content.kind.as_str())
}

impl Members {
    pub fn new(root: Option<Hash>) -> Members {
        Members { root, ..Default::default() }
    }

    /// Replays `global`'s node entries, sorted in replay order.
    pub fn replay(root: Option<Hash>, member_entries: &[Entry]) -> Members {
        let mut m = Members::new(root);
        for e in member_entries {
            m.step(e);
        }
        m
    }

    /// Applies one of `global`'s node entries, if it counts. Returns whether it did.
    pub fn step(&mut self, e: &Entry) -> bool {
        let counted = (e.content.kind == "genesis" || self.counts(e)) && self.apply(e).is_ok();
        if !counted {
            tracing::debug!("{} entry {} doesn't count", e.content.kind, e.id());
        }
        counted
    }

    /// A scope's current epoch: 0 while it has none yet.
    pub fn epoch(&self, scope: &Scope) -> u32 {
        self.epochs.get(scope).map_or(0, |e| e.n)
    }

    /// The key things are sealed to for an endpoint of the install, as it's listed.
    pub fn seal_key(&self, key: &Key) -> Option<[u8; 32]> {
        self.machine(key).map(|m| m.seal)
    }

    /// The grants of standing work in force at `now`: (endpoint, scope, app) each.
    pub fn grants_at(&self, now: u64) -> impl Iterator<Item = &(Key, Scope, String)> {
        self.grants.iter().filter(move |(_, g)| g.until > now).map(|(k, _)| k)
    }

    // Who is who.

    pub fn user(&self, id: &str) -> Option<&User> {
        self.users.get(id).filter(|u| !u.removed)
    }

    /// Whether `id` was a person of the install, and was removed.
    pub fn removed_user(&self, id: &str) -> bool {
        self.users.get(id).is_some_and(|u| u.removed)
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

    /// Whether a scope's calls run on `endpoint`: a compute endpoint its owners trust, which is one listed as a device
    /// of the scope's person, for a user's own scope; of an admin of the group or of the install, for a group's; of an
    /// admin of the install, for `global`.
    pub fn runs_for(&self, endpoint: &Key, scope: &Scope) -> bool {
        let Some(m) = self.compute(endpoint) else { return false };
        match scope {
            Scope::User(u) => m.user == *u,
            Scope::Group(g) => self.manages(&m.user, g),
            Scope::Global => self.is_admin(&m.user),
        }
    }

    /// Whether `endpoint` runs any of the calls of `user`: a compute endpoint that runs some scope they use.
    pub fn serves(&self, endpoint: &Key, user: &str) -> bool {
        self.user(user).is_some() && self.scopes().iter().any(|s| self.may_use(user, s) && self.runs_for(endpoint, s))
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
        if !self.may_use(e.user(), &scope) || e.epoch() > self.epoch(&scope) {
            return false;
        }
        let author = e.author();
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
                let machine = listed(device, &user)?;
                ensure!(machine.key == author && machine.is(Kind::Client), "genesis is signed by the first device it lists, a client");
                self.name = text("name")?;
                self.recovery = Some(key_of("recovery")?);
                self.users.insert(user.clone(), User { id: user.clone(), name: text("user_name")?, role: Role::Admin, removed: false });
                self.machines.insert(author, machine);
                self.devices_at = e.time();
            }
            "device" => {
                let user = id_of("user")?;
                let machine = listed(data, &user)?;
                ensure!(!self.machines.contains_key(&machine.key), "that key is listed already");
                match self.users.get(&user) {
                    Some(u) => ensure!(!u.removed && (w == user || (self.is_admin(&w) && !machine.is(Kind::Client))), "only a person's own device lists their devices; an admin's lists compute and services"),
                    None => {
                        ensure!(self.is_admin(&w), "only admins add people");
                        let role = role_of("role")?;
                        self.users.insert(user.clone(), User { id: user.clone(), name: text("user_name")?, role, removed: false });
                    }
                }
                self.machines.insert(machine.key, machine);
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
                    self.users.get_mut(&user).context("no such user")?.removed = true;
                } else {
                    let key = key_of("key")?;
                    ensure!(self.machine(&key).is_some(), "no such key");
                    ensure!(self.looks_after(&w, &key), "only its person or an admin removes a device");
                    self.machines.get_mut(&key).unwrap().removed = true;
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
                self.epochs.insert(scope, Epoch { n: n as u32, to });
            }
            "grant" => {
                let endpoint = key_of("endpoint")?;
                let scope = Scope::parse(&text("scope")?)?;
                let app = text("app")?;
                let until = data["until"].as_u64().context("no until")?;
                let allowed = match &scope {
                    Scope::User(u) => *u == w,
                    Scope::Group(g) => self.group_role(&w, g) == Some(Role::Admin),
                    Scope::Global => self.is_admin(&w),
                };
                ensure!(allowed, "only the scope's person, or its group's admins, grant standing work in it");
                ensure!(until == 0 || self.machine(&endpoint).is_some(), "no such endpoint");
                match until {
                    0 => drop(self.grants.remove(&(endpoint, scope, app))),
                    until => drop(self.grants.insert((endpoint, scope, app), Grant { by: w, until })),
                }
            }
            "group" => {
                let group = id_of("group")?;
                ensure!(!self.groups.contains_key(&group), "that group exists");
                let members = BTreeMap::from([(w, Role::Admin)]);
                self.groups.insert(group.clone(), Group { id: group, name: text("name")?, members, invited: BTreeMap::new() });
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
                        g.members.remove(&user);
                        g.invited.remove(&user);
                    }
                }
            }
            "rename" => {
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
                ensure!(!name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "setting names are lowercase words joined by _");
                match &data["value"] {
                    Value::Null => self.settings.remove(&name),
                    value => self.settings.insert(name, value.as_str().context("a setting is text")?.to_string()),
                };
            }
            "recovery" => {
                ensure!(self.is_admin(&w), "admins only");
                self.recovery = Some(key_of("key")?);
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
            "sidecar" => {
                let device = key_of("device")?;
                let name = text("name")?;
                ensure!(valid_app_name(&name), "sidecar names are lowercase words joined by -");
                ensure!(self.machine(&device).is_some(), "no such device");
                ensure!(self.looks_after(&w, &device), "only a device's person or an admin installs or removes its sidecars");
                let blob = data["blob"].as_str().map(str::to_string);
                ensure!(blob.as_ref().is_none_or(|b| e.content.blobs.contains(b)), "a sidecar names its executable among its blobs");
                self.sidecars.insert((device, name), SidecarInstall { blob, local: data["local"] == true, by: w, time: e.time() });
            }
            "install" | "uninstall" | "node-release" => {}
            other => bail!("unknown kind {other}"),
        }
        Ok(())
    }
}

/// An endpoint as a `device` entry (or the genesis's `device`) lists it, for `user`.
fn listed(data: &Value, user: &str) -> Result<Machine> {
    let text = |key: &str| data[key].as_str().with_context(|| format!("no {key}"));
    let roles: BTreeSet<Kind> = serde_json::from_value(data["roles"].clone()).context("roles are client, compute or service")?;
    ensure!(!roles.is_empty(), "an endpoint does something: client, compute or service");
    let name = text("name")?.trim().to_string();
    ensure!(!name.is_empty(), "an endpoint has a name");
    Ok(Machine {
        key: unhex(text("key")?)?,
        seal: unhex(text("seal")?)?,
        user: user.to_string(),
        name,
        roles,
        passkey: data["passkey"].as_str().map(str::to_string),
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

pub fn valid_app_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && name != APP
}

/// What a scope's own node entries add up to: the apps installed in it, every `open` of it, its Vault items, and its
/// schedules' switches.
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
    /// The Vault's items here, by id.
    pub items: BTreeMap<String, crate::vault::Item>,
    /// In a user's own scope: how they arranged each page, by page (`user`, `group:<id>` or `global`):
    /// `{ items, hidden }`, as their latest `arrange` of it said.
    pub arrangements: BTreeMap<String, Value>,
    /// The key this scope's backups are encrypted with: the first `backup-key` written here.
    pub backup_key: Option<[u8; 32]>,
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

fn apply_in_scope(m: &Members, state: &mut ScopeState, e: &Entry) -> Result<()> {
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
        "item" => {
            ensure!(manages_scope(m, w, &scope), "not allowed to change the Vault there");
            let data = &e.content.data;
            let id = crate::vault::apply(&mut state.items, data, w, e.time(), |a| !m.apps.get(a).is_some_and(|a| a.erased))?;
            answered(state, data, &id, w, e.time());
        }
        "backup-key" => {
            ensure!(state.backup_key.is_none(), "the scope has its backup key");
            let key = unhex(e.content.data["key"].as_str().context("no key")?)?;
            state.backup_key = Some(key);
        }
        // A blob the scope keeps, which its `blobs` names.
        "keep" => {}
        "forget" => {
            let blob = e.content.data["blob"].as_str().filter(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())).context("no blob")?;
            state.forgotten.insert(blob.to_string());
        }
        "secret-request" | "vault-request" => {
            ensure!(manages_scope(m, w, &scope), "not allowed to change the Vault there");
            let data = &e.content.data;
            let id = data["id"].as_str().filter(|id| is_id(id)).context("no id")?.to_string();
            if data["declined"] == true {
                let r = state.requests.get_mut(&id).context("no such request")?;
                ensure!(r.answer.is_none(), "already answered");
                r.answer = Some(("declined".into(), w.to_string(), e.time()));
                return Ok(());
            }
            ensure!(!state.requests.contains_key(&id), "that request is already made");
            let app = data["app"].as_str().context("no app")?.to_string();
            // An item the node itself uses (a backup place's) is asked for as the node's, and one agents use (vault_fetch)
            // as theirs.
            ensure!(m.apps.contains_key(&app) || app == APP || app == AGENTS, "no such app");
            // An earlier device's asks for one secret field, `name`.
            let (title, fields) = match data["name"].as_str() {
                Some(name) => (name.to_string(), vec![(name.to_string(), true)]),
                None => {
                    let fields: Vec<(String, bool)> = data["fields"].as_object().context("no fields")?.iter().map(|(n, f)| (n.clone(), f["secret"] == true)).collect();
                    (data["title"].as_str().unwrap_or("").trim().to_string(), fields)
                }
            };
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
pub fn replay(root: Option<Hash>, member_entries: &[Entry], scope_entries: &[Entry], f: impl FnMut(&Entry, &Members, &ScopeState)) -> (Members, ScopeState) {
    replay_from(root, member_entries, scope_entries, ScopeState::default(), f)
}

/// Replays more of a scope's entries onto `state`, what its earlier ones added up to.
pub fn replay_from(root: Option<Hash>, member_entries: &[Entry], scope_entries: &[Entry], mut state: ScopeState, mut f: impl FnMut(&Entry, &Members, &ScopeState)) -> (Members, ScopeState) {
    let mut m = Members::new(root);
    let mut members = member_entries.iter().peekable();
    for e in scope_entries {
        let order = e.order();
        while let Some(me) = members.next_if(|me| me.order() < order) {
            m.step(me);
            forget_erased(&m, &mut state, me);
        }
        let counted = if is_member_entry(e) {
            members.next_if(|me| me.order() == order);
            let counted = m.step(e) && (!matches!(e.content.kind.as_str(), "install" | "uninstall") || apply_in_scope(&m, &mut state, e).is_ok());
            forget_erased(&m, &mut state, e);
            counted
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
        forget_erased(&m, &mut state, me);
    }
    (m, state)
}

/// After `e`, an `app` that erased an app: it's off every item in the scope, and the items it made or only it used
/// go, as no app of its name runs again.
pub fn forget_erased(m: &Members, state: &mut ScopeState, e: &Entry) {
    if e.content.app == APP && e.content.kind == "app" && e.content.data["erased"] == true {
        if let Some(name) = e.content.data["name"].as_str().filter(|n| m.apps.get(*n).is_some_and(|a| a.erased)) {
            crate::vault::forget_app(&mut state.items, name);
        }
    }
}

/// An `item` (or earlier `secret`) that answers a request here, by an app it switches on for: the request is set.
fn answered(state: &mut ScopeState, data: &Value, id: &str, w: &str, time: u64) {
    let Some(r) = data["request"].as_str().and_then(|rid| state.requests.get_mut(rid)) else { return };
    if r.answer.is_none() && state.items.get(id).is_some_and(|i| i.apps.contains(&r.app)) {
        r.answer = Some(("set".into(), w.to_string(), time));
    }
}

// Building node entries.

pub fn node(kind: &str, data: Value) -> Content {
    Content::new(APP, kind, data)
}

/// The install's first entry: its name, its first person, the first device (signing it) and the recovery key.
pub fn genesis(name: &str, user: &str, user_name: &str, device: &crate::keys::Endpoint, device_name: &str, roles: &[Kind], recovery: &Key) -> Content {
    let device = json!({ "key": hex(&device.key), "seal": hex(&device.seal), "name": device_name, "roles": roles });
    node("genesis", json!({ "name": name, "user": user, "user_name": user_name, "device": device, "recovery": hex(recovery) }))
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

    fn setup() -> (Dev, Vec<Entry>) {
        let mut mac = Dev::new(1);
        let g = mac.write("global", ANDRE, genesis("Home", ANDRE, "Andre", &mac.endpoint(), "mac", BOTH, &[7; 32]), &[]);
        (mac, vec![g])
    }

    fn members(entries: &[Entry]) -> Members {
        let sorted = causal(entries.to_vec(), &Default::default());
        Members::replay(Some(entries[0].hash()), &sorted)
    }

    /// Writes `content` as `user` from `dev` after everything in `entries`, and keeps it there.
    fn add(entries: &mut Vec<Entry>, dev: &mut Dev, user: &str, content: Content) -> Entry {
        let e = dev.write("global", user, content, &entries.clone());
        entries.push(e.clone());
        e
    }

    #[test]
    fn only_this_installs_genesis_counts() {
        let mut other = Dev::new(2);
        let fake = other.write("global", ANDRE, genesis("Home", ANDRE, "x", &other.endpoint(), "y", BOTH, &[7; 32]), &[]);
        let (_, entries) = setup();
        let m = Members::replay(Some(entries[0].hash()), &[fake]);
        assert!(m.users.is_empty());
        let m = members(&entries);
        assert_eq!(m.users.len(), 1);
        assert_eq!(m.seal_key(&entries[0].author()), Some(Dev::new(1).endpoint().seal), "the seal key it was listed with");
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
    }

    #[test]
    fn a_removed_devices_later_entries_dont_count() {
        let (mut mac, mut entries) = setup();
        let mut laptop = Dev::new(3);
        add(&mut entries, &mut mac, ANDRE, device(&laptop.endpoint(), ANDRE, "laptop", &[Kind::Client], None));
        add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "before", "value": "1" })));
        add(&mut entries, &mut mac, ANDRE, node("remove", json!({ "key": hex(&laptop.id()) })));
        add(&mut entries, &mut laptop, ANDRE, node("setting", json!({ "name": "after", "value": "1" })));
        let m = members(&entries);
        assert!(m.settings.contains_key("before"), "what it wrote before still counts");
        assert!(!m.settings.contains_key("after"));
        assert!(m.machine(&laptop.id()).is_none());
    }

    #[test]
    fn a_new_epoch_counts_only_from_the_scopes_admins() {
        let (mut mac, mut entries) = setup();
        let mut bobs = Dev::new(3);
        add(&mut entries, &mut mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        let to = json!([hex(&mac.id())]);
        add(&mut entries, &mut bobs, BOB, node("epoch", json!({ "scope": "global", "n": 1, "to": to })));
        add(&mut entries, &mut bobs, BOB, node("epoch", json!({ "scope": format!("user:{ANDRE}"), "n": 1, "to": to })));
        assert!(members(&entries).epochs.is_empty(), "not Bob's to start");
        add(&mut entries, &mut mac, ANDRE, node("epoch", json!({ "scope": "global", "n": 2, "to": to })));
        assert_eq!(members(&entries).epoch(&Scope::Global), 0, "epochs go one at a time");
        add(&mut entries, &mut mac, ANDRE, node("epoch", json!({ "scope": "global", "n": 1, "to": to })));
        let bob = hex(&bobs.id());
        add(&mut entries, &mut bobs, BOB, node("epoch", json!({ "scope": format!("user:{BOB}"), "n": 1, "to": [bob] })));
        let m = members(&entries);
        assert_eq!(m.epochs[&Scope::Global], Epoch { n: 1, to: vec![mac.id()] });
        assert_eq!(m.epoch(&Scope::User(BOB.into())), 1, "a person's own scope is theirs");
        assert_eq!(m.epoch(&Scope::User(ANDRE.into())), 0);
    }

    #[test]
    fn a_nodes_scope_entries_count_where_it_runs_the_scopes_calls() {
        let (mut mac, mut entries) = setup();
        let mut linux = Dev::new(4);
        add(&mut entries, &mut mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        add(&mut entries, &mut mac, ANDRE, device(&Dev::new(3).endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        let m = members(&entries);
        let mine = linux.write(&format!("user:{ANDRE}"), ANDRE, node("keep", json!({})), &entries);
        let bobs = linux.write(&format!("user:{BOB}"), BOB, node("keep", json!({})), &entries);
        assert!(m.counts(&mine));
        assert!(!m.counts(&bobs), "linux doesn't run Bob's");
    }
}
