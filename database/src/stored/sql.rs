//! A database's SQLite: the one connection an app's SQL runs in, kept inside its own database (`confine`), the
//! changeset each write makes (SQLite's session extension) and applying one, and the state hash every device computes
//! the same.

use std::ffi::{c_char, c_int, c_void};

use anyhow::{Result, bail, ensure};
use rusqlite::Connection;
use rusqlite::types::{Value as Sql, ValueRef};
use serde_json::{Map, Value};

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
pub fn changes<R>(conn: &Connection, f: &mut impl FnMut(&Connection) -> Result<R>) -> Result<(R, Vec<u8>)> {
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
pub struct Open<'a>(&'a mut Connection);

impl<'a> Open<'a> {
    pub fn begin(conn: &'a mut Connection) -> Result<Self> {
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
pub fn apply_changes(conn: &Connection, changes: &[u8]) -> Result<()> {
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

pub fn check(conn: &Connection, rc: c_int) -> Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let message = unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_errmsg(conn.handle())) }.to_string_lossy().to_string();
    bail!("SQLite said {rc}: {message}")
}


/// A database's state, the same on every device that holds the same rows, whatever order its steps came in and however
/// SQLite laid out its pages: BLAKE3, hex, over `one/state`, its schema version, then each table by name but SQLite's
/// own and virtual ones (whose rows are their shadow tables'): its name, its columns' names, and its rows in primary
/// key order, each value with its type. A table without a primary key is its rows by rowid, `rowid` its first column:
/// its steps name its rows by rowid (`changes`), so two devices whose rowids differ hold different databases. A row's bytes are
/// `r`, then each value: `n` for null, `i` and 8 bytes big-endian, `f` and the 8 bytes of its IEEE 754 bits, or `s`
/// (text) or `b` (blob) with a 4-byte big-endian length and its bytes. A table's name is written as text is but with
/// `t`, and each column's name as text.
pub fn state_hash(conn: &Connection) -> Result<String> {
    fn bytes(h: &mut blake3::Hasher, kind: u8, b: &[u8]) {
        h.update(&[kind]);
        h.update(&(b.len() as u32).to_be_bytes());
        h.update(b);
    }
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let mut h = blake3::Hasher::new();
    h.update(b"one/state\0");
    h.update(&conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))?.to_be_bytes());
    let tables: Vec<String> = conn
        .prepare("select name from sqlite_schema where type = 'table' and name not like 'sqlite\\_%' escape '\\' and sql not like 'create virtual table%' order by name")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for table in tables {
        let mut keys: Vec<(i64, String)> = conn.prepare("select pk, name from pragma_table_info(?1) where pk > 0")?.query_map([&table], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        keys.sort();
        let sql = match keys.is_empty() {
            true => format!("select rowid, * from {} order by rowid", quote(&table)),
            false => format!("select * from {} order by {}", quote(&table), keys.iter().map(|(_, k)| quote(k)).collect::<Vec<_>>().join(", ")),
        };
        let mut stmt = conn.prepare(&sql)?;
        bytes(&mut h, b't', table.as_bytes());
        for name in stmt.column_names() {
            bytes(&mut h, b's', name.as_bytes());
        }
        let n = stmt.column_count();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            h.update(b"r");
            for i in 0..n {
                match row.get_ref(i)? {
                    ValueRef::Null => drop(h.update(b"n")),
                    ValueRef::Integer(v) => drop(h.update(b"i").update(&v.to_be_bytes())),
                    ValueRef::Real(v) => drop(h.update(b"f").update(&v.to_bits().to_be_bytes())),
                    ValueRef::Text(t) => bytes(&mut h, b's', t),
                    ValueRef::Blob(b) => bytes(&mut h, b'b', b),
                }
            }
        }
    }
    Ok(h.finalize().to_hex().to_string())
}

/// A copy's bytes as a database of its own, to read: if they're one.
pub fn copied(bytes: &[u8]) -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact("main", bytes, bytes.len(), true)?;
    conn.query_row("select count(*) from sqlite_schema", [], |_| Ok(()))?;
    Ok(conn)
}

/// Keeps an app's SQL inside its own database: every statement it gives (queries, writes and schema changes, which run on
/// every device that reads the database) runs here, and SQLite lets SQL open other files (`attach`, `vacuum into`),
/// load code (`load_extension`), and turn off what keeps a database whole (pragmas). So no database can be attached,
/// which `vacuum into` needs too, and only pragmas that read the schema or set what an app's own data needs are allowed.
pub fn confine(conn: &mut Connection) -> Result<()> {
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

pub fn run(stmt: &mut rusqlite::Statement, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
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

pub fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(*b as i64),
        Value::Number(n) => n.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

pub fn from_sql(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => f.into(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
        ValueRef::Blob(b) => b.iter().map(|b| Value::from(*b)).collect(),
    }
}
