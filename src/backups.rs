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
use serde_json::{Value, json};

use crate::bucket::{Place, folder, scope_of};
use crate::database::scopes::ScopeDb;
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
    /// Every epoch's key the device holds of the scope, which open its databases' copies to choose one that holds.
    pub epochs: crate::db::Keys,
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
    let Job { place, install, scope, key, epochs, sealed, device, now, previous, entries, blobs, bucket } = job;
    let mut snap = previous.clone().unwrap_or_else(|| Snapshot { scope: scope.to_string(), ..Default::default() });
    if let Some(bucket) = bucket {
        copy_bucket(bucket, place, install, scope, &epochs, &mut snap)?;
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
/// as the bucket is: each epoch's key and Vault key sealed to the recovery key, each app's key in each epoch (sealed under it), and
/// each database's new steps, the signatures beside them (and those of the last few steps it copied before, which may
/// have come just after their steps), and newest copy that holds (sealed as they are), past what `snap` says it has.
fn copy_bucket(bucket: &Place, place: &Place, install: &str, scope: &str, epochs: &crate::db::Keys, snap: &mut Snapshot) -> Result<()> {
    /// How many of the last steps a run copied the next looks for signatures of again.
    const SIGNED_LATE: u64 = 4;
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
    // And each of its Vault's keys, sealed to the recovery key, so what's sealed in its items comes back with it.
    for n in bucket.list(&format!("{folder}/_vault"))? {
        copy(&format!("{folder}/_vault/{n}/recovery"), true)?;
    }
    for app in bucket.list(&folder)?.into_iter().filter(|a| crate::members::valid_app_name(a) || a == crate::members::APP) {
        for n in bucket.list(&format!("{folder}/{app}/_key"))? {
            copy(&format!("{folder}/{app}/_key/{n}"), true)?;
        }
        for database in bucket.list(&format!("{folder}/{app}"))?.into_iter().filter(|d| !d.starts_with('_')) {
            let prefix = format!("{folder}/{app}/{database}");
            let Some(hint) = bucket.get(&crate::db::newest_name(&prefix))? else { continue };
            let (_, hinted, epoch) = crate::db::newest(&hint);
            let (mut at, mut copied) = snap.databases.get(&format!("{app}/{database}")).copied().unwrap_or((0, 0));
            let newest_copy = newest_holding(bucket, scope, &prefix, &app, &database, epochs, hinted, copied)?;
            if newest_copy > copied && copy(&crate::db::copy_name(&prefix, newest_copy), false)? {
                copied = newest_copy;
                at = at.max(newest_copy);
            }
            // The steps before the copy aren't kept here, so a reader of a restored bucket goes on from it.
            let tidied = crate::db::tidied_name(&prefix);
            if copied > 0 && !copy(&tidied, true)? {
                place.put(&format!("{install}/bucket/{tidied}"), b"{}")?;
            }
            let signed = crate::db::signed_beside(&prefix);
            // And the step at the copy's position, without which a reader doesn't start from it (`db::start`), with the
            // signature beside it, which says what the copy comes to.
            if copied > 0 {
                copy(&crate::db::step_name(&prefix, copied), true)?;
                if signed {
                    copy(&crate::db::sig_name(&prefix, copied), true)?;
                }
            }
            // A signature is written just after its step, so the last steps a run copied may have come without theirs.
            for n in (copied.max(at.saturating_sub(SIGNED_LATE)) + 1..=at).filter(|_| signed) {
                copy(&crate::db::sig_name(&prefix, n), true)?;
            }
            while copy(&crate::db::step_name(&prefix, at + 1), false)? {
                if signed {
                    copy(&crate::db::sig_name(&prefix, at + 1), false)?;
                }
                at += 1;
            }
            place.put(&format!("{install}/bucket/{}", crate::db::newest_name(&prefix)), &serde_json::to_vec(&serde_json::json!({ "at": at, "copy": copied, "epoch": if copied == hinted { epoch } else { 0 } }))?)?;
            snap.databases.insert(format!("{app}/{database}"), (at, copied));
        }
    }
    Ok(())
}

/// The newest copy of a database above `above` that holds, as a reader chooses it (`db::sealed_copy_holds`): the newest
/// hint's, `hinted`, first, then the others, newest first; `above` if none does. A scope's log, whose copies are its
/// entries, not a database, takes the hint's.
#[allow(clippy::too_many_arguments)]
fn newest_holding(bucket: &Place, scope: &str, prefix: &str, app: &str, database: &str, epochs: &crate::db::Keys, hinted: u64, above: u64) -> Result<u64> {
    if (app, database) == (crate::members::APP, crate::log::NAME) {
        return Ok(hinted.max(above));
    }
    let mut keys = crate::db::Keys::new();
    for (n, epoch) in epochs {
        let Some(sealed) = bucket.get(&crate::keys::app_key_name(scope, app, *n))? else { continue };
        if let Ok(key) = crate::kernel::open_app_key(epoch, app, &sealed) {
            keys.insert(*n, crate::kernel::database_key(&key, database));
        }
    }
    let mut copies: Vec<u64> = bucket.list(&format!("{prefix}/copy"))?.iter().filter_map(|n| n.parse().ok()).filter(|n| *n != hinted).collect();
    copies.sort_unstable_by(|a, b| b.cmp(a));
    for at in std::iter::once(hinted).chain(copies).filter(|c| *c > above) {
        let Some(copy) = bucket.get(&crate::db::copy_name(prefix, at))? else { continue };
        let step = bucket.get(&crate::db::step_name(prefix, at))?;
        let beside = bucket.get(&crate::db::sig_name(prefix, at))?;
        if crate::db::sealed_copy_holds(prefix, &keys, at, &copy, step.as_deref(), beside.as_deref()) {
            return Ok(at);
        }
    }
    Ok(above)
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
            // The step at the copy's position stays with it, and its signature: a reader starts from a copy only beside
            // its step. The rest go with the signatures beside them.
            let position = |name: &str| name.strip_suffix(".sig").unwrap_or(name).parse::<u64>().ok();
            let steps = place.objects(&prefix)?.into_iter().filter(|o| position(&o.0).is_some_and(|n| n < floor));
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
    use crate::skills::{confirm_input, hints};
    use serde_json::json;
    vec![
        json!({ "name": "backups_status", "description": "How backups are going: each place backed up to (a folder on a device's disk, or S3-compatible storage), when it last backed up and by which device, what's failing and since when, and the recent snapshots.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(true, false) }),
        json!({ "name": "backups_now", "description": "Back up now, without waiting for the hour: each device backs up what it holds to each place.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(false, false) }),
        json!({ "name": "backups_add_place", "description": "Add S3-compatible storage (Cloudflare R2, Backblaze B2, MinIO…) to back up to. You give the endpoint; the person gets a link to a sheet on their own device where they type the bucket, access key ID and secret key, so you never see the key. backups_status then shows its first backup, or what went wrong.", "inputSchema": { "type": "object", "required": ["endpoint"], "properties": { "endpoint": { "type": "string", "description": "like https://<account id>.r2.cloudflarestorage.com" }, "title": { "type": "string", "description": "what the person calls it, like Cloudflare R2" } } }, "annotations": hints(false, false) }),
        json!({ "name": "backups_remove_place", "description": "Stop backing up to a place. What's already there stays.", "inputSchema": { "type": "object", "required": ["place"], "properties": { "place": { "type": "string", "description": "its title or id, as backups_status lists it" }, "confirm": confirm_input() } }, "annotations": hints(false, true) }),
        json!({ "name": "backups_check", "description": "Restore each place's newest backup into a throwaway copy now, as the weekly check does: every entry's signature and order, that they replay to the same people and apps, every database to its newest step as the install has it, and every blob's hash. Says what came back, and what's wrong.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(true, false) }),
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

/// Puts restored entries in a store as sync does, each once everything it follows is there, so every chain is checked.
pub fn import(store: &crate::store::Store, entries: &[Entry]) -> Result<usize> {
    Ok(store.take(entries.to_vec())?.0.len())
}

/// A restore check, as a device runs one: what it restores, and what it compares that with.
pub struct Check<'a> {
    pub place: &'a Place,
    /// The install's folder in the place.
    pub install: &'a str,
    /// The keys that may open each scope's snapshots: its epochs' keys.
    pub keys: &'a BTreeMap<String, Vec<[u8; 32]>>,
    pub root: Option<crate::entry::Hash>,
    /// Each scope's entries as this device holds them: every one a snapshot should hold must be in it.
    pub live: Option<&'a BTreeMap<String, Vec<Entry>>>,
}

/// Restores each scope's newest snapshot from a place into a throwaway store, checking every signature, chain and
/// blob; with `live`, that it holds every entry this device had then, and that its entries replay to the same people,
/// devices, groups, apps and settings. Says what came back: each scope's entries, blobs and databases, and by name the
/// people and apps; how much the install's backup takes there; what's wrong (`problems`), and the scopes whose snapshots
/// none of `keys` opens (`unchecked`); no scopes at all if nothing's there yet. Databases are `check_databases`'.
pub fn check(c: &Check) -> Result<(Value, Vec<Restored>)> {
    let (place, install) = (c.place, c.install);
    let store = crate::store::Store::memory()?;
    let mut scopes = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut unchecked = Vec::new();
    let mut restored = Vec::new();
    let mut expected: Vec<Entry> = Vec::new();
    for dir in place.list(&format!("{install}/snapshots"))? {
        let scope = scope_of(&dir);
        if snapshots(place, install, &scope)?.is_empty() {
            continue;
        }
        let Some(keys) = c.keys.get(&scope).filter(|k| !k.is_empty()) else {
            unchecked.push(scope);
            continue;
        };
        let r = match restore(place, install, &scope, keys) {
            Ok(r) => r,
            Err(err) => {
                problems.push(format!("{scope}: {err:#}"));
                continue;
            }
        };
        let added = import(&store, &r.entries)?;
        if added < r.entries.len() {
            problems.push(format!("{scope}: {} of its {} entries aren't signed or in order", r.entries.len() - added, r.entries.len()));
        }
        if let Some(live) = c.live.and_then(|l| l.get(&scope)) {
            let had: Vec<&Entry> = live.iter().filter(|e| e.seq() < r.snapshot.frontier.get(&hex(&e.author())).copied().unwrap_or(0)).collect();
            let backed: BTreeSet<_> = r.entries.iter().map(Entry::hash).collect();
            let lacks = had.iter().filter(|e| !backed.contains(&e.hash())).count();
            if lacks > 0 {
                problems.push(format!("{scope}: it lacks {lacks} of the entries it should hold"));
            }
            expected.extend(had.into_iter().cloned());
        }
        scopes.push(json!({ "scope": scope, "time": r.snapshot.time, "entries": added, "blobs": r.blobs.len(), "databases": r.snapshot.databases.len() }));
        restored.push(r);
    }
    let root = c.root.or_else(|| store.ordered().ok()?.iter().find(|e| e.content.app == crate::members::APP && e.content.kind == "genesis").map(|e| e.hash()));
    let global_keys = c.keys.get("global").map(Vec::as_slice).unwrap_or(&[]);
    let m = steps_of(members_of(&store, root)?, &store, root, place, install, global_keys)?;
    // The same entries say the same on every device: the backup's, replayed, are who's who as this device had it.
    if c.live.is_some() && problems.is_empty() {
        let here = crate::store::Store::memory()?;
        here.take(expected)?;
        if said(&steps_of(members_of(&here, root)?, &here, root, place, install, global_keys)?) != said(&m) {
            problems.push("its entries don't replay to the people, devices and apps this device had then".into());
        }
    }
    let bytes: u64 = place.objects(&format!("{install}/objects"))?.iter().map(|o| o.2).sum();
    let report = json!({
        "install": m.name(),
        "bytes": bytes,
        "scopes": scopes,
        "people": m.users().into_iter().filter(|u| !u.removed).map(|u| u.name).collect::<Vec<_>>(),
        "apps": m.apps().into_iter().filter(|a| !a.erased && a.bundle.is_some()).map(|a| a.name).collect::<Vec<_>>(),
        "problems": problems,
        "unchecked": unchecked,
    });
    Ok((report, restored))
}

/// Who's who once `global`'s log is frozen, as its scope database's steps the place's copy of the bucket holds say it
/// (`database::steps`), opened with `keys`, its epochs' keys; `replayed`, as the entries say it, before.
fn steps_of(replayed: ScopeDb, store: &crate::store::Store, root: Option<crate::entry::Hash>, place: &Place, install: &str, keys: &[[u8; 32]]) -> Result<ScopeDb> {
    let global: Vec<Entry> = store.ordered()?.into_iter().filter(|e| e.log() == "global").collect();
    let Some(frozen) = crate::members::frozen(root, &global, &store.forks()?) else { return Ok(replayed) };
    // The log's key in each epoch: the node's app key there, opened with whichever of the epochs' keys does.
    let mut log_keys: BTreeMap<u32, [u8; 32]> = BTreeMap::new();
    for n in 1..=(keys.len() as u32 + 8) {
        let Some(sealed) = place.get(&format!("{install}/bucket/{}", crate::keys::app_key_name("global", crate::members::APP, n)))? else { continue };
        if let Some(app) = keys.iter().find_map(|k| crate::kernel::open_app_key(k, crate::members::APP, &sealed).ok()) {
            log_keys.insert(n, crate::kernel::database_key(&app, crate::log::NAME));
        }
    }
    let prefix = crate::database::steps::prefix("global");
    let mut steps = Vec::new();
    for at in 1.. {
        let name = crate::db::step_name(&prefix, at);
        let Some(sealed) = place.get(&format!("{install}/bucket/{name}"))? else { break };
        let opened = crate::keys::epoch_of(&sealed).ok().and_then(|e| log_keys.get(&e)).and_then(|k| crate::keys::open(k, &name, &sealed).ok());
        steps.push(crate::database::steps::given(&crate::entry::Scope::Global, at, opened.as_deref()));
    }
    crate::database::steps::truth(&frozen, &steps, crate::node::now_ms())
}

/// Who's who, as the member entries in a store replay: `global`'s scope database.
fn members_of(store: &crate::store::Store, root: Option<crate::entry::Hash>) -> Result<ScopeDb> {
    let member_entries: Vec<Entry> = store.ordered()?.into_iter().filter(crate::members::is_member_entry).collect();
    let m = crate::members::Members::replay(root, &member_entries, &store.forks()?);
    ScopeDb::build(&m, &crate::entry::Scope::Global, &Default::default(), crate::node::now_ms())
}

/// What a restore must bring back the same: the install's name, its people, devices, groups, apps and settings, and
/// each scope's epochs.
fn said(m: &ScopeDb) -> Value {
    let epochs = m.epochs().into_keys().flat_map(|s| m.epoch_hashes(&s).into_iter().map(move |(n, h)| format!("{s} {n} {h}"))).collect::<Vec<_>>();
    json!({
        "name": m.name(),
        "users": m.users().into_iter().map(|u| (u.id, u.name, u.removed)).collect::<Vec<_>>(),
        "machines": m.machines().iter().map(|d| hex(&d.key)).collect::<Vec<_>>(),
        "groups": m.groups().into_iter().map(|g| g.id).collect::<Vec<_>>(),
        "apps": m.apps().into_iter().map(|a| (a.name, a.bundle, a.erased)).collect::<Vec<_>>(),
        "settings": m.settings(),
        "epochs": epochs,
    })
}

/// A place's copy of the install's bucket (`<install>/bucket/`), read as the bucket is, so a database restores from it
/// as from the bucket itself. Nothing writes to it.
struct Copy<'a> {
    place: &'a Place,
    root: String,
}

impl crate::bucket::Bucket for Copy<'_> {
    async fn get(&self, name: &str) -> Result<Option<crate::bucket::Object>> {
        crate::bucket::Bucket::get(self.place, &format!("{}{name}", self.root)).await
    }
    async fn create(&self, _: &str, _: &[u8]) -> Result<bool> {
        anyhow::bail!("a backup is only read")
    }
    async fn replace(&self, _: &str, _: &[u8], _: Option<&str>) -> Result<bool> {
        anyhow::bail!("a backup is only read")
    }
    async fn wait(&self, ms: u64) {
        crate::bucket::Bucket::wait(self.place, ms).await
    }
}

/// Checks each of a scope's databases as its snapshot holds them: its copy, the step beside it and every step after,
/// to the snapshot's last, are in the place, and are the same bytes as the install's bucket has (`live`), or without
/// it, open with their epoch's key; then it restores from them into a throwaway database, which must reach that step.
/// `epochs` are the scope's epoch keys. → what's wrong, one line each; nothing when all is well.
pub async fn check_databases(place: &Place, install: &str, snapshot: &Snapshot, epochs: &crate::db::Keys, live: Option<&impl crate::bucket::Bucket>) -> Vec<String> {
    use crate::bucket::Bucket;
    let (scope, folder) = (&snapshot.scope, folder(&snapshot.scope));
    let copy = Copy { place, root: format!("{install}/bucket/") };
    let mut out = Vec::new();
    for (database, (at, copied)) in &snapshot.databases {
        let Some((app, db)) = database.split_once('/') else { continue };
        let prefix = format!("{folder}/{app}/{db}");
        // What a person calls it: the app, or of the node's own, its Vault, its history (entries) or its status rows.
        let said = match (app, db) {
            (crate::members::APP, "vault") => "Vault".to_string(),
            (crate::members::APP, crate::log::NAME) => "history".to_string(),
            (crate::members::APP, crate::database::steps::NAME) => "who's who".to_string(),
            (crate::members::APP, _) => "status".to_string(),
            (app, "main") => app.to_string(),
            (app, db) => format!("{app} {db}"),
        };
        let mut keys = crate::db::Keys::new();
        for (n, epoch) in epochs {
            let Ok(Some(sealed)) = copy.get(&crate::keys::app_key_name(scope, app, *n)).await else { continue };
            match crate::kernel::open_app_key(epoch, app, &sealed.bytes) {
                // A scope database's steps are sealed as its log is (`database::steps`).
                Ok(key) => drop(keys.insert(*n, crate::kernel::database_key(&key, if (app, db) == (crate::members::APP, crate::database::steps::NAME) { crate::log::NAME } else { db }))),
                Err(err) => out.push(format!("{scope} {said}: its key in epoch {n}: {err:#}")),
            }
        }
        let copy_of = (*copied > 0).then(|| (format!("its copy at step {copied}"), crate::db::copy_name(&prefix, *copied)));
        let names = copy_of.into_iter().chain((((*copied).max(1))..=*at).map(|n| (format!("step {n}"), crate::db::step_name(&prefix, n))));
        let mut wrong = None;
        for (said_as, name) in names {
            let problem = match copy.get(&name).await {
                Err(err) => Some(format!("{err:#}")),
                Ok(None) => Some("isn't there".to_string()),
                Ok(Some(o)) => match live {
                    Some(live) => match live.get(&name).await {
                        Ok(Some(l)) if l.bytes != o.bytes => Some("isn't what your devices have".to_string()),
                        Ok(Some(_)) => None,
                        // Gone from the bucket, or not reached: it must open, at least.
                        _ => opens(&keys, &name, &o.bytes),
                    },
                    None => opens(&keys, &name, &o.bytes),
                },
            };
            if let Some(problem) = problem {
                wrong = Some(format!("{scope} {said}: {said_as} {problem}"));
                break;
            }
        }
        if let Some(wrong) = wrong {
            out.push(wrong);
            continue;
        }
        // The node's log of a scope is entries, which `check` restored; the rest are SQLite.
        if database == &format!("{}/{}", crate::members::APP, crate::log::NAME) || keys.is_empty() {
            continue;
        }
        match crate::db::Db::open(&copy, &prefix, keys).await {
            Ok(restored) if restored.at() >= *at => {}
            Ok(restored) => out.push(format!("{scope} {said}: it restores only to step {} of {at}", restored.at())),
            Err(err) => out.push(format!("{scope} {said}: it doesn't restore: {err:#}")),
        }
    }
    out
}

/// What's wrong with a sealed step or copy, if it doesn't open with its epoch's key: nothing, too, when that key isn't
/// one of `keys`.
fn opens(keys: &crate::db::Keys, name: &str, sealed: &[u8]) -> Option<String> {
    let epoch = match crate::keys::epoch_of(sealed) {
        Ok(e) => e,
        Err(err) => return Some(format!("{err:#}")),
    };
    let key = keys.get(&epoch)?;
    crate::keys::open(key, name, sealed).err().map(|_| "doesn't open".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{Content, Ext};
    use ed25519_dalek::SigningKey;
    use serde_json::json;

    fn entries(key: &SigningKey, log: &str, n: u32, blob: Option<&str>) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        for i in 0..n {
            let mut content = Content::new("board", "post", json!({ "text": format!("post {i}") }));
            if let Some(b) = blob {
                content.blobs.push(b.to_string());
            }
            let ext = Ext { log: log.into(), time: 1000 + i as u64, user: "0123456789abcdef".into(), epoch: 0, previous: Vec::new() };
            out.push(Entry::create(key, ext, out.last().map(|e| e.head()), content));
        }
        out
    }

    #[tokio::test]
    async fn a_restore_check_passes_a_whole_backup_and_catches_one_corrupted_or_missing() {
        let dir = std::env::temp_dir().join(format!("backups-check-{}", hex(&rand::random())));
        let (place, bucket) = (Place::Folder(dir.join("place")), Place::Folder(dir.join("bucket")));
        let install = "0123456789abcdef";
        let (recovery, device) = (SigningKey::from_bytes(&rand::random()), SigningKey::from_bytes(&rand::random()));
        let epoch: [u8; 32] = rand::random();
        let blob = b"a deploy's bundle".to_vec();
        let hash = blake3::hash(&blob).to_hex().to_string();
        let all = entries(&device, "global", 3, Some(&hash));

        // A database with more steps than one copy holds, so a reader starts from its copy.
        let prefix = "global/board/main";
        let app = crate::db::app_key(&bucket, &epoch, "global", "board", 1, true).await.unwrap().unwrap();
        let keys = crate::db::Keys::from([(1, crate::kernel::database_key(&app, "main"))]);
        let mut db = crate::db::Db::open(&bucket, prefix, keys).await.unwrap();
        db.sign_as(device.clone());
        db.migrate(&bucket, &["create table posts (text text)".to_string()]).await.unwrap();
        for n in 0..crate::db::COPY_EVERY + 5 {
            db.write(&bucket, |c| Ok(c.execute("insert into posts values (?)", [format!("post {n}")])?)).await.unwrap();
        }
        let at = db.at();
        let (_, copy, _) = crate::db::newest(&crate::bucket::Bucket::get(&bucket, &crate::db::newest_name(prefix)).await.unwrap().unwrap().bytes);
        assert!(copy > 0 && copy < at, "it has a copy, and steps after it");
        // The last step's signature, which comes just after it, comes after the run that copies the step.
        let late = crate::db::sig_name(prefix, at);
        let sig = bucket.get(&late).unwrap().unwrap();
        bucket.delete(&late).unwrap();

        // Junk at a copy's position, and the newest hint naming it, from a device without the key: the backup copies
        // the copy that holds.
        let junk = crate::db::copy_name(prefix, 10 * crate::db::COPY_EVERY);
        bucket.put(&junk, &crate::keys::seal(1, &rand::random(), &junk, b"junk")).unwrap();
        bucket.put(&crate::db::newest_name(prefix), &serde_json::to_vec(&json!({ "at": at, "copy": 10 * crate::db::COPY_EVERY })).unwrap()).unwrap();

        let job = |previous: Option<Snapshot>, now: u64| Job {
            place: &place,
            install,
            scope: "global",
            key: epoch,
            epochs: crate::db::Keys::from([(1, epoch)]),
            sealed: crate::kernel::seal_to(&crate::kernel::twin_public(recovery.verifying_key().as_bytes()).unwrap(), &epoch).unwrap(),
            device: *device.verifying_key().as_bytes(),
            now,
            previous,
            entries: all.clone(),
            blobs: [hash.clone()].into(),
            bucket: Some(&bucket),
        };
        let first = back_up(job(None, 1_000_000), |h| (h == hash).then(|| blob.clone())).unwrap().unwrap();
        let backed_up = format!("{install}/bucket/{late}");
        assert!(place.get(&backed_up).unwrap().is_none());
        bucket.put(&late, &sig).unwrap();
        assert_eq!(back_up(job(Some(first), 1_500_000), |h| (h == hash).then(|| blob.clone())).unwrap(), None, "nothing new for a snapshot");
        assert_eq!(place.get(&backed_up).unwrap(), Some(sig), "the next run copies it");

        let keys: BTreeMap<String, Vec<[u8; 32]>> = [("global".to_string(), vec![epoch])].into();
        let live: BTreeMap<String, Vec<Entry>> = [("global".to_string(), all.clone())].into();
        let epochs = crate::db::Keys::from([(1, epoch)]);
        let checked = |live: Option<&BTreeMap<String, Vec<Entry>>>| {
            let (report, restored) = check(&Check { place: &place, install, keys: &keys, root: None, live }).unwrap();
            (report["problems"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect::<Vec<_>>(), restored)
        };
        let (problems, restored) = checked(Some(&live));
        assert_eq!(problems, Vec::<String>::new());
        let snapshot = restored[0].snapshot.clone();
        assert_eq!(snapshot.databases["board/main"], (at, copy));
        assert_eq!(check_databases(&place, install, &snapshot, &epochs, Some(&bucket)).await, Vec::<String>::new(), "every database restores from the backup");
        assert_eq!(check_databases(&place, install, &snapshot, &epochs, None::<&Place>).await, Vec::<String>::new(), "and opens without the install's bucket");

        // A step that's changed in the backup, then one that's gone: the one its copy starts beside.
        let name = |n: u64| format!("{install}/bucket/{}", crate::db::step_name(prefix, n));
        let good = place.get(&name(at)).unwrap().unwrap();
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        place.put(&name(at), &bad).unwrap();
        let wrong = check_databases(&place, install, &snapshot, &epochs, Some(&bucket)).await;
        assert!(wrong.len() == 1 && wrong[0].contains(&format!("global board: step {at} isn't what your devices have")), "{wrong:?}");
        let wrong = check_databases(&place, install, &snapshot, &epochs, None::<&Place>).await;
        assert!(wrong.len() == 1 && wrong[0].contains("doesn't open"), "{wrong:?}");
        place.put(&name(at), &good).unwrap();
        place.delete(&name(copy)).unwrap();
        let wrong = check_databases(&place, install, &snapshot, &epochs, Some(&bucket)).await;
        assert!(wrong.len() == 1 && wrong[0].contains(&format!("global board: step {copy} isn't there")), "{wrong:?}");

        // A blob that's gone, then a pack that's corrupted.
        let object = |name: &str| format!("{install}/objects/{name}");
        let blob_object = place.get(&object(&snapshot.blobs[&hash])).unwrap().unwrap();
        place.delete(&object(&snapshot.blobs[&hash])).unwrap();
        let (problems, _) = checked(Some(&live));
        assert!(problems.len() == 1 && problems[0].contains("is missing"), "{problems:?}");
        place.put(&object(&snapshot.blobs[&hash]), &blob_object).unwrap();
        let good = place.get(&object(&snapshot.packs[0])).unwrap().unwrap();
        let mut bad = good.clone();
        bad[0] ^= 1;
        place.put(&object(&snapshot.packs[0]), &bad).unwrap();
        let (problems, _) = checked(Some(&live));
        assert!(problems.len() == 1 && problems[0].contains("didn't decrypt"), "{problems:?}");
        place.put(&object(&snapshot.packs[0]), &good).unwrap();

        // And a newest snapshot that says it holds an entry this device had, which it doesn't.
        let more: BTreeMap<String, Vec<Entry>> = [("global".to_string(), entries(&device, "global", 4, Some(&hash)))].into();
        let mut said_more = snapshot.clone();
        said_more.frontier.insert(hex(device.verifying_key().as_bytes()), 4);
        place.put(&format!("{install}/snapshots/global/{}", snapshot_name(2_000_000, device.verifying_key().as_bytes())), &lock_snapshot(&epoch, &said_more)).unwrap();
        let (problems, _) = checked(Some(&more));
        assert!(problems.len() == 1 && problems[0].contains("lacks 1 of the entries"), "{problems:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
