//! Backups: each scope's signed entries and the blobs they name, encrypted with the scope's own key and sent to a
//! place that can't read them: a folder on one server's disk, or S3-compatible storage. A backup is what sync moves,
//! sent to a peer that can't think; a restore is an import. Each scope's key is sealed to the install's recovery key,
//! so a backup and that key are all a recovery needs.
//!
//! A place holds a folder for each install, named by its genesis, so installs can share a bucket:
//!
//! ```text
//! <install>/keys/<scope>/<key id>              the scope's key, sealed to the recovery key
//! <install>/objects/<name>                     a pack of entries, or a blob, encrypted
//! <install>/snapshots/<scope>/<time>-<server>  what the scope's backup held then
//! ```
//!
//! A scope's folder names write `user:<id>` as `user-<id>`, which every disk takes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, bail, ensure};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use crate::entry::{Entry, Key, hex};

/// The Vault item kind a place is: an item in Everyone, with a folder's `server` and `path`, or S3's `endpoint`,
/// `bucket`, `region`, `key_id` and `secret`.
pub const KIND: &str = "Backup place";
/// A snapshot every hour, if anything changed.
pub const EVERY: u64 = 3_600_000;
/// How long a scope's changes may wait for the servers before this one in line, each, before it backs them up itself.
pub const TAKE_OVER: u64 = 2 * 3_600_000;

const SEAL: &str = "the one app 2026-11-01 backups: a scope key sealed to the recovery key";
const OBJECT: &str = "the one app 2026-11-01 backups: an object's key";
const SNAPSHOT: &str = "the one app 2026-11-01 backups: a scope's snapshots' key";
const KEY_ID: &str = "the one app 2026-11-01 backups: a key's id";

// Keys.

/// Seals a scope's key to the recovery key's public half, as age seals to an ssh-ed25519 key: the Ed25519 key's
/// X25519 twin, an ephemeral X25519 share, and a key derived from both that encrypts it. → share ‖ ciphertext.
pub fn seal(recovery: &Key, scope_key: &[u8; 32]) -> Result<Vec<u8>> {
    let to = CompressedEdwardsY(*recovery).decompress().context("the recovery key isn't a key")?.to_montgomery();
    let ephemeral: [u8; 32] = rand::random();
    let share = MontgomeryPoint::mul_base_clamped(ephemeral);
    let shared = to.mul_clamped(ephemeral);
    ensure!(shared.0 != [0; 32], "the recovery key isn't a key");
    let mut out = share.0.to_vec();
    out.extend(cipher(&wrap_key(&shared.0, &share.0, &to.0)).encrypt(&zero(), &scope_key[..]).map_err(|_| anyhow!("sealing failed"))?);
    Ok(out)
}

/// Opens a sealed scope key with the recovery key's secret (its Ed25519 seed).
pub fn unseal(recovery_secret: &[u8; 32], sealed: &[u8]) -> Result<[u8; 32]> {
    ensure!(sealed.len() == 32 + 32 + 16, "not a sealed key");
    let scalar: [u8; 32] = Sha512::digest(recovery_secret)[..32].try_into()?;
    let me = MontgomeryPoint::mul_base_clamped(scalar);
    let share = MontgomeryPoint(sealed[..32].try_into()?);
    let shared = share.mul_clamped(scalar);
    let key = cipher(&wrap_key(&shared.0, &share.0, &me.0)).decrypt(&zero(), &sealed[32..]).map_err(|_| anyhow!("that recovery key doesn't open this backup"))?;
    key.try_into().map_err(|_| anyhow!("not a sealed key"))
}

fn wrap_key(shared: &[u8; 32], share: &[u8; 32], to: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key(SEAL, &[&shared[..], share, to].concat())
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(key.into())
}

fn zero() -> aes_gcm::Nonce<aes_gcm::aead::consts::U12> {
    Default::default()
}

/// A scope key's short, public name: which key a snapshot needs.
pub fn key_id(key: &[u8; 32]) -> String {
    hex(&blake3::derive_key(KEY_ID, key))[..16].to_string()
}

// Objects and snapshots.

/// An object's name: a keyed hash of its content, so the place sees neither, and the same content is the same object
/// whichever server sends it.
fn name_of(key: &[u8; 32], bytes: &[u8]) -> String {
    blake3::keyed_hash(key, bytes).to_hex().to_string()
}

/// Encrypts an object under a key derived from its name: each key encrypts one content, so a fixed nonce is safe.
fn lock_object(key: &[u8; 32], name: &str, bytes: &[u8]) -> Vec<u8> {
    cipher(&blake3::derive_key(OBJECT, &[&key[..], name.as_bytes()].concat())).encrypt(&zero(), bytes).expect("encrypting in memory")
}

fn open_object(key: &[u8; 32], name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let plain = cipher(&blake3::derive_key(OBJECT, &[&key[..], name.as_bytes()].concat())).decrypt(&zero(), bytes).map_err(|_| anyhow!("object {} didn't decrypt", &name[..12]))?;
    ensure!(name_of(key, &plain) == name, "object {} isn't what its name says", &name[..12]);
    Ok(plain)
}

/// What a scope's backup held at a time: its entries (as each author's next seq), the packs they're in, and its blobs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub scope: String,
    pub time: u64,
    /// The server that wrote it, by its key.
    pub server: String,
    /// Each author's next seq the backup lacks, by their key.
    pub frontier: BTreeMap<String, u32>,
    pub packs: Vec<String>,
    /// Each blob, by its hash, and the object it is.
    pub blobs: BTreeMap<String, String>,
    pub entries: u64,
    /// The time of the newest entry it holds.
    pub upto: u64,
    /// How big its objects are, in bytes.
    pub bytes: u64,
}

/// A snapshot as a place keeps it: its key's id, a random nonce, and the snapshot, encrypted.
fn lock_snapshot(key: &[u8; 32], s: &Snapshot) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = key_id(key).into_bytes();
    out.extend(nonce);
    out.extend(cipher(&blake3::derive_key(SNAPSHOT, key)).encrypt(&nonce.into(), serde_json::to_vec(s).expect("json").as_slice()).expect("encrypting in memory"));
    out
}

/// Opens a snapshot with whichever of `keys` it was written with; None if none of them.
fn open_snapshot(keys: &[[u8; 32]], bytes: &[u8]) -> Result<Option<(Snapshot, [u8; 32])>> {
    ensure!(bytes.len() > 16 + 12, "not a snapshot");
    let Some(key) = keys.iter().find(|k| key_id(k).as_bytes() == &bytes[..16]) else { return Ok(None) };
    let plain = cipher(&blake3::derive_key(SNAPSHOT, key)).decrypt(bytes[16..28].into(), &bytes[28..]).map_err(|_| anyhow!("a snapshot didn't decrypt"))?;
    Ok(Some((serde_json::from_slice(&plain)?, *key)))
}

/// A scope as a folder names it.
pub fn folder(scope: &str) -> String {
    scope.replacen(':', "-", 1)
}

/// The scope a folder is.
pub fn scope_of(folder: &str) -> String {
    folder.replacen('-', ":", 1)
}

/// A pack: entries, as CBOR, each its header and body.
fn pack(entries: &[&Entry]) -> Vec<u8> {
    let list: Vec<ciborium::Value> = entries
        .iter()
        .map(|e| {
            let (h, b) = e.encode();
            ciborium::Value::Array(vec![ciborium::Value::Bytes(h), ciborium::Value::Bytes(b)])
        })
        .collect();
    let mut out = Vec::new();
    ciborium::into_writer(&ciborium::Value::Array(list), &mut out).expect("cbor in memory");
    out
}

/// A pack's entries, each checked as if it came from another server: signature, body and content.
fn unpack(bytes: &[u8]) -> Result<Vec<Entry>> {
    let ciborium::Value::Array(list) = ciborium::from_reader(bytes)? else { bail!("not a pack") };
    list.into_iter()
        .map(|v| match v {
            ciborium::Value::Array(hb) => match hb.as_slice() {
                [ciborium::Value::Bytes(h), ciborium::Value::Bytes(b)] => Entry::decode(h, b),
                _ => bail!("not a pack"),
            },
            _ => bail!("not a pack"),
        })
        .collect()
}

// Places.

/// Where backups go.
#[derive(Clone, Debug)]
pub enum Place {
    /// A folder on this server's disk, like a USB disk's.
    Folder(PathBuf),
    S3(S3),
}

/// An S3-compatible bucket: `endpoint` like `https://<account>.r2.cloudflarestorage.com`, path-style.
#[derive(Clone)]
pub struct S3 {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub key_id: String,
    pub secret: String,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S3 {{ {}/{}, key {} }}", self.endpoint, self.bucket, self.key_id)
    }
}

impl Place {
    /// A place from its item's fields.
    pub fn from_fields(fields: &BTreeMap<String, String>) -> Result<Place> {
        let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(path) = get("path") {
            return Ok(Place::Folder(path.into()));
        }
        let endpoint = get("endpoint").context("a place has a folder's path, or S3's endpoint")?;
        ensure!(endpoint.starts_with("https://") || endpoint.starts_with("http://"), "an endpoint is a URL, like https://s3.example.com");
        Ok(Place::S3(S3 {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket: get("bucket").context("which bucket?")?,
            region: get("region").unwrap_or_else(|| "auto".into()),
            key_id: get("key_id").context("which key ID?")?,
            secret: get("secret").context("the secret key?")?,
        }))
    }

    pub fn put(&self, path: &str, bytes: &[u8]) -> Result<()> {
        match self {
            Place::Folder(dir) => {
                let file = dir.join(path);
                std::fs::create_dir_all(file.parent().context("no folder")?)?;
                let tmp = file.with_extension("partial");
                std::fs::write(&tmp, bytes)?;
                std::fs::rename(&tmp, &file)?;
                Ok(())
            }
            Place::S3(s3) => match s3.request("PUT", path, &[], bytes)? {
                (200, _) => Ok(()),
                (status, body) => bail!("{}", s3.failed(status, &body)),
            },
        }
    }

    pub fn get(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match self {
            Place::Folder(dir) => match std::fs::read(dir.join(path)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            Place::S3(s3) => match s3.request("GET", path, &[], &[])? {
                (200, body) => Ok(Some(body)),
                (404, _) => Ok(None),
                (status, body) => bail!("{}", s3.failed(status, &body)),
            },
        }
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        match self {
            Place::Folder(dir) => match std::fs::remove_file(dir.join(path)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
            Place::S3(s3) => match s3.request("DELETE", path, &[], &[])? {
                (200 | 204 | 404, _) => Ok(()),
                (status, body) => bail!("{}", s3.failed(status, &body)),
            },
        }
    }

    /// The names directly in a folder of the place, sorted.
    pub fn list(&self, folder: &str) -> Result<Vec<String>> {
        let mut out = match self {
            Place::Folder(dir) => match std::fs::read_dir(dir.join(folder)) {
                Ok(entries) => entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| !n.ends_with(".partial") && !n.starts_with('.')).collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e.into()),
            },
            Place::S3(s3) => s3.list(&format!("{}/", folder.trim_end_matches('/')))?,
        };
        out.sort();
        Ok(out)
    }

    /// Writes a test file, reads it back and deletes it: each step, and what went wrong if it did.
    pub fn try_it(&self) -> Vec<(&'static str, Option<String>)> {
        let path = format!("test-{}", hex(&rand::random::<[u8; 32]>()));
        let bytes = format!("{} tried this place", crate::config::NAME).into_bytes();
        let wrote = self.put(&path, &bytes);
        let read = wrote.as_ref().map_err(|_| anyhow!("not written")).and_then(|_| match self.get(&path)? {
            Some(b) if b == bytes => Ok(()),
            Some(_) => bail!("it read back different"),
            None => bail!("it wasn't there"),
        });
        let deleted = wrote.as_ref().map_err(|_| anyhow!("not written")).and_then(|_| self.delete(&path));
        let say = |r: Result<()>| r.err().map(|e| format!("{e:#}"));
        vec![("Wrote a test file", say(wrote)), ("Read it back", say(read)), ("Deleted it", say(deleted))]
    }

    /// For a folder: whether its disk is there now. A disk that's unplugged waits, rather than fails.
    pub fn present(&self) -> bool {
        match self {
            Place::Folder(dir) => dir.parent().is_none_or(|p| p.exists()),
            Place::S3(_) => true,
        }
    }
}

impl S3 {
    fn host(&self) -> &str {
        self.endpoint.split("://").nth(1).unwrap_or(&self.endpoint)
    }

    /// Says what a refused request means, in words, from S3's error code.
    fn failed(&self, status: u16, body: &[u8]) -> String {
        let text = String::from_utf8_lossy(body);
        let code = between(&text, "<Code>", "</Code>").unwrap_or_default();
        match (status, code.as_str()) {
            (_, "InvalidAccessKeyId" | "SignatureDoesNotMatch") | (401, _) => format!("{} rejected the key", self.host()),
            (_, "NoSuchBucket") => format!("{} has no bucket {}", self.host(), self.bucket),
            (403, _) | (_, "AccessDenied") => format!("{} refused: the key may not write to {}", self.host(), self.bucket),
            _ => format!("{} said {status}{}", self.host(), if code.is_empty() { String::new() } else { format!(" ({code})") }),
        }
    }

    /// The names under `prefix`, one level down: objects' and folders'.
    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2".to_string()), ("prefix", prefix.to_string()), ("delimiter", "/".to_string())];
            if let Some(t) = &token {
                query.push(("continuation-token", t.clone()));
            }
            let q: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let (status, body) = self.request("GET", "", &q, &[])?;
            ensure!(status == 200, "{}", self.failed(status, &body));
            let text = String::from_utf8_lossy(&body);
            for tag in ["Key", "Prefix"] {
                let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
                let mut rest = text.as_ref();
                while let Some(start) = rest.find(&open) {
                    rest = &rest[start + open.len()..];
                    let Some(end) = rest.find(&close) else { break };
                    let name = unescape(&rest[..end]);
                    if let Some(name) = name.strip_prefix(prefix).map(|n| n.trim_end_matches('/')).filter(|n| !n.is_empty()) {
                        out.push(name.to_string());
                    }
                    rest = &rest[end..];
                }
            }
            match between(&text, "<NextContinuationToken>", "</NextContinuationToken>") {
                Some(t) if text.contains("<IsTruncated>true</IsTruncated>") => token = Some(unescape(&t)),
                _ => break,
            }
        }
        out.dedup();
        Ok(out)
    }

    /// A request signed with AWS Signature Version 4, path-style: → its status and body.
    fn request(&self, method: &str, key: &str, query: &[(&str, &str)], body: &[u8]) -> Result<(u16, Vec<u8>)> {
        let path = match key {
            "" => format!("/{}", encode(&self.bucket, true)),
            key => format!("/{}/{}", encode(&self.bucket, true), encode(key, false)),
        };
        let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (encode(k, true), encode(v, true))).collect();
        q.sort();
        let query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        let now = jiff::Timestamp::now();
        let stamp = now.strftime("%Y%m%dT%H%M%SZ").to_string();
        let date = &stamp[..8];
        let payload = hex(&Sha256::digest(body).into());
        let host = self.host();
        let headers = format!("host:{host}\nx-amz-content-sha256:{payload}\nx-amz-date:{stamp}\n");
        let signed = "host;x-amz-content-sha256;x-amz-date";
        let request = format!("{method}\n{path}\n{query}\n{headers}\n{signed}\n{payload}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}", hex(&Sha256::digest(request.as_bytes()).into()));
        let mut k = hmac(format!("AWS4{}", self.secret).as_bytes(), date.as_bytes());
        for part in [self.region.as_str(), "s3", "aws4_request"] {
            k = hmac(&k, part.as_bytes());
        }
        let signature = hex(&hmac(&k, to_sign.as_bytes()));
        let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", self.key_id);
        let url = if query.is_empty() { format!("{}{path}", self.endpoint) } else { format!("{}{path}?{query}", self.endpoint) };
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(300))).build().into();
        let builder = ureq::http::Request::builder().method(method).uri(&url).header("x-amz-content-sha256", &payload).header("x-amz-date", &stamp).header("authorization", auth);
        let mut resp = match method {
            "PUT" => agent.run(builder.body(body.to_vec())?),
            _ => agent.run(builder.body(())?),
        }
        .with_context(|| format!("couldn't reach {host}"))?;
        let bytes = resp.body_mut().with_config().limit(4 << 30).read_to_vec()?;
        Ok((resp.status().as_u16(), bytes))
    }
}

/// HMAC-SHA256 (RFC 2104).
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    match key.len() > 64 {
        true => k[..32].copy_from_slice(&Sha256::digest(key)),
        false => k[..key.len()].copy_from_slice(key),
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// URI-encodes as SigV4 wants: everything but unreserved characters, and `/` too unless it's a path.
fn encode(s: &str, slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if !slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn between(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].to_string())
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// Backing up.

/// One scope's backup to one place, as a server does it.
pub struct Job<'a> {
    pub place: &'a Place,
    /// The install's folder in the place.
    pub install: &'a str,
    pub scope: &'a str,
    pub key: [u8; 32],
    /// The recovery key's public half, which the scope's key is sealed to.
    pub recovery: Key,
    pub server: Key,
    pub now: u64,
    /// The latest snapshot of the scope in the place with this key, if any.
    pub previous: Option<Snapshot>,
    /// The scope's entries the backup may lack: all those after `previous`'s frontier, or more.
    pub entries: Vec<Entry>,
    /// The blobs the scope's entries name, which the backup keeps.
    pub blobs: BTreeSet<String>,
}

/// Sends a scope what its backup lacks: its key, sealed, the first time; a pack of its new entries, its new blobs,
/// and a snapshot. Returns the snapshot, or None when nothing new would be in it. `blob` reads a blob here.
pub fn back_up(job: Job, blob: impl Fn(&str) -> Option<Vec<u8>>) -> Result<Option<Snapshot>> {
    let Job { place, install, scope, key, recovery, server, now, previous, entries, blobs } = job;
    let mut snap = previous.clone().unwrap_or_else(|| Snapshot { scope: scope.to_string(), ..Default::default() });
    let new: Vec<&Entry> = entries.iter().filter(|e| e.seq() >= snap.frontier.get(&hex(&e.author())).copied().unwrap_or(0)).collect();
    let new_blobs: Vec<&String> = blobs.iter().filter(|b| !snap.blobs.contains_key(*b)).collect();
    if previous.is_some() && new.is_empty() && new_blobs.is_empty() {
        return Ok(None);
    }
    let keys = format!("{install}/keys/{}/{}", folder(scope), key_id(&key));
    if previous.is_none() && place.get(&keys)?.is_none() {
        place.put(&keys, &seal(&recovery, &key)?)?;
    }
    if !new.is_empty() {
        let bytes = pack(&new);
        let name = name_of(&key, &bytes);
        let locked = lock_object(&key, &name, &bytes);
        place.put(&format!("{install}/objects/{name}"), &locked)?;
        snap.bytes += locked.len() as u64;
        snap.packs.push(name);
        snap.entries += new.len() as u64;
        for e in &new {
            let next = snap.frontier.entry(hex(&e.author())).or_insert(0);
            *next = (*next).max(e.seq() + 1);
            snap.upto = snap.upto.max(e.time());
        }
    }
    for hash in new_blobs {
        // A blob this server lacks waits for a run on one that has it, or for it to arrive here.
        let Some(bytes) = blob(hash) else { continue };
        let name = name_of(&key, &bytes);
        let locked = lock_object(&key, &name, &bytes);
        place.put(&format!("{install}/objects/{name}"), &locked)?;
        snap.bytes += locked.len() as u64;
        snap.blobs.insert(hash.clone(), name);
    }
    if previous.as_ref().is_some_and(|p| p.packs == snap.packs && p.blobs == snap.blobs) {
        return Ok(None);
    }
    snap.time = now;
    snap.server = hex(&server);
    place.put(&format!("{install}/snapshots/{}/{}", folder(scope), snapshot_name(now, &server)), &lock_snapshot(&key, &snap))?;
    Ok(Some(snap))
}

/// A snapshot's file name: its time, so names sort by it, and its server.
fn snapshot_name(time: u64, server: &Key) -> String {
    format!("{time:013}-{}", crate::entry::short(server))
}

/// The newest snapshot of a scope in a place that one of `keys` opens, with its key.
pub fn latest(place: &Place, install: &str, scope: &str, keys: &[[u8; 32]]) -> Result<Option<(Snapshot, [u8; 32])>> {
    let dir = format!("{install}/snapshots/{}", folder(scope));
    for name in place.list(&dir)?.iter().rev() {
        let Some(bytes) = place.get(&format!("{dir}/{name}"))? else { continue };
        if let Some(found) = open_snapshot(keys, &bytes)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The snapshots of a scope in a place, newest first, by their times and servers' short keys, without opening them.
pub fn snapshots(place: &Place, install: &str, scope: &str) -> Result<Vec<(u64, String)>> {
    let names = place.list(&format!("{install}/snapshots/{}", folder(scope)))?;
    Ok(names.iter().rev().filter_map(|n| n.split_once('-').and_then(|(t, s)| Some((t.parse().ok()?, s.to_string())))).collect())
}

/// Backups' tools, which are the node's, not an app's.
pub fn tools() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({ "name": "backups_status", "description": "How backups are going: each place backed up to (a folder on a server's disk, or S3-compatible storage), when it last backed up and by which server, what's failing and since when, and the recent snapshots.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } }),
        json!({ "name": "backups_now", "description": "Back up now, without waiting for the hour: each server backs up what it holds to each place.", "inputSchema": { "type": "object", "properties": {} } }),
        json!({ "name": "backups_add_place", "description": "Add S3-compatible storage (Cloudflare R2, Backblaze B2, MinIO…) to back up to. You give the endpoint; the person gets a link to a sheet on their own device where they type the bucket, access key ID and secret key, so you never see the key. backups_status then shows its first backup, or what went wrong.", "inputSchema": { "type": "object", "required": ["endpoint"], "properties": { "endpoint": { "type": "string", "description": "like https://<account id>.r2.cloudflarestorage.com" }, "title": { "type": "string", "description": "what the person calls it, like Cloudflare R2" } } } }),
        json!({ "name": "backups_remove_place", "description": "Stop backing up to a place. What's already there stays.", "inputSchema": { "type": "object", "required": ["place"], "properties": { "place": { "type": "string", "description": "its title or id, as backups_status lists it" } } } }),
        json!({ "name": "backups_check", "description": "Restore each place's newest backup into a throwaway copy, checking every entry's signature and order and every blob's hash, and say what came back: people, apps and Vault items by name.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } }),
    ]
}

// Restoring.

/// The installs a place holds backups of, by their folders.
pub fn installs(place: &Place) -> Result<Vec<String>> {
    Ok(place.list("")?.into_iter().filter(|n| n.len() == 16 && n.bytes().all(|b| b.is_ascii_hexdigit())).collect())
}

/// Every scope's keys in an install's backup, opened with the recovery key's secret.
pub fn open_keys(place: &Place, install: &str, recovery_secret: &[u8; 32]) -> Result<BTreeMap<String, Vec<[u8; 32]>>> {
    let mut out = BTreeMap::new();
    for dir in place.list(&format!("{install}/keys"))? {
        for id in place.list(&format!("{install}/keys/{dir}"))? {
            let sealed = place.get(&format!("{install}/keys/{dir}/{id}"))?.context("a key went missing")?;
            out.entry(scope_of(&dir)).or_insert_with(Vec::new).push(unseal(recovery_secret, &sealed)?);
        }
    }
    ensure!(!out.is_empty(), "no backup of this install here");
    Ok(out)
}

/// A scope as its newest snapshot holds it: every entry, checked, and every blob, checked against its hash.
pub struct Restored {
    pub snapshot: Snapshot,
    pub entries: Vec<Entry>,
    pub blobs: Vec<(String, Vec<u8>)>,
}

/// Restores a scope from its newest snapshot that one of `keys` opens.
pub fn restore(place: &Place, install: &str, scope: &str, keys: &[[u8; 32]]) -> Result<Restored> {
    let (snapshot, key) = latest(place, install, scope, keys)?.with_context(|| format!("no snapshot of {scope} opens with its keys"))?;
    let mut entries = Vec::new();
    for name in &snapshot.packs {
        let bytes = place.get(&format!("{install}/objects/{name}"))?.with_context(|| format!("pack {} is missing", &name[..12]))?;
        entries.extend(unpack(&open_object(&key, name, &bytes)?)?);
    }
    ensure!(entries.iter().all(|e| e.log() == scope), "a pack of {scope} holds another scope's entries");
    let mut blobs = Vec::new();
    for (hash, name) in &snapshot.blobs {
        let bytes = place.get(&format!("{install}/objects/{name}"))?.with_context(|| format!("blob {} is missing", &hash[..12]))?;
        let bytes = open_object(&key, name, &bytes)?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "blob {} isn't what its hash says", &hash[..12]);
        blobs.push((hash.clone(), bytes));
    }
    Ok(Restored { snapshot, entries, blobs })
}

/// Puts restored entries in a store, each log in order, so every chain is checked as sync checks it.
pub fn import(store: &crate::store::Store, entries: &[Entry]) -> Result<usize> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by_key(|e| (e.author(), e.log().to_string(), e.seq()));
    let mut added = 0;
    for e in sorted {
        if store.insert(e).with_context(|| format!("entry {} of {}", e.id(), e.log()))? {
            added += 1;
        }
    }
    Ok(added)
}

/// Restores every scope `keys` opens from a place into a throwaway store, checking every signature, chain and blob,
/// then says what came back: each scope's entries and blobs, and by name the people, apps and Vault items.
pub fn check(place: &Place, install: &str, keys: &BTreeMap<String, Vec<[u8; 32]>>, root: Option<p2panda_core::Hash>) -> Result<serde_json::Value> {
    let store = crate::store::Store::memory()?;
    let mut scopes = Vec::new();
    let mut all = Vec::new();
    for (scope, keys) in keys {
        if snapshots(place, install, scope)?.is_empty() {
            continue;
        }
        let r = restore(place, install, scope, keys)?;
        let added = import(&store, &r.entries)?;
        scopes.push(serde_json::json!({ "scope": scope, "time": r.snapshot.time, "entries": added, "blobs": r.blobs.len() }));
        all.extend(r.entries);
    }
    ensure!(!scopes.is_empty(), "nothing backed up there yet");
    all.sort_by_key(|e| e.order());
    let root = root.or_else(|| all.iter().find(|e| e.content.app == crate::members::APP && e.content.kind == "genesis").map(|e| e.hash()));
    let member_entries: Vec<Entry> = all.iter().filter(|e| crate::members::is_member_entry(e)).cloned().collect();
    let m = crate::members::Members::replay(root, &member_entries);
    let mut vault = Vec::new();
    for scope in scopes.iter().filter_map(|s| s["scope"].as_str()) {
        let entries: Vec<Entry> = all.iter().filter(|e| e.log() == scope).cloned().collect();
        let (_, state) = crate::members::replay(root, &member_entries, &entries, |_, _, _| {});
        vault.extend(state.items.values().map(|it| it.title.clone()));
    }
    Ok(serde_json::json!({
        "install": m.name,
        "scopes": scopes,
        "people": m.users.values().filter(|u| !u.removed).map(|u| u.name.clone()).collect::<Vec<_>>(),
        "apps": m.apps.values().filter(|a| !a.erased && a.bundle.is_some()).map(|a| a.name.clone()).collect::<Vec<_>>(),
        "vault": vault,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Content, Ext};
    use p2panda_core::SigningKey;
    use serde_json::json;

    fn entries(key: &SigningKey, log: &str, n: u32, blob: Option<&str>) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        for i in 0..n {
            let mut content = Content::new("board", "post", json!({ "text": format!("post {i}") }));
            if let Some(b) = blob {
                content.blobs.push(b.to_string());
            }
            let ext = Ext { log: log.into(), time: 1000 + i as u64, user: "0123456789abcdef".into(), by: None };
            out.push(Entry::create(key, ext, out.last().map(|e| e.head()), content));
        }
        out
    }

    #[test]
    fn a_sealed_key_opens_only_with_the_recovery_key() {
        let recovery = SigningKey::generate();
        let key: [u8; 32] = rand::random();
        let sealed = seal(recovery.verifying_key().as_bytes(), &key).unwrap();
        assert_eq!(unseal(recovery.as_bytes(), &sealed).unwrap(), key);
        assert!(unseal(SigningKey::generate().as_bytes(), &sealed).is_err());
        assert_ne!(sealed, seal(recovery.verifying_key().as_bytes(), &key).unwrap(), "a new share each time");
    }

    #[test]
    fn sigv4_signs_as_aws_does() {
        // AWS's own example: GET /test.txt, with the example key, on 2013-05-24 (the S3 SigV4 docs' header example).
        let k = {
            let mut k = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", b"20130524");
            for part in ["us-east-1", "s3", "aws4_request"] {
                k = hmac(&k, part.as_bytes());
            }
            k
        };
        let request = "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let to_sign = format!("AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n{}", hex(&Sha256::digest(request.as_bytes()).into()));
        assert_eq!(hex(&hmac(&k, to_sign.as_bytes())), "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
    }

    #[test]
    fn a_folder_backup_sends_only_whats_new_and_restores() {
        let dir = std::env::temp_dir().join(format!("backups-test-{}", hex(&rand::random())));
        let place = Place::Folder(dir.clone());
        let recovery = SigningKey::generate();
        let server = SigningKey::generate();
        let key: [u8; 32] = rand::random();
        let blob = b"a repo's pack".to_vec();
        let hash = blake3::hash(&blob).to_hex().to_string();
        let all = entries(&server, "global", 5, Some(&hash));
        let job = |previous: Option<Snapshot>, entries: Vec<Entry>, now: u64| Job {
            place: &place,
            install: "0123456789abcdef",
            scope: "global",
            key,
            recovery: *recovery.verifying_key().as_bytes(),
            server: *server.verifying_key().as_bytes(),
            now,
            previous,
            entries,
            blobs: [hash.clone()].into(),
        };
        let first = back_up(job(None, all[..3].to_vec(), 10), |h| (h == hash).then(|| blob.clone())).unwrap().unwrap();
        assert_eq!((first.entries, first.packs.len(), first.blobs.len()), (3, 1, 1));
        assert!(back_up(job(Some(first.clone()), all[..3].to_vec(), 20), |_| None).unwrap().is_none(), "nothing new, nothing written");
        let second = back_up(job(Some(first.clone()), all.clone(), 30), |_| None).unwrap().unwrap();
        assert_eq!((second.entries, second.packs.len()), (5, 2), "only the two new entries, in a second pack");
        let (newest, _) = latest(&place, "0123456789abcdef", "global", &[key]).unwrap().unwrap();
        assert_eq!(newest, second);

        // From nothing but the place and the recovery key.
        assert_eq!(installs(&place).unwrap(), vec!["0123456789abcdef".to_string()]);
        let keys = open_keys(&place, "0123456789abcdef", recovery.as_bytes()).unwrap();
        let restored = restore(&place, "0123456789abcdef", "global", &keys["global"]).unwrap();
        let store = crate::store::Store::memory().unwrap();
        assert_eq!(import(&store, &restored.entries).unwrap(), 5);
        assert_eq!(restored.blobs, vec![(hash.clone(), blob.clone())]);
        assert!(open_keys(&place, "0123456789abcdef", SigningKey::generate().as_bytes()).is_err());

        // The place sees no content: no entry's text, and no blob's.
        for file in walk(&dir) {
            let bytes = std::fs::read(&file).unwrap();
            assert!(!bytes.windows(4).any(|w| w == b"post") && !bytes.windows(4).any(|w| w == b"repo"), "{file:?} isn't encrypted");
        }
        // A tampered object doesn't restore.
        let pack = dir.join("0123456789abcdef/objects").join(&second.packs[1]);
        let mut bytes = std::fs::read(&pack).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&pack, bytes).unwrap();
        assert!(restore(&place, "0123456789abcdef", "global", &keys["global"]).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            match e.path().is_dir() {
                true => out.extend(walk(&e.path())),
                false => out.push(e.path()),
            }
        }
        out
    }

    #[test]
    fn a_place_is_tried_before_its_added() {
        let dir = std::env::temp_dir().join(format!("backups-try-{}", hex(&rand::random())));
        let steps = Place::Folder(dir.clone()).try_it();
        assert!(steps.iter().all(|(_, err)| err.is_none()), "{steps:?}");
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "the test file is gone");
        let steps = Place::Folder("/proc/no/such/place".into()).try_it();
        assert!(steps[0].1.is_some() && steps[1].1.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
