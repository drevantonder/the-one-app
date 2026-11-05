//! The scope database: who and what a scope has (people, groups, devices, who holds which role, epochs, keeps, apps,
//! deploys, installs, services, settings, the Vault's bookkeeping, recovery) as SQLite rows. Until the log is frozen
//! it's built, never written: `build` makes it from what replay made (`Members`, and the scope's `ScopeState`), so
//! replay stays the truth and this is its projection, the same on every reader. It's small, so it's built again as
//! entries arrive.
//!
//! Every scope's database has the same tables, `permissions`' among them with its defaults. What `global`'s entries
//! say of the whole install (people, groups, invites, devices, relations, epochs and keeps of every scope, apps,
//! services, settings, Vault keys, recovery) is filled in only in `global`'s, which everyone in the install reads; a
//! person's or group's has only its own state (installs, schedules, Vault requests, pages, forgotten blobs), which
//! only those who use it may read.
//!
//! Who may do what is `permissions::may` over `relations`, whose objects are `object(scope)`, `device(key)`,
//! `service(key)` and `app(name)`. Whether an object exists (a device still listed, a group made) is the tables' to
//! say, not `may`'s: the install's admins may remove every device, listed or not.
//!
//! People and groups are named as `person:<id>` and `group:<id>`; an app owned by the install's admins as `admins`.
//! Keys and hashes are lowercase hex, times milliseconds, flags 0 or 1, and lists and arrangements JSON.
//!
//! Its state hash is any database's (`db::state_hash`): its rows', whatever order they were written in, and whatever
//! SQLite's pages hold.

use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::json;

use crate::entry::{Key, Scope, hex};
use crate::members::{Kind, Members, Principal, Role, ScopeState};
use crate::permissions;

/// The tables, the same in every scope's database, besides the permissions' (`permissions::SCHEMA`). STRICT, so a
/// value is stored as the type it was given, and every table has a primary key, which changesets need.
pub const SCHEMA: &str = "
CREATE TABLE install (
  id INTEGER PRIMARY KEY CHECK (id = 0),
  root TEXT,
  name TEXT NOT NULL,
  devices_at INTEGER NOT NULL,
  recovery_used_at INTEGER,
  recovery_used_by TEXT
) STRICT;
CREATE TABLE principals (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('person', 'group')),
  name TEXT NOT NULL,
  handle TEXT NOT NULL,
  removed INTEGER NOT NULL
) STRICT;
CREATE TABLE invites (
  principal TEXT NOT NULL,
  invited_to TEXT NOT NULL,
  role TEXT NOT NULL,
  PRIMARY KEY (principal, invited_to)
) STRICT;
CREATE TABLE devices (
  key TEXT PRIMARY KEY,
  seal TEXT NOT NULL,
  principal TEXT NOT NULL,
  name TEXT NOT NULL,
  client INTEGER NOT NULL,
  compute INTEGER NOT NULL,
  service TEXT,
  shared INTEGER NOT NULL,
  passkey TEXT,
  device TEXT,
  listed_by TEXT NOT NULL,
  listed_at INTEGER NOT NULL,
  -- What it signed in with, as JSON (`sign_in_proof::Account`), when that was checked as it was let in.
  sign_in TEXT,
  place TEXT,
  removed INTEGER NOT NULL
) STRICT;
CREATE TABLE runs (
  principal TEXT NOT NULL,
  device TEXT NOT NULL,
  PRIMARY KEY (principal, device)
) STRICT;
-- Each person whose own device said where their things run, nowhere besides their own machines among them.
CREATE TABLE runs_chosen (
  principal TEXT PRIMARY KEY
) STRICT;
CREATE TABLE epochs (
  scope TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  key_hash TEXT NOT NULL,
  PRIMARY KEY (scope, epoch)
) STRICT;
CREATE TABLE sealed (
  scope TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  device TEXT NOT NULL,
  PRIMARY KEY (scope, epoch, device)
) STRICT;
CREATE TABLE owed (
  scope TEXT PRIMARY KEY
) STRICT;
CREATE TABLE keeps (
  device TEXT NOT NULL,
  scope TEXT NOT NULL,
  app TEXT NOT NULL,
  principal TEXT NOT NULL,
  until INTEGER NOT NULL,
  PRIMARY KEY (device, scope, app)
) STRICT;
CREATE TABLE keep_keys (
  device TEXT NOT NULL,
  scope TEXT NOT NULL,
  app TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  key_hash TEXT NOT NULL,
  PRIMARY KEY (device, scope, app, epoch)
) STRICT;
CREATE TABLE apps (
  name TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  forked_from TEXT,
  bundle TEXT,
  deploys INTEGER NOT NULL,
  deleted_at INTEGER,
  deleted_by TEXT,
  erased INTEGER NOT NULL,
  source TEXT
) STRICT;
CREATE TABLE deploys (
  app TEXT NOT NULL,
  n INTEGER NOT NULL,
  bundle TEXT NOT NULL,
  deployed_at INTEGER NOT NULL,
  deployed_by TEXT NOT NULL,
  PRIMARY KEY (app, n)
) STRICT;
CREATE TABLE services (
  device TEXT NOT NULL,
  name TEXT NOT NULL,
  switched_on INTEGER NOT NULL,
  switched_by TEXT NOT NULL,
  switched_at INTEGER NOT NULL,
  PRIMARY KEY (device, name)
) STRICT;
CREATE TABLE settings (
  name TEXT PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;
CREATE TABLE vault (
  scope TEXT NOT NULL,
  n INTEGER NOT NULL,
  public TEXT NOT NULL,
  PRIMARY KEY (scope, n)
) STRICT;
CREATE TABLE vault_opens (
  scope TEXT NOT NULL,
  device TEXT NOT NULL,
  PRIMARY KEY (scope, device)
) STRICT;
CREATE TABLE recovery (
  n INTEGER PRIMARY KEY,
  key TEXT NOT NULL,
  set_at INTEGER
) STRICT;
CREATE TABLE installs (
  app TEXT PRIMARY KEY,
  installed INTEGER NOT NULL,
  installed_by TEXT NOT NULL,
  installed_at INTEGER NOT NULL
) STRICT;
CREATE TABLE schedules (
  app TEXT NOT NULL,
  schedule TEXT NOT NULL,
  switched_on INTEGER NOT NULL,
  switched_by TEXT NOT NULL,
  switched_at INTEGER NOT NULL,
  PRIMARY KEY (app, schedule)
) STRICT;
CREATE TABLE vault_requests (
  id TEXT PRIMARY KEY,
  app TEXT NOT NULL,
  title TEXT NOT NULL,
  fields TEXT NOT NULL,
  hosts TEXT NOT NULL,
  why TEXT NOT NULL,
  need TEXT,
  asked_by TEXT NOT NULL,
  agent TEXT,
  asked_at INTEGER NOT NULL,
  answer TEXT,
  answered_by TEXT,
  answered_at INTEGER
) STRICT;
CREATE TABLE pages (
  page TEXT PRIMARY KEY,
  arrangement TEXT NOT NULL
) STRICT;
CREATE TABLE forgotten (
  blob TEXT PRIMARY KEY
) STRICT;
-- Once the log is frozen, what its steps name that no other table says (`steps::apply`): each blob the scope keeps,
-- and whether a node entry other than a keep named it (a deploy's bundle), which forgetting it doesn't drop.
CREATE TABLE blobs (
  blob TEXT PRIMARY KEY,
  pinned INTEGER NOT NULL
) STRICT;
-- And each release of the node's and services' programs a step said, its whole content (JSON), at its position.
CREATE TABLE releases (
  at INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  content TEXT NOT NULL,
  author TEXT NOT NULL,
  time INTEGER NOT NULL
) STRICT;
";

/// A person, as the scope database names them.
pub fn person(id: &str) -> String {
    format!("person:{id}")
}

/// A group, as the scope database names it.
pub fn group(id: &str) -> String {
    format!("group:{id}")
}

/// A device, as an object or subject of permissions.
pub fn device(key: &Key) -> String {
    format!("device:{}", hex(key))
}

/// A service endpoint, as an object of permissions.
pub fn service(key: &Key) -> String {
    format!("service:{}", hex(key))
}

/// An app, as an object of permissions.
pub fn app(name: &str) -> String {
    format!("app:{name}")
}

/// A scope, as an object of permissions: the install, a group, or a person's own.
pub fn object(scope: &Scope) -> String {
    match scope {
        Scope::Global => permissions::INSTALL.into(),
        Scope::Group(g) => group(g),
        Scope::User(u) => person(u),
    }
}

/// A role in the install or a group, as `permissions` names it.
pub fn role(r: Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

/// An app's owner, as the scope database names it.
pub fn owner(p: &Principal) -> String {
    match p {
        Principal::User(id) => person(id),
        Principal::Group(id) => group(id),
        Principal::Admins => "admins".into(),
    }
}

/// A time or number from an entry, as SQLite keeps it: one past what it holds is kept as its most.
fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// A scope's database, as replay says it is: `members` from `global`'s entries, and `state` from the scope's own.
/// What `global`'s entries say of the whole install is filled in only for `global`.
pub fn build(members: &Members, scope: &Scope, state: &ScopeState) -> Result<Connection> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(SCHEMA)?;
    db.execute_batch(permissions::SCHEMA)?;
    let tx = db.unchecked_transaction()?;
    permissions::defaults(&tx)?;
    if *scope == Scope::Global {
        install(&tx, members)?;
        relations(&tx, members)?;
    }
    own(&tx, state)?;
    tx.commit()?;
    permissions::close(&db, None)?;
    Ok(db)
}

/// What `global`'s entries say of the whole install.
fn install(db: &Connection, m: &Members) -> Result<()> {
    let used = m.recovery_used.as_ref();
    db.execute(
        "INSERT INTO install VALUES (0, ?, ?, ?, ?, ?)",
        params![m.root.map(|r| r.to_hex()), m.name, int(m.devices_at), used.map(|(t, _)| int(*t)), used.map(|(_, u)| person(u))],
    )?;
    for u in m.users.values() {
        db.execute("INSERT INTO principals VALUES (?, 'person', ?, ?, ?)", params![person(&u.id), u.name, u.handle, u.removed])?;
    }
    for g in m.groups.values() {
        db.execute("INSERT INTO principals VALUES (?, 'group', ?, ?, 0)", params![group(&g.id), g.name, g.handle])?;
        for (user, r) in g.invited.iter().filter(|(u, _)| m.user(u).is_some()) {
            db.execute("INSERT INTO invites VALUES (?, ?, ?)", params![person(user), group(&g.id), role(*r)])?;
        }
    }
    for d in m.machines.values() {
        db.execute(
            "INSERT INTO devices VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                hex(&d.key),
                hex(&d.seal),
                person(&d.user),
                d.name,
                d.is(Kind::Client),
                d.is(Kind::Compute),
                d.service,
                d.shared,
                d.passkey,
                d.device.as_ref().map(hex),
                person(&d.by),
                int(d.since),
                d.sign_in.as_ref().map(|a| json!(a).to_string()),
                d.place,
                d.removed
            ],
        )?;
    }
    for (user, on) in &m.runs {
        db.execute("INSERT INTO runs_chosen VALUES (?)", params![person(user)])?;
        for key in on {
            db.execute("INSERT INTO runs VALUES (?, ?)", params![person(user), hex(key)])?;
        }
    }
    for ((scope, n), hash) in &m.epoch_hashes {
        db.execute("INSERT INTO epochs VALUES (?, ?, ?)", params![scope.to_string(), n, hash])?;
    }
    for (scope, e) in &m.epochs {
        for key in &e.to {
            db.execute("INSERT OR IGNORE INTO sealed VALUES (?, ?, ?)", params![scope.to_string(), e.n, hex(key)])?;
        }
    }
    for scope in &m.owed {
        db.execute("INSERT INTO owed VALUES (?)", params![scope.to_string()])?;
    }
    for ((key, scope, app), g) in &m.grants {
        db.execute("INSERT INTO keeps VALUES (?, ?, ?, ?, ?)", params![hex(key), scope.to_string(), app, person(&g.by), int(g.until)])?;
        for (n, hash) in &g.hashes {
            db.execute("INSERT INTO keep_keys VALUES (?, ?, ?, ?, ?)", params![hex(key), scope.to_string(), app, n, hash])?;
        }
    }
    for a in m.apps.values() {
        let deleted = a.deleted.as_ref();
        db.execute(
            "INSERT INTO apps VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![a.name, owner(&a.owner), a.from, a.bundle, a.deploys, deleted.map(|(t, _)| int(*t)), deleted.map(|(_, u)| person(u)), a.erased, a.source],
        )?;
        for (n, (bundle, time, user)) in a.history.iter().enumerate() {
            db.execute("INSERT INTO deploys VALUES (?, ?, ?, ?, ?)", params![a.name, int(n as u64 + 1), bundle, int(*time), person(user)])?;
        }
    }
    for ((key, name), s) in &m.services {
        db.execute("INSERT INTO services VALUES (?, ?, ?, ?, ?)", params![hex(key), name, s.on, person(&s.by), int(s.time)])?;
    }
    for (name, value) in &m.settings {
        db.execute("INSERT INTO settings VALUES (?, ?)", params![name, value])?;
    }
    for ((scope, n), public) in &m.vault_publics {
        db.execute("INSERT INTO vault VALUES (?, ?, ?)", params![scope.to_string(), n, hex(public)])?;
    }
    for (scope, v) in &m.vaults {
        for key in &v.to {
            db.execute("INSERT OR IGNORE INTO vault_opens VALUES (?, ?)", params![scope.to_string(), hex(key)])?;
        }
    }
    // The recovery keys in the order they were set: the last is the install's, and only its time is known.
    let keys: Vec<&Key> = m.recovery_was.iter().chain(&m.recovery).collect();
    for (n, key) in (0i64..).zip(&keys) {
        let current = n as usize + 1 == keys.len() && m.recovery.is_some();
        db.execute("INSERT INTO recovery VALUES (?, ?, ?)", params![n, hex(key), current.then(|| int(m.recovery_set))])?;
    }
    Ok(())
}

/// Who holds which role on what, in `permissions`' words, as replay says now: only people still in, and devices
/// still listed, hold any. Which devices run a scope's calls is a `runner` row for each, as `Members::runs_for`
/// says: for a group or `global` that follows from its admins, so it's worked out here rather than kept as a rule.
/// The Cloudflare node, which an admin's setting names, is the install's admins' device.
fn relations(db: &Connection, m: &Members) -> Result<()> {
    let add = |subject: &str, relation: &str, object: &str| db.execute("INSERT OR IGNORE INTO relations VALUES (?, ?, ?, NULL)", params![subject, relation, object]);
    for u in m.users.values().filter(|u| !u.removed) {
        add(&person(&u.id), role(u.role), permissions::INSTALL)?;
        add(&person(&u.id), "owner", &person(&u.id))?;
    }
    for g in m.groups.values() {
        for (user, r) in g.members.iter().filter(|(u, _)| m.user(u).is_some()) {
            add(&person(user), role(*r), &group(&g.id))?;
        }
    }
    for d in m.machines.values().filter(|d| m.machine(&d.key).is_some()) {
        add(&person(&d.user), "owner", &device(&d.key))?;
        if m.service(&d.key).is_some() {
            let users = if d.shared { format!("{}#member", permissions::INSTALL) } else { person(&d.user) };
            add(&users, "user", &service(&d.key))?;
        }
    }
    if let Some(key) = m.cloudflare_node() {
        add(&format!("{}#admin", permissions::INSTALL), "admin", &device(&key))?;
    }
    let runners = m.computes().map(|c| c.key).chain(m.cloudflare_node());
    for key in runners {
        for scope in m.scopes().iter().filter(|s| m.runs_for(&key, s)) {
            add(&device(&key), "runner", &object(scope))?;
        }
    }
    for a in m.apps.values() {
        let owners = match &a.owner {
            Principal::User(u) if m.user(u).is_some() => person(u),
            Principal::User(_) => continue,
            Principal::Group(g) => format!("{}#admin", group(g)),
            Principal::Admins => format!("{}#admin", permissions::INSTALL),
        };
        add(&owners, "owner", &app(&a.name))?;
    }
    Ok(())
}

/// What a scope's own entries say of it.
fn own(db: &Connection, s: &ScopeState) -> Result<()> {
    for (app, (user, time)) in &s.installers {
        db.execute("INSERT INTO installs VALUES (?, ?, ?, ?)", params![app, s.installed.contains(app), person(user), int(*time)])?;
    }
    for ((app, schedule), (on, user, time)) in &s.switches {
        db.execute("INSERT INTO schedules VALUES (?, ?, ?, ?, ?)", params![app, schedule, on, person(user), int(*time)])?;
    }
    for (id, r) in &s.requests {
        let fields = json!(r.fields.iter().map(|(name, secret)| json!({ "name": name, "secret": secret })).collect::<Vec<_>>());
        let answer = r.answer.as_ref();
        db.execute(
            "INSERT INTO vault_requests VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                r.app,
                r.title,
                fields.to_string(),
                json!(r.hosts).to_string(),
                r.why,
                r.need,
                person(&r.user),
                r.agent,
                int(r.time),
                answer.map(|(how, _, _)| how),
                answer.map(|(_, u, _)| person(u)),
                answer.map(|(_, _, t)| int(*t))
            ],
        )?;
    }
    for (page, arrangement) in &s.arrangements {
        db.execute("INSERT INTO pages VALUES (?, ?)", params![page, arrangement.to_string()])?;
    }
    for blob in &s.forgotten {
        db.execute("INSERT INTO forgotten VALUES (?)", params![blob])?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::members::manages_scope;

    /// That `may` over `global`'s scope database says what replay's `members` says, of everyone, every scope, device,
    /// service and app; and that its state hash is the same however its rows were written.
    pub(crate) fn agrees(m: &Members) {
        let db = build(m, &Scope::Global, &ScopeState::default()).unwrap();
        let may = |who: &str, what: &str, on: &str| permissions::may(&db, who, what, on, 0).unwrap();
        let scopes = m.scopes();
        for u in m.users.values().filter(|u| !u.removed).map(|u| u.id.as_str()) {
            let p = person(u);
            assert_eq!(may(&p, "change-settings", permissions::INSTALL), m.is_admin(u), "{u} is an admin");
            for s in &scopes {
                let o = object(s);
                assert_eq!(may(&p, "use", &o), m.may_use(u, s), "{u} uses {s}");
                assert_eq!(may(&p, "keep", &o), m.grants_in(u, s), "{u} keeps apps running in {s}");
                assert_eq!(may(&p, "add-apps", &o), manages_scope(m, u, s), "{u} adds apps to {s}");
            }
            for g in m.groups.keys() {
                assert_eq!(may(&p, "invite", &group(g)), m.manages(u, g), "{u} manages {g}");
                assert_eq!(may(&p, "remove", &group(g)), m.manages(u, g), "{u} removes from {g}");
            }
            for k in m.machines.keys() {
                assert_eq!(may(&p, "remove", &device(k)), m.looks_after(u, k), "{u} looks after {}", hex(k));
                assert_eq!(may(&p, "switch", &device(k)), m.machine(k).is_some_and(|d| d.user == u), "{u} switches {}'s parts", hex(k));
                assert_eq!(may(&p, "use", &service(k)), m.service_serves(k, u), "{} serves {u}", hex(k));
            }
            for a in m.apps.values() {
                assert_eq!(may(&p, "deploy", &app(&a.name)), m.acts_for(u, &a.owner), "{u} acts for {}", a.name);
            }
        }
        let rows = |sql: &str| -> Vec<String> { db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect() };
        let listed = rows("SELECT d.key FROM devices d JOIN principals p ON p.id = d.principal WHERE d.removed = 0 AND p.removed = 0 ORDER BY d.key");
        assert_eq!(listed, m.machines.keys().filter(|k| m.machine(k).is_some()).map(hex).collect::<Vec<_>>(), "the devices listed");
        let chosen = rows("SELECT principal FROM runs_chosen ORDER BY principal");
        assert_eq!(chosen, m.runs.keys().map(|u| person(u)).collect::<Vec<_>>(), "who said where their things run, even nowhere else");
        let clients = rows("SELECT d.key FROM devices d JOIN principals p ON p.id = d.principal WHERE d.removed = 0 AND p.removed = 0 AND d.client = 1 ORDER BY d.key");
        assert_eq!(clients, m.clients().map(|c| hex(&c.key)).collect::<Vec<_>>(), "the clients, which sign for their people");
        for k in m.machines.keys().chain(m.cloudflare_node().as_ref()) {
            for s in &scopes {
                assert_eq!(may(&device(k), "run", &object(s)), m.runs_for(k, s), "{} runs {s}", hex(k));
            }
        }
        // The same rows written the other way round hash the same, and building again gives the same database.
        let state_hash = crate::db::state_hash;
        let hash = state_hash(&db).unwrap();
        assert_eq!(hash, state_hash(&build(m, &Scope::Global, &ScopeState::default()).unwrap()).unwrap());
        let copy = Connection::open_in_memory().unwrap();
        copy.execute_batch(SCHEMA).unwrap();
        copy.execute_batch(permissions::SCHEMA).unwrap();
        for t in rows("SELECT name FROM sqlite_schema WHERE type = 'table'") {
            let mut rows = db.prepare(&format!("SELECT * FROM \"{t}\" ORDER BY rowid DESC")).unwrap();
            let n = rows.column_count();
            let insert = format!("INSERT INTO \"{t}\" VALUES ({})", vec!["?"; n].join(", "));
            let mut rows = rows.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i).unwrap()).collect();
                copy.execute(&insert, rusqlite::params_from_iter(values)).unwrap();
            }
        }
        assert_eq!(state_hash(&copy).unwrap(), hash, "the state hash is the rows', not the order they came in");
        super::super::scopes::tests::reads(m);
    }
}
