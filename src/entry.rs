//! Entries: the one kind of thing anyone ever writes. See SPEC.md, "Entries".

use anyhow::{Context, Result, bail, ensure};
use p2panda_core::traits::Provenance;
use p2panda_core::{Body, Hash, Header, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL: u32 = 8;

/// The audience every entry has in protocol version 1.
pub const SPACE_LOG: &str = "space";

pub type Key = [u8; 32];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ext {
    pub space: Key,
    pub log: String,
    pub time: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Content {
    pub app: String,
    pub kind: String,
    pub data: Value,
}

/// A verified entry with its decoded content.
#[derive(Clone, Debug)]
pub struct Entry {
    pub header: Header<Ext>,
    pub body: Vec<u8>,
    pub content: Content,
}

/// Where an entry sits in the replay order: (time, author, seq).
pub type Order = (u64, Key, u32);

impl Entry {
    pub fn create(key: &SigningKey, space: Key, time: u64, prev: Option<&Entry>, content: Content) -> Self {
        let body = serde_json::to_vec(&content).expect("json");
        let header = Header::<Ext>::builder()
            .seq_num(prev.map_or(0, |p| p.seq() + 1))
            .backlink(prev.map(|p| p.hash()))
            .body(&body)
            .build(key, Ext { space, log: SPACE_LOG.into(), time });
        Entry { header, body, content }
    }

    /// Decodes and checks an entry from the wire: signature, body hash and content.
    pub fn decode(header: &[u8], body: &[u8]) -> Result<Self> {
        let header = Header::<Ext>::decode(header).context("decode header")?;
        ensure!(header.verify(), "bad signature");
        ensure!(header.extensions.log == SPACE_LOG, "unknown log {}", header.extensions.log);
        ensure!(header.extensions.time <= i64::MAX as u64, "time out of range");
        ensure!(header.payload_size as usize == body.len(), "body size mismatch");
        if !body.is_empty() {
            ensure!(header.payload_hash == Some(Body::from_bytes(body).hash()), "body hash mismatch");
        }
        let content: Content = serde_json::from_slice(body).context("decode body")?;
        Ok(Entry { header, body: body.to_vec(), content })
    }

    /// Decodes an entry this device already checked when it arrived.
    pub fn decode_trusted(header: &[u8], body: &[u8]) -> Result<Self> {
        let header = Header::<Ext>::decode(header).context("decode header")?;
        let content: Content = serde_json::from_slice(body).context("decode body")?;
        Ok(Entry { header, body: body.to_vec(), content })
    }

    pub fn encode(&self) -> (Vec<u8>, Vec<u8>) {
        (self.header.encode(), self.body.clone())
    }

    pub fn author(&self) -> Key {
        *self.header.verifying_key.as_bytes()
    }
    pub fn seq(&self) -> u32 {
        self.header.seq_num
    }
    pub fn hash(&self) -> Hash {
        self.header.hash()
    }
    pub fn space(&self) -> Key {
        self.header.extensions.space
    }
    pub fn log(&self) -> &str {
        &self.header.extensions.log
    }
    pub fn time(&self) -> u64 {
        self.header.extensions.time
    }
    pub fn order(&self) -> Order {
        (self.time(), self.author(), self.seq())
    }

    /// Checks that `self` follows `prev` in the same log.
    pub fn follows(&self, prev: Option<&Entry>) -> Result<()> {
        match prev {
            None if self.seq() == 0 && self.header.backlink.is_none() => Ok(()),
            None => bail!("entry {} has no predecessor", self.seq()),
            Some(p) => {
                ensure!(self.seq() == p.seq() + 1, "seq gap");
                ensure!(self.header.backlink == Some(p.hash()), "backlink mismatch");
                ensure!(self.time() >= p.time(), "time went backwards");
                Ok(())
            }
        }
    }
}

pub fn verifying(key: &Key) -> Result<VerifyingKey> {
    Ok(VerifyingKey::try_from(key)?)
}

pub fn hex(key: &Key) -> String {
    data_encoding::HEXLOWER.encode(key)
}

pub fn unhex(s: &str) -> Result<Key> {
    let bytes = data_encoding::HEXLOWER.decode(s.as_bytes()).context("not hex")?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("not 32 bytes"))
}

/// A short, readable form of a key for people and logs.
pub fn short(key: &Key) -> String {
    hex(key)[..8].to_string()
}
