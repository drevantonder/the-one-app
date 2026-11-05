//! The scope database: who and what a scope has (people, groups, devices, epochs, keeps, apps, deploys, installs,
//! services, settings, the Vault's bookkeeping, recovery) as SQLite rows. Until the log is frozen it's built, never
//! written: `build` makes it from what replay made (`Members`, and the scope's `ScopeState`), so replay stays the
//! truth and this is its projection, the same on every reader. It's small, so it's built again as entries arrive.
//!
//! Every scope's database has the same tables. What `global`'s entries say of the whole install (people, groups,
//! devices, epochs and keeps of every scope, apps, services, settings, Vault keys, recovery) is filled in only in
//! `global`'s, which everyone in the install reads; a person's or group's has only its own state (installs,
//! schedules, Vault requests, pages, forgotten blobs), which only those who use it may read.
//!
//! People and groups are named as `person:<id>` and `group:<id>`; an app owned by the install's admins as `admins`.
//! Keys and hashes are lowercase hex, times milliseconds, flags 0 or 1, and lists and arrangements JSON.
//!
//! `state_hash` is the hash of its rows, whatever order they were written in, and whatever SQLite's pages hold.

use anyhow::Result;
use rusqlite::{Connection, params, types::ValueRef};
use serde_json::json;

use crate::entry::{Key, Scope, hex};
use crate::members::{Kind, Members, Principal, ScopeState};

/// The tables, the same in every scope's database. STRICT, so a value is stored as the type it was given, and every
/// table has a primary key, which changesets need.
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
  removed INTEGER NOT NULL
) STRICT;
CREATE TABLE runs (
  principal TEXT NOT NULL,
  device TEXT NOT NULL,
  PRIMARY KEY (principal, device)
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
";

/// A person, as the scope database names them.
pub fn person(id: &str) -> String {
    format!("person:{id}")
}

/// A group, as the scope database names it.
pub fn group(id: &str) -> String {
    format!("group:{id}")
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
    let tx = db.unchecked_transaction()?;
    if *scope == Scope::Global {
        install(&tx, members)?;
    }
    own(&tx, state)?;
    tx.commit()?;
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
    }
    for d in m.machines.values() {
        db.execute(
            "INSERT INTO devices VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                d.removed
            ],
        )?;
    }
    for (user, on) in &m.runs {
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

/// What the hash of a scope database's state is derived for.
const STATE: &str = "one/scope-database/state";

/// The hash of a database's state: every table's definition and rows, each table's rows in the order of their values,
/// so it's the same however they got there. Each value is its type (0 null, 1 integer, 2 real, 3 text, 4 blob) and
/// then the integer's 8 bytes little-endian, the real's bits so, or the text's or blob's length so and its bytes.
pub fn state_hash(db: &Connection) -> Result<[u8; 32]> {
    fn bytes(h: &mut blake3::Hasher, b: &[u8]) {
        h.update(&(b.len() as u64).to_le_bytes());
        h.update(b);
    }
    let mut h = blake3::Hasher::new_derive_key(STATE);
    let tables: Vec<(String, String)> = db
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (name, sql) in tables {
        bytes(&mut h, name.as_bytes());
        bytes(&mut h, sql.as_bytes());
        let columns: i64 = db.query_row("SELECT count(*) FROM pragma_table_info(?)", [&name], |r| r.get(0))?;
        let order: Vec<String> = (1..=columns).map(|i| i.to_string()).collect();
        let mut rows = db.prepare(&format!("SELECT * FROM \"{name}\" ORDER BY {}", order.join(", ")))?;
        let mut rows = rows.query([])?;
        while let Some(row) = rows.next()? {
            for i in 0..columns as usize {
                match row.get_ref(i)? {
                    ValueRef::Null => {
                        h.update(&[0]);
                    }
                    ValueRef::Integer(n) => {
                        h.update(&[1]).update(&n.to_le_bytes());
                    }
                    ValueRef::Real(x) => {
                        h.update(&[2]).update(&x.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(t) => bytes(h.update(&[3]), t),
                    ValueRef::Blob(b) => bytes(h.update(&[4]), b),
                }
            }
        }
    }
    Ok(*h.finalize().as_bytes())
}
