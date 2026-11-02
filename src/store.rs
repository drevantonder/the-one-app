//! Every log of every scope this device holds, kept in SQLite exactly as the entries arrived, and how far it has read
//! each scope's entries in the bucket.

use std::path::Path;

use anyhow::{Result, ensure};
use p2panda_core::Hash;
use rusqlite::{Connection, OptionalExtension, params};

use crate::entry::{Entry, Head, Key};

/// Entries other than the node's backup runs and keys, which change nothing a backup needs to send at once.
const NOT_BACKUPS: &str = "not (json_extract(cast(body as text), '$.app') = 'node' and json_extract(cast(body as text), '$.kind') in ('backup', 'backup-key', 'backup-alert'))";

pub struct Store {
    db: Connection,
}

/// For each log: (author, log, next seq the holder lacks).
pub type Frontier = Vec<(Key, String, u32)>;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch(
            "pragma journal_mode = wal;
             create table if not exists entries (
               author blob not null, log text not null, seq integer not null,
               time integer not null, header blob not null, body blob not null,
               primary key (author, log, seq));
             create index if not exists entries_order on entries (log, time, author, seq);
             -- The end of each of this device's own logs, kept when a scope is deleted, so a log it holds again
             -- goes on from where it was and never has two entries with one seq.
             create table if not exists heads (log text primary key, seq integer not null, hash blob not null, time integer not null);
             -- The last position of each scope's entries in the bucket this device has read.
             create table if not exists positions (log text primary key, at integer not null);",
        )?;
        Ok(Store { db })
    }

    /// Adds an entry if it is new and follows its log. Returns whether it was new.
    pub fn insert(&self, entry: &Entry) -> Result<bool> {
        if self.has(&entry.author(), entry.log(), entry.seq())? {
            return Ok(false);
        }
        let prev = match entry.seq() {
            0 => None,
            n => self.get(&entry.author(), entry.log(), n - 1)?.map(|e| e.head()),
        };
        entry.follows(prev)?;
        self.put(entry)?;
        Ok(true)
    }

    /// Adds an entry this device just wrote after `head`, the end of its log, which it may no longer hold.
    pub fn insert_own(&self, entry: &Entry, head: Option<Head>) -> Result<()> {
        entry.follows(head)?;
        self.put(entry)?;
        self.db.execute(
            "insert or replace into heads values (?, ?, ?, ?)",
            params![entry.log(), entry.seq(), entry.hash().as_bytes().to_vec(), entry.time() as i64],
        )?;
        Ok(())
    }

    fn put(&self, entry: &Entry) -> Result<()> {
        let (header, body) = entry.encode();
        self.db.execute(
            "insert into entries values (?, ?, ?, ?, ?, ?)",
            params![entry.author(), entry.log(), entry.seq(), entry.time() as i64, header, body],
        )?;
        Ok(())
    }

    /// The end of this device's own log for `log`, whether or not it holds the entries.
    pub fn own_head(&self, author: &Key, log: &str) -> Result<Option<Head>> {
        let held = self.last(author, log)?.map(|e| e.head());
        let row = self
            .db
            .query_row("select seq, hash, time from heads where log = ?", [log], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, i64>(2)?)))
            .optional()?;
        let kept = match row {
            Some((seq, hash, time)) => {
                let hash: [u8; 32] = hash.try_into().map_err(|_| anyhow::anyhow!("bad head"))?;
                Some((seq, Hash::from_bytes(hash), time as u64))
            }
            None => None,
        };
        Ok(held.into_iter().chain(kept).max_by_key(|h| h.0))
    }

    /// Whether `entry` can't be added yet because the one before it in its log hasn't arrived.
    pub fn lacks_predecessor(&self, entry: &Entry) -> Result<bool> {
        Ok(entry.seq() > 0 && !self.has(&entry.author(), entry.log(), entry.seq() - 1)?)
    }

    pub fn has(&self, author: &Key, log: &str, seq: u32) -> Result<bool> {
        Ok(self.db.query_row("select 1 from entries where author = ? and log = ? and seq = ?", params![author, log, seq], |_| Ok(())).optional()?.is_some())
    }

    pub fn get(&self, author: &Key, log: &str, seq: u32) -> Result<Option<Entry>> {
        let row = self
            .db
            .query_row(
                "select header, body from entries where author = ? and log = ? and seq = ?",
                params![author, log, seq],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        row.map(|(h, b)| Entry::decode_trusted(&h, &b)).transpose()
    }

    pub fn last(&self, author: &Key, log: &str) -> Result<Option<Entry>> {
        let seq: Option<u32> = self.db.query_row("select max(seq) from entries where author = ? and log = ?", params![author, log], |r| r.get(0))?;
        match seq {
            Some(seq) => self.get(author, log, seq),
            None => Ok(None),
        }
    }

    /// For each log, the first seq this device lacks (a log it holds again may lack its start).
    pub fn frontier(&self) -> Result<Frontier> {
        let mut stmt = self.db.prepare("select author, log, count(*), max(seq) from entries group by author, log")?;
        let rows: Vec<(Key, String, u32, u32)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for (author, log, count, max) in rows {
            let next = if count == max + 1 {
                max + 1
            } else {
                let mut next = 0;
                while self.has(&author, &log, next)? {
                    next += 1;
                }
                next
            };
            out.push((author, log, next));
        }
        Ok(out)
    }

    fn query(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Entry>> {
        let mut stmt = self.db.prepare(sql)?;
        let rows = stmt.query_map(params, |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        rows.map(|r| {
            let (h, b) = r?;
            Entry::decode_trusted(&h, &b)
        })
        .collect()
    }

    /// A scope's entries, in replay order.
    pub fn scope(&self, log: &str) -> Result<Vec<Entry>> {
        self.query("select header, body from entries where log = ? order by time, author, log, seq", [log])
    }

    /// Every entry, in replay order.
    pub fn ordered(&self) -> Result<Vec<Entry>> {
        self.query("select header, body from entries order by time, author, log, seq", [])
    }

    /// The scopes this device has entries in.
    pub fn logs(&self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("select distinct log from entries order by log")?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
    }

    /// Deletes every entry of a scope.
    pub fn delete(&self, log: &str) -> Result<()> {
        ensure!(log != "global", "every device holds global");
        self.db.execute("delete from entries where log = ?", [log])?;
        self.db.execute("delete from positions where log = ?", [log])?;
        Ok(())
    }

    /// The last position of a scope's entries in the bucket this device has read.
    pub fn position(&self, log: &str) -> Result<u64> {
        Ok(self.db.query_row("select at from positions where log = ?", [log], |r| r.get::<_, i64>(0)).optional()?.unwrap_or(0) as u64)
    }

    pub fn set_position(&self, log: &str, at: u64) -> Result<()> {
        self.db.execute("insert or replace into positions values (?, ?)", params![log, at as i64])?;
        Ok(())
    }

    /// Whether a scope has entries past `frontier` (each author's next seq, by key) other than backups' own.
    pub fn changed_since(&self, log: &str, frontier: &std::collections::BTreeMap<String, u32>) -> Result<bool> {
        let mut stmt = self.db.prepare(&format!("select author, max(seq) from entries where log = ? and {NOT_BACKUPS} group by author"))?;
        let rows: Vec<(Key, u32)> = stmt.query_map([log], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows.iter().any(|(a, max)| frontier.get(&crate::entry::hex(a)).is_none_or(|next| max >= next)))
    }

    /// When the first of a scope's entries after `time` was written, other than backups' own.
    pub fn first_change(&self, log: &str, time: u64) -> Result<Option<u64>> {
        let t: Option<i64> = self.db.query_row(&format!("select min(time) from entries where log = ? and time > ? and {NOT_BACKUPS}"), params![log, time as i64], |r| r.get(0))?;
        Ok(t.map(|t| t as u64))
    }

    pub fn max_time(&self) -> Result<u64> {
        let t: Option<i64> = self.db.query_row("select max(time) from entries", [], |r| r.get(0))?;
        let h: Option<i64> = self.db.query_row("select max(time) from heads", [], |r| r.get(0))?;
        Ok(t.max(h).unwrap_or(0) as u64)
    }

    pub fn count(&self) -> Result<u64> {
        Ok(self.db.query_row("select count(*) from entries", [], |r| r.get::<_, i64>(0))? as u64)
    }
}
