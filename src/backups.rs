//! Backups: each scope's signed entries and the blobs they name, encrypted with the key of the scope's newest epoch and
//! sent to a place that can't read them: a folder on one device's disk, or S3-compatible storage; a restore is an
//! import. Each epoch's key is sealed to the install's recovery key (`keys::epoch_name`'s `recovery`), and the place
//! keeps that copy, so a backup and that key are all a recovery needs.
//!
//! A place holds a folder for each install, named by its genesis, so installs can share a bucket:
//!
//! ```text
//! <install>/keys/<scope>/<key id>              an epoch's key, sealed to the recovery key
//! <install>/objects/<name>                     a pack of entries, or a blob, encrypted
//! <install>/snapshots/<scope>/<time>-<device>  what the scope's backup held then
//! ```
//!
//! A scope's folder names write `user:<id>` as `user-<id>`, which every disk takes.

use std::collections::{BTreeMap, BTreeSet};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};

use crate::bucket::{Place, folder, scope_of};
use crate::entry::{Entry, Key, hex, pack, unpack};

/// The Vault item kind a place is: an item in Everyone, with a folder's `device` and `path`, or S3's `endpoint`,
/// `bucket`, `region`, `key_id` and `secret`.
pub const KIND: &str = "Backup place";
/// A snapshot every hour, if anything changed.
pub const EVERY: u64 = 3_600_000;
/// How long a scope's changes may wait for the devices before this one in line, each, before it backs them up itself.
pub const TAKE_OVER: u64 = 2 * 3_600_000;

const OBJECT: &str = "the one app 2026-11-01 backups: an object's key";
const SNAPSHOT: &str = "the one app 2026-11-01 backups: a scope's snapshots' key";
const KEY_ID: &str = "the one app 2026-11-01 backups: a key's id";

// Keys.

/// Opens an epoch's key sealed to the recovery key, with the recovery key's secret (its Ed25519 seed).
pub fn unseal(recovery_secret: &[u8; 32], sealed: &[u8]) -> Result<[u8; 32]> {
    let key = crate::kernel::open_sealed(&crate::kernel::twin(recovery_secret).0, sealed).map_err(|_| anyhow!("that recovery key doesn't open this backup"))?;
    key.try_into().map_err(|_| anyhow!("not a sealed key"))
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
/// whichever device sends it.
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
    /// How far each of the scope's databases (`<app>/<database>`) is copied: its last step, and its newest copy.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub databases: BTreeMap<String, (u64, u64)>,
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

// Places.

// Backing up.

/// One scope's backup to one place, as a device does it.
pub struct Job<'a> {
    pub place: &'a Place,
    /// The install's folder in the place.
    pub install: &'a str,
    pub scope: &'a str,
    /// The key of the scope's newest epoch, and that key sealed to the recovery key, as the bucket keeps it.
    pub key: [u8; 32],
    pub sealed: Vec<u8>,
    pub device: Key,
    pub now: u64,
    /// The latest snapshot of the scope in the place with this key, if any.
    pub previous: Option<Snapshot>,
    /// The scope's entries the backup may lack: all those after `previous`'s frontier, or more.
    pub entries: Vec<Entry>,
    /// The blobs the scope's entries name, which the backup keeps.
    pub blobs: BTreeSet<String>,
    /// The install's bucket, whose scope's folder the place keeps a copy of.
    pub bucket: Option<&'a Place>,
}

/// Sends a scope what its backup lacks: its key, sealed, the first time; a pack of its new entries, its new blobs,
/// and a snapshot. Returns the snapshot, or None when nothing new would be in it. `blob` reads a blob here.
pub fn back_up(job: Job, blob: impl Fn(&str) -> Option<Vec<u8>>) -> Result<Option<Snapshot>> {
    let Job { place, install, scope, key, sealed, device, now, previous, entries, blobs, bucket } = job;
    let mut snap = previous.clone().unwrap_or_else(|| Snapshot { scope: scope.to_string(), ..Default::default() });
    if let Some(bucket) = bucket {
        copy_bucket(bucket, place, install, scope, &mut snap)?;
    }
    let new: Vec<&Entry> = entries.iter().filter(|e| e.seq() >= snap.frontier.get(&hex(&e.author())).copied().unwrap_or(0)).collect();
    // Blobs the scope no longer keeps (a release that's been superseded, a blob its scope forgot) leave the snapshot,
    // and leave the place once no snapshot kept still has them.
    snap.blobs.retain(|hash, _| blobs.contains(hash));
    let new_blobs: Vec<&String> = blobs.iter().filter(|b| !snap.blobs.contains_key(*b)).collect();
    if previous.as_ref().is_some_and(|p| p.blobs.len() == snap.blobs.len() && p.databases == snap.databases) && new.is_empty() && new_blobs.is_empty() {
        return Ok(None);
    }
    let keys = format!("{install}/keys/{}/{}", folder(scope), key_id(&key));
    if previous.is_none() && place.get(&keys)?.is_none() {
        place.put(&keys, &sealed)?;
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
        // A blob this device lacks waits for a run on one that has it, or for it to arrive here.
        let Some(bytes) = blob(hash) else { continue };
        let name = name_of(&key, &bytes);
        let locked = lock_object(&key, &name, &bytes);
        place.put(&format!("{install}/objects/{name}"), &locked)?;
        snap.bytes += locked.len() as u64;
        snap.blobs.insert(hash.clone(), name);
    }
    if previous.as_ref().is_some_and(|p| p.packs == snap.packs && p.blobs == snap.blobs && p.databases == snap.databases) {
        return Ok(None);
    }
    snap.time = now;
    place.put(&format!("{install}/snapshots/{}/{}", folder(scope), snapshot_name(now, &device)), &lock_snapshot(&key, &snap))?;
    Ok(Some(snap))
}

/// Copies what a scope's folder in the install's bucket has that the place lacks, into `<install>/bucket/`, laid out
/// as the bucket is: each epoch's key sealed to the recovery key, each app's key in each epoch (sealed under it), and
/// each database's new steps and newest copy (sealed as they are), past what `snap` says it has.
fn copy_bucket(bucket: &Place, place: &Place, install: &str, scope: &str, snap: &mut Snapshot) -> Result<()> {
    let folder = folder(scope);
    let mut copy = |name: &str, known: bool| -> Result<bool> {
        if known && place.get(&format!("{install}/bucket/{name}"))?.is_some() {
            return Ok(true);
        }
        let Some(bytes) = bucket.get(name)? else { return Ok(false) };
        place.put(&format!("{install}/bucket/{name}"), &bytes)?;
        snap.bytes += bytes.len() as u64;
        Ok(true)
    };
    for n in bucket.list(&format!("{folder}/_epochs"))? {
        copy(&format!("{folder}/_epochs/{n}/recovery"), true)?;
    }
    for app in bucket.list(&folder)?.into_iter().filter(|a| crate::members::valid_app_name(a) || a == crate::members::APP) {
        for n in bucket.list(&format!("{folder}/{app}/_key"))? {
            copy(&format!("{folder}/{app}/_key/{n}"), true)?;
        }
        for database in bucket.list(&format!("{folder}/{app}"))?.into_iter().filter(|d| !d.starts_with('_')) {
            let prefix = format!("{folder}/{app}/{database}");
            let Some(hint) = bucket.get(&crate::db::newest_name(&prefix))? else { continue };
            let (_, newest_copy, epoch) = crate::db::newest(&hint)?;
            let (mut at, mut copied) = snap.databases.get(&format!("{app}/{database}")).copied().unwrap_or((0, 0));
            if newest_copy > copied && copy(&crate::db::copy_name(&prefix, newest_copy), false)? {
                copied = newest_copy;
                at = at.max(newest_copy);
            }
            while copy(&crate::db::step_name(&prefix, at + 1), false)? {
                at += 1;
            }
            place.put(&format!("{install}/bucket/{}", crate::db::newest_name(&prefix)), &serde_json::to_vec(&serde_json::json!({ "at": at, "copy": copied, "epoch": if copied == newest_copy { epoch } else { 0 } }))?)?;
            snap.databases.insert(format!("{app}/{database}"), (at, copied));
        }
    }
    Ok(())
}

/// Whether the device `at` places down a scope's line backs it up itself: the scope's first change the place lacks
/// (`waiting`) has waited `TAKE_OVER` for each device before it, counting from when the place was added at the earliest.
pub fn takes_over(at: usize, waiting: Option<u64>, added: u64, now: u64) -> bool {
    at == 0 || waiting.is_some_and(|t| now.saturating_sub(t.max(added)) >= TAKE_OVER * at as u64)
}

/// A snapshot's file name: its time, so names sort by it, and its device.
fn snapshot_name(time: u64, device: &Key) -> String {
    format!("{time:013}-{}", crate::entry::short(device))
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

/// The snapshots of a scope in a place, newest first, by their times and devices' short keys, without opening them.
pub fn snapshots(place: &Place, install: &str, scope: &str) -> Result<Vec<(u64, String)>> {
    let names = place.list(&format!("{install}/snapshots/{}", folder(scope)))?;
    Ok(names.iter().rev().filter_map(|n| n.split_once('-').and_then(|(t, s)| Some((t.parse().ok()?, s.to_string())))).collect())
}

// Pruning.

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;
/// How long an object no kept snapshot names may stay, in case a run that's sending it again hasn't written its
/// snapshot yet.
const SETTLE: u64 = HOUR;
/// How long an object no snapshot names at all may stay, in case it's from a run still going.
const STRAY: u64 = DAY;

/// Which snapshots to keep, by time: the last of each hour for 2 days, the last of each day for 30 days, the last of each month
/// for 12 months, and always the newest. Days and months are UTC's, so every device keeps the same ones.
pub fn kept(times: &[u64], now: u64) -> BTreeSet<u64> {
    let mut out: BTreeSet<u64> = times.iter().copied().max().into_iter().collect();
    let mut hours: BTreeMap<u64, u64> = BTreeMap::new();
    let mut days: BTreeMap<u64, u64> = BTreeMap::new();
    let mut months: BTreeMap<(i16, i8), u64> = BTreeMap::new();
    for &t in times {
        if t + 2 * DAY > now {
            let last = hours.entry(t / HOUR).or_insert(t);
            *last = (*last).max(t);
        }
        if t + 30 * DAY > now {
            let last = days.entry(t / DAY).or_insert(t);
            *last = (*last).max(t);
        }
        if t + 365 * DAY > now {
            let date = jiff::Timestamp::from_millisecond(t as i64).unwrap_or_default().to_zoned(jiff::tz::TimeZone::UTC);
            let last = months.entry((date.year(), date.month())).or_insert(t);
            *last = (*last).max(t);
        }
    }
    out.extend(hours.into_values());
    out.extend(days.into_values());
    out.extend(months.into_values());
    out
}

/// What a prune did.
#[derive(Debug, Default, PartialEq)]
pub struct Pruned {
    pub snapshots: usize,
    pub objects: usize,
    pub bytes: u64,
}

/// Prunes an install's backup in a place: drops the snapshots `kept` doesn't keep of each scope in `scopes` (by its
/// key), takes the blobs in `fetchable` (releases, which can be fetched again from where they're published) out of
/// every snapshot but each scope's newest, then deletes the objects no snapshot kept still names.
///
/// A run may be backing up at the same moment, so an object goes only once it has gone unwritten for `SETTLE` (a
/// run that sends it again has rewritten it), and the newest snapshot, which a run builds on, is never touched. When
/// `scopes` opens every snapshot in the place, objects no snapshot names at all (from a run that failed partway)
/// go too, once they've waited `STRAY`.
pub fn prune(place: &Place, install: &str, scopes: &BTreeMap<String, [u8; 32]>, fetchable: &BTreeSet<String>, now: u64) -> Result<Pruned> {
    let mut out = Pruned::default();
    let mut keep: BTreeSet<String> = BTreeSet::new();
    let mut drop: BTreeSet<String> = BTreeSet::new();
    let mut opened_all = true;
    let names = |s: &Snapshot| s.packs.iter().chain(s.blobs.values()).cloned().collect::<Vec<_>>();
    // A release leaves a snapshot only once its object has settled, so it leaves the place in the same prune.
    let unsettled: BTreeSet<String> = place.objects(&format!("{install}/objects"))?.into_iter().filter(|o| o.1 + SETTLE > now).map(|o| o.0).collect();
    let folders = place.list(&format!("{install}/snapshots"))?;
    for dir in &folders {
        let scope = scope_of(dir);
        let Some(key) = scopes.get(&scope) else {
            opened_all = false;
            continue;
        };
        let mut snaps: Vec<(String, Snapshot)> = Vec::new();
        let mut opened = true;
        for name in place.list(&format!("{install}/snapshots/{dir}"))? {
            let path = format!("{install}/snapshots/{dir}/{name}");
            let Some(bytes) = place.get(&path)? else { continue };
            match open_snapshot(&[*key], &bytes)? {
                Some((s, _)) => snaps.push((path, s)),
                None => opened = false,
            }
        }
        opened_all &= opened;
        let Some(newest) = snaps.iter().map(|(_, s)| s.time).max() else { continue };
        let latest: BTreeSet<String> = snaps.iter().filter(|(_, s)| s.time == newest).flat_map(|(_, s)| s.blobs.keys().cloned()).collect();
        let times: Vec<u64> = snaps.iter().map(|(_, s)| s.time).collect();
        let kept = kept(&times, now);
        // Each database's oldest copy a kept snapshot starts from, or one made in the last `SETTLE`: a run going now
        // may be building on it, whether it's kept or not.
        let mut floors: BTreeMap<String, u64> = BTreeMap::new();
        for (_, s) in snaps.iter().filter(|(_, s)| kept.contains(&s.time) || s.time + SETTLE > now) {
            for (database, (_, copied)) in &s.databases {
                let floor = floors.entry(database.clone()).or_insert(*copied);
                *floor = (*floor).min(*copied);
            }
        }
        for (path, mut s) in snaps {
            if !kept.contains(&s.time) {
                place.delete(&path)?;
                out.snapshots += 1;
                drop.extend(names(&s));
                continue;
            }
            if s.time != newest {
                let gone: Vec<String> = s.blobs.iter().filter(|(h, o)| fetchable.contains(*h) && !latest.contains(*h) && !unsettled.contains(*o)).map(|(h, _)| h.clone()).collect();
                if !gone.is_empty() {
                    for h in gone {
                        drop.extend(s.blobs.remove(&h));
                    }
                    place.put(&path, &lock_snapshot(key, &s))?;
                }
            }
            keep.extend(names(&s));
        }
        // The bucket's copy of each database goes up to there: no kept or recent snapshot starts from an older copy or
        // step. A run builds on one of those, from its copy or a newer one.
        if !opened {
            continue;
        }
        for (database, floor) in floors.into_iter().filter(|(_, f)| *f > 0) {
            let prefix = format!("{install}/bucket/{dir}/{database}");
            let steps = place.objects(&prefix)?.into_iter().filter(|o| o.0.parse::<u64>().is_ok_and(|n| n <= floor));
            let copies = place.objects(&format!("{prefix}/copy"))?.into_iter().filter(|o| o.0.parse::<u64>().is_ok_and(|n| n < floor));
            for (name, size) in steps.map(|o| (format!("{prefix}/{}", o.0), o.2)).chain(copies.map(|o| (format!("{prefix}/copy/{}", o.0), o.2))).collect::<Vec<_>>() {
                place.delete(&name)?;
                out.bytes += size;
                out.objects += 1;
            }
        }
    }
    // Listed again: an object a run sent since the first listing has just been written.
    for (name, written, size) in place.objects(&format!("{install}/objects"))? {
        let settled = |wait: u64| written + wait <= now;
        if keep.contains(&name) || !(drop.contains(&name) && settled(SETTLE) || opened_all && settled(STRAY)) {
            continue;
        }
        place.delete(&format!("{install}/objects/{name}"))?;
        out.bytes += size;
        out.objects += 1;
    }
    Ok(out)
}

/// Backups' tools, which are the node's, not an app's.
pub fn tools() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({ "name": "backups_status", "description": "How backups are going: each place backed up to (a folder on a device's disk, or S3-compatible storage), when it last backed up and by which device, what's failing and since when, and the recent snapshots.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } }),
        json!({ "name": "backups_now", "description": "Back up now, without waiting for the hour: each device backs up what it holds to each place.", "inputSchema": { "type": "object", "properties": {} } }),
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

/// Every scope's epoch keys in an install's backup, opened with the recovery key's secret.
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
/// then says what came back: each scope's entries and blobs, and by name the people, apps and Vault items; and how
/// much the install's backup takes there.
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
        scopes.push(serde_json::json!({ "scope": scope, "time": r.snapshot.time, "entries": added, "blobs": r.blobs.len(), "databases": r.snapshot.databases.len() }));
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
    let bytes: u64 = place.objects(&format!("{install}/objects"))?.iter().map(|o| o.2).sum();
    Ok(serde_json::json!({
        "install": m.name,
        "bytes": bytes,
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
    fn pruning_drops_old_snapshots_and_releases_and_what_only_they_named() {
        let dir = std::env::temp_dir().join(format!("backups-prune-{}", hex(&rand::random())));
        let place = Place::Folder(dir.clone());
        let install = "0123456789abcdef";
        let recovery = SigningKey::generate();
        let device = SigningKey::generate();
        let key: [u8; 32] = rand::random();
        let blob = |b: &[u8]| (blake3::hash(b).to_hex().to_string(), b.to_vec());
        let (release1, release2, repo) = (blob(b"release 1"), blob(b"release 2"), blob(b"a repo's pack"));
        let all = entries(&device, "global", 8, None);
        let real = jiff::Timestamp::now().as_millisecond() as u64;
        let ten_days_ago = (real / DAY - 10) * DAY;
        let mut previous = None;
        // Two on one day ten days ago, with the old release; then an hour ago and now, with the new one.
        let runs = [(ten_days_ago + HOUR, 2, &release1), (ten_days_ago + 2 * HOUR, 4, &release1), (real - HOUR, 6, &release2), (real, 8, &release2)];
        let mut written = Vec::new();
        for (now, n, release) in runs {
            let job = Job {
                place: &place,
                install,
                scope: "global",
                key,
                sealed: crate::kernel::seal_to(&crate::kernel::twin_public(recovery.verifying_key().as_bytes()).unwrap(), &key).unwrap(),
                device: *device.verifying_key().as_bytes(),
                now,
                previous: previous.clone(),
                entries: all[..n].to_vec(),
                blobs: [release.0.clone(), repo.0.clone()].into(),
                bucket: None,
            };
            let s = back_up(job, |h| [&release1, &release2, &repo].into_iter().find(|b| b.0 == h).map(|b| b.1.clone())).unwrap().unwrap();
            written.push(s.clone());
            previous = Some(s);
        }
        assert!(!written[2].blobs.contains_key(&release1.0), "a new snapshot leaves out the release it no longer keeps");
        let objects = |place: &Place| place.objects(&format!("{install}/objects")).unwrap().into_iter().map(|o| o.0).collect::<BTreeSet<_>>();
        let release1_object = written[0].blobs[&release1.0].clone();
        let scopes: BTreeMap<String, [u8; 32]> = [("global".to_string(), key)].into();
        let fetchable: BTreeSet<String> = [release1.0.clone(), release2.0.clone()].into();

        // Just written, the old release waits: a run may be sending it again right now.
        let first = prune(&place, install, &scopes, &fetchable, real).unwrap();
        assert_eq!((first.snapshots, first.objects), (1, 0), "the day's earlier snapshot goes; no object yet");
        assert!(objects(&place).contains(&release1_object));
        let stray = format!("{install}/objects/{}", hex(&rand::random()));
        place.put(&stray, b"from a run that failed partway").unwrap();

        // Once it has waited, it goes, and the snapshot that named it no longer does.
        let later = prune(&place, install, &scopes, &fetchable, real + 2 * HOUR).unwrap();
        assert_eq!((later.snapshots, later.objects), (0, 1));
        assert!(later.bytes > b"release 1".len() as u64);
        assert!(!objects(&place).contains(&release1_object));
        assert!(place.get(&stray).unwrap().is_some(), "a stray object waits a day, for a run still going");
        let names = place.list(&format!("{install}/snapshots/global")).unwrap();
        assert_eq!(names.len(), 3);
        let day_old = open_snapshot(&[key], &place.get(&format!("{install}/snapshots/global/{}", names[0])).unwrap().unwrap()).unwrap().unwrap().0;
        assert_eq!(day_old.blobs.keys().collect::<Vec<_>>(), vec![&repo.0], "the old snapshot keeps the repo, not the release");

        // A day on, the stray goes too, since every snapshot here opened.
        assert_eq!(prune(&place, install, &scopes, &fetchable, real + 2 * DAY).unwrap().objects, 1);
        assert!(place.get(&stray).unwrap().is_none());
        // Not when a scope's snapshots are another key's: they may name it.
        place.put(&stray, b"again").unwrap();
        place.put(&format!("{install}/snapshots/user-0123456789abcdef/{}", snapshot_name(real, &[0; 32])), &lock_snapshot(&rand::random(), &Snapshot::default())).unwrap();
        assert_eq!(prune(&place, install, &scopes, &fetchable, real + 2 * DAY).unwrap().objects, 0);

        // What's left restores, and every object it names is there.
        let restored = restore(&place, install, "global", &[key]).unwrap();
        assert_eq!(restored.entries.len(), 8);
        assert_eq!(restored.blobs.iter().map(|b| &b.0).collect::<BTreeSet<_>>(), [&release2.0, &repo.0].into());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retention_keeps_the_last_of_each_hour_and_the_bucket_copy_only_what_kept_snapshots_need() {
        let now = 100 * DAY;
        let times = [now - 3 * HOUR + 60_000, now - 3 * HOUR + 50 * 60_000, now - 2 * DAY - HOUR, now - HOUR + 60_000, now - 10];
        assert_eq!(kept(&times, now), [now - 2 * DAY - HOUR, now - 3 * HOUR + 50 * 60_000, now - 10].into(), "the last of each hour, and of each day");

        let dir = std::env::temp_dir().join(format!("backups-copy-{}", hex(&rand::random())));
        let (place, bucket) = (Place::Folder(dir.join("place")), Place::Folder(dir.join("bucket")));
        let install = "0123456789abcdef";
        let (recovery, device) = (SigningKey::generate(), SigningKey::generate());
        let key: [u8; 32] = rand::random();
        let all = entries(&device, "global", 2, None);
        let prefix = "global/board/main";
        let real = jiff::Timestamp::now().as_millisecond() as u64;
        let hour = (real / HOUR - 2) * HOUR;
        let mut previous = None;
        // Steps 1 to 5; then a copy at 8 and steps to 10, both backed up in one hour.
        for (n, (at, copy)) in [(1, (5, 0)), (2, (10, 8))] {
            for step in 1..=at {
                bucket.put(&crate::db::step_name(prefix, step), format!("step {step}").as_bytes()).unwrap();
            }
            if copy > 0 {
                bucket.put(&crate::db::copy_name(prefix, copy), b"copy").unwrap();
            }
            bucket.put(&crate::db::newest_name(prefix), &serde_json::to_vec(&json!({ "at": at, "copy": copy })).unwrap()).unwrap();
            let job = Job {
                place: &place,
                install,
                scope: "global",
                key,
                sealed: crate::kernel::seal_to(&crate::kernel::twin_public(recovery.verifying_key().as_bytes()).unwrap(), &key).unwrap(),
                device: *device.verifying_key().as_bytes(),
                now: hour + n as u64 * 60_000,
                previous: previous.clone(),
                entries: all[..n].to_vec(),
                blobs: BTreeSet::new(),
                bucket: Some(&bucket),
            };
            previous = back_up(job, |_| None).unwrap();
        }
        let copied = |name: &str| place.get(&format!("{install}/bucket/{name}")).unwrap().is_some();
        assert!((1..=5).all(|n| copied(&crate::db::step_name(prefix, n))));

        let scopes: BTreeMap<String, [u8; 32]> = [("global".to_string(), key)].into();
        // While another device's run may still be building on the hour's first snapshot, the steps it starts from stay.
        let pruned = prune(&place, install, &scopes, &BTreeSet::new(), hour + 3 * 60_000).unwrap();
        assert_eq!((pruned.snapshots, pruned.objects), (1, 0), "the hour's first snapshot, not yet the steps it started from");
        assert!((1..=5).all(|n| copied(&crate::db::step_name(prefix, n))));
        let pruned = prune(&place, install, &scopes, &BTreeSet::new(), real).unwrap();
        assert_eq!((pruned.snapshots, pruned.objects), (0, 5), "then the steps only it started from");
        assert!((1..=5).all(|n| !copied(&crate::db::step_name(prefix, n))));
        assert!(copied(&crate::db::copy_name(prefix, 8)) && (9..=10).all(|n| copied(&crate::db::step_name(prefix, n))), "the kept snapshot's copy and steps stay");
        assert_eq!(restore(&place, install, "global", &[key]).unwrap().snapshot.databases["board/main"], (10, 8));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
