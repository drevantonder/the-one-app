//! Entries: the one kind of thing anyone ever writes. See SPEC.md, "Entries".

use std::fmt;

use anyhow::{Context, Result, bail, ensure};
use p2panda_core::traits::Provenance;
use p2panda_core::{Body, Hash, Header, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL: u32 = 10;

pub type Key = [u8; 32];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ext {
    /// The scope the entry is in.
    pub log: String,
    pub time: u64,
    /// The user it was written as.
    pub user: String,
    /// The client or server it ran for, when that isn't the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Key>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Content {
    pub app: String,
    pub kind: String,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<String>,
}

impl Content {
    pub fn new(app: &str, kind: &str, data: Value) -> Self {
        Content { app: app.into(), kind: kind.into(), data, blobs: Vec::new() }
    }
}

/// Where everything lives. See SPEC.md, "Scopes".
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Global,
    Group(String),
    User(String),
}

impl Scope {
    pub fn parse(s: &str) -> Result<Scope> {
        Ok(match s.split_once(':') {
            None if s == "global" => Scope::Global,
            Some(("group", id)) if is_id(id) => Scope::Group(id.into()),
            Some(("user", id)) if is_id(id) => Scope::User(id.into()),
            _ => bail!("{s} isn't a scope: global, group:<id> or user:<id>"),
        })
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Global => write!(f, "global"),
            Scope::Group(id) => write!(f, "group:{id}"),
            Scope::User(id) => write!(f, "user:{id}"),
        }
    }
}

/// User and group ids: 16 random hex characters.
pub fn is_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn new_id() -> String {
    data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>())
}

/// A verified entry with its decoded content.
#[derive(Clone, Debug)]
pub struct Entry {
    pub header: Header<Ext>,
    pub body: Vec<u8>,
    pub content: Content,
}

/// Where an entry sits in the replay order: (time, author, log, seq).
pub type Order = (u64, Key, String, u32);

/// The end of a log as its author last wrote it: (seq, hash, time).
pub type Head = (u32, Hash, u64);

impl Entry {
    pub fn create(key: &SigningKey, ext: Ext, prev: Option<Head>, content: Content) -> Self {
        let body = serde_json::to_vec(&content).expect("json");
        let header = Header::<Ext>::builder()
            .seq_num(prev.map_or(0, |p| p.0 + 1))
            .backlink(prev.map(|p| p.1))
            .body(&body)
            .build(key, ext);
        Entry { header, body, content }
    }

    /// Decodes and checks an entry from the wire: signature, body hash and content.
    pub fn decode(header: &[u8], body: &[u8]) -> Result<Self> {
        let header = Header::<Ext>::decode(header).context("decode header")?;
        ensure!(header.verify(), "bad signature");
        Scope::parse(&header.extensions.log)?;
        ensure!(header.extensions.time <= i64::MAX as u64, "time out of range");
        ensure!(header.payload_size as usize == body.len(), "body size mismatch");
        if !body.is_empty() {
            ensure!(header.payload_hash == Some(Body::from_bytes(body).hash()), "body hash mismatch");
        }
        let content: Content = serde_json::from_slice(body).context("decode body")?;
        Ok(Entry { header, body: body.to_vec(), content })
    }

    /// Decodes an entry this server already checked when it arrived.
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
    pub fn log(&self) -> &str {
        &self.header.extensions.log
    }
    pub fn scope(&self) -> Scope {
        Scope::parse(self.log()).expect("checked when it arrived")
    }
    pub fn time(&self) -> u64 {
        self.header.extensions.time
    }
    pub fn user(&self) -> &str {
        &self.header.extensions.user
    }
    pub fn by(&self) -> Option<Key> {
        self.header.extensions.by
    }
    pub fn head(&self) -> Head {
        (self.seq(), self.hash(), self.time())
    }
    pub fn order(&self) -> Order {
        (self.time(), self.author(), self.log().to_string(), self.seq())
    }
    /// How apps name an entry: the first 8 hex characters of its author, and its seq.
    pub fn id(&self) -> String {
        format!("{}:{}", short(&self.author()), self.seq())
    }

    /// Checks that `self` follows `prev`, the end of the same log.
    pub fn follows(&self, prev: Option<Head>) -> Result<()> {
        match prev {
            None if self.seq() == 0 && self.header.backlink.is_none() => Ok(()),
            None => bail!("entry {} has no predecessor", self.seq()),
            Some((seq, hash, time)) => {
                ensure!(self.seq() == seq + 1, "seq gap");
                ensure!(self.header.backlink == Some(hash), "backlink mismatch");
                ensure!(self.time() > time, "time didn't move forward");
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
