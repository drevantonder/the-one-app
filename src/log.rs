//! A scope's entries in the bucket: the node's `log` of the scope, kept as an app's database is (`src/db.rs`). Each
//! write is a step at the first free position, a pack of its entries, sealed under the scope's newest epoch; and a
//! newest hint and now and then a copy of every entry in the steps before it, so a node that holds nothing (the
//! serverless node) reads one copy and the few steps after it, not every step. As in a database, a step under an
//! epoch lower than the step before it, or that doesn't open, is passed over.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m, "epoch": e}, as a database's
//! <prefix>/<n>               step n, from 1: a pack of entries
//! <prefix>/copy/<n>          a pack of every entry in steps 1 to n (and perhaps some after, which readers already have)
//! ```
//!
//! No clock, disk or network: the bucket is all it reads and writes.

use std::collections::BTreeSet;

use anyhow::{Context, Result};

use crate::bucket::Bucket;
use crate::db::{COPY_EVERY, Keys, Missing, copy_name, newest, newest_name, step_name};
use crate::entry::{Entry, Key, pack, unpack};

/// Where a scope's entries are in the bucket: the node's (`members::APP`) database `log`.
pub fn prefix(scope: &str) -> String {
    format!("{}/{}/{}", crate::bucket::folder(scope), crate::members::APP, NAME)
}

/// The node's database of a scope's entries, under its own name: its key is that database's key.
pub const NAME: &str = "log";

/// Seals a step or copy of entries under the newest epoch `keys` has.
pub fn seal(keys: &Keys, name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let (epoch, key) = keys.last_key_value().with_context(|| format!("no key to write {name} with"))?;
    Ok(crate::keys::seal(*epoch, key, name, bytes))
}

/// The entries in the steps after `from` of `log`'s entries at `prefix`: (the last step there, the entries). A step
/// that doesn't open, is under an epoch lower than the one before it, or holds another scope's entries, is passed
/// over; one under an epoch `keys` lacks stops it with `Missing`.
pub async fn read(bucket: &impl Bucket, keys: &Keys, prefix: &str, log: &str, from: u64) -> Result<(u64, Vec<Entry>)> {
    // The epoch of the step read last, which those after it may not go below.
    let mut floor = 0;
    if from > 0
        && let Some(o) = bucket.get(&step_name(prefix, from)).await?
    {
        floor = step(keys, &step_name(prefix, from), &o.bytes, log, &mut 0)?.map_or(0, |(epoch, _)| epoch);
    }
    read_from(bucket, keys, prefix, log, from, &mut floor).await
}

async fn read_from(bucket: &impl Bucket, keys: &Keys, prefix: &str, log: &str, from: u64, floor: &mut u32) -> Result<(u64, Vec<Entry>)> {
    let (mut at, mut entries) = (from, Vec::new());
    while let Some(o) = bucket.get(&step_name(prefix, at + 1)).await? {
        if let Some((_, list)) = step(keys, &step_name(prefix, at + 1), &o.bytes, log, floor)? {
            entries.extend(list);
        }
        at += 1;
    }
    Ok((at, entries))
}

/// Every one of `log`'s entries: its newest copy's and those in the steps after it, each once. → (the last step, the
/// entries).
pub async fn open(bucket: &impl Bucket, keys: &Keys, prefix: &str, log: &str) -> Result<(u64, Vec<Entry>)> {
    let copy = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes)?.1,
        None => 0,
    };
    let (mut entries, mut floor) = (Vec::new(), 0);
    if copy > 0 {
        let name = copy_name(prefix, copy);
        let o = bucket.get(&name).await?.with_context(|| format!("{name} is missing"))?;
        entries = step(keys, &name, &o.bytes, log, &mut floor)?.map(|(_, list)| list).unwrap_or_default();
    }
    let (at, after) = read_from(bucket, keys, prefix, log, copy, &mut floor).await?;
    let mut seen: BTreeSet<(Key, u32)> = entries.iter().map(|e| (e.author(), e.seq())).collect();
    entries.extend(after.into_iter().filter(|e| seen.insert((e.author(), e.seq()))));
    Ok((at, entries))
}

/// A step or copy's entries and epoch, moving `floor` up to it; None for one passed over.
fn step(keys: &Keys, name: &str, sealed: &[u8], log: &str, floor: &mut u32) -> Result<Option<(u32, Vec<Entry>)>> {
    let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Ok(None) };
    if epoch < *floor {
        tracing::debug!("{name} is under epoch {epoch}, after {floor}: passed over");
        return Ok(None);
    }
    let key = keys.get(&epoch).ok_or(Missing { epoch })?;
    match crate::keys::open(key, name, sealed).and_then(|b| unpack(&b)) {
        Ok(list) if list.iter().all(|e| e.log() == log) => {
            *floor = epoch;
            Ok(Some((epoch, list)))
        }
        Ok(_) => {
            tracing::debug!("{name} is another scope's");
            Ok(None)
        }
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Ok(None)
        }
    }
}

/// Once a device has read the steps up to `at`: a copy of `entries` (every entry in them) under the newest epoch
/// `keys` has, if steps have piled up since the newest, and the newest hint moved on. Both only save readers work.
pub async fn hint(bucket: &impl Bucket, keys: &Keys, prefix: &str, at: u64, entries: impl FnOnce() -> Result<Vec<Entry>>) -> Result<()> {
    let (copy, epoch) = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes).map(|(_, copy, epoch)| (copy, epoch))?,
        None => (0, 0),
    };
    let (copy, epoch) = if at >= copy + COPY_EVERY {
        let name = copy_name(prefix, at);
        let entries = entries()?;
        bucket.create(&name, &seal(keys, &name, &pack(&entries.iter().collect::<Vec<_>>()))?).await?;
        (at, keys.last_key_value().map_or(0, |(e, _)| *e))
    } else {
        (copy, epoch)
    };
    crate::db::hint(bucket, prefix, at, copy, epoch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::Memory;
    use crate::entry::{Content, Ext};
    use ed25519_dalek::SigningKey;

    #[test]
    fn a_reader_from_the_newest_copy_gets_every_entry_once() {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let (bucket, key, device) = (Memory::default(), Keys::from([(1, [7u8; 32])]), SigningKey::from_bytes(&[1u8; 32]));
            let prefix = prefix("global");
            let (mut written, mut head) = (Vec::new(), None);
            for n in 1..=COPY_EVERY + 20 {
                let ext = Ext { log: "global".into(), time: n, user: "a000000000000001".into(), epoch: 0, previous: Vec::new() };
                let entry = Entry::create(&device, ext, head, Content::new("node", "setting", serde_json::json!({ "name": "n", "value": n.to_string() })));
                head = Some(entry.head());
                let name = step_name(&prefix, n);
                assert!(bucket.create(&name, &seal(&key, &name, &pack(&[&entry])).unwrap()).await.unwrap());
                written.push(entry);
                if n == COPY_EVERY + 5 {
                    // A device that has read the step before, and holds this one's entry, which it just wrote.
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
