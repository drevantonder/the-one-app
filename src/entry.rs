//! Entries: the one kind of thing anyone ever writes, each signed by the endpoint that wrote it.
//!
//! ```text
//! header     canonical JSON { v: 1, author, log, seq, backlink, previous: [hash], time, user, epoch, body, size }
//! signature  Ed25519 by `author` over "one/entry\0" ‖ header (`keys::sign`)
//! hash       sha256(header)
//! ```
//!
//! `backlink` is the author's entry before it in that log, `previous` the heads of the other logs its author had read
//! (of its scope, and of `global`), `body` the sha256 of the body (hex) and `size` its length. Entries are replayed in
//! causal order, each after everything it follows, never by the `time` it claims. `kernel/shared/crypto.js` signs and
//! checks the same (`signEntry`, `checkEntry`), and `tests/contracts.rs` checks they agree.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use anyhow::{Context, Result, anyhow, bail, ensure};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::keys;

pub type Key = [u8; 32];

/// What an entry's signature is for.
pub const DOMAIN: &str = "one/entry";

/// The most any number in an entry's header may be: 2^53 - 1, the most JavaScript's JSON keeps exactly.
pub const NUMBER_MAX: u64 = (1 << 53) - 1;

/// An entry's hash: the sha256 of its header.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, std::hash::Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn from_bytes(bytes: [u8; 32]) -> Hash {
        Hash(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }
    pub fn from_hex(s: &str) -> Result<Hash> {
        Ok(Hash(unhex(s)?))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..12])
    }
}

/// What a writer says of an entry besides its content and place in its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ext {
    /// The scope the entry is in.
    pub log: String,
    pub time: u64,
    /// The user it was written as.
    pub user: String,
    /// The scope's epoch its writer knew.
    pub epoch: u32,
    /// The heads of the other logs its writer had read.
    pub previous: Vec<Hash>,
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

/// Where everything lives.
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

/// An entry's header, as signed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub author: Key,
    pub log: String,
    pub seq: u32,
    pub backlink: Option<Hash>,
    pub previous: Vec<Hash>,
    pub time: u64,
    pub user: String,
    pub epoch: u32,
    /// The sha256 of the body.
    pub body: [u8; 32],
    pub size: u64,
}

impl Header {
    fn json(&self) -> Value {
        json!({
            "v": 1, "author": hex(&self.author), "log": self.log, "seq": self.seq,
            "backlink": self.backlink.map(|h| h.to_hex()), "previous": self.previous.iter().map(Hash::to_hex).collect::<Vec<_>>(),
            "time": self.time, "user": self.user, "epoch": self.epoch, "body": hex(&self.body), "size": self.size,
        })
    }

    fn parse(v: &Value) -> Result<Header> {
        ensure!(v["v"] == 1, "not an entry this device reads");
        let text = |k: &str| v[k].as_str().with_context(|| format!("no {k}"));
        let number = |k: &str| v[k].as_u64().filter(|n| *n <= NUMBER_MAX).with_context(|| format!("no {k}, or one above 2^53"));
        let previous = v["previous"].as_array().context("no previous")?.iter().map(|h| Hash::from_hex(h.as_str().unwrap_or(""))).collect::<Result<Vec<_>>>()?;
        ensure!(previous.windows(2).all(|w| w[0] < w[1]), "previous isn't in order");
        Ok(Header {
            author: unhex(text("author")?)?,
            log: text("log")?.to_string(),
            seq: u32::try_from(number("seq")?)?,
            backlink: match &v["backlink"] {
                Value::Null => None,
                h => Some(Hash::from_hex(h.as_str().context("backlink isn't a hash")?)?),
            },
            previous,
            time: number("time")?,
            user: text("user")?.to_string(),
            epoch: u32::try_from(number("epoch")?)?,
            body: unhex(text("body")?)?,
            size: number("size")?,
        })
    }
}

/// A checked entry with its decoded content.
#[derive(Clone, Debug)]
pub struct Entry {
    pub header: Header,
    /// The header's bytes, as signed and hashed.
    raw: Vec<u8>,
    signature: [u8; 64],
    hash: Hash,
    pub body: Vec<u8>,
    pub content: Content,
    /// How far it is from the start of its scope in causal order: one more than the furthest it follows. Known once a
    /// device holds everything it follows (`Store::take`, `causal`).
    pub depth: u64,
}

/// Where an entry sits in the replay order: (depth, hash), so each comes after everything it follows, and entries
/// none of which follows the other sort by hash, the same on every device.
pub type Order = (u64, Hash);

/// The end of a log as its author last wrote it: (seq, hash).
pub type Head = (u32, Hash);

impl Entry {
    /// Signs an entry after `prev`, the end of its author's log.
    pub fn create(key: &SigningKey, ext: Ext, prev: Option<Head>, content: Content) -> Self {
        let body = serde_json::to_vec(&content).expect("json");
        let mut previous = ext.previous;
        previous.sort();
        previous.dedup();
        let header = Header {
            author: key.verifying_key().to_bytes(),
            log: ext.log,
            seq: prev.map_or(0, |p| p.0 + 1),
            backlink: prev.map(|p| p.1),
            previous,
            time: ext.time,
            user: ext.user,
            epoch: ext.epoch,
            body: keys::sha256(&body),
            size: body.len() as u64,
        };
        let v = header.json();
        let signature = keys::sign(key, DOMAIN, &v);
        let raw = keys::canonical(&v);
        Entry { hash: Hash(keys::sha256(&raw)), header, raw, signature, body, content, depth: 0 }
    }

    /// Decodes and checks an entry from the wire: its header canonical, its signature, body and content.
    pub fn decode(header: &[u8], signature: &[u8], body: &[u8]) -> Result<Self> {
        let v: Value = serde_json::from_slice(header).context("decode header")?;
        ensure!(keys::canonical(&v) == header, "its header isn't canonical JSON");
        let h = Header::parse(&v)?;
        keys::verify(&h.author, DOMAIN, &v, signature).context("bad signature")?;
        Scope::parse(&h.log)?;
        ensure!(h.size == body.len() as u64, "body size mismatch");
        ensure!(h.body == keys::sha256(body), "body hash mismatch");
        ensure!((h.seq == 0) == h.backlink.is_none(), "only an entry that starts its log has no backlink");
        let content: Content = serde_json::from_slice(body).context("decode body")?;
        Ok(Entry { hash: Hash(keys::sha256(header)), header: h, raw: header.to_vec(), signature: signature.try_into().expect("checked"), body: body.to_vec(), content, depth: 0 })
    }

    /// Decodes an entry this device already checked when it arrived.
    pub fn decode_trusted(header: &[u8], signature: &[u8], body: &[u8], depth: u64) -> Result<Self> {
        let h = Header::parse(&serde_json::from_slice(header).context("decode header")?)?;
        let content: Content = serde_json::from_slice(body).context("decode body")?;
        let signature = signature.try_into().map_err(|_| anyhow!("not a signature"))?;
        Ok(Entry { hash: Hash(keys::sha256(header)), header: h, raw: header.to_vec(), signature, body: body.to_vec(), content, depth })
    }

    /// (header, signature, body), as packs and the store keep it.
    pub fn encode(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        (self.raw.clone(), self.signature.to_vec(), self.body.clone())
    }

    /// As `POST /v1/entries` takes it: `{ header, signature, body }`, each base64url.
    pub fn to_json(&self) -> Value {
        let b = |v: &[u8]| data_encoding::BASE64URL_NOPAD.encode(v);
        json!({ "header": b(&self.raw), "signature": b(&self.signature), "body": b(&self.body) })
    }

    pub fn from_json(v: &Value) -> Result<Self> {
        let b = |k: &str| data_encoding::BASE64URL_NOPAD.decode(v[k].as_str().unwrap_or("").as_bytes()).map_err(|_| anyhow!("{k} isn't base64url"));
        Entry::decode(&b("header")?, &b("signature")?, &b("body")?)
    }

    pub fn author(&self) -> Key {
        self.header.author
    }
    pub fn seq(&self) -> u32 {
        self.header.seq
    }
    pub fn hash(&self) -> Hash {
        self.hash
    }
    pub fn log(&self) -> &str {
        &self.header.log
    }
    pub fn scope(&self) -> Scope {
        Scope::parse(self.log()).expect("checked when it arrived")
    }
    pub fn time(&self) -> u64 {
        self.header.time
    }
    pub fn user(&self) -> &str {
        &self.header.user
    }
    pub fn epoch(&self) -> u32 {
        self.header.epoch
    }
    pub fn backlink(&self) -> Option<Hash> {
        self.header.backlink
    }
    pub fn previous(&self) -> &[Hash] {
        &self.header.previous
    }
    /// Everything it follows directly: its backlink and previous.
    pub fn follows_all(&self) -> impl Iterator<Item = Hash> + '_ {
        self.header.backlink.into_iter().chain(self.header.previous.iter().copied())
    }
    pub fn head(&self) -> Head {
        (self.seq(), self.hash())
    }
    pub fn order(&self) -> Order {
        (self.depth, self.hash)
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
            Some((seq, hash)) => {
                ensure!(self.seq() == seq + 1, "seq gap");
                ensure!(self.header.backlink == Some(hash), "backlink mismatch");
                Ok(())
            }
        }
    }
}

/// Puts entries in causal order, each with its depth: after its backlink and previous, and among those that don't
/// follow each other, by hash. `known` gives the depths of entries they may follow that aren't among them. An entry
/// that follows one neither among them nor known is left out, as is one whose backlink isn't its log's entry before.
pub fn causal(entries: Vec<Entry>, known: &HashMap<Hash, u64>) -> Vec<Entry> {
    let mut by_hash: HashMap<Hash, Entry> = HashMap::new();
    for e in entries {
        by_hash.entry(e.hash()).or_insert(e);
    }
    let mut depths: HashMap<Hash, u64> = HashMap::new();
    let mut waiting: HashMap<Hash, usize> = HashMap::new();
    let mut followers: HashMap<Hash, Vec<Hash>> = HashMap::new();
    let mut ready = BTreeSet::new();
    for (hash, e) in &by_hash {
        let mut lacks = 0;
        let mut depth = 0;
        let mut whole = true;
        for f in e.follows_all() {
            if by_hash.contains_key(&f) {
                lacks += 1;
                followers.entry(f).or_default().push(*hash);
            } else if let Some(d) = known.get(&f) {
                depth = depth.max(d + 1);
            } else {
                whole = false;
            }
        }
        if let Some(b) = e.backlink().and_then(|b| by_hash.get(&b)) {
            whole &= b.author() == e.author() && b.log() == e.log() && b.seq() + 1 == e.seq();
        }
        if !whole {
            continue;
        }
        depths.insert(*hash, depth);
        match lacks {
            0 => drop(ready.insert((depth, *hash))),
            n => drop(waiting.insert(*hash, n)),
        }
    }
    // Each entry once all it follows is placed, at one more than the furthest of them.
    let mut out = Vec::new();
    let mut placed: BTreeMap<Hash, u64> = BTreeMap::new();
    while let Some((depth, hash)) = ready.pop_first() {
        placed.insert(hash, depth);
        for f in followers.remove(&hash).unwrap_or_default() {
            let Some(d) = depths.get_mut(&f) else { continue };
            *d = (*d).max(depth + 1);
            let n = waiting.get_mut(&f).expect("counted");
            *n -= 1;
            if *n == 0 {
                ready.insert((*d, f));
            }
        }
        let mut e = by_hash.remove(&hash).expect("held");
        e.depth = depth;
        out.push(e);
    }
    out.sort_by_key(Entry::order);
    out
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

/// A pack: entries, as CBOR, each `[header, signature, body]`.
pub fn pack(entries: &[&Entry]) -> Vec<u8> {
    let list: Vec<ciborium::Value> = entries
        .iter()
        .map(|e| {
            let (h, s, b) = e.encode();
            ciborium::Value::Array(vec![ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)])
        })
        .collect();
    let mut out = Vec::new();
    ciborium::into_writer(&ciborium::Value::Array(list), &mut out).expect("cbor in memory");
    out
}

/// A pack's entries, each checked as if it came from another device: signature, body and content.
pub fn unpack(bytes: &[u8]) -> Result<Vec<Entry>> {
    let ciborium::Value::Array(list) = ciborium::from_reader(bytes)? else { bail!("not a pack") };
    list.into_iter()
        .map(|v| match v {
            ciborium::Value::Array(hsb) => match hsb.as_slice() {
                [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)] => Entry::decode(h, s, b),
                _ => bail!("not a pack"),
            },
            _ => bail!("not a pack"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(key: &SigningKey, log: &str, time: u64, prev: Option<&Entry>, previous: &[&Entry]) -> Entry {
        let ext = Ext { log: log.into(), time, user: "a000000000000001".into(), epoch: 0, previous: previous.iter().map(|e| e.hash()).collect() };
        Entry::create(key, ext, prev.map(Entry::head), Content::new("node", "setting", json!({ "name": "n", "value": time.to_string() })))
    }

    #[test]
    fn an_entry_checks_only_as_it_was_signed() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let e = write(&key, "global", 5, None, &[]);
        let (h, s, b) = e.encode();
        let back = Entry::decode(&h, &s, &b).unwrap();
        assert_eq!(back.hash(), e.hash());
        assert!(Entry::decode(&h, &s, br#"{"app":"node","kind":"setting","data":{}}"#).is_err(), "not with another body");
        let other = String::from_utf8(h.clone()).unwrap().replace("\"time\":5", "\"time\":6");
        assert!(Entry::decode(other.as_bytes(), &s, &b).is_err(), "not with another time");
        let spaced = String::from_utf8(h).unwrap().replace(',', ", ");
        assert!(Entry::decode(spaced.as_bytes(), &s, &b).is_err(), "only as canonical JSON");
    }

    #[test]
    fn concurrent_entries_order_the_same_whatever_the_time_they_claim_or_the_order_they_arrive() {
        let (a, b, c) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
        let g = write(&a, "global", 1, None, &[]);
        // b and c each write without having read the other's, b claiming a time long after c's.
        let b1 = write(&b, "global", 9_000_000, None, &[&g]);
        let c1 = write(&c, "global", 2, None, &[&g]);
        let c2 = write(&c, "global", 3, Some(&c1), &[&g]);
        // a read both, and says so, though its clock is behind.
        let a1 = write(&a, "global", 0, Some(&g), &[&b1, &c2]);
        let all = vec![g.clone(), b1.clone(), c1.clone(), c2.clone(), a1.clone()];
        let order = |list: Vec<Entry>| causal(list, &HashMap::new()).iter().map(Entry::hash).collect::<Vec<_>>();
        let first = order(all.clone());
        assert_eq!(first, order(all.iter().rev().cloned().collect()));
        assert_eq!(first.len(), 5);
        assert_eq!(first[0], g.hash());
        assert_eq!(*first.last().unwrap(), a1.hash(), "after everything it read");
        let at = |h: Hash| first.iter().position(|x| *x == h).unwrap();
        assert!(at(c1.hash()) < at(c2.hash()));
        // One whose backlink hasn't arrived waits.
        assert_eq!(order(vec![g.clone(), c2.clone()]), vec![g.hash()]);
    }
}
