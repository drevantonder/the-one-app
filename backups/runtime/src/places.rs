//! The places a network backs up to, as Everyone's Vault items and the `backups` setting name them, which devices back
//! each scope up to each, and what's wrong with each: read from who's who and the rows, never written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use api::Caller;
use api::runtime::prelude::*;
use bucket::Place;
use key::{Key, short, unhex};
use scope::read::Read;
use scope::scopes::{self, Scope, ScopeDb};

use crate::engine as b;
use crate::rows::{BackupRun, CheckRun};
use crate::{Seen, seen};

/// How long changes may wait for a place before admins hear of it, which catches a device that's gone.
pub(crate) const LATE: u64 = 24 * 3_600_000;

/// What's wrong with a place, for admins: what, in a sentence and in a few words for its row, its fix, since when,
/// and from when it's worth a notification.
#[derive(Clone, Debug)]
pub(crate) struct Problem {
    pub place: String,
    /// What the box on Backups says: the place couldn't be backed up to, or its restore check failed (`check`).
    pub title: String,
    pub check: bool,
    pub text: String,
    pub status: &'static str,
    pub fix: String,
    pub since: u64,
    pub notify: Option<u64>,
}

impl Problem {
    pub fn json(&self) -> Value {
        json!({ "place": self.place, "title": self.title, "check": self.check, "text": self.text, "status": self.status, "fix": self.fix, "since": self.since })
    }
}

/// The first of the backups' problems, for the badge on Settings (`Runs.problem`): only an admin's to see.
pub(crate) fn problem(rt: &Rt, me: &Caller) -> Result<Option<Value>> {
    let seen = seen(rt);
    rt.read(|r| {
        if !r.db().may(&me.user, "change-settings", scopes::NETWORK) {
            return Ok(None);
        }
        Ok(problems(r, &seen, custom_build::now_ms())?.into_iter().min_by_key(|p| p.since).map(|p| p.json()))
    })
}

/// Each place's problem, as this device sees it: its runs failing (worth a notification from the second failure in a
/// row), or a scope this device holds with changes that have waited a day for it, whoever should have backed them up.
/// Both end by themselves once a run works.
pub(crate) fn problems(r: &Read, seen: &Seen, now: u64) -> Result<Vec<Problem>> {
    let m = r.db();
    let rows = &seen.rows;
    let name = |k: &Key| machine_name(m, k);
    let mut out = Vec::new();
    for p in places(r, seen)? {
        let runs: Vec<&BackupRun> = rows.backups.iter().filter(|r| r.place == p.id).collect();
        let failing: Vec<&&BackupRun> = runs.iter().rev().take_while(|r| r.error.is_some()).collect();
        if let Some(last) = failing.first() {
            let error = last.error.clone().unwrap_or_default();
            let trouble = last.trouble.as_deref().and_then(::bucket::Trouble::parse);
            let text = match trouble {
                Some(_) => format!("{} {error}.", p.title),
                None => format!("Couldn't back up to {} ({error}).", p.title),
            };
            let status = match trouble {
                Some(::bucket::Trouble::Key) => "Key not accepted",
                Some(::bucket::Trouble::Bucket) => "No such bucket",
                Some(::bucket::Trouble::Refused) => "Not allowed to write",
                Some(::bucket::Trouble::Unreachable) => "Not reachable",
                Some(::bucket::Trouble::Disk) => "Couldn't be used",
                None => "Couldn't back up",
            };
            let device = name(&last.device)?;
            let fix = match (trouble, &p.place) {
                (_, Err(_)) if p.device.is_none() => "Fix it in the Vault.".to_string(),
                (Some(::bucket::Trouble::Key), _) => "Change its key in the Vault.".to_string(),
                (Some(::bucket::Trouble::Bucket), _) => "Check the bucket's name in the Vault.".to_string(),
                (Some(::bucket::Trouble::Refused), _) => "Give the key permission to write there, or change it in the Vault.".to_string(),
                (Some(::bucket::Trouble::Unreachable), _) => format!("Check {device} is connected to the internet. It tries again every hour."),
                (Some(::bucket::Trouble::Disk), _) => format!("Check the disk on {device} is plugged in and has room."),
                (None, _) => "It tries again every hour.".to_string(),
            };
            let since = failing.last().map_or(last.time, |r| r.time);
            out.push(Problem { place: p.id.clone(), title: format!("Couldn't back up to {}", p.title), check: false, text, status, fix, since, notify: failing.iter().rev().nth(1).map(|r| r.time) });
            continue;
        }
        // Scopes the last runs couldn't back up while they backed up the rest: theirs alone, worth a notification from the
        // second run in a row.
        let missing: Vec<&&BackupRun> = runs.iter().rev().take_while(|r| !r.missed.is_empty()).collect();
        if let Some(last) = missing.first() {
            let whose = last.missed.iter().map(|m| Ok(format!("{}'s", scope_named(r, &m.scope)?))).collect::<Result<Vec<_>>>()?.join(" and ");
            let since = missing.last().map_or(last.time, |r| r.time);
            let text = format!("{}.", last.missed[0].error.trim_end_matches('.'));
            out.push(Problem { place: p.id.clone(), title: format!("Couldn't back up {whose} things to {}", p.title), check: false, text, status: "Partly backed up", fix: "Everything else is backed up. It tries again every hour.".to_string(), since, notify: missing.get(1).map(|r| r.time) });
            continue;
        }
        // A restore check that failed, worth a notification at once: what's there can't be counted on.
        let checks: Vec<&CheckRun> = rows.checks.iter().filter(|c| c.place == p.id).collect();
        let failed: Vec<&&CheckRun> = checks.iter().rev().take_while(|c| c.error.is_some()).collect();
        if let Some(last) = failed.first() {
            let since = failed.last().map_or(last.time, |c| c.time);
            let text = format!("{}'s newest backup didn't restore as it should: {}.", p.title, last.error.as_deref().unwrap_or(""));
            let fix = "Don't count on it until a check passes. It's checked again tomorrow.".to_string();
            out.push(Problem { place: p.id.clone(), title: format!("{} didn't pass its restore check", p.title), check: true, text, status: "Restore check failed", fix, since, notify: Some(since) });
            continue;
        }
        let mut late: Option<(u64, String)> = None;
        for scope in m.scopes()? {
            let log = scope.to_string();
            let backer = match p.device {
                Some(_) => p.device.filter(|k| m.runs_for(k, &scope)),
                None => line(m, &scope, &p.id)?.first().copied(),
            };
            let Some(backer) = backer.filter(|_| r.holds(&scope)) else { continue };
            let covered = rows.covered.get(&(p.id.clone(), log.clone())).map_or(0, |c| c.0);
            let Some(waiting) = seen.first_change(&scope, covered) else { continue };
            let since = waiting.max(p.made);
            if now.saturating_sub(since) >= LATE && late.as_ref().is_none_or(|l| since < l.0) {
                late = Some((since, name(&backer)?));
            }
        }
        if let Some((since, backer)) = late {
            let fix = match p.device {
                Some(_) => format!("Check {backer} is on, with the disk plugged in."),
                None => format!("Check {backer} is on: it backs this up."),
            };
            out.push(Problem { place: p.id.clone(), title: format!("Couldn't back up to {}", p.title), check: false, text: format!("Nothing has gone to {} for a day.", p.title), status: "Not backed up for a day", fix, since, notify: Some(since + LATE) });
        }
    }
    // A place an admin named whose details changed since, by someone who isn't one: nothing goes there until an admin
    // saves them, which names them again (`saved_place`), so a member can't send backups somewhere else.
    let pinned = pinned(m)?;
    for (id, it) in seen.own.iter().filter(|(id, it)| it.apps.contains(key::APP) && it.kind == backups::KIND && !it.fields.contains_key("device") && pinned.get(*id).is_some_and(|h| *h != item_hash(it))) {
        out.push(Problem {
            place: id.clone(),
            title: format!("Not backing up to {}", it.title),
            check: false,
            text: format!("{}'s details were changed, so nothing goes there until an admin checks them.", it.title),
            status: "Details changed",
            fix: "An admin opens it in the Vault and saves it.".to_string(),
            since: it.time,
            notify: Some(it.time),
        });
    }
    Ok(out)
}

/// The hash of an item's fields, as `PINNED` names a place's.
pub(crate) fn item_hash(it: &vault::Item) -> String {
    place_hash(&it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect())
}

/// A place to back up to, as its Vault item says: one in Everyone that the app runtime is switched on for.
pub(crate) struct PlaceItem {
    pub id: String,
    pub title: String,
    pub place: Result<Place>,
    /// For a folder: the device whose disk it's on, and where it is there, as that device said.
    pub device: Option<Key>,
    pub shown: String,
    /// When it was added.
    pub made: u64,
}

/// The setting that names the S3 places the network backs up to: each item's id, and the hash of its fields
/// (`place_hash`), as an admin's device wrote it. A Vault item is anyone's to write; this is only admins'.
pub(crate) const PINNED: &str = "backups";

/// The hash of a place's fields, as `PINNED` names it.
pub(crate) fn place_hash(fields: &BTreeMap<String, String>) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(serde_json::to_vec(fields).unwrap_or_default()))
}

/// The S3 places `PINNED` names: id → hash.
pub(crate) fn pinned(m: &ScopeDb) -> Result<BTreeMap<String, String>> {
    Ok(m.setting(PINNED)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

/// A device's name as the network lists it, or its short key if it isn't.
pub(crate) fn machine_name(m: &ScopeDb, k: &Key) -> Result<String> {
    Ok(m.machine(k)?.map(|s| s.name.clone()).unwrap_or_else(|| short(k)))
}

/// A scope as users call it, by its name.
pub(crate) fn scope_named(r: &Read, scope: &str) -> Result<String> {
    r.scope_name(&Scope::parse(scope)?)
}

/// What a place's item says, read from global's items. A folder's item names only its device: where the folder is
/// comes from that device's own file (`folders.json`), never from an item, which anyone may write. An S3 place counts
/// only as `PINNED` names it.
pub(crate) fn places(r: &Read, seen: &Seen) -> Result<Vec<PlaceItem>> {
    let pinned = pinned(r.db())?;
    Ok(seen
        .own
        .iter()
        .filter(|(_, it)| it.apps.contains(key::APP) && it.kind == backups::KIND)
        .filter(|(id, it)| it.fields.contains_key("device") || pinned.get(*id).is_some_and(|h| *h == item_hash(it)))
        .map(|(id, it)| {
            let mut fields: BTreeMap<String, String> = it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect();
            let device = fields.get("device").and_then(|s| unhex(s).ok());
            let shown = fields.remove("where").unwrap_or_default();
            fields.remove("path");
            let place = match device {
                Some(d) if d == seen.device => seen.folders.get(id).map(|path| Place::Folder(path.into())).context("this device keeps no folder for it"),
                Some(_) => Err(anyhow!("a folder on another device")),
                None => Place::from_fields(&fields).and_then(|p| match &p {
                    // Secrets stay put: the key goes only to the item's own hosts.
                    Place::S3(s3) => {
                        let host = s3.endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
                        ensure!(it.hosts.iter().any(|h| h == host), "its key is for {}, not {host}", it.hosts.join(", "));
                        Ok(p)
                    }
                    Place::Folder(_) => bail!("a folder is only its own device's"),
                }),
            };
            PlaceItem { id: id.clone(), title: it.title.clone(), place, device, shown, made: it.made }
        })
        .collect())
}

/// The devices that back up a scope to an S3 place, in the order they take it: those that run its calls, in an order
/// the place and scope pick. Every device works it out alike.
pub(crate) fn line(m: &ScopeDb, scope: &Scope, place: &str) -> Result<Vec<Key>> {
    let mut devices: Vec<([u8; 32], Key)> = m
        .computes()?
        .into_iter()
        .filter(|s| m.runs_for(&s.key, scope))
        .map(|s| {
            let mut h = blake3::Hasher::new();
            h.update(&s.key);
            h.update(format!("{scope}/backups/{place}").as_bytes());
            (*h.finalize().as_bytes(), s.key)
        })
        .collect();
    devices.sort();
    Ok(devices.into_iter().map(|s| s.1).collect())
}

/// The blobs `global`'s releases name (`ScopeDb::releases`): every one, which a backup needn't keep, since releases can
/// be fetched again from where they're published; and the newest of each platform's (and each service's), which it
/// keeps, so a custom build with no published releases can still recover.
pub(crate) fn release_blobs(db: &ScopeDb) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let mut newest: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut all = BTreeSet::new();
    for (c, _, time) in db.releases()? {
        all.extend(c.blobs.iter().cloned());
        let target = format!("{} {}", c.data["target"].as_str().unwrap_or(""), c.data["service"].as_str().unwrap_or(""));
        if let Some(blob) = c.blobs.first()
            && newest.get(&target).is_none_or(|(t, _)| time >= *t)
        {
            newest.insert(target, (time, blob.clone()));
        }
    }
    Ok((all, newest.into_values().map(|(_, b)| b).collect()))
}

/// The disks plugged into this machine, where a folder place could go: each one's name, folder and free space.
pub(crate) fn disks() -> Vec<Value> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push("/Volumes".into());
    } else {
        let user = std::env::var("USER").unwrap_or_default();
        roots.extend([format!("/media/{user}"), format!("/run/media/{user}"), "/mnt".into()].map(PathBuf::from));
    }
    let mut out = Vec::new();
    for root in roots {
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = e.path();
            // The Mac's own disk is in /Volumes too, as a link to /.
            if std::fs::read_link(&path).is_ok() || !path.is_dir() {
                continue;
            }
            let Some(free) = free_space(&path) else { continue };
            out.push(json!({ "name": e.file_name().to_string_lossy(), "path": path.join(format!("{} Backups", custom_build::config::NAME)).display().to_string(), "free": free }));
        }
    }
    out
}

fn free_space(path: &Path) -> Option<u64> {
    platform::os::disk_space(path).map(|(_, free)| free)
}

/// Whether a device further down a scope's line takes over (`engine::takes_over`), here from what's covered.
pub(crate) fn takes_over(seen: &Seen, at: usize, scope: &Scope, place: &PlaceItem, now: u64) -> bool {
    let covered = seen.rows.covered.get(&(place.id.clone(), scope.to_string())).map_or(0, |c| c.0);
    b::takes_over(at, seen.first_change(scope, covered), place.made, now)
}
