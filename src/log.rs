//! A scope's entries in the bucket: the node's `log` of the scope, kept as an app's database is (`src/db.rs`). Each
//! write is a step at the first free position, a pack of its entries, sealed under the scope's newest epoch; and a
//! newest hint and now and then a copy of every entry in the steps before it, so a node that holds nothing (the
//! serverless node) reads one copy and the few steps after it, not every step. As in a database, a step under an
//! epoch lower than the step before it, or that doesn't open, is passed over, one under an epoch above every key the
//! reader has stops it, and a copy is used only if it holds (`db::start`).
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
use crate::db::{Ahead, COPY_EVERY, Keys, Missing, copy_name, newest, newest_name, step_name};
use crate::entry::{Entry, pack, unpack};

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

/// What reading a log's steps came to.
#[derive(Debug, Default)]
pub struct Read {
    /// The last step read or passed over.
    pub at: u64,
    pub entries: Vec<Entry>,
    /// The epoch of the last step read, which those after it may not go below.
    pub floor: u32,
    /// Where it stopped, before the end: a step under an epoch above every key it was given (`db::Ahead`).
    pub ahead: Option<Ahead>,
}

/// The entries in the steps after `from` of `log`'s entries at `prefix`, from `floor`, the epoch of the last step
/// read before. A step that doesn't open, is under an epoch lower than the one before it, or holds another scope's
/// entries, is passed over; one under an epoch above every key in `keys` stops it there (`Read::ahead`); one under an
/// epoch below that `keys` lacks fails with `Missing`.
pub async fn read(bucket: &impl Bucket, keys: &Keys, prefix: &str, log: &str, from: u64, floor: u32) -> Result<Read> {
    let mut read = Read { at: from, floor, ..Default::default() };
    while let Some(o) = bucket.get(&step_name(prefix, read.at + 1)).await? {
        match step(keys, &step_name(prefix, read.at + 1), &o.bytes, log, read.floor)? {
            Step::Took(epoch, list) => {
                read.entries.extend(list);
                read.floor = epoch;
            }
            Step::Passed => {}
            Step::Ahead(epoch) => {
                read.ahead = Some(Ahead { epoch, at: read.at + 1 });
                break;
            }
        }
        read.at += 1;
    }
    Ok(read)
}

/// Every one of `log`'s entries: its newest copy's, if it holds (`db::start`), and those in the steps after it, each
/// once.
pub async fn open(bucket: &impl Bucket, keys: &Keys, prefix: &str, log: &str) -> Result<Read> {
    let copied = match crate::db::start(bucket, prefix, keys).await? {
        Some((copy, epoch, bytes)) => match unpack(&bytes) {
            Ok(list) if list.iter().all(|e| e.log() == log) => Some((copy, epoch, list)),
            _ => None,
        },
        None => None,
    };
    let (copy, floor, entries) = copied.unwrap_or_default();
    let mut read = read(bucket, keys, prefix, log, copy, floor).await?;
    let mut seen: BTreeSet<crate::entry::Hash> = entries.iter().map(Entry::hash).collect();
    let after = std::mem::take(&mut read.entries);
    read.entries = entries;
    read.entries.extend(after.into_iter().filter(|e| seen.insert(e.hash())));
    Ok(read)
}

enum Step {
    Took(u32, Vec<Entry>),
    Passed,
    Ahead(u32),
}

/// A step's entries and epoch, after `floor`.
fn step(keys: &Keys, name: &str, sealed: &[u8], log: &str, floor: u32) -> Result<Step> {
    let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Ok(Step::Passed) };
    if epoch < floor {
        tracing::debug!("{name} is under epoch {epoch}, after {floor}: passed over");
        return Ok(Step::Passed);
    }
    if keys.keys().next_back().is_none_or(|newest| epoch > *newest) {
        return Ok(Step::Ahead(epoch));
    }
    let key = keys.get(&epoch).ok_or(Missing { epoch })?;
    match crate::keys::open(key, name, sealed).and_then(|b| unpack(&b)) {
        Ok(list) if list.iter().all(|e| e.log() == log) => Ok(Step::Took(epoch, list)),
        Ok(_) => {
            tracing::debug!("{name} is another scope's");
            Ok(Step::Passed)
        }
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Ok(Step::Passed)
        }
    }
}

/// Once a device has read the steps up to `at`: a copy of `entries` (every entry in them) under the newest epoch
/// `keys` has, if steps have piled up since the newest, and the newest hint moved on. Both only save readers work.
pub async fn hint(bucket: &impl Bucket, keys: &Keys, prefix: &str, at: u64, entries: impl FnOnce() -> Result<Vec<Entry>>) -> Result<()> {
    let (_, copy, epoch) = bucket.get(&newest_name(prefix)).await?.map_or((0, 0, 0), |o| newest(&o.bytes));
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
            let read = open(&bucket, &key, &prefix, "global").await.unwrap();
            assert_eq!(read.at, COPY_EVERY + 20);
            let seqs: Vec<u32> = read.entries.iter().map(|e| e.seq()).collect();
            assert_eq!(seqs, (0..(COPY_EVERY + 20) as u32).collect::<Vec<_>>(), "every entry, once, in order");
        });
    }
}
