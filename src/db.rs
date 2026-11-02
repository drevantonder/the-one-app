//! App databases: an app's data in a scope is one SQLite database, the truth, kept in the bucket as steps at numbered
//! positions, each sealed: the changeset of one write (SQLite's session extension), or one of the app's schema
//! changes. Reading it is its newest copy and the steps after; writing it is a step created only if its position is
//! free, so when two servers write at once, one wins and the other catches up and runs its write again.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m}, replaced while unchanged: a hint, so readers needn't list
//! <prefix>/<n>               step n, from 1
//! <prefix>/copy/<n>          the whole database after step n
//! ```
//!
//! No clock, disk or network: the bucket is all it reads and writes, so it runs wherever SQLite does.

use std::ffi::{c_char, c_int, c_void};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, bail, ensure};
use rusqlite::Connection;
use rusqlite::types::{Value as Sql, ValueRef};
use serde_json::{Map, Value, json};

use crate::bucket::Bucket;

/// A copy is written once this many steps follow the last one.
const COPY_EVERY: u64 = 100;
/// How many times a write runs again after losing a race before it gives up.
const TRIES: usize = 50;

/// An app's key in a scope, from the scope's: what a server is given to reach only that app's data there. The kernel's
/// vault derives the same (`kernel/shared/crypto.js`).
pub fn app_key(scope_key: &[u8; 32], app: &str) -> [u8; 32] {
    hkdf(scope_key, format!("one/app\0{app}").as_bytes())
}

/// A database's key, from its app's.
pub fn database_key(app_key: &[u8; 32], database: &str) -> [u8; 32] {
    hkdf(app_key, format!("one/database\0{database}").as_bytes())
}

/// HKDF-SHA256 with no salt, as WebCrypto in workerd has it.
fn hkdf(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(None, ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

/// Seals an object under its key and name, so it can't be read, changed, or passed off as another object: → a random
/// nonce ‖ the ciphertext.
pub fn seal(key: &[u8; 32], name: &str, bytes: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(Aes256Gcm::new(key.into()).encrypt(&nonce.into(), Payload { msg: bytes, aad: name.as_bytes() }).expect("encrypts"));
    out
}

pub fn open(key: &[u8; 32], name: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 12 + 16, "{name} is too short to be sealed");
    let nonce: [u8; 12] = sealed[..12].try_into().expect("12 bytes");
    Aes256Gcm::new(key.into()).decrypt(&nonce.into(), Payload { msg: &sealed[12..], aad: name.as_bytes() }).map_err(|_| anyhow!("{name} didn't open with its key"))
}

/// One SQLite database of an app's, at a position in its steps.
pub struct Db {
    prefix: String,
    key: [u8; 32],
    conn: Connection,
    /// The last step applied.
    at: u64,
    /// The newest copy this server knows of.
    copy: u64,
}

impl Db {
    /// Reads a database as its newest steps leave it: empty if it has none.
    pub async fn open(bucket: &impl Bucket, prefix: &str, key: [u8; 32]) -> Result<Db> {
        let (at, copy) = match bucket.get(&format!("{prefix}/newest")).await? {
            Some(o) => newest(&o.bytes)?,
            None => (0, 0),
        };
        let mut conn = Connection::open_in_memory()?;
        if copy > 0 {
            let name = format!("{prefix}/copy/{copy:012}");
            let sealed = bucket.get(&name).await?.with_context(|| format!("{name} is missing"))?;
            let bytes = open(&key, &name, &sealed.bytes)?;
            conn.deserialize_read_exact("main", &bytes[..], bytes.len(), false)?;
        }
        let mut db = Db { prefix: prefix.into(), key, conn, at: copy, copy };
        db.catch_up(bucket).await?;
        ensure!(db.at >= at, "{prefix}/newest says step {at} was written, but it's missing");
        Ok(db)
    }

    pub fn at(&self) -> u64 {
        self.at
    }

    /// Applies the steps written since this server last read: whether there were any.
    pub async fn catch_up(&mut self, bucket: &impl Bucket) -> Result<bool> {
        let from = self.at;
        loop {
            let name = self.step(self.at + 1);
            let Some(o) = bucket.get(&name).await? else { break };
            self.apply(&open(&self.key, &name, &o.bytes)?)?;
            self.at += 1;
        }
        Ok(self.at > from)
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
            if bucket.create(&self.step(self.at + 1), &seal(&self.key, &self.step(self.at + 1), &step)).await? {
                self.apply(&step)?;
                self.at += 1;
                self.written(bucket).await?;
            }
        }
        bail!("{} changed too often to change its schema", self.prefix)
    }

    /// The schema changes this database has had.
    pub fn version(&self) -> Result<usize> {
        Ok(self.conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Runs a write in one transaction, which is the step written after this server's newest: when another server
    /// wrote that first, it catches up and runs `f` again, so `f` must do only database work.
    pub async fn write<R>(&mut self, bucket: &impl Bucket, mut f: impl FnMut(&Connection) -> Result<R>) -> Result<R> {
        for _ in 0..TRIES {
            self.catch_up(bucket).await?;
            self.conn.execute_batch("begin")?;
            let made = changes(&self.conn, &mut f);
            let (result, changes) = match made {
                Ok(made) => made,
                Err(err) => {
                    self.conn.execute_batch("rollback")?;
                    return Err(err);
                }
            };
            if changes.is_empty() {
                self.conn.execute_batch("commit")?;
                return Ok(result);
            }
            let name = self.step(self.at + 1);
            let step = [b"c".as_slice(), &changes].concat();
            match bucket.create(&name, &seal(&self.key, &name, &step)).await {
                Ok(true) => {
                    self.conn.execute_batch("commit")?;
                    self.at += 1;
                    self.written(bucket).await?;
                    return Ok(result);
                }
                Ok(false) => self.conn.execute_batch("rollback")?,
                Err(err) => {
                    self.conn.execute_batch("rollback")?;
                    return Err(err);
                }
            }
        }
        bail!("{} changed too often to write to", self.prefix)
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
        format!("{}/{n:012}", self.prefix)
    }

    /// After writing a step: a copy, if steps have piled up since the last, and the newest hint moved on.
    async fn written(&mut self, bucket: &impl Bucket) -> Result<()> {
        if self.at >= self.copy + COPY_EVERY {
            let name = format!("{}/copy/{:012}", self.prefix, self.at);
            let bytes = self.conn.serialize("main")?.to_vec();
            bucket.create(&name, &seal(&self.key, &name, &bytes)).await?;
            self.copy = self.at;
        }
        let name = format!("{}/newest", self.prefix);
        for _ in 0..TRIES {
            let current = bucket.get(&name).await?;
            let said = current.as_ref().map(|o| newest(&o.bytes)).transpose()?;
            let (at, copy) = said.unwrap_or((0, 0));
            let (at, copy) = (at.max(self.at), copy.max(self.copy));
            if said == Some((at, copy)) {
                return Ok(());
            }
            if bucket.replace(&name, &serde_json::to_vec(&json!({ "at": at, "copy": copy }))?, current.as_ref().map(|o| o.tag.as_str())).await? {
                return Ok(());
            }
        }
        Ok(())
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
            // A kind of step a newer server writes.
            _ => Ok(()),
        }
    }
}

fn newest(bytes: &[u8]) -> Result<(u64, u64)> {
    let v: Value = serde_json::from_slice(bytes)?;
    Ok((v["at"].as_u64().unwrap_or(0), v["copy"].as_u64().unwrap_or(0)))
}

/// Reads with a statement that only reads.
pub fn query(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    let mut stmt = conn.prepare(sql)?;
    ensure!(stmt.readonly(), "a query only reads");
    run(&mut stmt, params)
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

/// Applies a changeset written from the state this database is in, so nothing should clash; if something does, the
/// change wins where it can, the same way on every server.
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
    use crate::bucket::Object;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// A bucket in memory that can lose a race on purpose.
    #[derive(Default)]
    struct Memory {
        objects: Mutex<BTreeMap<String, Vec<u8>>>,
        /// Names another server takes just before this one writes them.
        taken: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl Bucket for Memory {
        async fn get(&self, name: &str) -> Result<Option<Object>> {
            Ok(self.objects.lock().unwrap().get(name).map(|b| Object { bytes: b.clone(), tag: blake3::hash(b).to_hex().to_string() }))
        }
        async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
            let mut objects = self.objects.lock().unwrap();
            let mut taken = self.taken.lock().unwrap();
            if let Some(i) = taken.iter().position(|(n, _)| n == name) {
                let (n, b) = taken.remove(i);
                objects.insert(n, b);
            }
            if objects.contains_key(name) {
                return Ok(false);
            }
            objects.insert(name.into(), bytes.to_vec());
            Ok(true)
        }
        async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
            let mut objects = self.objects.lock().unwrap();
            let now = objects.get(name).map(|b| blake3::hash(b).to_hex().to_string());
            if now.as_deref() != tag {
                return Ok(false);
            }
            objects.insert(name.into(), bytes.to_vec());
            Ok(true)
        }
    }

    const SCHEMA: &str = "create table counts (name text primary key, n integer not null); create table notes (text text)";

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }

    fn bump(conn: &Connection) -> Result<()> {
        execute(conn, "insert into counts values ('x', 1) on conflict (name) do update set n = n + 1", &[])?;
        Ok(())
    }

    fn count(db: &Db) -> i64 {
        db.query("select n from counts where name = 'x'", &[]).unwrap().first().map_or(0, |r| r["n"].as_i64().unwrap())
    }

    #[test]
    fn two_servers_writing_at_once_lose_nothing() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = rand::random();
            let mut a = Db::open(&bucket, "i/db/global/board", key).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            let mut b = Db::open(&bucket, "i/db/global/board", key).await.unwrap();
            b.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            assert_eq!(b.at(), 1, "b found a's schema and didn't write its own");
            for _ in 0..3 {
                a.write(&bucket, bump).await.unwrap();
                b.write(&bucket, bump).await.unwrap();
            }
            // Another server takes b's next position just before b writes it.
            let mut c = Db::open(&bucket, "i/db/global/board", key).await.unwrap();
            c.write(&bucket, bump).await.unwrap();
            assert_eq!(count(&c), 7);
            a.catch_up(&bucket).await.unwrap();
            b.catch_up(&bucket).await.unwrap();
            assert_eq!((count(&a), count(&b)), (7, 7));
            let next = b.step(b.at() + 1);
            let mut d = Db::open(&bucket, "i/db/global/board", key).await.unwrap();
            let theirs = {
                let made = changes(&d.conn, &mut |c: &Connection| bump(c)).unwrap();
                seal(&key, &next, &[b"c".as_slice(), &made.1].concat())
            };
            bucket.taken.lock().unwrap().push((next, theirs));
            b.write(&bucket, bump).await.unwrap();
            assert_eq!(count(&b), 9, "b ran its write again after theirs");
            d = Db::open(&bucket, "i/db/global/board", key).await.unwrap();
            assert_eq!(count(&d), 9);
        });
    }

    #[test]
    fn a_copy_saves_reading_every_step_and_reads_the_same() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = rand::random();
            let mut a = Db::open(&bucket, "p", key).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for i in 0..(COPY_EVERY + 10) {
                a.write(&bucket, |c| execute(c, "insert into notes values (?)", &[json!(format!("note {i}"))]).map(|_| ())).await.unwrap();
            }
            assert!(bucket.objects.lock().unwrap().contains_key(&format!("p/copy/{COPY_EVERY:012}")));
            // Without the steps the copy holds, it still reads to the end.
            bucket.objects.lock().unwrap().retain(|n, _| !(n.starts_with("p/0") && n.as_str() <= format!("p/{COPY_EVERY:012}").as_str()));
            let b = Db::open(&bucket, "p", key).await.unwrap();
            let n = |db: &Db| db.query("select count(*) as n from notes", &[]).unwrap()[0]["n"].as_i64().unwrap();
            assert_eq!((n(&b), b.at(), b.version().unwrap()), (n(&a), a.at(), 1));
        });
    }

    #[test]
    fn what_the_bucket_keeps_is_sealed_to_its_name() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = rand::random();
            let mut a = Db::open(&bucket, "p", key).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            a.write(&bucket, |c| execute(c, "insert into notes values ('a secret note')", &[]).map(|_| ())).await.unwrap();
            a.write(&bucket, |c| execute(c, "insert into notes values ('another')", &[]).map(|_| ())).await.unwrap();
            for (name, bytes) in bucket.objects.lock().unwrap().iter() {
                assert!(name.ends_with("newest") || !bytes.windows(6).any(|w| w == b"secret" || w == b"create"), "{name} isn't sealed");
            }
            assert!(Db::open(&bucket, "p", rand::random()).await.is_err(), "another key opens nothing");
            // A step moved to another position doesn't open there.
            let mut objects = bucket.objects.lock().unwrap();
            let second = objects.get("p/000000000002").unwrap().clone();
            objects.insert("p/000000000003".into(), second);
            drop(objects);
            assert!(Db::open(&bucket, "p", key).await.is_err());
        });
    }

    #[test]
    fn a_failed_write_writes_nothing_and_queries_only_read() {
        rt().block_on(async {
            let bucket = Memory::default();
            let mut a = Db::open(&bucket, "p", rand::random()).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into(), "alter table notes add column at integer".into()]).await.unwrap();
            assert_eq!((a.at(), a.version().unwrap()), (2, 2));
            assert!(a.write(&bucket, |c| -> Result<()> { bump(c)?; bail!("the call failed") }).await.is_err());
            assert_eq!((count(&a), a.at()), (0, 2));
            assert!(a.query("delete from notes", &[]).is_err());
            assert!(a.migrate(&bucket, &[SCHEMA.into(), "alter table notes add column at integer".into(), "not sql".into()]).await.is_err());
            assert_eq!(a.at(), 2, "a schema that fails isn't written");
            let rows = a.write(&bucket, |c| execute(c, "insert into notes values ('x', 5) returning at", &[])).await.unwrap();
            assert_eq!(rows[0]["at"], 5);
        });
    }
}
