//! App databases: an app's data in a scope is one SQLite database, the truth, kept in the bucket as steps at numbered
//! positions, each sealed under one of its scope's epochs (`src/keys.rs`) and naming it: the changeset of one write
//! (SQLite's session extension), or one of the app's schema changes. Reading it is its newest copy and the steps after;
//! writing it is a step created only if its position is free, so when two devices write at once, one wins and the
//! other catches up and runs its write again.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m, "epoch": e}, replaced while unchanged: a hint, so readers needn't
//!                            list; e is the epoch copy m is sealed under, the oldest a reader starting there needs
//! <prefix>/<n>               step n, from 1
//! <prefix>/copy/<n>          the whole database after step n, under the epoch of the steps it holds
//! ```
//!
//! Along a database's positions epochs never go down: a step under an epoch lower than the step before it, or that
//! doesn't open with the key of the epoch it names, is skipped by every reader, its position taken, and the next write
//! takes the following one. A writer writes under the newest epoch it was given, at once; the next copy is under it.
//! A step under an epoch above every key a reader was given stops it (`Ahead`): its caller looks for that epoch's
//! entry, which is written before anything is sealed under it, and if there's none, the step is junk, passed over.
//! The newest hint is only a hint, and a copy is used only if it opens and its epoch is at least its step's: otherwise
//! a reader reads the steps from the start.
//!
//! No clock, disk or network: the bucket is all it reads and writes, so it runs wherever SQLite does. A database's key
//! in each epoch comes from its app's key in that epoch (`app_key`, `kernel::database_key`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, c_int, c_void};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::Connection;
use rusqlite::types::{Value as Sql, ValueRef};
use serde_json::{Map, Value, json};

use crate::bucket::Bucket;

/// A copy is written once this many steps follow the last one.
pub const COPY_EVERY: u64 = 100;
/// How many times a write runs again after losing a race before it gives up.
const TRIES: usize = 50;

/// A database's keys, by the epoch each opens.
pub type Keys = BTreeMap<u32, [u8; 32]>;

/// What a read or write stopped at: a copy or step sealed under an epoch whose key it wasn't given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub epoch: u32,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "it needs the key of epoch {}, which it wasn't given", self.epoch)
    }
}

impl std::error::Error for Missing {}

/// What a read stopped at: step `at`, under an epoch above every key it was given, which is either a new epoch's (its
/// entry says so) or junk (none does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ahead {
    pub epoch: u32,
    pub at: u64,
}

impl std::fmt::Display for Ahead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "step {} is under epoch {}, newer than any key it was given", self.at, self.epoch)
    }
}

impl std::error::Error for Ahead {}

/// The most epochs a scope looks for sealed to a device that knows none of its entries yet.
pub const EPOCHS_MAX: u32 = 10_000;

/// Where a reader starts: the copy the newest hint names, as (its position, its epoch, its bytes), if it holds: it's
/// there, its epoch is one `keys` reaches, at least that of the step at its position, and it opens. Otherwise none, and
/// the reader reads every step. Fails with `Missing` for a copy under an epoch below the newest it was given but not given.
pub async fn start(bucket: &impl Bucket, prefix: &str, keys: &Keys) -> Result<Option<(u64, u32, Vec<u8>)>> {
    let copy = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes).1,
        None => 0,
    };
    if copy == 0 {
        return Ok(None);
    }
    let name = copy_name(prefix, copy);
    let Some(sealed) = bucket.get(&name).await? else { return Ok(None) };
    let Ok(epoch) = crate::keys::epoch_of(&sealed.bytes) else { return Ok(None) };
    let step = bucket.get(&step_name(prefix, copy)).await?.and_then(|s| crate::keys::epoch_of(&s.bytes).ok());
    if epoch > keys.keys().next_back().copied().unwrap_or(0) || step.is_none_or(|s| s > epoch) {
        tracing::debug!("{name} isn't a copy to start from: reading every step");
        return Ok(None);
    }
    let key = keys.get(&epoch).ok_or(Missing { epoch })?;
    match crate::keys::open(key, &name, &sealed.bytes) {
        Ok(bytes) => Ok(Some((copy, epoch, bytes))),
        Err(err) => {
            tracing::debug!("{name}: {err:#}: reading every step");
            Ok(None)
        }
    }
}

/// An app's key in epoch `n` of a scope, from its copy in the bucket (`keys::app_key_name`), sealed under that epoch's
/// key. The first to need it makes it, at random, unless `make` is false (an epoch nothing was written under yet,
/// where a reader has nothing to open): whichever is written first is the key.
pub async fn app_key(bucket: &impl Bucket, epoch_key: &[u8; 32], scope: &str, app: &str, n: u32, make: bool) -> Result<Option<[u8; 32]>> {
    let name = crate::keys::app_key_name(scope, app, n);
    if let Some(o) = bucket.get(&name).await? {
        return crate::kernel::open_app_key(epoch_key, app, &o.bytes).map(Some);
    }
    if !make {
        return Ok(None);
    }
    let key = rand::random();
    if bucket.create(&name, &crate::kernel::seal_app_key(epoch_key, app, &key)).await? {
        return Ok(Some(key));
    }
    let made = bucket.get(&name).await?.context("an app's key went as it was made")?;
    crate::kernel::open_app_key(epoch_key, app, &made.bytes).map(Some)
}

/// One SQLite database of an app's, at a position in its steps.
pub struct Db {
    prefix: String,
    keys: Keys,
    conn: Connection,
    /// The last step applied (or passed over).
    at: u64,
    /// The newest copy this device knows of.
    copy: u64,
    /// The epoch of the last step applied, or of the copy it started from: 0 before either.
    epoch: u32,
    /// The epoch of the copy it started from, as the newest hint says it.
    copy_epoch: u32,
    /// The epochs of what it read: the copy it started from and the steps it applied.
    used: BTreeSet<u32>,
    /// Positions its caller found hold junk under an epoch no entry made (`Ahead`), passed over.
    junk: BTreeSet<u64>,
}

impl Db {
    /// Reads a database as its newest steps leave it, with its keys by epoch: empty if it has none. Fails with `Missing` at a copy or step under an epoch whose key it wasn't given, and with
    /// `Ahead` at a step under an epoch above them all.
    pub async fn open(bucket: &impl Bucket, prefix: &str, keys: Keys) -> Result<Db> {
        Db::open_passing(bucket, prefix, keys, BTreeSet::new()).await
    }

    /// `open`, passing over the steps at `junk`.
    pub async fn open_passing(bucket: &impl Bucket, prefix: &str, keys: Keys, junk: BTreeSet<u64>) -> Result<Db> {
        let mut db = Db { prefix: prefix.into(), keys, conn: Connection::open_in_memory()?, at: 0, copy: 0, epoch: 0, copy_epoch: 0, used: BTreeSet::new(), junk };
        if let Some((copy, epoch, bytes)) = start(bucket, prefix, &db.keys).await? {
            db.conn.deserialize_read_exact("main", &bytes[..], bytes.len(), false)?;
            (db.at, db.copy, db.epoch, db.copy_epoch) = (copy, copy, epoch, epoch);
            db.used.insert(epoch);
        }
        // After the copy, which SQLite reads in by attaching it.
        confine(&mut db.conn)?;
        db.catch_up(bucket).await?;
        Ok(db)
    }

    /// Passes over step `at`, under an epoch no entry made, as every reader does.
    pub fn pass_over(&mut self, at: u64) {
        self.junk.insert(at);
    }

    pub fn at(&self) -> u64 {
        self.at
    }

    /// The epoch of the last step it applied, or the copy it started from: 0 before either.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// The keys it was opened with, by epoch.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Takes keys of epochs it wasn't given, as a device does when it's handed a scope's new epoch: what it holds
    /// stays as it was read.
    pub fn add_keys(&mut self, keys: &Keys) {
        for (epoch, key) in keys {
            self.keys.entry(*epoch).or_insert(*key);
        }
    }

    /// Whether `keys` open everything it read, and are the same as its own for every epoch both have: so a call given
    /// them may use it as it is.
    pub fn opens_for(&self, keys: &Keys) -> bool {
        self.used.iter().all(|e| keys.contains_key(e)) && keys.iter().all(|(e, k)| self.keys.get(e).is_none_or(|mine| mine == k))
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Applies the steps written since this device last read: whether there were any.
    pub async fn catch_up(&mut self, bucket: &impl Bucket) -> Result<bool> {
        let from = self.at;
        loop {
            let name = self.step(self.at + 1);
            let Some(o) = bucket.get(&name).await? else { break };
            if self.junk.contains(&(self.at + 1)) {
                self.at += 1;
                continue;
            }
            if let Some((epoch, step)) = self.unseal(self.at + 1, &name, &o.bytes)? {
                self.apply(&step)?;
                self.epoch = epoch;
                self.used.insert(epoch);
            }
            self.at += 1;
        }
        Ok(self.at > from)
    }

    /// Step `at`, opened: None for one every reader passes over, under an epoch lower than the step before it or that
    /// doesn't open with the key of the epoch it names.
    fn unseal(&self, at: u64, name: &str, sealed: &[u8]) -> Result<Option<(u32, Vec<u8>)>> {
        let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Ok(None) };
        if epoch < self.epoch {
            tracing::debug!("{name} is under epoch {epoch}, after {}: passed over", self.epoch);
            return Ok(None);
        }
        if self.keys.keys().next_back().is_none_or(|newest| epoch > *newest) {
            bail!(Ahead { epoch, at });
        }
        let key = self.keys.get(&epoch).ok_or(Missing { epoch })?;
        match crate::keys::open(key, name, sealed) {
            Ok(step) => Ok(Some((epoch, step))),
            Err(err) => {
                tracing::debug!("{name}: {err:#}: passed over");
                Ok(None)
            }
        }
    }

    /// The newest epoch it was given, and its key: what it writes under.
    fn newest_key(&self) -> Result<(u32, [u8; 32])> {
        let (epoch, key) = self.keys.last_key_value().with_context(|| format!("{} was opened with no keys to write with", self.prefix))?;
        ensure!(*epoch >= self.epoch, Missing { epoch: self.epoch });
        Ok((*epoch, *key))
    }

    /// Brings the database's schema to the app's newest: each of `migrations` it lacks, in order, as a step.
    pub async fn migrate(&mut self, bucket: &impl Bucket, migrations: &[String]) -> Result<()> {
        for _ in 0..TRIES {
            self.catch_up(bucket).await?;
            let version = self.version()?;
            let Some(sql) = migrations.get(version) else { return Ok(()) };
            let step = [b"s".as_slice(), &serde_json::to_vec(&json!({ "version": version + 1, "sql": sql }))?].concat();
            // Tried here first, so a schema that fails is never written.
            self.conn.execute_batch("savepoint migrate")?;
            let tried = self.apply(&step);
            self.conn.execute_batch("rollback to migrate; release migrate")?;
            tried?;
            let (epoch, key) = self.newest_key()?;
            let name = self.step(self.at + 1);
            if bucket.create(&name, &crate::keys::seal(epoch, &key, &name, &step)).await? {
                self.apply(&step)?;
                self.wrote(epoch);
                self.written(bucket).await;
            }
        }
        bail!("{} changed too often to change its schema", self.prefix)
    }

    /// The schema changes this database has had.
    pub fn version(&self) -> Result<usize> {
        Ok(self.conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Runs a write in one transaction, which is the step written after this device's newest, under the newest epoch
    /// it was given: when another device wrote that first, it catches up and runs `f` again, so `f` must do only
    /// database work.
    pub async fn write<R>(&mut self, bucket: &impl Bucket, mut f: impl FnMut(&Connection) -> Result<R>) -> Result<R> {
        for _ in 0..TRIES {
            self.catch_up(bucket).await?;
            let (epoch, key) = self.newest_key()?;
            let open = Open::begin(&mut self.conn)?;
            let (result, changes) = changes(&open, &mut f)?;
            if changes.is_empty() {
                open.execute_batch("commit")?;
                return Ok(result);
            }
            let name = step_name(&self.prefix, self.at + 1);
            let step = [b"c".as_slice(), &changes].concat();
            let sealed = crate::keys::seal(epoch, &key, &name, &step);
            if created(bucket, &name, &sealed).await? {
                open.execute_batch("commit")?;
                drop(open);
                self.wrote(epoch);
                self.written(bucket).await;
                return Ok(result);
            }
        }
        bail!("{} changed too often to write to", self.prefix)
    }

    /// After its own step under `epoch` was created at the next position.
    fn wrote(&mut self, epoch: u32) {
        self.at += 1;
        self.epoch = epoch;
        self.used.insert(epoch);
    }

    /// Reads with a statement that only reads: its rows, each column by name.
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
        query(&self.conn, sql, params)
    }

    /// The database's own connection, for reads that need more than `query`.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn step(&self, n: u64) -> String {
        step_name(&self.prefix, n)
    }

    /// After writing a step: a copy, if steps have piled up since the last, and the newest hint moved on. Both only
    /// save readers work, so a link that fails them fails nothing.
    async fn written(&mut self, bucket: &impl Bucket) {
        if let Err(err) = self.hint(bucket).await {
            tracing::debug!("{}: {err:#}", self.prefix);
        }
    }

    async fn hint(&mut self, bucket: &impl Bucket) -> Result<()> {
        if self.at >= self.copy + COPY_EVERY
            && let Some(key) = self.keys.get(&self.epoch)
        {
            let name = copy_name(&self.prefix, self.at);
            let bytes = self.conn.serialize("main")?.to_vec();
            bucket.create(&name, &crate::keys::seal(self.epoch, key, &name, &bytes)).await?;
            (self.copy, self.copy_epoch) = (self.at, self.epoch);
        }
        hint(bucket, &self.prefix, self.at, self.copy, self.copy_epoch).await
    }

    /// Applies a step: a changeset, or a schema change this database is due.
    fn apply(&self, step: &[u8]) -> Result<()> {
        match step.split_first() {
            Some((b'c', changes)) => apply_changes(&self.conn, changes),
            Some((b's', change)) => {
                let change: Value = serde_json::from_slice(change)?;
                let version = change["version"].as_u64().context("a schema change has its version")? as usize;
                if self.version()? + 1 == version {
                    self.conn.execute_batch(&format!("{}\n;pragma user_version = {version}", change["sql"].as_str().context("a schema change has its SQL")?))?;
                }
                Ok(())
            }
            // A kind of step a newer device writes.
            _ => Ok(()),
        }
    }
}

/// How long an object whose answer was lost is sent again for, waiting longer each time, before it may have been
/// written: a call's own time runs out sooner.
const ANSWERED_WITHIN_MS: u64 = 60_000;

/// Creates an object that only this device would write with these bytes, such as a write's step: whether they're
/// there now, rather than another's. When the answer is lost, the same is sent again until the bucket says, rather than
/// asked what's there, since the first may still be on its way and land after a read that found nothing: a write that
/// took that for a lost race would run again on top of itself, and one told it failed would be there after all. One the
/// bucket refused, or that never left, isn't there, unless one sent before is.
pub async fn created(bucket: &impl Bucket, name: &str, bytes: &[u8]) -> Result<bool> {
    let may_have = "the bucket didn't answer whether it was written, so it may have been: look before trying again";
    let (mut sent, mut wait, mut waited) = (false, 100, 0);
    loop {
        match bucket.create(name, bytes).await {
            Ok(true) => return Ok(true),
            Ok(false) if !sent => return Ok(false),
            // One sent before, whose answer was lost, may be what's there.
            Ok(false) => match bucket.get(name).await {
                Ok(o) => return Ok(o.with_context(|| format!("{name} went once written"))?.bytes == bytes),
                Err(err) if !unanswered(&err) => return Err(err.context(may_have)),
                Err(_) => {}
            },
            Err(err) if !unanswered(&err) => return Err(if sent { err.context(may_have) } else { err }),
            Err(_) => sent = true,
        }
        if waited >= ANSWERED_WITHIN_MS {
            bail!("{may_have}");
        }
        bucket.wait(wait).await;
        waited += wait;
        wait = (wait * 2).min(5_000);
    }
}

/// Whether a request's answer never came, so it may have done what it asked.
fn unanswered(err: &anyhow::Error) -> bool {
    crate::bucket::Trouble::of(err) == Some(crate::bucket::Trouble::Unreachable)
}

/// A database's objects in the bucket, by its prefix: its newest hint, step `n`, and its copy after step `n`.
pub fn newest_name(prefix: &str) -> String {
    format!("{prefix}/newest")
}

pub fn step_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/{n:012}")
}

pub fn copy_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/copy/{n:012}")
}

/// Moves the newest hint at `prefix` on to say at least step `at` and copy `copy` (sealed under `epoch`), replacing it
/// only while unchanged.
pub async fn hint(bucket: &impl Bucket, prefix: &str, at: u64, copy: u64, epoch: u32) -> Result<()> {
    let name = newest_name(prefix);
    for _ in 0..TRIES {
        let current = bucket.get(&name).await?;
        let mut said = current.as_ref().map(|o| newest(&o.bytes));
        // One that says a step is written that isn't is someone's junk, not raised on.
        if let Some((was_at, _, _)) = said
            && was_at > at
            && bucket.get(&step_name(prefix, was_at)).await?.is_none()
        {
            said = Some((0, 0, 0));
        }
        let (was_at, was_copy, was_epoch) = said.unwrap_or((0, 0, 0));
        let epoch = if copy >= was_copy { epoch } else { was_epoch };
        let (at, copy) = (at.max(was_at), copy.max(was_copy));
        if current.as_ref().map(|o| newest(&o.bytes)) == Some((at, copy, epoch)) {
            return Ok(());
        }
        if bucket.replace(&name, &serde_json::to_vec(&json!({ "at": at, "copy": copy, "epoch": epoch }))?, current.as_ref().map(|o| o.tag.as_str())).await? {
            return Ok(());
        }
    }
    Ok(())
}

/// What a database's newest hint says: (its last step, its newest copy, the epoch that copy is sealed under). One
/// that isn't a hint says nothing.
pub fn newest(bytes: &[u8]) -> (u64, u64, u32) {
    let v: Value = serde_json::from_slice(bytes).unwrap_or_default();
    (v["at"].as_u64().unwrap_or(0), v["copy"].as_u64().unwrap_or(0), u32::try_from(v["epoch"].as_u64().unwrap_or(0)).unwrap_or(0))
}

/// Keeps an app's SQL inside its own database: every statement it gives (queries, writes and schema changes, which run on
/// every device that reads the database) runs here, and SQLite lets SQL open other files (`attach`, `vacuum into`),
/// load code (`load_extension`), and turn off what keeps a database whole (pragmas). So no database can be attached,
/// which `vacuum into` needs too, and only pragmas that read the schema or set what an app's own data needs are allowed.
fn confine(conn: &mut Connection) -> Result<()> {
    use rusqlite::hooks::{AuthAction, Authorization};
    use rusqlite::limits::Limit;
    const PRAGMAS: &[&str] = &[
        "user_version", "application_id", "foreign_keys", "defer_foreign_keys", "recursive_triggers", "table_info",
        "table_xinfo", "table_list", "index_list", "index_info", "index_xinfo", "foreign_key_list", "foreign_key_check",
        "integrity_check", "quick_check", "optimize", "data_version", "page_count", "freelist_count",
    ];
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| match ctx.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, .. } if !PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str()) => Authorization::Deny,
        AuthAction::Function { function_name } if ["load_extension", "fts3_tokenizer"].contains(&function_name.to_ascii_lowercase().as_str()) => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))?;
    Ok(())
}

/// Reads with a statement that only reads.
pub fn query(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    let mut stmt = conn.prepare(sql)?;
    ensure!(stmt.readonly(), "a query only reads");
    run(&mut stmt, params)
}

/// Runs a call's statements on a scope's database, in the order it ran them: each (sql, params, check), where a
/// check's first value must be true, else the write fails with its message.
pub fn statements(conn: &Connection, list: &[(String, Vec<Value>, Option<String>)]) -> Result<()> {
    for (sql, params, check) in list {
        let rows = execute(conn, sql, params)?;
        if let Some(message) = check {
            let held = rows.first().and_then(|r| r.values().next()).is_some_and(|v| v.as_i64().is_some_and(|n| n != 0) || v == true);
            ensure!(held, "{message}");
        }
    }
    Ok(())
}

/// Runs a statement that may write: its rows, if it gives any (`returning`).
pub fn execute(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    run(&mut conn.prepare(sql)?, params)
}

fn run(stmt: &mut rusqlite::Statement, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    ensure!(stmt.parameter_count() == params.len(), "the statement takes {} values, given {}", stmt.parameter_count(), params.len());
    for (i, p) in params.iter().enumerate() {
        stmt.raw_bind_parameter(i + 1, to_sql(p))?;
    }
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
    let mut rows = stmt.raw_query();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut m = Map::new();
        for (i, name) in names.iter().enumerate() {
            m.insert(name.clone(), from_sql(row.get_ref(i)?));
        }
        out.push(m);
    }
    Ok(out)
}

fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(*b as i64),
        Value::Number(n) => n.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn from_sql(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => f.into(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
        ValueRef::Blob(b) => b.iter().map(|b| Value::from(*b)).collect(),
    }
}

// SQLite's session extension, which the bundled SQLite is built with (`.cargo/config.toml`).

#[repr(C)]
struct Session {
    _private: [u8; 0],
}

#[repr(C)]
struct Iter {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn sqlite3session_create(db: *mut rusqlite::ffi::sqlite3, name: *const c_char, out: *mut *mut Session) -> c_int;
    fn sqlite3session_object_config(s: *mut Session, op: c_int, arg: *mut c_void) -> c_int;
    fn sqlite3session_attach(s: *mut Session, table: *const c_char) -> c_int;
    fn sqlite3session_changeset(s: *mut Session, n: *mut c_int, out: *mut *mut c_void) -> c_int;
    fn sqlite3session_delete(s: *mut Session);
    fn sqlite3changeset_apply(
        db: *mut rusqlite::ffi::sqlite3,
        n: c_int,
        changes: *mut c_void,
        filter: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int>,
        conflict: Option<unsafe extern "C" fn(*mut c_void, c_int, *mut Iter) -> c_int>,
        ctx: *mut c_void,
    ) -> c_int;
}

const SESSION_OBJCONFIG_ROWID: c_int = 2;
const CHANGESET_DATA: c_int = 1;
const CHANGESET_CONFLICT: c_int = 3;
const CHANGESET_OMIT: c_int = 0;
const CHANGESET_REPLACE: c_int = 1;

/// Runs `f` with every change it makes recorded: → its result, and its changeset (empty if it changed nothing).
fn changes<R>(conn: &Connection, f: &mut impl FnMut(&Connection) -> Result<R>) -> Result<(R, Vec<u8>)> {
    let db = unsafe { conn.handle() };
    let mut s: *mut Session = std::ptr::null_mut();
    check(conn, unsafe { sqlite3session_create(db, c"main".as_ptr(), &mut s) })?;
    let made = (|| {
        // Tables without a primary key are recorded by rowid.
        let mut on: c_int = 1;
        check(conn, unsafe { sqlite3session_object_config(s, SESSION_OBJCONFIG_ROWID, (&raw mut on).cast()) })?;
        check(conn, unsafe { sqlite3session_attach(s, std::ptr::null()) })?;
        let result = f(conn)?;
        let (mut n, mut p): (c_int, *mut c_void) = (0, std::ptr::null_mut());
        check(conn, unsafe { sqlite3session_changeset(s, &mut n, &mut p) })?;
        let bytes = if p.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(p as *const u8, n as usize) }.to_vec() };
        unsafe { rusqlite::ffi::sqlite3_free(p) };
        Ok((result, bytes))
    })();
    unsafe { sqlite3session_delete(s) };
    made
}

/// A write's transaction, rolled back when it's dropped uncommitted: on an error, and when the write's future is
/// dropped while it waits on the bucket (a call that ran out of time, or whose caller went away), which would otherwise
/// leave it open, its changes seen by reads, and every later write failing to begin. It holds the connection mutably,
/// as only that is `Send` across the bucket's await.
struct Open<'a>(&'a mut Connection);

impl<'a> Open<'a> {
    fn begin(conn: &'a mut Connection) -> Result<Self> {
        conn.execute_batch("begin")?;
        Ok(Open(conn))
    }
}

impl std::ops::Deref for Open<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0
    }
}

impl Drop for Open<'_> {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("rollback");
        }
    }
}

/// Applies a changeset written from the state this database is in, so nothing should clash; if something does, the
/// change wins where it can, the same way on every device.
fn apply_changes(conn: &Connection, changes: &[u8]) -> Result<()> {
    unsafe extern "C" fn conflict(_: *mut c_void, kind: c_int, _: *mut Iter) -> c_int {
        match kind {
            CHANGESET_DATA | CHANGESET_CONFLICT => CHANGESET_REPLACE,
            _ => CHANGESET_OMIT,
        }
    }
    let mut bytes = changes.to_vec();
    let db = unsafe { conn.handle() };
    check(conn, unsafe { sqlite3changeset_apply(db, bytes.len() as c_int, bytes.as_mut_ptr().cast(), None, Some(conflict), std::ptr::null_mut()) })
}

fn check(conn: &Connection, rc: c_int) -> Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let message = unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_errmsg(conn.handle())) }.to_string_lossy().to_string();
    bail!("SQLite said {rc}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::Memory;

    const SCHEMA: &str = "create table counts (name text primary key, n integer not null); create table notes (text text)";

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }

    fn keys(list: &[(u32, [u8; 32])]) -> Keys {
        list.iter().copied().collect()
    }

    async fn note(db: &mut Db, bucket: &impl Bucket, text: &str) {
        db.write(bucket, |c| execute(c, "insert into notes values (?)", &[json!(text)]).map(|_| ())).await.unwrap();
    }

    fn notes(db: &Db) -> Vec<String> {
        db.query("select text from notes order by rowid", &[]).unwrap().into_iter().map(|r| r["text"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn a_new_epoch_is_written_under_at_once_and_what_came_before_still_reads() {
        rt().block_on(async {
            let bucket = Memory::default();
            let (one, two): ([u8; 32], [u8; 32]) = (rand::random(), rand::random());
            let mut old = Db::open(&bucket, "p", keys(&[(1, one)])).await.unwrap();
            old.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            note(&mut old, &bucket, "before").await;

            // A writer given epoch 2 writes under it at once, with no copy in its write.
            let mut new = Db::open(&bucket, "p", keys(&[(1, one), (2, two)])).await.unwrap();
            note(&mut new, &bucket, "after").await;
            assert_eq!(crate::keys::epoch_of(&bucket.get(&step_name("p", 3)).await.unwrap().unwrap().bytes).unwrap(), 2);
            assert!(bucket.get(&copy_name("p", 3)).await.unwrap().is_none());
            assert_eq!(notes(&new), ["before", "after"]);

            // A writer still on epoch 1 can't read past epoch 2, so writes nothing after it.
            assert_eq!(old.write(&bucket, |_| Ok(())).await.unwrap_err().downcast::<Ahead>().unwrap(), Ahead { epoch: 2, at: 3 });
            // A step under epoch 1 after it anyway (a device that didn't read first) is passed over, its position taken.
            let stale = {
                let mut scratch = Db::open(&Memory::default(), "q", keys(&[(1, one)])).await.unwrap();
                scratch.migrate(&Memory::default(), &[SCHEMA.into()]).await.unwrap();
                let open = Open::begin(&mut scratch.conn).unwrap();
                changes(&open, &mut |c: &Connection| execute(c, "insert into notes values ('stale')", &[]).map(|_| ())).unwrap().1
            };
            let name = step_name("p", 4);
            assert!(bucket.create(&name, &crate::keys::seal(1, &one, &name, &[b"c".as_slice(), &stale].concat())).await.unwrap());
            // And so is junk that names epoch 2 but doesn't open with its key.
            let name = step_name("p", 5);
            assert!(bucket.create(&name, &crate::keys::seal(2, &rand::random(), &name, b"cjunk")).await.unwrap());
            note(&mut new, &bucket, "later").await;
            assert_eq!(new.at(), 6);
            let fresh = Db::open(&bucket, "p", keys(&[(1, one), (2, two)])).await.unwrap();
            assert_eq!(notes(&fresh), ["before", "after", "later"], "data from before the change reads, the stale step doesn't");

            // Given only epoch 2, it needs epoch 1 until a copy under 2 is written, then opens with 2 alone.
            let missing = Db::open(&bucket, "p", keys(&[(2, two)])).await.err().unwrap().downcast::<Missing>().unwrap();
            assert_eq!(missing, Missing { epoch: 1 });
            for n in 0..COPY_EVERY {
                note(&mut new, &bucket, &format!("{n}")).await;
            }
            let (_, copy, epoch) = newest(&bucket.get(&newest_name("p")).await.unwrap().unwrap().bytes);
            assert!(copy > 0 && epoch == 2, "the routine copy is under the newest epoch");
            let two_only = Db::open(&bucket, "p", keys(&[(2, two)])).await.unwrap();
            assert_eq!(notes(&two_only)[..3], ["before", "after", "later"]);
            assert!(two_only.opens_for(&keys(&[(2, two)])) && !new.opens_for(&keys(&[(2, two)])), "a call given epoch 2 alone doesn't use what epoch 1 opened");
        });
    }

    #[test]
    fn what_the_bucket_keeps_is_sealed_to_its_name() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            note(&mut a, &bucket, "a secret note").await;
            note(&mut a, &bucket, "another").await;
            for (name, bytes) in bucket.objects.lock().unwrap().iter() {
                assert!(name.ends_with("newest") || !bytes.windows(6).any(|w| w == b"secret" || w == b"create"), "{name} isn't sealed");
            }
            let other = Db::open(&bucket, "p", keys(&[(1, rand::random())])).await.unwrap();
            assert_eq!(other.version().unwrap(), 0, "another key opens nothing");
            // A step moved to another position doesn't open there.
            let mut objects = bucket.objects.lock().unwrap();
            let second = objects.get("p/000000000002").unwrap().clone();
            objects.insert("p/000000000004".into(), second);
            drop(objects);
            assert_eq!(notes(&Db::open(&bucket, "p", key).await.unwrap()), ["a secret note", "another"]);
        });
    }

    #[test]
    fn an_apps_sql_reaches_no_file_but_its_database() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key: [u8; 32] = rand::random();
            let mut db = Db::open(&bucket, "p", keys(&[(1, key)])).await.unwrap();
            db.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            let file = std::env::temp_dir().join(format!("attached-{}.db", rand::random::<u64>()));
            let file = file.to_str().unwrap();
            let attach = format!("attach database '{file}' as x");
            assert!(db.query(&attach, &[]).is_err(), "a query");
            let wrote = db.write(&bucket, |c| statements(c, &[(format!("{attach}; create table x.t (a)"), vec![], None)])).await;
            assert!(wrote.is_err(), "a write");
            for sql in ["pragma writable_schema = 1", "pragma temp_store_directory = '/tmp'", "select load_extension('x')"] {
                assert!(db.query(sql, &[]).is_err() && execute(db.connection(), sql, &[]).is_err(), "{sql}");
            }
            // A schema step runs on every device that reads the database, outside any transaction: written straight to
            // the bucket, as a hostile app's would be, it fails there rather than reaching the file.
            for sql in [format!("{attach}; create table x.t (a)"), format!("vacuum into '{file}'")] {
                let name = step_name("p", 2);
                let step = [b"s".as_slice(), &serde_json::to_vec(&json!({ "version": 2, "sql": sql })).unwrap()].concat();
                bucket.objects.lock().unwrap().insert(name.clone(), crate::keys::seal(1, &key, &name, &step));
                assert!(Db::open(&bucket, "p", keys(&[(1, key)])).await.is_err(), "{sql}");
                bucket.objects.lock().unwrap().remove(&name);
            }
            assert!(!std::path::Path::new(file).exists());
            assert_eq!(db.version().unwrap(), 1, "and its own schema still changes");
        });
    }

    /// A bucket whose writes, while its flag is up, never finish.
    #[derive(Default)]
    struct Stalls(Memory, std::sync::atomic::AtomicBool);

    impl Bucket for Stalls {
        async fn get(&self, name: &str) -> Result<Option<crate::bucket::Object>> {
            self.0.get(name).await
        }
        async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
            if self.1.load(std::sync::atomic::Ordering::Relaxed) {
                std::future::pending::<()>().await;
            }
            self.0.create(name, bytes).await
        }
        async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
            self.0.replace(name, bytes, tag).await
        }
        async fn wait(&self, _: u64) {}
    }

    #[test]
    fn a_write_given_up_on_leaves_nothing_behind() {
        rt().block_on(async {
            let bucket = Stalls::default();
            let mut db = Db::open(&bucket, "p", keys(&[(1, rand::random())])).await.unwrap();
            db.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            // A call that runs out of time, or whose caller goes away, drops its write while the bucket hasn't answered.
            bucket.1.store(true, std::sync::atomic::Ordering::Relaxed);
            {
                let mut write = std::pin::pin!(db.write(&bucket, |c| execute(c, "insert into notes values ('given up')", &[]).map(|_| ())));
                std::future::poll_fn(|cx| std::task::Poll::Ready(assert!(write.as_mut().poll(cx).is_pending()))).await;
            }
            assert!(db.query("select * from notes", &[]).unwrap().is_empty(), "what it wrote isn't read");
            bucket.1.store(false, std::sync::atomic::Ordering::Relaxed);
            note(&mut db, &bucket, "next").await;
            assert_eq!(notes(&db), ["next"]);
        });
    }
}
