//! Members: users, groups, servers and clients, ACLs and trust, apps and deploys, made by replaying `global`'s
//! node entries; and which entries count. Pure: entries in, members out.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Key, Order, Scope, hex, is_id, unhex};

/// The node's own entries. Apps never see them.
pub const APP: &str = "node";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

/// Who may own or use a server, or own an app.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    User(String),
    Group(String),
    /// The install's admins, as an app's owner.
    Admins,
    /// The install itself, as a server's owner or user: every user.
    Install,
}

impl Principal {
    pub fn parse(s: &str) -> Result<Principal> {
        Ok(match s.split_once(':') {
            None if s == "install" => Principal::Install,
            None if s == "admins" => Principal::Admins,
            Some(("user", id)) if is_id(id) => Principal::User(id.into()),
            Some(("group", id)) if is_id(id) => Principal::Group(id.into()),
            _ => bail!("{s} isn't user:<id>, group:<id>, admins or install"),
        })
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Principal::User(id) => write!(f, "user:{id}"),
            Principal::Group(id) => write!(f, "group:{id}"),
            Principal::Admins => write!(f, "admins"),
            Principal::Install => write!(f, "install"),
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

#[derive(Clone, Debug)]
pub struct Machine {
    pub key: Key,
    pub user: String,
    pub name: String,
    /// A server runs the node; a client (a phone) only asks one.
    pub server: bool,
    pub removed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: BTreeMap<String, Role>,
    pub invited: BTreeMap<String, Role>,
}

#[derive(Clone, Debug)]
pub struct Acl {
    pub owners: Vec<Principal>,
    pub users: Vec<Principal>,
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

/// The latest `sidecar` entry for a sidecar on a server: removed there, or let register there again.
#[derive(Clone, Debug)]
pub struct SidecarInstall {
    /// Its executable, for its server's node to start *(not built yet)*.
    pub blob: Option<String>,
    /// Installed by registering over the local API, as `sidecars/install.sh` does.
    pub local: bool,
    pub by: String,
    pub time: u64,
}

impl SidecarInstall {
    /// Removed from its server: it isn't served there until it's installed again.
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
    pub acls: BTreeMap<Key, Acl>,
    /// (user, server) for each `trust` not since undone by `untrust`.
    pub trusted: BTreeSet<(String, Key)>,
    pub relays: Vec<String>,
    /// The install's settings its admins set: each name's latest value. The rest are the white label's defaults.
    pub settings: BTreeMap<String, String>,
    pub apps: BTreeMap<String, App>,
    /// The latest `sidecar` entry of each sidecar on each server: (server, sidecar).
    pub sidecars: BTreeMap<(Key, String), SidecarInstall>,
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

    pub fn machine(&self, key: &Key) -> Option<&Machine> {
        self.machines.get(key).filter(|m| !m.removed && self.user(&m.user).is_some())
    }

    pub fn server(&self, key: &Key) -> Option<&Machine> {
        self.machine(key).filter(|m| m.server)
    }

    pub fn client(&self, key: &Key) -> Option<&Machine> {
        self.machine(key).filter(|m| !m.server)
    }

    pub fn servers(&self) -> impl Iterator<Item = &Machine> {
        self.machines.values().filter(|m| m.server && self.machine(&m.key).is_some())
    }

    pub fn clients(&self) -> impl Iterator<Item = &Machine> {
        self.machines.values().filter(|m| !m.server && self.machine(&m.key).is_some())
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

    /// Whether `user` acts for `owner`: a user for themselves, a group's admins for the group, the install's
    /// admins for `admins` and `install`.
    pub fn acts_for(&self, user: &str, owner: &Principal) -> bool {
        match owner {
            Principal::User(u) => u == user && self.user(user).is_some(),
            Principal::Group(g) => self.group_role(user, g) == Some(Role::Admin),
            Principal::Admins | Principal::Install => self.is_admin(user),
        }
    }

    /// Whether `user` manages `group`: its admins, and the install's (so a group whose last admin left isn't stuck).
    pub fn manages(&self, user: &str, group: &str) -> bool {
        self.groups.contains_key(group) && (self.group_role(user, group) == Some(Role::Admin) || self.is_admin(user))
    }

    // Servers.

    pub fn owners(&self, server: &Key) -> Vec<Principal> {
        match self.acls.get(server) {
            Some(acl) => acl.owners.clone(),
            None => self.machines.get(server).map(|m| vec![Principal::User(m.user.clone())]).unwrap_or_default(),
        }
    }

    /// A server's users, its owners among them.
    pub fn users_of(&self, server: &Key) -> Vec<Principal> {
        let mut users = match self.acls.get(server) {
            Some(acl) => acl.users.clone(),
            None => self.owners(server),
        };
        for o in self.owners(server) {
            if !users.contains(&o) {
                users.push(o);
            }
        }
        users
    }

    fn includes(&self, principals: &[Principal], user: &str) -> bool {
        self.user(user).is_some()
            && principals.iter().any(|p| match p {
                Principal::User(u) => u == user,
                Principal::Group(g) => self.group_role(user, g).is_some(),
                Principal::Install => true,
                Principal::Admins => self.is_admin(user),
            })
    }

    /// Owned by `install`: holds everything and serves everyone in its users.
    pub fn privileged(&self, server: &Key) -> bool {
        self.server(server).is_some() && self.owners(server).contains(&Principal::Install)
    }

    /// Whether `user` trusts `server`: everyone trusts a privileged server; otherwise a user of it trusts it if they're
    /// directly one of its owners or wrote `trust`.
    pub fn trusts(&self, user: &str, server: &Key) -> bool {
        if self.server(server).is_none() || self.user(user).is_none() {
            return false;
        }
        self.privileged(server)
            || (self.includes(&self.users_of(server), user)
                && (self.owners(server).contains(&Principal::User(user.into())) || self.trusted.contains(&(user.to_string(), *server))))
    }

    /// Whether `server` runs calls as `user`: a server that user trusts and is a user of.
    pub fn serves(&self, server: &Key, user: &str) -> bool {
        self.trusts(user, server) && self.includes(&self.users_of(server), user)
    }

    /// Whether `server` holds `scope`: `global`, every scope its owners can read, and the scope of each user who trusts it.
    pub fn holds(&self, server: &Key, scope: &Scope) -> bool {
        if self.server(server).is_none() {
            return false;
        }
        let reads = |owner: &Principal| match (owner, scope) {
            (_, Scope::Global) | (Principal::Install, _) => true,
            (Principal::User(u), Scope::User(v)) => u == v,
            (Principal::User(u), Scope::Group(g)) => self.group_role(u, g).is_some(),
            (Principal::Group(g), Scope::Group(h)) => g == h,
            _ => false,
        };
        self.owners(server).iter().any(reads) || matches!(scope, Scope::User(u) if self.trusts(u, server))
    }

    /// Every scope there is: `global`, each user's (removed ones too: removing deletes nothing) and each group's.
    pub fn scopes(&self) -> Vec<Scope> {
        let mut out = vec![Scope::Global];
        out.extend(self.groups.keys().map(|g| Scope::Group(g.clone())));
        out.extend(self.users.keys().map(|u| Scope::User(u.clone())));
        out
    }

    /// Whether an entry counts at this point in replay, by the rules every entry follows. A node entry's own kind
    /// rules are checked as it's applied (`apply`), or, for one in a user or group scope, by `counts_in_scope`.
    pub fn counts(&self, e: &Entry) -> bool {
        let scope = e.scope();
        let author = e.author();
        if !self.holds(&author, &scope) || !self.trusts(e.user(), &author) {
            return false;
        }
        if !self.may_use(e.user(), &scope) {
            return false;
        }
        match e.by() {
            None => true,
            Some(by) => self.client(&by).is_some_and(|c| c.user == e.user()) || self.trusts(e.user(), &by),
        }
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
                self.name = text("name")?;
                self.recovery = data["recovery"].as_str().and_then(|k| unhex(k).ok());
                self.users.insert(user.clone(), User { id: user.clone(), name: text("user_name")?, role: Role::Admin, removed: false });
                self.machines.insert(author, Machine { key: author, user, name: text("server_name")?, server: true, removed: false });
            }
            "admit" => {
                let key = key_of("key")?;
                let user = id_of("user")?;
                let server = match text("as")?.as_str() {
                    "server" => true,
                    "client" => false,
                    other => bail!("can't admit as {other}"),
                };
                ensure!(!self.machines.contains_key(&key), "that key was already admitted");
                match self.users.get(&user) {
                    Some(u) => ensure!(!u.removed && (w == user || self.is_admin(&w)), "only a user or an admin admits that user's machines"),
                    None => {
                        ensure!(self.is_admin(&w), "only admins add users");
                        let role = role_of("role")?;
                        self.users.insert(user.clone(), User { id: user.clone(), name: text("user_name")?, role, removed: false });
                    }
                }
                self.machines.insert(key, Machine { key, user, name: text("name")?, server, removed: false });
            }
            "acl" => {
                let server = key_of("server")?;
                ensure!(self.server(&server).is_some(), "no such server");
                ensure!(self.owners(&server).iter().any(|o| self.acts_for(&w, o)), "only someone acting for an owner changes a server's ACL");
                let list = |key: &str| -> Result<Vec<Principal>> {
                    data[key].as_array().context("no list")?.iter().map(|p| Principal::parse(p.as_str().unwrap_or(""))).collect()
                };
                let (owners, users) = (list("owners")?, list("users")?);
                ensure!(!owners.is_empty(), "a server needs an owner");
                ensure!(!owners.contains(&Principal::Admins) && !users.contains(&Principal::Admins), "servers are owned by users, groups or the install");
                ensure!(!owners.contains(&Principal::Install) || self.is_admin(&w), "only admins make the install an owner");
                self.acls.insert(server, Acl { owners, users });
            }
            "trust" => {
                let server = key_of("server")?;
                ensure!(self.server(&server).is_some(), "no such server");
                self.trusted.insert((w, server));
            }
            "untrust" => {
                let server = key_of("server")?;
                ensure!(!self.privileged(&server), "nobody untrusts a privileged server");
                self.trusted.remove(&(w, server));
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
                    let m = self.machine(&key).context("no such key")?;
                    let allowed = self.is_admin(&w)
                        || (m.server && self.owners(&key).iter().any(|o| self.acts_for(&w, o)))
                        || (!m.server && m.user == w);
                    ensure!(allowed, "not allowed to remove that key");
                    self.machines.get_mut(&key).unwrap().removed = true;
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
                    let m = self.machine(&key).context("no such key")?;
                    ensure!(m.user == w || self.is_admin(&w), "not allowed to rename that");
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
            "relays" => {
                ensure!(self.is_admin(&w), "admins only");
                self.relays = data["urls"].as_array().context("no urls")?.iter().filter_map(|u| u.as_str().map(str::to_string)).collect();
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
                ensure!(!matches!(owner, Principal::Install), "apps are owned by users, groups or admins");
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
            // `driver`: what `sidecar` was called first.
            "sidecar" | "driver" => {
                let server = key_of("server")?;
                let name = text("name")?;
                ensure!(valid_app_name(&name), "sidecar names are lowercase words joined by -");
                ensure!(self.server(&server).is_some(), "no such server");
                ensure!(self.owners(&server).iter().any(|o| self.acts_for(&w, o)), "only someone acting for one of a server's owners installs or removes its sidecars");
                let blob = data["blob"].as_str().map(str::to_string);
                ensure!(blob.as_ref().is_none_or(|b| e.content.blobs.contains(b)), "a sidecar names its executable among its blobs");
                self.sidecars.insert((server, name), SidecarInstall { blob, local: data["local"] == true, by: w, time: e.time() });
            }
            "secret" => ensure!(self.is_admin(&w), "admins only"),
            "install" | "uninstall" | "node-release" => {}
            other => bail!("unknown kind {other}"),
        }
        Ok(())
    }
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
        "secret" => {
            // An earlier server's: the item of its own it is.
            ensure!(manages_scope(m, w, &scope), "not allowed to change the Vault there");
            let data = &e.content.data;
            ensure!(data["app"].as_str().is_some_and(|a| m.apps.get(a).is_some_and(|a| !a.erased)), "no such app");
            let id = crate::vault::apply_legacy(&mut state.items, data, w, e.time())?;
            answered(state, data, &id, w, e.time());
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
            // An item the node itself uses (a backup place's) is asked for as the node's.
            ensure!(m.apps.contains_key(&app) || app == APP, "no such app");
            // An earlier server's asks for one secret field, `name`.
            let (title, fields) = match data["name"].as_str() {
                Some(name) => (name.to_string(), vec![(name.to_string(), true)]),
                None => {
                    let fields: Vec<(String, bool)> = data["fields"].as_object().context("no fields")?.iter().map(|(n, f)| (n.clone(), f["secret"] == true)).collect();
                    (data["title"].as_str().unwrap_or("").trim().to_string(), fields)
                }
            };
            ensure!(!title.is_empty() && !fields.is_empty() && fields.iter().all(|(n, _)| valid_secret_name(n)), "a request asks for named fields, with a title");
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
            state.arrangements.insert(page.to_string(), json!({ "items": items, "hidden": hidden, "time": e.time() }));
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
            let counted = m.step(e) && (!matches!(e.content.kind.as_str(), "install" | "uninstall" | "secret") || apply_in_scope(&m, &mut state, e).is_ok());
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

/// The members as they were just before `order`.
pub fn at(root: Option<Hash>, member_entries: &[Entry], order: &Order) -> Members {
    let mut m = Members::new(root);
    for e in member_entries.iter().take_while(|e| &e.order() < order) {
        m.step(e);
    }
    m
}

// Building node entries.

pub fn node(kind: &str, data: Value) -> Content {
    Content::new(APP, kind, data)
}

pub fn genesis(name: &str, user: &str, user_name: &str, server_name: &str, recovery: &Key) -> Content {
    node("genesis", json!({ "name": name, "user": user, "user_name": user_name, "server_name": server_name, "recovery": hex(recovery) }))
}

pub fn admit(key: &Key, user: &str, name: &str, server: bool, new_user: Option<(&str, Role)>) -> Content {
    let mut data = json!({ "key": hex(key), "user": user, "name": name, "as": if server { "server" } else { "client" } });
    if let Some((user_name, role)) = new_user {
        data["user_name"] = json!(user_name);
        data["role"] = json!(role);
    }
    node("admit", data)
}

pub fn acl(server: &Key, owners: &[Principal], users: &[Principal]) -> Content {
    let list = |ps: &[Principal]| ps.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    node("acl", json!({ "server": hex(server), "owners": list(owners), "users": list(users) }))
}

pub fn trust(server: &Key, trust: bool) -> Content {
    node(if trust { "trust" } else { "untrust" }, json!({ "server": hex(server) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Ext;
    use p2panda_core::SigningKey;

    struct Srv {
        key: SigningKey,
        last: BTreeMap<String, crate::entry::Head>,
        time: u64,
    }

    impl Srv {
        fn new(seed: u8) -> Self {
            Srv { key: SigningKey::from([seed; 32]), last: BTreeMap::new(), time: 0 }
        }
        fn id(&self) -> Key {
            *self.key.verifying_key().as_bytes()
        }
        fn write(&mut self, log: &str, user: &str, content: Content) -> Entry {
            self.time += 10;
            let ext = Ext { log: log.into(), time: self.time, user: user.into(), by: None };
            let e = Entry::create(&self.key, ext, self.last.get(log).copied(), content);
            self.last.insert(log.into(), e.head());
            e
        }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";

    fn setup() -> (Srv, Vec<Entry>) {
        let mut s = Srv::new(1);
        let g = s.write("global", ANDRE, genesis("Home", ANDRE, "Andre", "mac", &[7; 32]));
        (s, vec![g])
    }

    fn members(entries: &[Entry]) -> Members {
        let mut sorted = entries.to_vec();
        sorted.sort_by_key(Entry::order);
        Members::replay(Some(entries[0].hash()), &sorted)
    }

    #[test]
    fn only_this_installs_genesis_counts() {
        let mut other = Srv::new(2);
        let fake = other.write("global", ANDRE, genesis("Home", ANDRE, "x", "y", &[7; 32]));
        let (_, entries) = setup();
        let m = Members::replay(Some(entries[0].hash()), &[fake]);
        assert!(m.users.is_empty());
        assert_eq!(members(&entries).users.len(), 1);
    }

    #[test]
    fn an_entry_counts_once_its_authors_admit_arrives_and_not_after_removal() {
        let (mut mac, mut entries) = setup();
        let mut bobs = Srv::new(3);
        bobs.time = 100;
        let early = bobs.write("global", BOB, Content::new("board", "post", json!({})));
        let root = Some(entries[0].hash());
        let count = |members: &[Entry], e: &Entry| {
            let mut sorted = members.to_vec();
            sorted.sort_by_key(Entry::order);
            let mut n = 0;
            replay(root, &sorted, std::slice::from_ref(e), |_, _, _| n += 1);
            n
        };
        assert_eq!(count(&entries, &early), 0);
        entries.push(mac.write("global", ANDRE, admit(&bobs.id(), BOB, "bob's", true, Some(("Bob", Role::Member)))));
        assert_eq!(count(&entries, &early), 1, "its admit sorts before it");
        mac.time = 500;
        entries.push(mac.write("global", ANDRE, node("remove", json!({ "key": hex(&bobs.id()) }))));
        bobs.time = 600;
        let late = bobs.write("global", BOB, Content::new("board", "post", json!({})));
        assert_eq!(count(&entries, &early), 1, "what it wrote before removal still counts");
        assert_eq!(count(&entries, &late), 0);
    }
}
