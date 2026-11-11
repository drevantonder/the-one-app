//! Backing up, as a device does it: each scope it holds to each place it backs it up to, the weekly restore checks,
//! telling admins what's wrong, and adding and removing places.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use api::Caller;
use api::runtime::prelude::*;
use bucket::Place;
use custom_build::now_ms;
use key::{Key, hex};
use scope::scopes::{self, Scope};

use crate::engine::{self as b, Snapshot};
use crate::places::{PINNED, PlaceItem, Problem, item_hash, line, machine_name, pinned, place_hash, places, problems, release_blobs, scope_named, takes_over};
use crate::rows::{BackedUp, BackupRun, CheckRun, Said};
use crate::{State, keep_folder, rows, seen};

/// How often a device prunes each place it backs up to: after its first run, then daily.
const PRUNE_EVERY: u64 = 24 * 3_600_000;
/// How long each device after the first in line waits before telling admins itself, in case those before it are down.
const STAGGER: u64 = 10 * 60_000;
/// How often each place's newest backup is restored and checked; and again after one that failed.
const CHECK_EVERY: u64 = 7 * 24 * 3_600_000;
const CHECK_AGAIN: u64 = 24 * 3_600_000;
/// How long a device that just started waits before its checks: by then a run has sent each scope what its places
/// lack, as this build sends it.
const CHECK_SETTLE: u64 = 2 * 3_600_000;

/// One scope to back up to one place in a run.
struct Due {
    scope: String,
    key: Option<[u8; 32]>,
}

/// What's due: backs up, checks and tells admins (`Runs.every`).
pub(crate) async fn tick(rt: Rt) -> Result<()> {
    let started = *rt.state::<State>().started.get_or_init(now_ms);
    if !rt.ready() {
        return Ok(());
    }
    rows::read(&rt).await?;
    let force = rt.state::<State>().forced.swap(false, Ordering::Relaxed);
    if let Err(err) = back_up(&rt, force).await {
        tracing::warn!("backups: {err:#}");
    }
    if let Err(err) = check_due(&rt, started).await {
        tracing::warn!("checking backups: {err:#}");
    }
    if let Err(err) = tell_admins(&rt).await {
        tracing::warn!("telling admins about backups: {err:#}");
    }
    Ok(())
}

/// Backs up now, everything this device backs up, whether or not an hour has passed.
pub fn back_up_now(rt: &Rt) {
    rt.state::<State>().forced.store(true, Ordering::Relaxed);
    rt.wake("backups");
}

/// Notifies the network's admins of a place's problem once it's worth it. Every device watches, so a device that's gone
/// is noticed too; the first in line tells them, each after it only once the ones before have had time to, and the
/// alert it writes keeps the rest quiet until the next problem.
async fn tell_admins(rt: &Rt) -> Result<()> {
    let now = now_ms();
    let me = rt.id();
    let seen = seen(rt);
    let (due, admins) = rt.read(|r| {
        let m = r.db();
        if r.state(&Scope::Global).is_none() {
            return Ok((Vec::new(), Vec::new()));
        }
        let at = line(m, &Scope::Global, "alerts")?.iter().position(|k| *k == me).unwrap_or(usize::MAX / 2) as u64;
        let due: Vec<Problem> = problems(r, &seen, now)?.into_iter().filter(|p| p.notify.is_some_and(|t| now >= t + at * STAGGER) && seen.rows.alerted.get(&p.place).is_none_or(|t| *t < p.since)).collect();
        let admins: Vec<String> = m.users()?.into_iter().filter(|u| !u.removed && m.may(&u.id, "change-settings", scopes::NETWORK)).map(|u| u.id).collect();
        Ok((due, admins))
    })?;
    let state = rt.state::<State>();
    for p in due {
        // Said only once an admin was told, so a notification that didn't go is tried again, hourly.
        {
            let mut tried = state.alert_tried.lock().unwrap();
            if tried.get(&p.place).is_some_and(|t| now < t + 3_600_000) {
                continue;
            }
            tried.insert(p.place.clone(), now);
        }
        let mut told = false;
        for admin in &admins {
            let note = notifications::Send { title: p.title.clone(), body: p.fix.clone(), app: "settings".into(), screen: "backups".into(), ..Default::default() }.note("")?;
            match notifications_runtime::send(rt, admin, &note).await {
                Ok(sent) => {
                    told = true;
                    tracing::info!("told {admin} about backups: {sent}");
                }
                Err(err) => tracing::warn!("couldn't tell {admin} about backups: {err:#}"),
            }
        }
        if told {
            rows::say(rt, Said::Alerted { place: p.place.clone() }).await?;
            state.alert_tried.lock().unwrap().remove(&p.place);
        }
    }
    Ok(())
}

/// Backs up each scope to each place this device backs it up to, if an hour passed and anything changed (or `force`),
/// and writes how each place's run went.
async fn back_up(rt: &Rt, force: bool) -> Result<()> {
    let now = now_ms();
    let (me, device) = (rt.id(), rt.device());
    let keys = held_keys(rt).await;
    let seen = seen(rt);
    let state = rt.state::<State>();
    let (network, plan) = rt.read(|r| {
        let m = r.db();
        if r.state(&Scope::Global).is_none() {
            return Ok((String::new(), Vec::new()));
        }
        let network = r.replica.root().map(|r| r.to_hex()[..16].to_string()).context("no network")?;
        m.recovery()?.context("the network has no recovery key")?;
        let cache = state.backed.lock().unwrap();
        let failed = state.failed.lock().unwrap();
        let pruned = state.pruned.lock().unwrap();
        let looked = state.looked.lock().unwrap();
        let mut plan = Vec::new();
        for p in places(r, &seen)? {
            if !force && failed.get(&p.id).is_some_and(|t| now - t < b::EVERY) {
                continue;
            }
            let folder = p.device.is_some();
            if folder && p.device != Some(device) {
                continue;
            }
            // Every device prunes each scope it holds the key of, so pruning needs no one device.
            let prune = (force || pruned.get(&p.id).is_none_or(|t| now - t >= PRUNE_EVERY)).then(|| release_blobs(m).map(|(all, _)| (keys.clone(), all))).transpose()?;
            let mut due = Vec::new();
            let mut logs: Vec<String> = m.scopes()?.iter().map(Scope::to_string).collect();
            logs.sort_by_key(|l| l != "global");
            for log in logs {
                let Ok(scope) = Scope::parse(&log) else { continue };
                if !r.holds(&scope) {
                    continue;
                }
                if !folder {
                    let line = line(m, &scope, &p.id)?;
                    let Some(at) = line.iter().position(|k| *k == me) else { continue };
                    // A device further down the line takes a scope's changes only once they've waited for each device
                    // before it.
                    if at > 0 && !takes_over(&seen, at, &scope, &p, now) {
                        continue;
                    }
                }
                let key = keys.get(&log).copied();
                let known = cache.get(&(p.id.clone(), log.clone())).filter(|s| key.is_some_and(|k| s.1 == b::key_id(&k)));
                let changed = match known {
                    None => true,
                    Some((s, _)) => {
                        let since = s.time + b::EVERY <= now || force;
                        // Its databases are looked at in the bucket once an hour, and as soon as its scope database
                        // changes.
                        let looked = looked.get(&(p.id.clone(), log.clone())).is_some_and(|t| t + b::EVERY > now);
                        since && (seen.first_change(&scope, s.upto).is_some() || !looked || force)
                    }
                };
                if !changed {
                    continue;
                }
                due.push(Due { scope: log, key });
            }
            if !due.is_empty() || prune.is_some() {
                plan.push((p, due, prune));
            }
        }
        Ok((network, plan))
    })?;
    for (p, due, prune) in plan {
        let place = match p.place {
            Ok(place) => place,
            Err(err) => {
                if !due.is_empty() {
                    state.failed.lock().unwrap().insert(p.id.clone(), now);
                    backed_up(rt, &p.id, now, Some(err), Vec::new()).await?;
                }
                continue;
            }
        };
        if !place.present() {
            continue;
        }
        let mut snapshots = Vec::new();
        let mut error = None;
        for d in due {
            match back_up_scope(rt, &place, &p.id, &network, &d, now, force).await {
                Ok(Some(s)) => snapshots.push(s),
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!("backing up {} to {}: {err:#}", d.scope, p.title);
                    error.get_or_insert(err);
                }
            }
        }
        match &error {
            Some(_) => state.failed.lock().unwrap().insert(p.id.clone(), now),
            None => state.failed.lock().unwrap().remove(&p.id),
        };
        let failed = error.is_some();
        if error.is_some() || !snapshots.is_empty() {
            backed_up(rt, &p.id, now, error, snapshots).await?;
        }
        if let (Some((scopes, fetchable)), false) = (prune, failed) {
            state.pruned.lock().unwrap().insert(p.id.clone(), now);
            let (place, network, title) = (place.clone(), network.clone(), p.title.clone());
            match tokio::task::spawn_blocking(move || b::prune(&place, &network, &scopes, &fetchable, now_ms())).await? {
                Ok(pruned) if pruned != b::Pruned::default() => tracing::info!("pruned {title}: {} snapshots, {} objects, {} bytes", pruned.snapshots, pruned.objects, pruned.bytes),
                Ok(_) => {}
                Err(err) => tracing::warn!("pruning {title}: {err:#}"),
            }
        }
    }
    Ok(())
}

/// The key of the newest epoch of each scope this device holds, by its name, of those it opens.
async fn held_keys(rt: &Rt) -> BTreeMap<String, [u8; 32]> {
    held_epochs(rt).await.into_iter().filter_map(|(s, k)| Some((s, *k.last_key_value()?.1))).collect()
}

/// Every epoch's key of each scope this device holds, by its name, of those it opens.
async fn held_epochs(rt: &Rt) -> BTreeMap<String, database::stored::Keys> {
    let scopes = rt.read_or(Vec::new(), |r| Ok(r.db().scopes()?.into_iter().filter(|s| r.holds(s)).collect::<Vec<_>>()));
    let mut out = BTreeMap::new();
    for scope in scopes {
        match rt.epochs(&scope).await {
            Ok(keys) if !keys.is_empty() => drop(out.insert(scope.to_string(), keys)),
            _ => {}
        }
    }
    out
}

/// Backs up one scope to one place, with its newest epoch's key: sends what the place lacks, unless (but for `force`)
/// the place has a snapshot of it from within the hour, which a device that just started doesn't know of yet.
async fn back_up_scope(rt: &Rt, place: &Place, place_id: &str, network: &str, d: &Due, now: u64, force: bool) -> Result<Option<Snapshot>> {
    let scope = Scope::parse(&d.scope)?;
    let state = rt.state::<State>();
    let epochs = rt.epochs(&scope).await?;
    let (n, key) = epochs.last_key_value().map(|(n, k)| (*n, *k)).context("no epoch")?;
    ensure!(d.key.is_none_or(|k| k == key), "{scope}'s newest epoch changed: it's backed up on the next run");
    let name = database::keys::epoch_name(&d.scope, n, "recovery");
    let sealed = rt.object(&name).await?.with_context(|| format!("{name} is missing"))?;
    let cached = state.backed.lock().unwrap().get(&(place_id.to_string(), d.scope.clone())).filter(|s| s.1 == b::key_id(&key)).map(|s| s.0.clone());
    let blobs = rt.read(|r| {
        // What its database names, of the releases only the newest of each.
        let (releases, newest) = release_blobs(r.db())?;
        let named = match r.scope_db(&scope) {
            Some(db) => db.blobs()?,
            None => BTreeSet::new(),
        };
        Ok(named.into_iter().filter(|b| !releases.contains(b) || newest.contains(b)).collect::<BTreeSet<String>>())
    })?;
    let (blob_rt, runtime) = (rt.clone(), tokio::runtime::Handle::current());
    let bucket = rt.bucket().ok();
    let (place, network, scope_name, device) = (place.clone(), network.to_string(), d.scope.clone(), rt.id());
    let (written, known) = tokio::task::spawn_blocking(move || -> Result<(Option<Snapshot>, Option<Snapshot>)> {
        // What the place holds: the newest snapshot there, which another device may have written since.
        let newest = b::snapshots(&place, &network, &scope_name)?.into_iter().next();
        let previous = match (cached, newest) {
            (Some(c), Some((t, _))) if c.time >= t => Some(c),
            (_, None) => None,
            _ => b::latest(&place, &network, &scope_name, &[key])?.map(|(s, _)| s),
        };
        if !force && previous.as_ref().is_some_and(|p| p.time + b::EVERY > now) {
            return Ok((None, previous));
        }
        let job = b::Job { place: &place, network: &network, scope: &scope_name, key, epochs, sealed, device, now, previous: previous.clone(), blobs, bucket: bucket.as_ref() };
        // A blob this device doesn't keep is read from the bucket, where the device that wrote it put it.
        let written = b::back_up(job, |hash| runtime.block_on(database_runtime::blob(&blob_rt, hash)).inspect_err(|err| tracing::debug!("backing up blob {hash}: {err:#}")).ok())?;
        Ok((written.clone(), written.or(previous)))
    })
    .await??;
    state.looked.lock().unwrap().insert((place_id.to_string(), d.scope.clone()), now);
    // With nothing new, what the place holds is said anyway where the rows don't say it yet (they were cut before each
    // scope's last was kept), so a scope that hasn't changed isn't taken for one a day late.
    let unsaid = written.is_none() && known.as_ref().is_some_and(|k| state.rows.lock().unwrap().covered.get(&(place_id.to_string(), d.scope.clone())).is_none_or(|c| c.0 < k.upto));
    let said = if unsaid { known.clone() } else { written };
    if let Some(known) = known {
        state.backed.lock().unwrap().insert((place_id.to_string(), d.scope.clone()), (known, b::key_id(&key)));
    }
    Ok(said)
}

/// Says how a run to a place went, in `global`.
async fn backed_up(rt: &Rt, place: &str, now: u64, error: Option<anyhow::Error>, snapshots: Vec<Snapshot>) -> Result<()> {
    let snapshots = snapshots.iter().map(|s| BackedUp { scope: s.scope.clone(), upto: s.upto, blobs: s.blobs.len() as u64, bytes: s.bytes }).collect();
    let trouble = error.as_ref().and_then(::bucket::Trouble::of).map(|t| t.name().to_string());
    let error = error.map(|e| format!("{e:#}").chars().take(500).collect::<String>());
    rows::say(rt, Said::BackedUp(BackupRun { place: place.into(), device: rt.id(), time: now, error, trouble, snapshots })).await
}

/// Backups as a user sees them: each place, how its last run went, the recent snapshots, and the disks here.
pub async fn status(rt: &Rt) -> Result<Value> {
    rows::read(rt).await?;
    let seen = seen(rt);
    let here = rt.device_name();
    rt.read(|r| {
        let m = r.db();
        let name = |k: &Key| machine_name(m, k);
        let (runs, checks) = (&seen.rows.backups, &seen.rows.checks);
        let problems = problems(r, &seen, now_ms())?;
        let mut out = Vec::new();
        for p in places(r, &seen)? {
            let mine: Vec<&BackupRun> = runs.iter().filter(|r| r.place == p.id).collect();
            let last_ok = mine.iter().rev().find(|r| r.error.is_none());
            let (kind, place_where, endpoint) = match &p.place {
                _ if p.device.is_some() => ("folder", p.shown.clone(), None),
                Ok(Place::S3(s3)) => ("s3", s3.bucket.clone(), Some(s3.endpoint.clone())),
                _ => ("s3", String::new(), None),
            };
            out.push(json!({
                "id": p.id,
                "title": p.title,
                "kind": kind,
                "where": place_where,
                "endpoint": endpoint,
                "device": p.device.map(|k| name(&k)).transpose()?,
                "problem": problems.iter().find(|q| q.place == p.id).map(Problem::json),
                "last": last_ok.map(|r| Ok::<_, anyhow::Error>(json!({ "time": r.time, "device": name(&r.device)? }))).transpose()?,
                "checked": checks.iter().rev().find(|c| c.place == p.id).map(|c| Ok::<_, anyhow::Error>(json!({ "time": c.time, "device": name(&c.device)?, "error": c.error }))).transpose()?,
            }));
        }
        let titles: BTreeMap<String, String> = places(r, &seen)?.into_iter().map(|p| (p.id, p.title)).collect();
        // The network's snapshots, newest first: the runs of each minute, whichever devices and places, as one.
        let mut moments: Vec<(u64, BTreeSet<String>, BTreeSet<String>, BTreeSet<String>)> = Vec::new();
        for run in runs.iter().rev().filter(|r| !r.snapshots.is_empty() && titles.contains_key(&r.place)) {
            let minute = run.time / 60_000;
            if moments.last().is_none_or(|m| m.0 / 60_000 != minute) {
                if moments.len() == 20 {
                    break;
                }
                moments.push((run.time, BTreeSet::new(), BTreeSet::new(), BTreeSet::new()));
            }
            let moment = moments.last_mut().expect("one was just added");
            moment.1.insert(name(&run.device)?);
            moment.2.insert(titles[&run.place].clone());
            for s in &run.snapshots {
                moment.3.insert(scope_named(r, &s.scope)?);
            }
        }
        let snapshots: Vec<Value> = moments
            .into_iter()
            .map(|(time, devices, places, scopes)| {
                let (devices, places): (Vec<String>, Vec<String>) = (devices.into_iter().collect(), places.into_iter().collect());
                json!({ "time": time, "devices": devices, "places": places, "scopes": scopes })
            })
            .collect();
        let mut status = json!({ "places": out, "snapshots": snapshots, "here": here, "disks": crate::places::disks() });
        // The tools say it alone (`api::tools::said`); the Settings screen reads the rest.
        if out.is_empty() {
            status["said"] = json!("No backups yet: backups_add_place adds a place to back up to, a folder or S3-compatible storage.");
        }
        Ok(status)
    })
}

/// Tries a place (writes, reads and deletes a test file), and if that works, keeps it as a Vault item in Everyone and
/// backs up to it now. → each step tried, and the item's id if it was kept.
pub async fn add_place(rt: &Rt, caller: &Caller, body: &Value) -> Result<Value> {
    let text = |k: &str| body[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for k in ["path", "endpoint", "bucket", "region", "key_id", "secret"] {
        if let Some(v) = text(k) {
            fields.insert(k.into(), v);
        }
    }
    let place = Place::from_fields(&fields)?;
    let tried = {
        let place = place.clone();
        tokio::task::spawn_blocking(move || place.try_it()).await?
    };
    let steps: Vec<Value> = tried.iter().map(|(step, error)| json!({ "step": step, "error": error })).collect();
    if tried.iter().any(|(_, e)| e.is_some()) || body["test"] == true {
        return Ok(json!({ "tried": steps }));
    }
    let (title, item_fields, hosts) = match &place {
        Place::Folder(path) => {
            let title = text("title").unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string()));
            (title, json!({ "where": { "value": path.display().to_string() }, "device": { "value": hex(&rt.device()) } }), json!([]))
        }
        Place::S3(s3) => {
            let host = s3.endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
            let ::bucket::Auth::Keys { key_id, secret } = &s3.auth else { bail!("a place is added with its keys") };
            let title = text("title").unwrap_or_else(|| format!("{} on {host}", s3.bucket));
            let fields = json!({
                "endpoint": { "value": s3.endpoint }, "bucket": { "value": s3.bucket }, "region": { "value": s3.region },
                "key_id": { "value": key_id }, "secret": { "value": secret, "secret": true },
            });
            (title, fields, json!([host]))
        }
    };
    let data = json!({ "title": title, "type": backups::KIND, "fields": item_fields, "hosts": hosts, "apps": [key::APP] });
    let id = vault_runtime::set_item(rt, caller, &Scope::Global, data).await?;
    match &place {
        Place::Folder(path) => keep_folder(rt, &id, Some(path))?,
        Place::S3(_) => {
            let fields = item_fields.as_object().into_iter().flatten().map(|(n, f)| (n.clone(), f["value"].as_str().unwrap_or("").to_string())).collect();
            pin_place(rt, caller, &id, Some(&fields)).await?;
        }
    }
    back_up_now(rt);
    Ok(json!({ "tried": steps, "id": id }))
}

/// Asks the user for an S3 place's details in a sheet on their device, so an agent never sees its key.
pub async fn request_place(rt: &Rt, caller: &Caller, body: &Value) -> Result<Value> {
    let endpoint = body["endpoint"].as_str().map(str::trim).filter(|e| e.starts_with("https://") || e.starts_with("http://")).context("give the endpoint, like https://<account>.r2.cloudflarestorage.com")?;
    let host = endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
    let fields = json!({ "endpoint": { "secret": false }, "bucket": { "secret": false }, "key_id": { "secret": false }, "secret": { "secret": true } });
    let ask = json!({ "title": body["title"].as_str().unwrap_or("Backups"), "fields": fields, "hosts": [host] });
    let why = format!("Backups go to {endpoint}, encrypted: its bucket, access key ID and secret key.");
    vault_runtime::request_item(rt, caller, &Scope::Global, key::APP, Some(backups::KIND), &ask, &why).await
}

/// Stops backing up to a place: deletes its item. What's in the place stays there.
pub(crate) async fn remove_place(rt: &Rt, caller: &Caller, id: &str) -> Result<()> {
    let seen = seen(rt);
    let (known, was_pinned) = rt.read(|r| Ok((places(r, &seen)?.iter().any(|p| p.id == id), pinned(r.db())?.contains_key(id))))?;
    ensure!(known, "That place was already removed: there's no place {id}");
    vault_runtime::set_item(rt, caller, &Scope::Global, json!({ "id": id, "deleted": true })).await?;
    keep_folder(rt, id, None)?;
    if was_pinned {
        pin_place(rt, caller, id, None).await?;
    }
    Ok(())
}

/// A place's item in Everyone just saved by `caller` (`vault_runtime::watch`): when they're an admin and it's an S3
/// place the network backs up to, or one that `answered` Backups' request, its details as they are now are the ones the
/// network backs up to, so changing its key in the Vault keeps backups going. Any other item is no place.
pub(crate) async fn saved_place(rt: &Rt, caller: &Caller, id: &str, answered: bool) -> Result<()> {
    let own = vault_runtime::own_items(rt);
    let fields = rt.read(|r| {
        let m = r.db();
        let pinned = pinned(m)?;
        let changed = own.get(id).filter(|it| m.may(&caller.user, "change-settings", scopes::NETWORK) && it.apps.contains(key::APP) && !it.fields.contains_key("device") && pinned.get(id) != Some(&item_hash(it)) && (answered || pinned.contains_key(id)));
        Ok(changed.map(|it| it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect::<BTreeMap<_, _>>()))
    })?;
    match fields {
        Some(fields) => pin_place(rt, caller, id, Some(&fields)).await,
        None => Ok(()),
    }
}

/// Names an S3 place the network backs up to in `PINNED` (or with `None`, stops naming it), for an admin.
async fn pin_place(rt: &Rt, caller: &Caller, id: &str, fields: Option<&BTreeMap<String, String>>) -> Result<()> {
    let mut now = rt.read(|r| pinned(r.db()))?;
    match fields {
        Some(fields) => drop(now.insert(id.to_string(), place_hash(fields))),
        None => drop(now.remove(id)),
    }
    rt.as_member(caller, scope::content::setting(PINNED, Some(&serde_json::to_string(&now)?))).await
}

/// Restores each place's newest backup that's due its weekly check (`started`: when this runtime started), on the
/// device that checks it: a folder's own device, and for S3 the first in a line of the devices that run `global`, each
/// after it once the ones before have had time to. Not before a place's first backup.
async fn check_due(rt: &Rt, started: u64) -> Result<()> {
    let now = now_ms();
    if now < started + CHECK_SETTLE {
        return Ok(());
    }
    let (me, device) = (rt.id(), rt.device());
    let seen = seen(rt);
    let due: Vec<String> = rt.read(|r| {
        let m = r.db();
        let mut due = Vec::new();
        for p in places(r, &seen)?.into_iter().filter(|p| seen.rows.covered.keys().any(|(place, _)| *place == p.id)) {
            let last = seen.rows.checks.iter().rev().find(|c| c.place == p.id);
            let next = last.map_or(0, |c| c.time + if c.error.is_some() { CHECK_AGAIN } else { CHECK_EVERY });
            let is_due = match p.device {
                Some(d) => d == device && now >= next,
                None => line(m, &Scope::Global, &format!("{}/check", p.id))?.iter().position(|k| *k == me).is_some_and(|at| now >= next && b::takes_over(at, Some(next), 0, now)),
            };
            if is_due {
                due.push(p.id);
            }
        }
        Ok(due)
    })?;
    if !due.is_empty() {
        check_places(rt, Some(&due)).await?;
    }
    Ok(())
}

/// Restores each scope's newest snapshot from each place into a throwaway store and throwaway databases, checks them
/// (`engine::check`, `engine::check_databases`), and says what came back.
pub async fn check_backups(rt: &Rt) -> Result<Value> {
    check_places(rt, None).await
}

/// Checks the places `only` names, or every one this device reaches, and writes how each went in `global`.
async fn check_places(rt: &Rt, only: Option<&[String]>) -> Result<Value> {
    let epochs = held_epochs(rt).await;
    let seen = seen(rt);
    let (network, places, root) = rt.read(|r| {
        ensure!(r.state(&Scope::Global).is_some(), "no network");
        let places: Vec<(String, String, Place)> = places(r, &seen)?.into_iter().filter(|p| only.is_none_or(|o| o.contains(&p.id))).filter_map(|p: PlaceItem| Some((p.id, p.title, p.place.ok()?))).filter(|(_, _, p)| p.present()).collect();
        let root = r.replica.root();
        Ok((root.map(|r| r.to_hex()[..16].to_string()).unwrap_or_default(), places, root))
    })?;
    let bucket = rt.bucket().ok();
    let mut out = Vec::new();
    for (id, title, place) in places {
        let now = now_ms();
        let checked = {
            let keys: BTreeMap<String, Vec<[u8; 32]>> = epochs.iter().map(|(s, k)| (s.clone(), k.values().copied().collect())).collect();
            let (place, network) = (place.clone(), network.clone());
            tokio::task::spawn_blocking(move || b::check(&b::Check { place: &place, network: &network, keys: &keys, root })).await?
        };
        let (mut report, error) = match checked {
            Ok((mut report, restored)) => {
                let mut problems: Vec<Value> = report["problems"].as_array().cloned().unwrap_or_default();
                for r in &restored {
                    let Some(keys) = epochs.get(&r.snapshot.scope) else { continue };
                    problems.extend(b::check_databases(&place, &network, &r.snapshot, keys, bucket.as_ref()).await.into_iter().map(Value::from));
                }
                // Each says whose data it's about by its scope; users read their names.
                let named: Vec<(String, String)> = rt.read(|r| epochs.keys().map(|s| Ok((s.clone(), scope_named(r, s)?))).collect())?;
                for p in problems.iter_mut() {
                    let t = p.as_str().unwrap_or("");
                    let renamed = named.iter().find(|(s, _)| t.strip_prefix(s.as_str()).is_some_and(|r| r.starts_with([' ', ':']))).map(|(s, n)| format!("{n}{}", &t[s.len()..]));
                    if let Some(renamed) = renamed {
                        *p = json!(renamed);
                    }
                }
                let error = problems.first().and_then(Value::as_str).map(|first| match problems.len() {
                    1 => first.to_string(),
                    n => format!("{first}, and {} more", n - 1),
                });
                report["problems"] = json!(problems);
                let unchecked: Vec<String> = report["unchecked"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
                report["unchecked"] = json!(rt.read(|r| unchecked.iter().map(|s| scope_named(r, s)).collect::<Result<Vec<_>>>())?);
                for s in report["scopes"].as_array_mut().into_iter().flatten() {
                    s["name"] = json!(rt.read(|r| scope_named(r, s["scope"].as_str().unwrap_or("")))?);
                }
                (report, error)
            }
            Err(err) => (json!({}), Some(format!("{err:#}"))),
        };
        if let Some(error) = &error {
            tracing::warn!("restore check of {title}: {error}");
        }
        // A check that opened nothing here (this device holds none of the keys) says so, and counts for nothing.
        if error.is_none() && report["scopes"].as_array().is_none_or(Vec::is_empty) {
            report["place"] = json!(title);
            out.push(report);
            continue;
        }
        let kept = json!({ "scopes": report["scopes"], "unchecked": report["unchecked"], "problems": report["problems"].as_array().map(|p| p.iter().take(20).cloned().collect::<Vec<_>>()) });
        let error500 = error.as_ref().map(|e| e.chars().take(500).collect::<String>());
        rows::say(rt, Said::Checked(CheckRun { place: id, device: rt.id(), time: now, error: error500, report: kept })).await?;
        report["place"] = json!(title);
        report["databases"] = json!(true);
        if let Some(error) = error {
            report["error"] = json!(error);
        }
        out.push(report);
    }
    Ok(json!(out))
}

/// Backups copy everyone's data to places an admin chose, so all of it is admins': what's there, running one, and
/// places.
pub(crate) fn only_admins(rt: &Rt, me: &Caller) -> Result<()> {
    rt.read(|r| {
        if !r.db().may(&me.user, "change-settings", scopes::NETWORK) {
            return Err(custom_build::protocol::Coded::refused("Only admins use Backups.").into());
        }
        Ok(())
    })
}
