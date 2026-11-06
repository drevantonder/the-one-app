//! The scope database: who and what a scope has (people, groups, devices, who holds which role, epochs, app schedules, apps,
//! deploys, installs, services, settings, the Vault's bookkeeping, recovery) as SQLite rows: the truth, written as
//! signed steps (`steps`). Each step is applied by its kind's rule (`rules`) to a fresh copy of the database
//! (`fresh`), and what follows from its rows is worked out again (`derive`), the same on every reader. It's small, so
//! it's copied at each step.
//!
//! Every scope's database has the same tables, `permissions`' among them with its defaults. What `global`'s entries
//! say of the whole network (people, groups, invites, devices, relations, epochs and app schedules of every scope, apps,
//! services, settings, Vault keys, recovery) is filled in only in `global`'s, which everyone in the network reads; a
//! person's or group's has only its own state (installs, schedules, Vault requests, pages, forgotten blobs), which
//! only those who use it may read.
//!
//! Who may do what is `permissions::may` over `relations`, whose objects are `object(scope)`, `device(key)`,
//! `service(key)` and `app(name)`. Whether an object exists (a device still listed, a group made) is the tables' to
//! say, not `may`'s: the network's admins may remove every device, listed or not.
//!
//! People and groups are named as `person:<id>` and `group:<id>`; an app owned by the network's admins as `admins`.
//! Keys and hashes are lowercase hex, times milliseconds, flags 0 or 1, and lists and arrangements JSON.
//!
//! Its state hash is any database's (`stored::state_hash`): its rows', whatever order they were written in, and whatever
//! SQLite's pages hold.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::keys::{Key, hex};

use crate::database::scopes::Scope;
use crate::database::scopes::{Principal, Role};
use crate::permissions;

/// The tables, the same in every scope's database, besides the permissions' (`permissions::SCHEMA`). STRICT, so a
/// value is stored as the type it was given, and every table has a primary key, which changesets need.
pub const SCHEMA: &str = "
CREATE TABLE network (
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
CREATE TABLE app_schedules (
  device TEXT NOT NULL,
  scope TEXT NOT NULL,
  app TEXT NOT NULL,
  principal TEXT NOT NULL,
  until INTEGER NOT NULL,
  PRIMARY KEY (device, scope, app)
) STRICT;
CREATE TABLE app_schedule_keys (
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
  scope TEXT,
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

pub fn person(id: &str) -> String {
    format!("person:{id}")
}

pub fn group(id: &str) -> String {
    format!("group:{id}")
}

pub fn device(key: &Key) -> String {
    format!("device:{}", hex(key))
}

pub fn service(key: &Key) -> String {
    format!("service:{}", hex(key))
}

pub fn app(name: &str) -> String {
    format!("app:{name}")
}

/// A scope, as an object of permissions: the network, a group, or a person's own.
pub fn object(scope: &Scope) -> String {
    match scope {
        Scope::Global => permissions::NETWORK.into(),
        Scope::Group(g) => group(g),
        Scope::User(u) => person(u),
    }
}

pub fn role(r: Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::Member => "member",
    }
}

pub fn owner(p: &Principal) -> String {
    match p {
        Principal::User(id) => person(id),
        Principal::Group(id) => group(id),
        Principal::Admins => "admins".into(),
    }
}

/// A time or number from an entry, as SQLite keeps it: one past what it holds is kept as its most.
pub(crate) fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// A row of `relations` that's a rule: the release's, never given or taken (`permissions`).
pub const RULE: &str = "(substr(subject, 1, 1) IN ('#', '@') OR instr(subject, '*') > 0 OR instr(object, '*') > 0)";

/// Tables of a scope's own state, in every scope's database.
const OWN: &[&str] = &["installs", "schedules", "vault_requests", "pages", "forgotten"];

/// Tables of what steps said that no rule changes, kept as they were: the blobs and releases they named, and the
/// release's types, roles and rules their `DEFAULTS` said (`steps::DEFAULTS`), not this reader's own release's.
const KEPT: &[&str] = &["blobs", "releases", "types", "roles", "relations"];

/// A scope's database with nothing in it yet: its tables, and this release's types, roles and rules.
pub fn empty(scope: &Scope) -> Result<Connection> {
    let db = bare()?;
    db.execute_batch(permissions::DEFAULTS)?;
    if *scope == Scope::Global {
        db.execute("INSERT INTO network VALUES (0, NULL, '', 0, NULL, NULL)", [])?;
    }
    Ok(db)
}

fn bare() -> Result<Connection> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(SCHEMA)?;
    db.execute_batch(permissions::SCHEMA)?;
    Ok(db)
}

/// `from`, `scope`'s database as any release made it (a copy, or the one before a step), as this release makes it: its
/// rows in this release's tables (`migrate`d first), only what a database of its kind of scope holds, and what follows
/// from them worked out again (`derive`); its version, types, roles and rules as they were.
pub fn fresh(from: &Connection, scope: &Scope) -> Result<Connection> {
    super::migrate::database(from)?;
    let to = bare()?;
    to.pragma_update(None, "user_version", super::steps::version(from)?)?;
    let global = *scope == Scope::Global;
    let tables: Vec<String> = to.prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let tx = to.unchecked_transaction()?;
    for t in tables.iter().filter(|t| global || OWN.contains(&t.as_str()) || KEPT.contains(&t.as_str())) {
        let columns = |db: &Connection| -> rusqlite::Result<Vec<String>> { db.prepare(&format!("SELECT name FROM pragma_table_info('{t}')"))?.query_map([], |r| r.get(0))?.collect() };
        let theirs = columns(from)?;
        anyhow::ensure!(!theirs.is_empty() || !KEPT.contains(&t.as_str()), "a scope database without {t}");
        let both: Vec<String> = columns(&tx)?.into_iter().filter(|c| theirs.contains(c)).collect();
        if both.is_empty() {
            continue;
        }
        let only = match t.as_str() {
            "roles" => " WHERE builtin = 1".to_string(),
            "relations" if !global => format!(" WHERE {RULE}"),
            _ => String::new(),
        };
        let list = both.join(", ");
        let mut q = from.prepare(&format!("SELECT {list} FROM {t}{only}"))?;
        let insert = format!("INSERT OR IGNORE INTO {t} ({list}) VALUES ({})", vec!["?"; both.len()].join(", "));
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<rusqlite::types::Value> = (0..both.len()).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            tx.execute(&insert, rusqlite::params_from_iter(values))?;
        }
    }
    if global {
        canonical(&tx)?;
        derive(&tx)?;
    }
    tx.commit()?;
    Ok(to)
}

/// What only one way of saying it is kept of in `global`'s database: one network row; each device's sign-in as this
/// release knows its shape, or none; the recovery keys in the order they were set, the network's last; each person's
/// one role in the network and each group; invites only of people still in, to groups that are; and keys sealed and
/// opened only for each scope's newest.
fn canonical(db: &Connection) -> Result<()> {
    db.execute("INSERT INTO network SELECT 0, NULL, '', 0, NULL, NULL WHERE NOT EXISTS (SELECT 1 FROM network)", [])?;
    let signed: Vec<(String, String)> = db.prepare("SELECT key, sign_in FROM devices WHERE sign_in IS NOT NULL")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (key, sign_in) in signed {
        let known = serde_json::from_str::<crate::sign_in_proof::Account>(&sign_in).ok().map(|a| json!(a).to_string());
        if known.as_deref() != Some(sign_in.as_str()) {
            db.execute("UPDATE devices SET sign_in = ? WHERE key = ?", params![known, key])?;
        }
    }
    let keys: Vec<(String, Option<i64>)> = db.prepare("SELECT key, set_at FROM recovery ORDER BY n")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let now = keys.iter().rposition(|(_, at)| at.is_some()).map(|i| keys[i].clone());
    let mut order: Vec<(String, Option<i64>)> = keys.into_iter().filter(|(_, at)| at.is_none()).collect();
    order.extend(now);
    db.execute("DELETE FROM recovery", [])?;
    for (n, (key, at)) in (0i64..).zip(&order) {
        db.execute("INSERT INTO recovery VALUES (?, ?, ?)", params![n, key, at])?;
    }
    db.execute_batch(
        "DELETE FROM roles WHERE builtin = 0;
        DELETE FROM sealed WHERE epoch IS NOT (SELECT max(epoch) FROM epochs e WHERE e.scope = sealed.scope);
        DELETE FROM vault_opens WHERE scope NOT IN (SELECT scope FROM vault);
        DELETE FROM app_schedule_keys WHERE NOT EXISTS (SELECT 1 FROM app_schedules a
            WHERE a.device = app_schedule_keys.device AND a.scope = app_schedule_keys.scope AND a.app = app_schedule_keys.app);
        DELETE FROM runs WHERE principal NOT IN (SELECT principal FROM runs_chosen);
        DELETE FROM invites WHERE principal NOT IN (SELECT id FROM principals WHERE kind = 'person' AND removed = 0)
            OR invited_to NOT IN (SELECT id FROM principals WHERE kind = 'group');
        UPDATE invites SET role = 'member' WHERE role != 'admin';",
    )?;
    Ok(())
}

/// Works out again, in `global`'s database, what follows from its rows, which no step writes: everyone owns their own
/// scope, each device listed its person, and each app its owners (the network's or a group's admins, or a person still
/// in); a shared service is everyone's to use; the Cloudflare node, which an admin's setting names, is the network's
/// admins' device; and which devices run each scope's calls. A person's own scope runs on their machines they listed
/// themselves (one an admin listed for them runs only what that admin's could), and on others their own device chose
/// (`runs`); a group's on its admins' machines and the network's; `global`'s on the network's admins'; and every one
/// but a person's own on the Cloudflare node, which runs a person's own only once they chose it. Only people still in,
/// and devices still listed, hold anything; a person's role in the network or a group is the only row a step writes.
pub fn derive(db: &Connection) -> Result<()> {
    let cloudflare: Option<String> = db.query_row("SELECT value FROM settings WHERE name = ?1", [super::scopes::CLOUDFLARE_NODE], |r| r.get(0)).optional()?;
    let cloudflare = cloudflare.and_then(|k| crate::keys::unhex(&k).ok()).map(|k| hex(&k));
    db.execute_batch(&format!(
        "DELETE FROM relations WHERE NOT {RULE} AND NOT (
            substr(subject, 1, 7) = 'person:' AND relation IN ('admin', 'member')
            AND subject IN (SELECT id FROM principals WHERE kind = 'person' AND removed = 0)
            AND (object = '{n}' OR object IN (SELECT id FROM principals WHERE kind = 'group')));
        DELETE FROM relations WHERE relation = 'member' AND substr(subject, 1, 7) = 'person:'
            AND EXISTS (SELECT 1 FROM relations a WHERE a.subject = relations.subject AND a.object = relations.object AND a.relation = 'admin');
        INSERT OR IGNORE INTO relations SELECT id, 'member', '{n}', NULL FROM principals p WHERE kind = 'person' AND removed = 0
            AND NOT EXISTS (SELECT 1 FROM relations r WHERE r.subject = p.id AND r.object = '{n}');
        INSERT OR IGNORE INTO relations SELECT id, 'owner', id, NULL FROM principals WHERE kind = 'person' AND removed = 0;
        CREATE TEMP TABLE listed AS SELECT d.key, d.principal, d.listed_by, d.compute, d.service, d.shared FROM devices d
            JOIN principals p ON p.id = d.principal AND p.kind = 'person' AND p.removed = 0 WHERE d.removed = 0;
        INSERT OR IGNORE INTO relations SELECT principal, 'owner', 'device:' || key, NULL FROM listed;
        INSERT OR IGNORE INTO relations SELECT iif(shared, '{n}#member', principal), 'user', 'service:' || key, NULL FROM listed WHERE service IS NOT NULL;
        INSERT OR IGNORE INTO relations
        SELECT iif(substr(owner, 1, 7) = 'person:', owner, iif(substr(owner, 1, 6) = 'group:', owner || '#admin', '{n}#admin')), 'owner', 'app:' || name, NULL
        FROM apps WHERE substr(owner, 1, 7) != 'person:' OR owner IN (SELECT id FROM principals WHERE kind = 'person' AND removed = 0);
        INSERT OR IGNORE INTO relations
        WITH admins AS (SELECT subject FROM relations WHERE relation = 'admin' AND object = '{n}'),
            computes AS (SELECT key, principal, listed_by FROM listed WHERE compute = 1),
            grps AS (SELECT id FROM principals WHERE kind = 'group'),
            people AS (SELECT id FROM principals WHERE kind = 'person')
        SELECT 'device:' || key, 'runner', '{n}', NULL FROM computes WHERE principal IN admins
        UNION ALL
        SELECT 'device:' || c.key, 'runner', g.id, NULL FROM computes c, grps g
            WHERE c.principal IN admins OR EXISTS (SELECT 1 FROM relations r WHERE r.subject = c.principal AND r.relation = 'admin' AND r.object = g.id)
        UNION ALL
        SELECT 'device:' || c.key, 'runner', u.id, NULL FROM computes c, people u
            WHERE (c.principal = u.id AND c.listed_by = u.id) OR EXISTS (SELECT 1 FROM runs WHERE principal = u.id AND device = c.key);
        DROP TABLE temp.listed;",
        n = permissions::NETWORK
    ))?;
    if let Some(k) = cloudflare {
        db.execute(&format!("INSERT OR IGNORE INTO relations VALUES ('{}#admin', 'admin', 'device:' || ?1, NULL)", permissions::NETWORK), [&k])?;
        // Unless it's a machine of someone's too, which runs what theirs do.
        let computes = "SELECT d.key FROM devices d JOIN principals p ON p.id = d.principal AND p.kind = 'person' AND p.removed = 0 WHERE d.removed = 0 AND d.compute = 1";
        db.execute(
            &format!(
                "INSERT OR IGNORE INTO relations
                SELECT 'device:' || ?1, 'runner', o, NULL FROM (
                    SELECT '{n}' o UNION ALL SELECT id FROM principals WHERE kind = 'group'
                    UNION ALL SELECT principal FROM runs WHERE device = ?1 AND principal IN (SELECT id FROM principals WHERE kind = 'person'))
                WHERE ?1 NOT IN ({computes})",
                n = permissions::NETWORK
            ),
            [&k],
        )?;
    }
    Ok(())
}
