//! Sync: what two devices say to each other. See SPEC.md, "Sync".
//! The protocol is pure; `net.rs` carries it over iroh and `tests/sim.rs` over a simulated network.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::entry::{Entry, Key};
use crate::store::{Frontier, Store};

pub const ALPN: &[u8] = b"one/sync/1";
const MAX_MESSAGE: usize = 16 << 20;

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

/// Handles one message. Returns the entries that were new to `store`, and a reply to send.
pub fn handle(store: &Store, msg: Msg) -> Result<(Vec<Entry>, Option<Msg>)> {
    match msg {
        Msg::Hello { space, frontier } if space == store.space() => {
            let missing = store.missing(&frontier)?;
            let reply = (!missing.is_empty()).then(|| entries(space, &missing));
            Ok((Vec::new(), reply))
        }
        Msg::Entries { space, entries } if space == store.space() => {
            let mut new = Vec::new();
            for (header, body) in entries {
                match Entry::decode(&header, &body).and_then(|e| store.insert(&e).map(|n| (e, n))) {
                    Ok((entry, true)) => new.push(entry),
                    Ok((_, false)) => {}
                    Err(err) => tracing::debug!("dropped entry: {err:#}"),
                }
            }
            Ok((new, None))
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
