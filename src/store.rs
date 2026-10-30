//! A space's entries, kept in SQLite exactly as they arrived.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};

use crate::entry::{Entry, Key};

pub struct Store {
    db: Connection,
    space: Key,
}

/// For each log: (author, log, next seq the holder lacks).
pub type Frontier = Vec<(Key, String, u32)>;

impl Store {
    pub fn open(path: &Path, space: Key) -> Result<Self> {
        Self::init(Connection::open(path)?, space)
    }

    pub fn memory(space: Key) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, space)
    }

    fn init(db: Connection, space: Key) -> Result<Self> {
        db.execute_batch(
            "pragma journal_mode = wal;
             create table if not exists entries (
               author blob not null, log text not null, seq integer not null,
               time integer not null, header blob not null, body blob not null,
               primary key (author, log, seq));
             create index if not exists entries_order on entries (time, author, seq);",
        )?;
        Ok(Store { db, space })
    }

    pub fn space(&self) -> Key {
        self.space
    }

    /// Adds an entry if it is new and follows its log. Returns whether it was new.
    pub fn insert(&self, entry: &Entry) -> Result<bool> {
        ensure!(entry.space() == self.space, "entry is for another space");
        if self.get(&entry.author(), entry.log(), entry.seq())?.is_some() {
            return Ok(false);
        }
        let prev = match entry.seq() {
            0 => None,
            n => self.get(&entry.author(), entry.log(), n - 1)?,
        };
        entry.follows(prev.as_ref())?;
        let (header, body) = entry.encode();
        self.db.execute(
            "insert into entries values (?, ?, ?, ?, ?, ?)",
            params![entry.author(), entry.log(), entry.seq(), entry.time() as i64, header, body],
        )?;
        Ok(true)
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
        row.map(|(h, b)| Entry::decode(&h, &b)).transpose()
    }

    pub fn last(&self, author: &Key, log: &str) -> Result<Option<Entry>> {
        let seq: Option<u32> = self.db.query_row(
            "select max(seq) from entries where author = ? and log = ?",
            params![author, log],
            |r| r.get(0),
        )?;
        match seq {
            Some(seq) => self.get(author, log, seq),
            None => Ok(None),
        }
    }

    pub fn frontier(&self) -> Result<Frontier> {
        let mut stmt = self.db.prepare("select author, log, max(seq) + 1 from entries group by author, log")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Key>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every entry the holder of `theirs` lacks, in log order.
    pub fn missing(&self, theirs: &Frontier) -> Result<Vec<Entry>> {
        let have: HashMap<(Key, String), u32> = theirs.iter().map(|(a, l, n)| ((*a, l.clone()), *n)).collect();
        let mut out = Vec::new();
        for (author, log, next) in self.frontier()? {
            let from = have.get(&(author, log.clone())).copied().unwrap_or(0);
            if from < next {
                out.extend(self.range(&author, &log, from)?);
            }
        }
        Ok(out)
    }

    fn range(&self, author: &Key, log: &str, from: u32) -> Result<Vec<Entry>> {
        let mut stmt =
            self.db.prepare("select header, body from entries where author = ? and log = ? and seq >= ? order by seq")?;
        let rows = stmt.query_map(params![author, log, from], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        rows.map(|r| {
            let (h, b) = r?;
            Entry::decode(&h, &b)
        })
        .collect()
    }

    /// Every entry, in fold order.
    pub fn ordered(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.db.prepare("select header, body from entries order by time, author, seq")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        rows.map(|r| {
            let (h, b) = r?;
            Entry::decode(&h, &b)
        })
        .collect()
    }

    pub fn max_time(&self) -> Result<u64> {
        let t: Option<i64> = self.db.query_row("select max(time) from entries", [], |r| r.get(0))?;
        Ok(t.unwrap_or(0) as u64)
    }

    pub fn count(&self) -> Result<u64> {
        Ok(self.db.query_row("select count(*) from entries", [], |r| r.get::<_, i64>(0))? as u64)
    }
}
