//! Sync: what two devices say to each other. See SPEC.md, "Sync".
//! The protocol has no clock and no network of its own; `node.rs` carries it over iroh and
//! `tests/sim.rs` over a simulated network.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::entry::{Entry, Key};
use crate::store::{Frontier, Store};

pub const ALPN: &[u8] = b"one/sync/1";
const MAX_MESSAGE: usize = 16 << 20;
/// Missing entries go in batches this big, so no message nears MAX_MESSAGE.
const BATCH: usize = 200;
/// Entries timed further ahead than this are refused: a wrong clock mustn't drag everyone's forward.
pub const FUTURE: u64 = 3_600_000;

pub type Wire = (Vec<u8>, Vec<u8>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Msg {
    Hello { space: Key, frontier: Frontier },
    Entries { space: Key, entries: Vec<Wire> },
}

pub fn hello(store: &Store) -> Result<Msg> {
    Ok(Msg::Hello { space: store.space(), frontier: store.frontier()? })
}

pub fn entries(space: Key, entries: &[Entry]) -> Msg {
    Msg::Entries { space, entries: entries.iter().map(Entry::encode).collect() }
}

/// Handles one message at time `now`. Returns the entries that were new to `store`, and replies to send.
pub fn handle(store: &Store, msg: Msg, now: u64) -> Result<(Vec<Entry>, Vec<Msg>)> {
    match msg {
        Msg::Hello { space, frontier } if space == store.space() => {
            let missing = store.missing(&frontier)?;
            Ok((Vec::new(), missing.chunks(BATCH).map(|batch| entries(space, batch)).collect()))
        }
        Msg::Entries { space, entries } if space == store.space() => {
            let (mut new, mut gap) = (Vec::new(), false);
            for (header, body) in entries {
                let entry = match Entry::decode(&header, &body) {
                    Ok(e) if e.time() <= now.saturating_add(FUTURE) => e,
                    Ok(_) => {
                        tracing::debug!("dropped an entry from the future");
                        continue;
                    }
                    Err(err) => {
                        tracing::debug!("dropped entry: {err:#}");
                        continue;
                    }
                };
                match store.insert(&entry) {
                    Ok(true) => new.push(entry),
                    Ok(false) => {}
                    Err(_) if store.lacks_predecessor(&entry)? => gap = true,
                    Err(err) => tracing::debug!("dropped entry: {err:#}"),
                }
            }
            // An entry arrived before the one it follows: say what we have, so the sender fills the gap now.
            Ok((new, if gap { vec![hello(store)?] } else { vec![] }))
        }
        _ => anyhow::bail!("message for another space"),
    }
}

pub async fn write<W: AsyncWrite + Unpin>(w: &mut W, msg: &Msg) -> Result<()> {
    let mut bytes = Vec::new();
    ciborium::into_writer(msg, &mut bytes)?;
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read<R: AsyncRead + Unpin>(r: &mut R) -> Result<Msg> {
    let len = r.read_u32().await? as usize;
    anyhow::ensure!(len <= MAX_MESSAGE, "message too large");
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes).await?;
    Ok(ciborium::from_reader(bytes.as_slice())?)
}
