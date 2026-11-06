//! Every log of every scope this device holds, kept in SQLite exactly as the entries arrived, in causal order, and how
//! far it has read each scope's entries in the bucket.

use std::path::Path;

use anyhow::{Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};

use crate::entry::{Entry, Hash, Head, Key};

/// Entries other than the node's backup runs and keys, which change nothing a backup needs to send at once.
const NOT_BACKUPS: &str = "not (json_extract(cast(body as text), '$.app') = 'node' and json_extract(cast(body as text), '$.kind') in ('backup', 'backup-alert'))";

pub struct Store {
    db: Connection,
}

/// For each log: (author, log, next seq the holder lacks).
pub type Frontier = Vec<(Key, String, u32)>;

/// What adding an entry found.
enum Added {
    New(Entry),
    /// One its device wrote at a seq it had written another at, or after such a one: kept as the fork's evidence.
    Forked,
    Known,
    /// What it follows hasn't arrived yet.
    Waits,
}

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
               hash blob not null unique, depth integer not null, time integer not null,
               header blob not null, signature blob not null, body blob not null,
               primary key (author, log, seq));
             create index if not exists entries_order on entries (log, depth, hash);
             -- Entries that follow one this device doesn't have yet, kept until it does.
             create table if not exists waiting (hash blob primary key, log text not null, header blob not null, signature blob not null, body blob not null);
             -- The end of each of this device's own logs, kept when a scope is deleted, so a log it holds again
             -- goes on from where it was and never has two entries with one seq.
             create table if not exists heads (log text primary key, seq integer not null, hash blob not null);
             -- Entries a device wrote at a seq of a log it had written another at, and those after them: the evidence
             -- of a fork, which every device holding either side keeps and passes on, so all stop counting it alike.
             create table if not exists forks (
               author blob not null, log text not null, seq integer not null, hash blob primary key, depth integer not null,
               header blob not null, signature blob not null, body blob not null);
             -- The last position of each scope's entries in the bucket this device has read, and the epoch of the last
             -- step it read, which those after may not go below.
             create table if not exists positions (log text primary key, at integer not null, floor integer not null);",
        )?;
        Ok(Store { db })
    }

    /// Adds entries as they arrive, each once everything it follows is here (those that came before and waited too):
    /// → the entries new to this device, in the order they were added, each with its depth, and whether a fork came to
    /// light. One that doesn't follow its log is dropped; one that follows what hasn't arrived waits.
    pub fn take(&self, entries: Vec<Entry>) -> Result<(Vec<Entry>, bool)> {
        let mut pending = self.waiting()?;
        let had = pending.len();
        pending.extend(entries);
        let (mut added, mut forked) = (Vec::new(), false);
        loop {
            let mut left = Vec::new();
            let before = pending.len();
            for e in pending {
                match self.add(&e) {
                    Ok(Added::New(e)) => added.push(e),
                    Ok(Added::Forked) => forked = true,
                    Ok(Added::Known) => {}
                    Ok(Added::Waits) => left.push(e),
                    Err(err) => tracing::debug!("dropped entry {}: {err:#}", e.id()),
                }
            }
            pending = left;
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        if had > 0 || !pending.is_empty() {
            self.db.execute("delete from waiting", [])?;
            for e in &pending {
                let (h, s, b) = e.encode();
                self.db.execute("insert or ignore into waiting values (?, ?, ?, ?, ?)", params![e.hash().as_bytes().to_vec(), e.log(), h, s, b])?;
            }
        }
        Ok((added, forked))
    }

    fn waiting(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.db.prepare("select header, signature, body from waiting")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?)))?;
        rows.map(|r| {
            let (h, s, b) = r?;
            Entry::decode_trusted(&h, &s, &b, 0)
        })
        .collect()
    }

    /// Adds an entry if it's new and everything it follows is here; or, at a seq its device wrote another at (or after
    /// one such), as a fork's evidence.
    fn add(&self, entry: &Entry) -> Result<Added> {
        let (author, log, seq) = (entry.author(), entry.log(), entry.seq());
        let hash = entry.hash().as_bytes().to_vec();
        let held: Option<Vec<u8>> = self.db.query_row("select hash from entries where author = ? and log = ? and seq = ?", params![author, log, seq], |r| r.get(0)).optional()?;
        let in_forks = self.db.query_row("select 1 from forks where hash = ?", [&hash], |_| Ok(())).optional()?.is_some();
        if held.as_ref() == Some(&hash) || in_forks {
            return Ok(Added::Known);
        }
        let mut fork = held.is_some();
        if seq > 0 && !fork {
            match self.get(&author, log, seq - 1)? {
                Some(prev) if entry.backlink() == Some(prev.hash()) => {}
                // After the other side of a fork.
                Some(_) if entry.backlink().is_some_and(|b| self.in_forks(&b, &author, log, seq - 1).unwrap_or(false)) => fork = true,
                Some(prev) => entry.follows(Some(prev.head()))?,
                None => return Ok(Added::Waits),
            }
        }
        let Some(depth) = self.depth_after(entry)? else { return Ok(Added::Waits) };
        if fork {
            let (header, signature, body) = entry.encode();
            self.db.execute("insert or ignore into forks values (?, ?, ?, ?, ?, ?, ?, ?)", params![author, log, seq, hash, depth as i64, header, signature, body])?;
            tracing::warn!("device {} wrote two entries at {seq} of {log}: it's treated as removed from there", crate::entry::short(&author));
            return Ok(Added::Forked);
        }
        let mut e = entry.clone();
        e.depth = depth;
        self.put(&e)?;
        Ok(Added::New(e))
    }

    fn in_forks(&self, hash: &Hash, author: &Key, log: &str, seq: u32) -> Result<bool> {
        Ok(self.db.query_row("select 1 from forks where hash = ? and author = ? and log = ? and seq = ?", params![hash.as_bytes().to_vec(), author, log, seq], |_| Ok(())).optional()?.is_some())
    }

    /// An entry's depth, once everything it follows is here (forks' evidence too): None while it isn't.
    fn depth_after(&self, entry: &Entry) -> Result<Option<u64>> {
        let mut depth = 0;
        for hash in entry.follows_all() {
            let h = hash.as_bytes().to_vec();
            let found: Option<(String, u64)> = match self.db.query_row("select log, depth from entries where hash = ?", [&h], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))).optional()? {
                Some(f) => Some(f),
                None => self.db.query_row("select log, depth from forks where hash = ?", [&h], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))).optional()?,
            };
            match found {
                Some((log, d)) => {
                    ensure!(log == entry.log() || log == "global", "it follows an entry of another scope");
                    depth = depth.max(d + 1);
                }
                None => return Ok(None),
            }
        }
        Ok(Some(depth))
    }

    /// A log's forks' evidence, for its copies: so a device that reads only a copy finds the fork too.
    pub fn forked(&self, log: &str) -> Result<Vec<Entry>> {
        self.query("select header, signature, body, depth from forks where log = ? order by depth, hash", [log])
    }

    /// Adds an entry this device just wrote after `head`, the end of its log, which it may no longer hold.
    pub fn insert_own(&self, entry: &Entry, head: Option<Head>) -> Result<Entry> {
        entry.follows(head)?;
        let e = match self.add(entry)? {
            Added::New(e) => e,
            Added::Known | Added::Forked => bail!("this device wrote entry {} of {} already", entry.seq(), entry.log()),
            Added::Waits => bail!("this device doesn't hold what entry {} of {} follows", entry.seq(), entry.log()),
        };
        self.db.execute("insert or replace into heads values (?, ?, ?)", params![e.log(), e.seq(), e.hash().as_bytes().to_vec()])?;
        Ok(e)
    }

    fn put(&self, entry: &Entry) -> Result<()> {
        let (header, signature, body) = entry.encode();
        self.db.execute(
            "insert into entries values (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![entry.author(), entry.log(), entry.seq(), entry.hash().as_bytes().to_vec(), entry.depth as i64, entry.time() as i64, header, signature, body],
        )?;
        Ok(())
    }

    /// The end of this device's own log for `log`, whether or not it holds the entries.
    pub fn own_head(&self, author: &Key, log: &str) -> Result<Option<Head>> {
        let held = self.last(author, log)?.map(|e| e.head());
        let row = self.db.query_row("select seq, hash from heads where log = ?", [log], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Vec<u8>>(1)?))).optional()?;
        let kept = match row {
            Some((seq, hash)) => Some((seq, Hash::from_bytes(hash.try_into().map_err(|_| anyhow::anyhow!("bad head"))?))),
            None => None,
        };
        Ok(held.into_iter().chain(kept).max_by_key(|h| h.0))
    }

    /// The newest entry of each author's log of `log`, but `except`'s: what an entry `except` writes there next follows.
    pub fn heads(&self, log: &str, except: Option<&Key>) -> Result<Vec<Hash>> {
        let mut stmt = self.db.prepare("select author, hash from entries e where log = ?1 and seq = (select max(seq) from entries where author = e.author and log = ?1)")?;
        let rows: Vec<(Key, Vec<u8>)> = stmt.query_map([log], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        Ok(rows.into_iter().filter(|(a, _)| Some(a) != except).filter_map(|(_, h)| Some(Hash::from_bytes(h.try_into().ok()?))).collect())
    }

    /// Whether this device holds an entry, or holds it as a fork's evidence.
    pub fn has_hash(&self, hash: &Hash) -> Result<bool> {
        let h = hash.as_bytes().to_vec();
        Ok(self.db.query_row("select 1 from entries where hash = ?1 union all select 1 from forks where hash = ?1", [h], |_| Ok(())).optional()?.is_some())
    }

    pub fn has(&self, author: &Key, log: &str, seq: u32) -> Result<bool> {
        Ok(self.db.query_row("select 1 from entries where author = ? and log = ? and seq = ?", params![author, log, seq], |_| Ok(())).optional()?.is_some())
    }

    pub fn get(&self, author: &Key, log: &str, seq: u32) -> Result<Option<Entry>> {
        Ok(self.query("select header, signature, body, depth from entries where author = ? and log = ? and seq = ?", params![author, log, seq])?.pop())
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
        let rows = stmt.query_map(params, |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, i64>(3)?)))?;
        rows.map(|r| {
            let (h, s, b, d) = r?;
            Entry::decode_trusted(&h, &s, &b, d as u64)
        })
        .collect()
    }

    /// A scope's entries, in replay order.
    pub fn scope(&self, log: &str) -> Result<Vec<Entry>> {
        self.query("select header, signature, body, depth from entries where log = ? order by depth, hash", [log])
    }

    /// Every entry, in replay order.
    pub fn ordered(&self) -> Result<Vec<Entry>> {
        self.query("select header, signature, body, depth from entries order by depth, hash", [])
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
        self.db.execute("delete from forks where log = ?", [log])?;
        self.db.execute("delete from waiting where log = ?", [log])?;
        self.db.execute("delete from positions where log = ?", [log])?;
        Ok(())
    }

    /// The last position of a scope's entries in the bucket this device has read, and the epoch of the last step read.
    pub fn position(&self, log: &str) -> Result<(u64, u32)> {
        Ok(self.db.query_row("select at, floor from positions where log = ?", [log], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, u32>(1)?))).optional()?.unwrap_or((0, 0)))
    }

    pub fn set_position(&self, log: &str, at: u64, floor: u32) -> Result<()> {
        self.db.execute("insert or replace into positions values (?, ?, ?)", params![log, at as i64, floor])?;
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

    pub fn count(&self) -> Result<u64> {
        Ok(self.db.query_row("select count(*) from entries", [], |r| r.get::<_, i64>(0))? as u64)
    }
}
