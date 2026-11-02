//! A scope's entries in the bucket: the node's `log` of the scope, kept as an app's database is (`src/db.rs`). Each
//! write is a step at the first free position, a pack of its entries, sealed; and a newest hint and now and then a
//! copy of every entry in the steps before it, so a node that holds nothing (the serverless node) reads one copy and
//! the few steps after it, not every step.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m}, as a database's
//! <prefix>/<n>               step n, from 1: a pack of entries
//! <prefix>/copy/<n>          a pack of every entry in steps 1 to n (and perhaps some after, which readers already have)
//! ```
//!
//! No clock, disk or network: the bucket is all it reads and writes.

use std::collections::BTreeSet;

use anyhow::{Context, Result};

use crate::bucket::Bucket;
use crate::db::{COPY_EVERY, copy_name, newest, newest_name, open as unseal, seal, step_name};
use crate::entry::{Entry, Key, pack, unpack};

/// Where a scope's entries are in the bucket: the node's (`members::APP`) database `log`.
pub fn prefix(scope: &str) -> String {
    format!("{}/{}/{}", crate::bucket::folder(scope), crate::members::APP, NAME)
}

/// The node's database of a scope's entries, under its own name: its key is that database's key.
pub const NAME: &str = "log";

/// The entries in the steps after `from` of `log`'s entries at `prefix`: (the last step there, the entries). A step
/// that doesn't open, or holds another scope's entries, is passed over.
pub async fn read(bucket: &impl Bucket, key: &[u8; 32], prefix: &str, log: &str, from: u64) -> Result<(u64, Vec<Entry>)> {
    let (mut at, mut entries) = (from, Vec::new());
    while let Some(o) = bucket.get(&step_name(prefix, at + 1)).await? {
        at += 1;
        entries.extend(step(key, &step_name(prefix, at), &o.bytes, log));
    }
    Ok((at, entries))
}

/// Every one of `log`'s entries: its newest copy's and those in the steps after it, each once. → (the last step, the
/// entries).
pub async fn open(bucket: &impl Bucket, key: &[u8; 32], prefix: &str, log: &str) -> Result<(u64, Vec<Entry>)> {
    let copy = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes)?.1,
        None => 0,
    };
    let mut entries = Vec::new();
    if copy > 0 {
        let name = copy_name(prefix, copy);
        let o = bucket.get(&name).await?.with_context(|| format!("{name} is missing"))?;
        entries = step(key, &name, &o.bytes, log);
    }
    let (at, after) = read(bucket, key, prefix, log, copy).await?;
    let mut seen: BTreeSet<(Key, u32)> = entries.iter().map(|e| (e.author(), e.seq())).collect();
    entries.extend(after.into_iter().filter(|e| seen.insert((e.author(), e.seq()))));
    Ok((at, entries))
}

fn step(key: &[u8; 32], name: &str, sealed: &[u8], log: &str) -> Vec<Entry> {
    match unseal(key, name, sealed).and_then(|b| unpack(&b)) {
        Ok(list) if list.iter().all(|e| e.log() == log) => list,
        Ok(_) => {
            tracing::debug!("{name} is another scope's");
            Vec::new()
        }
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Vec::new()
        }
    }
}

/// Once a server has read the steps up to `at`: a copy of `entries` (every entry in them) if steps have piled up since
/// the newest, and the newest hint moved on. Both only save readers work.
pub async fn hint(bucket: &impl Bucket, key: &[u8; 32], prefix: &str, at: u64, entries: impl FnOnce() -> Result<Vec<Entry>>) -> Result<()> {
    let copy = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes)?.1,
        None => 0,
    };
    let copy = if at >= copy + COPY_EVERY {
        let name = copy_name(prefix, at);
        let entries = entries()?;
        bucket.create(&name, &seal(key, &name, &pack(&entries.iter().collect::<Vec<_>>()))).await?;
        at
    } else {
        copy
    };
    crate::db::hint(bucket, prefix, at, copy).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::Memory;
    use crate::entry::{Content, Ext};
    use p2panda_core::SigningKey;

    #[test]
    fn a_reader_from_the_newest_copy_gets_every_entry_once() {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let (bucket, key, server) = (Memory::default(), [7u8; 32], SigningKey::from([1u8; 32]));
            let prefix = prefix("global");
            let (mut written, mut head) = (Vec::new(), None);
            for n in 1..=COPY_EVERY + 20 {
                let ext = Ext { log: "global".into(), time: n, user: "a000000000000001".into(), by: None };
                let entry = Entry::create(&server, ext, head, Content::new("node", "setting", serde_json::json!({ "name": "n", "value": n.to_string() })));
                head = Some(entry.head());
                let name = step_name(&prefix, n);
                assert!(bucket.create(&name, &seal(&key, &name, &pack(&[&entry]))).await.unwrap());
                written.push(entry);
                if n == COPY_EVERY + 5 {
                    // A server that has read the step before, and holds this one's entry, which it just wrote.
                    let held = written.clone();
                    hint(&bucket, &key, &prefix, n - 1, || Ok(held)).await.unwrap();
                }
            }
            assert!(bucket.get(&copy_name(&prefix, COPY_EVERY + 4)).await.unwrap().is_some());
            let (at, read) = open(&bucket, &key, &prefix, "global").await.unwrap();
            assert_eq!(at, COPY_EVERY + 20);
            let seqs: Vec<u32> = read.iter().map(|e| e.seq()).collect();
            assert_eq!(seqs, (0..(COPY_EVERY + 20) as u32).collect::<Vec<_>>(), "every entry, once, in order");
        });
    }
}
