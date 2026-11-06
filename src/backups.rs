//! Backups: each scope's databases, copied as the bucket keeps them (sealed), and the blobs they name, encrypted with the
//! key of the scope's newest epoch, sent to a place that can't read them: a folder on one device's disk, or
//! S3-compatible storage. Each epoch's key is sealed to the network's recovery key (`keys::epoch_name`'s `recovery`), and the place
//! keeps that copy, so a backup and that key are all a recovery needs.
//!
//! A place holds a folder for each network, named by its root, so networks can share a bucket:
//!
//! ```text
//! <network>/keys/<scope>/<key id>              an epoch's key, sealed to the recovery key
//! <network>/objects/<name>                     a blob, encrypted
//! <network>/bucket/<name>                      the bucket's own objects of each scope, as it keeps them
//! <network>/snapshots/<scope>/<time>-<device>  what the scope's backup held then
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
use crate::keys::{Key, hex};

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

/// Opens an epoch's key sealed to the recovery key, with the recovery key's secret (its Ed25519 seed).
pub fn unseal(recovery_secret: &[u8; 32], sealed: &[u8]) -> Result<[u8; 32]> {
    let key = crate::keys::open_sealed(&crate::keys::twin(recovery_secret).0, sealed).map_err(|_| anyhow!("that recovery key doesn't open this backup"))?;
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

/// What a scope's backup held at a time: its databases and its blobs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub scope: String,
    pub time: u64,
    /// Each blob, by its hash, and the object it is.
    pub blobs: BTreeMap<String, String>,
    /// The time it holds the scope as of.
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

/// One scope's backup to one place, as a device does it.
pub struct Job<'a> {
    pub place: &'a Place,
    /// The network's folder in the place.
    pub network: &'a str,
    pub scope: &'a str,
    /// The key of the scope's newest epoch, and that key sealed to the recovery key, as the bucket keeps it.
    pub key: [u8; 32],
    /// Every epoch's key the device holds of the scope, which open its databases' copies to choose one that holds.
    pub epochs: crate::database::stored::Keys,
    pub sealed: Vec<u8>,
    pub device: Key,
    pub now: u64,
    /// The latest snapshot of the scope in the place with this key, if any.
    pub previous: Option<Snapshot>,
    /// The blobs the scope's databases name, which the backup keeps.
    pub blobs: BTreeSet<String>,
    /// The network's bucket, whose scope's folder the place keeps a copy of.
    pub bucket: Option<&'a Place>,
}

/// Sends a scope what its backup lacks: its key, sealed, the first time; what its folder in the bucket has that the
/// place lacks, its new blobs, and a snapshot. Returns the snapshot, or None when nothing new would be in it. `blob`
/// reads a blob here.
pub fn back_up(job: Job, blob: impl Fn(&str) -> Option<Vec<u8>>) -> Result<Option<Snapshot>> {
    let Job { place, network, scope, key, epochs, sealed, device, now, previous, blobs, bucket } = job;
    let mut snap = previous.clone().unwrap_or_else(|| Snapshot { scope: scope.to_string(), ..Default::default() });
    if let Some(bucket) = bucket {
        copy_bucket(bucket, place, network, scope, &epochs, &mut snap)?;
    }
    // Blobs the scope no longer keeps (a release that's been superseded, a blob its scope forgot) leave the snapshot,
    // and leave the place once no snapshot kept still has them.
    snap.blobs.retain(|hash, _| blobs.contains(hash));
    let new_blobs: Vec<&String> = blobs.iter().filter(|b| !snap.blobs.contains_key(*b)).collect();
    if previous.as_ref().is_some_and(|p| p.blobs.len() == snap.blobs.len() && p.databases == snap.databases) && new_blobs.is_empty() {
        return Ok(None);
    }
    let keys = format!("{network}/keys/{}/{}", folder(scope), key_id(&key));
    if previous.is_none() && place.get(&keys)?.is_none() {
        place.put(&keys, &sealed)?;
    }
    for hash in new_blobs {
        // A blob this device lacks waits for a run on one that has it, or for it to arrive here.
        let Some(bytes) = blob(hash) else { continue };
        let name = name_of(&key, &bytes);
        let locked = lock_object(&key, &name, &bytes);
        place.put(&format!("{network}/objects/{name}"), &locked)?;
        snap.bytes += locked.len() as u64;
        snap.blobs.insert(hash.clone(), name);
    }
    if previous.as_ref().is_some_and(|p| p.blobs == snap.blobs && p.databases == snap.databases) {
        return Ok(None);
    }
    snap.time = now;
    snap.upto = now;
    place.put(&format!("{network}/snapshots/{}/{}", folder(scope), snapshot_name(now, &device)), &lock_snapshot(&key, &snap))?;
    Ok(Some(snap))
}

/// Copies what a scope's folder in the network's bucket has that the place lacks, into `<network>/bucket/`, laid out
/// as the bucket is: each epoch's key and Vault key sealed to the recovery key, each app's key in each epoch (sealed under it), and
/// each database's new steps, the signatures beside them (and those of the last few steps it copied before, which may
/// have come just after their steps), and newest copy that holds (sealed as they are), past what `snap` says it has.
fn copy_bucket(bucket: &Place, place: &Place, network: &str, scope: &str, epochs: &crate::database::stored::Keys, snap: &mut Snapshot) -> Result<()> {
    /// How many of the last steps a run copied the next looks for signatures of again.
    const SIGNED_LATE: u64 = 4;
    let folder = folder(scope);
    let mut copy = |name: &str, known: bool| -> Result<bool> {
        if known && place.get(&format!("{network}/bucket/{name}"))?.is_some() {
            return Ok(true);
        }
        let Some(bytes) = bucket.get(name)? else { return Ok(false) };
        place.put(&format!("{network}/bucket/{name}"), &bytes)?;
        snap.bytes += bytes.len() as u64;
        Ok(true)
    };
    for n in bucket.list(&crate::keys::epochs_folder(scope))?.iter().filter_map(|n| n.parse().ok()) {
        copy(&crate::keys::epoch_name(scope, n, "recovery"), true)?;
    }
    // And each of its Vault's keys, sealed to the recovery key, so what's sealed in its items comes back with it.
    for n in bucket.list(&crate::keys::vault_keys_folder(scope))?.iter().filter_map(|n| n.parse().ok()) {
        copy(&crate::keys::vault_key_name(scope, n, "recovery"), true)?;
    }
    for app in bucket.list(&folder)?.into_iter().filter(|a| crate::database::scopes::valid_app_name(a) || a == crate::database::scopes::APP) {
        for n in bucket.list(&crate::keys::app_keys_folder(scope, &app))?.iter().filter_map(|n| n.parse().ok()) {
            copy(&crate::keys::app_key_name(scope, &app, n), true)?;
        }
        for database in bucket.list(&format!("{folder}/{app}"))?.into_iter().filter(|d| !d.starts_with('_')) {
            let prefix = crate::bucket::database_prefix(scope, &app, &database);
            let Some(hint) = bucket.get(&crate::database::stored::newest_name(&prefix))? else { continue };
            let (_, hinted, epoch) = crate::database::stored::newest(&hint);
            let (mut at, mut copied) = snap.databases.get(&format!("{app}/{database}")).copied().unwrap_or((0, 0));
            let newest_copy = newest_holding(bucket, scope, &prefix, &app, &database, epochs, hinted, copied)?;
            if newest_copy > copied && copy(&crate::database::stored::copy_name(&prefix, newest_copy), false)? {
                copied = newest_copy;
                at = at.max(newest_copy);
            }
            // The steps before the copy aren't kept here, so a reader of a restored bucket goes on from it.
            let tidied = crate::database::stored::tidied_name(&prefix);
            if copied > 0 && !copy(&tidied, true)? {
                place.put(&format!("{network}/bucket/{tidied}"), b"{}")?;
            }
            let signed = crate::database::stored::signed_beside(&prefix);
            // And the step at the copy's position, without which a reader doesn't start from it (`stored::start`), with the
            // signature beside it, which says what the copy comes to.
            if copied > 0 {
                copy(&crate::database::stored::step_name(&prefix, copied), true)?;
                if signed {
                    copy(&crate::database::stored::sig_name(&prefix, copied), true)?;
                }
            }
            // A signature is written just after its step, so the last steps a run copied may have come without theirs.
            for n in (copied.max(at.saturating_sub(SIGNED_LATE)) + 1..=at).filter(|_| signed) {
                copy(&crate::database::stored::sig_name(&prefix, n), true)?;
            }
            while copy(&crate::database::stored::step_name(&prefix, at + 1), false)? {
                if signed {
                    copy(&crate::database::stored::sig_name(&prefix, at + 1), false)?;
                }
                at += 1;
            }
            place.put(&format!("{network}/bucket/{}", crate::database::stored::newest_name(&prefix)), &serde_json::to_vec(&serde_json::json!({ "at": at, "copy": copied, "epoch": if copied == hinted { epoch } else { 0 } }))?)?;
            snap.databases.insert(format!("{app}/{database}"), (at, copied));
        }
    }
    Ok(())
}

/// The newest copy of a database above `above` that holds, as a reader chooses it (`stored::sealed_copy_holds`): the newest
/// hint's, `hinted`, first, then the others, newest first; `above` if none does. A scope's frozen log, whose copies are
/// its entries, not a database, takes the hint's.
#[allow(clippy::too_many_arguments)]
fn newest_holding(bucket: &Place, scope: &str, prefix: &str, app: &str, database: &str, epochs: &crate::database::stored::Keys, hinted: u64, above: u64) -> Result<u64> {
    if (app, database) == (crate::database::scopes::APP, crate::database::steps::KEY_NAME) {
        return Ok(hinted.max(above));
    }
    let mut keys = crate::database::stored::Keys::new();
    for (n, epoch) in epochs {
        let Some(sealed) = bucket.get(&crate::keys::app_key_name(scope, app, *n))? else { continue };
        if let Ok(key) = crate::keys::open_database_key(epoch, app, database, &sealed) {
            keys.insert(*n, key);
        }
    }
    let mut copies: Vec<u64> = bucket.list(&format!("{prefix}/copy"))?.iter().filter_map(|n| n.parse().ok()).filter(|n| *n != hinted).collect();
    copies.sort_unstable_by(|a, b| b.cmp(a));
    for at in std::iter::once(hinted).chain(copies).filter(|c| *c > above) {
        let Some(copy) = bucket.get(&crate::database::stored::copy_name(prefix, at))? else { continue };
        let step = bucket.get(&crate::database::stored::step_name(prefix, at))?;
        let beside = if crate::database::stored::signed_beside(prefix) { bucket.get(&crate::database::stored::sig_name(prefix, at))? } else { None };
        if crate::database::stored::sealed_copy_holds(prefix, &keys, at, &copy, step.as_deref(), beside.as_deref()) {
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
    format!("{time:013}-{}", crate::keys::short(device))
}

/// The newest snapshot of a scope in a place that one of `keys` opens, with its key.
pub fn latest(place: &Place, network: &str, scope: &str, keys: &[[u8; 32]]) -> Result<Option<(Snapshot, [u8; 32])>> {
    let dir = format!("{network}/snapshots/{}", folder(scope));
    for name in place.list(&dir)?.iter().rev() {
        let Some(bytes) = place.get(&format!("{dir}/{name}"))? else { continue };
        if let Some(found) = open_snapshot(keys, &bytes)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The snapshots of a scope in a place, newest first, by their times and devices' short keys, without opening them.
pub fn snapshots(place: &Place, network: &str, scope: &str) -> Result<Vec<(u64, String)>> {
    let names = place.list(&format!("{network}/snapshots/{}", folder(scope)))?;
    Ok(names.iter().rev().filter_map(|n| n.split_once('-').and_then(|(t, s)| Some((t.parse().ok()?, s.to_string())))).collect())
}

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

/// Prunes a network's backup in a place: drops the snapshots `kept` doesn't keep of each scope in `scopes` (by its
/// key), takes the blobs in `fetchable` (releases, which can be fetched again from where they're published) out of
/// every snapshot but each scope's newest, then deletes the objects no snapshot kept still names.
///
/// A run may be backing up at the same moment, so an object goes only once it has gone unwritten for `SETTLE` (a
/// run that sends it again has rewritten it), and the newest snapshot, which a run builds on, is never touched. When
/// `scopes` opens every snapshot in the place, objects no snapshot names at all (from a run that failed partway)
/// go too, once they've waited `STRAY`.
pub fn prune(place: &Place, network: &str, scopes: &BTreeMap<String, [u8; 32]>, fetchable: &BTreeSet<String>, now: u64) -> Result<Pruned> {
    let mut out = Pruned::default();
    let mut keep: BTreeSet<String> = BTreeSet::new();
    let mut drop: BTreeSet<String> = BTreeSet::new();
    let mut opened_all = true;
    let names = |s: &Snapshot| s.blobs.values().cloned().collect::<Vec<_>>();
    // A release leaves a snapshot only once its object has settled, so it leaves the place in the same prune.
    let unsettled: BTreeSet<String> = place.objects(&format!("{network}/objects"))?.into_iter().filter(|o| o.1 + SETTLE > now).map(|o| o.0).collect();
    let folders = place.list(&format!("{network}/snapshots"))?;
    for dir in &folders {
        let scope = scope_of(dir);
        let Some(key) = scopes.get(&scope) else {
            opened_all = false;
            continue;
        };
        let mut snaps: Vec<(String, Snapshot)> = Vec::new();
        let mut opened = true;
        for name in place.list(&format!("{network}/snapshots/{dir}"))? {
            let path = format!("{network}/snapshots/{dir}/{name}");
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
            let prefix = format!("{network}/bucket/{dir}/{database}");
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
    for (name, written, size) in place.objects(&format!("{network}/objects"))? {
        let settled = |wait: u64| written + wait <= now;
        if keep.contains(&name) || !(drop.contains(&name) && settled(SETTLE) || opened_all && settled(STRAY)) {
            continue;
        }
        place.delete(&format!("{network}/objects/{name}"))?;
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
        json!({ "name": "backups_check", "description": "Restore each place's newest backup into a throwaway copy now, as the weekly check does: who's who from its steps, every database to its newest step as the network has it, and every blob's hash. Says what came back, and what's wrong.", "inputSchema": { "type": "object", "properties": {} }, "annotations": hints(true, false) }),
    ]
}

/// The installs a place holds backups of, by their folders.
pub fn networks(place: &Place) -> Result<Vec<String>> {
    Ok(place.list("")?.into_iter().filter(|n| n.len() == 16 && n.bytes().all(|b| b.is_ascii_hexdigit())).collect())
}

/// Every scope's epoch keys in a network's backup, opened with the recovery key's secret.
pub fn open_keys(place: &Place, network: &str, recovery_secret: &[u8; 32]) -> Result<BTreeMap<String, Vec<[u8; 32]>>> {
    let mut out = BTreeMap::new();
    for dir in place.list(&format!("{network}/keys"))? {
        for id in place.list(&format!("{network}/keys/{dir}"))? {
            let sealed = place.get(&format!("{network}/keys/{dir}/{id}"))?.context("a key went missing")?;
            out.entry(scope_of(&dir)).or_insert_with(Vec::new).push(unseal(recovery_secret, &sealed)?);
        }
    }
    ensure!(!out.is_empty(), "no backup of this network here");
    Ok(out)
}

/// A scope as its newest snapshot holds it: every blob, checked against its hash.
pub struct Restored {
    pub snapshot: Snapshot,
    pub blobs: Vec<(String, Vec<u8>)>,
}

/// Restores a scope from its newest snapshot that one of `keys` opens.
pub fn restore(place: &Place, network: &str, scope: &str, keys: &[[u8; 32]]) -> Result<Restored> {
    let (snapshot, key) = latest(place, network, scope, keys)?.with_context(|| format!("no snapshot of {scope} opens with its keys"))?;
    let mut blobs = Vec::new();
    for (hash, name) in &snapshot.blobs {
        let bytes = place.get(&format!("{network}/objects/{name}"))?.with_context(|| format!("blob {} is missing", &hash[..12]))?;
        let bytes = open_object(&key, name, &bytes)?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "blob {} isn't what its hash says", &hash[..12]);
        blobs.push((hash.clone(), bytes));
    }
    Ok(Restored { snapshot, blobs })
}

/// A restore check, as a device runs one: what it restores.
pub struct Check<'a> {
    pub place: &'a Place,
    /// The network's folder in the place.
    pub network: &'a str,
    /// The keys that may open each scope's snapshots: its epochs' keys.
    pub keys: &'a BTreeMap<String, Vec<[u8; 32]>>,
    pub root: Option<crate::keys::Hash>,
}

/// Restores each scope's newest snapshot from a place, checking every blob, and who's who from `global`'s steps the
/// place's copy of the bucket holds. Says what came back: each scope's blobs and databases, and by name the people and
/// apps; how much the network's backup takes there; what's wrong (`problems`), and the scopes whose snapshots none of
/// `keys` opens (`unchecked`); no scopes at all if nothing's there yet. Databases are `check_databases'`.
pub fn check(c: &Check) -> Result<(Value, Vec<Restored>)> {
    let (place, network) = (c.place, c.network);
    let mut scopes = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut unchecked = Vec::new();
    let mut restored = Vec::new();
    for dir in place.list(&format!("{network}/snapshots"))? {
        let scope = scope_of(&dir);
        if snapshots(place, network, &scope)?.is_empty() {
            continue;
        }
        let Some(keys) = c.keys.get(&scope).filter(|k| !k.is_empty()) else {
            unchecked.push(scope);
            continue;
        };
        let r = match restore(place, network, &scope, keys) {
            Ok(r) => r,
            Err(err) => {
                problems.push(format!("{scope}: {err:#}"));
                continue;
            }
        };
        scopes.push(json!({ "scope": scope, "time": r.snapshot.time, "blobs": r.blobs.len(), "databases": r.snapshot.databases.len() }));
        restored.push(r);
    }
    let global_keys = c.keys.get("global").map(Vec::as_slice).unwrap_or(&[]);
    let m = match who(place, network, c.root, global_keys) {
        Ok(m) => m,
        Err(err) => {
            problems.push(format!("global: {err:#}"));
            ScopeDb::empty(&crate::database::scopes::Scope::Global, crate::node::now_ms())?
        }
    };
    let bytes: u64 = place.objects(&format!("{network}/objects"))?.iter().map(|o| o.2).sum();
    let report = json!({
        "network": m.name(),
        "bytes": bytes,
        "scopes": scopes,
        "people": m.users().into_iter().filter(|u| !u.removed).map(|u| u.name).collect::<Vec<_>>(),
        "apps": m.apps().into_iter().filter(|a| !a.erased && a.bundle.is_some()).map(|a| a.name).collect::<Vec<_>>(),
        "problems": problems,
        "unchecked": unchecked,
    });
    Ok((report, restored))
}

/// Who's who as `global`'s scope database's steps the place's copy of the bucket holds say it (`steps::Reader`), from
/// its first freeze, of the network `root` (or for none, the one there), opened with `keys`, its epochs' keys.
fn who(place: &Place, network: &str, root: Option<crate::keys::Hash>, keys: &[[u8; 32]]) -> Result<ScopeDb> {
    use crate::database::steps;
    let bucket = |name: &str| place.get(&format!("{network}/bucket/{name}"));
    // The steps' key in each epoch the place holds: the node's app key there, opened with whichever of the epochs' keys does.
    let mut step_keys: BTreeMap<u32, [u8; 32]> = BTreeMap::new();
    for name in place.list(&format!("{network}/bucket/{}", crate::keys::app_keys_folder("global", crate::database::scopes::APP)))? {
        let Ok(n) = name.parse::<u32>() else { continue };
        let Some(sealed) = bucket(&crate::keys::app_key_name("global", crate::database::scopes::APP, n))? else { continue };
        if let Some(key) = keys.iter().find_map(|k| crate::keys::open_database_key(k, crate::database::scopes::APP, steps::NAME, &sealed).ok()) {
            step_keys.insert(n, key);
        }
    }
    // As `steps::fetch` fetches them, from the place rather than the bucket.
    let prefix = steps::prefix("global");
    let mut sealed = steps::Sealed::default();
    for at in 1.. {
        let Some(step) = bucket(&crate::database::stored::step_name(&prefix, at))? else {
            if let Some(after) = bucket(&crate::database::stored::step_name(&prefix, at + 1))? {
                sealed.steps.insert(at + 1, after);
            }
            break;
        };
        sealed.steps.insert(at, step);
    }
    let mut reader = steps::Reader::new(root);
    reader.add(&sealed, &step_keys, None, crate::node::now_ms(), |_, _, _| {})?;
    Ok(reader.into_truth().context("its who's who has no freeze")?.db)
}

/// A place's copy of the network's bucket (`<network>/bucket/`), read as the bucket is, so a database restores from it
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
/// to the snapshot's last, are in the place, and are the same bytes as the network's bucket has (`live`), or without
/// it, open with their epoch's key; then it restores from them into a throwaway database, which must reach that step.
/// `epochs` are the scope's epoch keys. → what's wrong, one line each; nothing when all is well.
pub async fn check_databases(place: &Place, network: &str, snapshot: &Snapshot, epochs: &crate::database::stored::Keys, live: Option<&impl crate::bucket::Bucket>) -> Vec<String> {
    use crate::bucket::Bucket;
    let scope = &snapshot.scope;
    let copy = Copy { place, root: format!("{network}/bucket/") };
    let mut out = Vec::new();
    for (database, (at, copied)) in &snapshot.databases {
        let Some((app, db)) = database.split_once('/') else { continue };
        let prefix = crate::bucket::database_prefix(scope, app, db);
        // What a person calls it: the app, or of the node's own, its Vault, its history (entries) or its status rows.
        let said = match (app, db) {
            (crate::database::scopes::APP, "vault") => "Vault".to_string(),
            (crate::database::scopes::APP, crate::database::steps::KEY_NAME) => "history".to_string(),
            (crate::database::scopes::APP, crate::database::steps::NAME) => "who's who".to_string(),
            (crate::database::scopes::APP, _) => "status".to_string(),
            (app, crate::database::scopes::APP_DATABASE) => app.to_string(),
            (app, db) => format!("{app} {db}"),
        };
        let mut keys = crate::database::stored::Keys::new();
        for (n, epoch) in epochs {
            let Ok(Some(sealed)) = copy.get(&crate::keys::app_key_name(scope, app, *n)).await else { continue };
            match crate::keys::open_database_key(epoch, app, db, &sealed.bytes) {
                Ok(key) => drop(keys.insert(*n, key)),
                Err(err) => out.push(format!("{scope} {said}: its key in epoch {n}: {err:#}")),
            }
        }
        let copy_of = (*copied > 0).then(|| (format!("its copy at step {copied}"), crate::database::stored::copy_name(&prefix, *copied)));
        let names = copy_of.into_iter().chain((((*copied).max(1))..=*at).map(|n| (format!("step {n}"), crate::database::stored::step_name(&prefix, n))));
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
        // The node's log of a scope is entries, which `check` restored, and its scope database's steps who's who, which
        // it checked too (`steps_of`); the rest are SQLite.
        if [crate::database::steps::KEY_NAME, crate::database::steps::NAME].map(|db| format!("{}/{db}", crate::database::scopes::APP)).contains(database) || keys.is_empty() {
            continue;
        }
        match crate::database::stored::Db::open(&copy, &prefix, keys).await {
            Ok(restored) if restored.at() >= *at => {}
            Ok(restored) => out.push(format!("{scope} {said}: it restores only to step {} of {at}", restored.at())),
            Err(err) => out.push(format!("{scope} {said}: it doesn't restore: {err:#}")),
        }
    }
    out
}

/// What's wrong with a sealed step or copy, if it doesn't open with its epoch's key: nothing, too, when that key isn't
/// one of `keys`.
fn opens(keys: &crate::database::stored::Keys, name: &str, sealed: &[u8]) -> Option<String> {
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

    use ed25519_dalek::SigningKey;
    use serde_json::json;

    #[tokio::test]
    async fn a_restore_check_passes_a_whole_backup_and_catches_one_corrupted_or_missing() {
        let dir = std::env::temp_dir().join(format!("backups-check-{}", hex(&rand::random())));
        let (place, bucket) = (Place::Folder(dir.join("place")), Place::Folder(dir.join("bucket")));
        let network = "0123456789abcdef";
        let (recovery, device) = (SigningKey::from_bytes(&rand::random()), SigningKey::from_bytes(&rand::random()));
        let epoch: [u8; 32] = rand::random();
        let blob = b"a deploy's bundle".to_vec();
        let hash = blake3::hash(&blob).to_hex().to_string();

        // Who's who: the freeze its recovery key signed.
        const ANDRE: &str = "0123456789abcdef";
        let node = crate::keys::app_key(&bucket, &epoch, "global", crate::database::scopes::APP, 1, true).await.unwrap().unwrap();
        let step_keys = crate::database::stored::Keys::from([(1, crate::keys::database_key(&node, crate::database::scopes::APP, crate::database::steps::NAME))]);
        let start = crate::database::content::first("Home", ANDRE, "Andre", &crate::keys::Endpoint::of(&device), "mac", &[crate::database::scopes::Kind::Client], recovery.verifying_key().as_bytes(), &crate::keys::epoch_hash(&epoch));
        let root = crate::keys::Hash::from_bytes(rand::random());
        let first = crate::database::steps::first_freeze(&recovery, root, &start, ANDRE, 0).unwrap();
        let steps = crate::database::steps::prefix("global");
        let name = crate::database::stored::step_name(&steps, 1);
        bucket.put(&name, &crate::database::steps::seal(&step_keys, &name, &crate::database::steps::encode(&first.step, first.copy.as_deref())).unwrap()).unwrap();
        crate::database::stored::hint(&bucket, &steps, 1, 0, 1).await.unwrap();

        // A database with more steps than one copy holds, so a reader starts from its copy.
        let prefix = "global/tally/main";
        let app = crate::keys::app_key(&bucket, &epoch, "global", "tally", 1, true).await.unwrap().unwrap();
        let keys = crate::database::stored::Keys::from([(1, crate::keys::database_key(&app, "tally", "main"))]);
        let mut db = crate::database::stored::Db::open(&bucket, prefix, keys).await.unwrap();
        db.sign_as(device.clone());
        db.migrate(&bucket, &["create table posts (text text)".to_string()]).await.unwrap();
        for n in 0..crate::database::stored::COPY_EVERY + 5 {
            db.write(&bucket, |c| Ok(c.execute("insert into posts values (?)", [format!("post {n}")])?)).await.unwrap();
        }
        let at = db.at();
        let (_, copy, _) = crate::database::stored::newest(&crate::bucket::Bucket::get(&bucket, &crate::database::stored::newest_name(prefix)).await.unwrap().unwrap().bytes);
        assert!(copy > 0 && copy < at, "it has a copy, and steps after it");

        // Junk at a copy's position, and the newest hint naming it, from a device without the key: the backup copies
        // the copy that holds.
        let junk = crate::database::stored::copy_name(prefix, 10 * crate::database::stored::COPY_EVERY);
        bucket.put(&junk, &crate::keys::seal(1, &rand::random(), &junk, b"junk")).unwrap();
        bucket.put(&crate::database::stored::newest_name(prefix), &serde_json::to_vec(&json!({ "at": at, "copy": 10 * crate::database::stored::COPY_EVERY })).unwrap()).unwrap();

        let job = |previous: Option<Snapshot>, now: u64| Job {
            place: &place,
            network,
            scope: "global",
            key: epoch,
            epochs: crate::database::stored::Keys::from([(1, epoch)]),
            sealed: crate::keys::seal_to(&crate::keys::twin_public(recovery.verifying_key().as_bytes()).unwrap(), &epoch).unwrap(),
            device: *device.verifying_key().as_bytes(),
            now,
            previous,
            blobs: [hash.clone()].into(),
            bucket: Some(&bucket),
        };
        let first = back_up(job(None, 1_000_000), |h| (h == hash).then(|| blob.clone())).unwrap().unwrap();
        assert_eq!(back_up(job(Some(first), 1_500_000), |h| (h == hash).then(|| blob.clone())).unwrap(), None, "nothing new for a snapshot");

        let keys: BTreeMap<String, Vec<[u8; 32]>> = [("global".to_string(), vec![epoch])].into();
        let epochs = crate::database::stored::Keys::from([(1, epoch)]);
        let checked = || {
            let (report, restored) = check(&Check { place: &place, network, keys: &keys, root: Some(root) }).unwrap();
            (report["problems"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect::<Vec<_>>(), restored, report)
        };
        let (problems, restored, report) = checked();
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!((report["network"].as_str(), report["people"].clone()), (Some("Home"), json!(["Andre"])), "who's who from its steps");
        let snapshot = restored[0].snapshot.clone();
        assert_eq!(snapshot.databases["tally/main"], (at, copy));
        assert_eq!(check_databases(&place, network, &snapshot, &epochs, Some(&bucket)).await, Vec::<String>::new(), "every database restores from the backup");
        assert_eq!(check_databases(&place, network, &snapshot, &epochs, None::<&Place>).await, Vec::<String>::new(), "and opens without the network's bucket");

        // A step that's changed in the backup, then one that's gone: the one its copy starts beside.
        let name = |n: u64| format!("{network}/bucket/{}", crate::database::stored::step_name(prefix, n));
        let good = place.get(&name(at)).unwrap().unwrap();
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        place.put(&name(at), &bad).unwrap();
        let wrong = check_databases(&place, network, &snapshot, &epochs, Some(&bucket)).await;
        assert!(wrong.len() == 1 && wrong[0].contains(&format!("global tally: step {at} isn't what your devices have")), "{wrong:?}");
        let wrong = check_databases(&place, network, &snapshot, &epochs, None::<&Place>).await;
        assert!(wrong.len() == 1 && wrong[0].contains("doesn't open"), "{wrong:?}");
        place.put(&name(at), &good).unwrap();
        place.delete(&name(copy)).unwrap();
        let wrong = check_databases(&place, network, &snapshot, &epochs, Some(&bucket)).await;
        assert!(wrong.len() == 1 && wrong[0].contains(&format!("global tally: step {copy} isn't there")), "{wrong:?}");

        // A blob that's gone, then one that's corrupted.
        let object = |name: &str| format!("{network}/objects/{name}");
        let blob_object = place.get(&object(&snapshot.blobs[&hash])).unwrap().unwrap();
        place.delete(&object(&snapshot.blobs[&hash])).unwrap();
        let (problems, ..) = checked();
        assert!(problems.len() == 1 && problems[0].contains("is missing"), "{problems:?}");
        let mut bad = blob_object.clone();
        bad[0] ^= 1;
        place.put(&object(&snapshot.blobs[&hash]), &bad).unwrap();
        let (problems, ..) = checked();
        assert!(problems.len() == 1 && problems[0].contains("didn't decrypt"), "{problems:?}");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
