//! Backups, as a device runs them: which scopes it backs up to which places, hourly when anything changed, and how
//! each run went, written in `global` so every device shows it. The format and the places are `crate::backups`.

use super::*;
use super::status::{BackedUp, BackupRun, CheckRun, Said};
use std::collections::BTreeSet;

use crate::backups::{self as b, Snapshot};
use crate::bucket::Place;

/// How often the node looks for backups that are due.
const TICK: Duration = Duration::from_secs(60);
/// How often a device prunes each place it backs up to: after its first run, then daily.
const PRUNE_EVERY: u64 = 24 * 3_600_000;
/// How long changes may wait for a place before admins hear of it, which catches a device that's gone.
const LATE: u64 = 24 * 3_600_000;
/// How long each device after the first in line waits before telling admins itself, in case those before it are down.
const STAGGER: u64 = 10 * 60_000;
/// How often each place's newest backup is restored and checked; and again after one that failed.
const CHECK_EVERY: u64 = 7 * 24 * 3_600_000;
const CHECK_AGAIN: u64 = 24 * 3_600_000;
/// How long a device that just started waits before its checks: by then a run has sent each scope what its places
/// lack, as this build sends it.
const CHECK_SETTLE: u64 = 2 * 3_600_000;

/// What's wrong with a place, for admins: what, in a sentence and in a few words for its row, its fix, since when,
/// and from when it's worth a push.
#[derive(Clone, Debug)]
pub struct Problem {
    pub place: String,
    /// What the box on Backups says: the place couldn't be backed up to, or its restore check failed (`check`).
    pub title: String,
    pub check: bool,
    pub text: String,
    pub status: &'static str,
    pub fix: String,
    pub since: u64,
    pub push: Option<u64>,
}

impl Problem {
    pub fn json(&self) -> Value {
        json!({ "place": self.place, "title": self.title, "check": self.check, "text": self.text, "status": self.status, "fix": self.fix, "since": self.since })
    }
}

/// Each place's problem, as this device sees it: its runs failing (worth a push from the second failure in a row),
/// or a scope this device holds with changes that have waited a day for it, whoever should have backed them up.
/// Both end by themselves once a run works.
fn problems(i: &Install, now: u64) -> Result<Vec<Problem>> {
    let m = &i.r.members;
    let status = i.status.get("global").cloned().unwrap_or_default();
    let name = |k: &Key| m.machine(k).map(|s| s.name.clone()).unwrap_or_else(|| short(k));
    let mut out = Vec::new();
    for p in places(i) {
        let runs: Vec<&BackupRun> = status.backups.iter().filter(|r| r.place == p.id).collect();
        let failing: Vec<&&BackupRun> = runs.iter().rev().take_while(|r| r.error.is_some()).collect();
        if let Some(last) = failing.first() {
            let error = last.error.clone().unwrap_or_default();
            let trouble = last.trouble.as_deref().and_then(crate::bucket::Trouble::parse);
            let text = match trouble {
                Some(_) => format!("{} {error}.", p.title),
                None => format!("Couldn't back up to {} ({error}).", p.title),
            };
            let status = match trouble {
                Some(crate::bucket::Trouble::Key) => "Key not accepted",
                Some(crate::bucket::Trouble::Bucket) => "No such bucket",
                Some(crate::bucket::Trouble::Refused) => "Not allowed to write",
                Some(crate::bucket::Trouble::Unreachable) => "Not reachable",
                Some(crate::bucket::Trouble::Disk) => "Couldn't be used",
                None => "Couldn't back up",
            };
            let device = name(&last.device);
            let fix = match (trouble, &p.place) {
                (_, Err(_)) if p.device.is_none() => "Fix it in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Key), _) => "Change its key in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Bucket), _) => "Check the bucket's name in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Refused), _) => "Give the key permission to write there, or change it in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Unreachable), _) => format!("Check {device} is online. It tries again every hour."),
                (Some(crate::bucket::Trouble::Disk), _) => format!("Check the disk on {device} is plugged in and has room."),
                (None, _) => "It tries again every hour.".to_string(),
            };
            let since = failing.last().map_or(last.time, |r| r.time);
            out.push(Problem { place: p.id.clone(), title: format!("Couldn't back up to {}", p.title), check: false, text, status, fix, since, push: failing.iter().rev().nth(1).map(|r| r.time) });
            continue;
        }
        // A restore check that failed, worth a push at once: what's there can't be counted on.
        let checks: Vec<&CheckRun> = status.checks.iter().filter(|c| c.place == p.id).collect();
        let failed: Vec<&&CheckRun> = checks.iter().rev().take_while(|c| c.error.is_some()).collect();
        if let Some(last) = failed.first() {
            let since = failed.last().map_or(last.time, |c| c.time);
            let text = format!("{}'s newest backup didn't restore as it should: {}.", p.title, last.error.as_deref().unwrap_or(""));
            let fix = "Don't count on it until a check passes. It's checked again tomorrow.".to_string();
            out.push(Problem { place: p.id.clone(), title: format!("{} didn't pass its restore check", p.title), check: true, text, status: "Restore check failed", fix, since, push: Some(since) });
            continue;
        }
        let mut late: Option<(u64, String)> = None;
        for log in i.r.store.logs()? {
            let Ok(scope) = Scope::parse(&log) else { continue };
            let backer = match p.device {
                Some(_) => p.device.filter(|k| m.runs_for(k, &scope)),
                None => line(m, &scope, &p.id).first().copied(),
            };
            let Some(backer) = backer.filter(|_| i.r.holds(&scope)) else { continue };
            let covered = status.covered.get(&(p.id.clone(), log.clone())).map_or(0, |c| c.0);
            let Some(waiting) = i.r.store.first_change(&log, covered)? else { continue };
            let since = waiting.max(p.made);
            if now.saturating_sub(since) >= LATE && late.as_ref().is_none_or(|l| since < l.0) {
                late = Some((since, name(&backer)));
            }
        }
        if let Some((since, backer)) = late {
            let fix = match p.device {
                Some(_) => format!("Check {backer} is on, with the disk plugged in."),
                None => format!("Check {backer} is on: it backs this up."),
            };
            out.push(Problem { place: p.id.clone(), title: format!("Couldn't back up to {}", p.title), check: false, text: format!("Nothing has gone to {} for a day.", p.title), status: "Not backed up for a day", fix, since, push: Some(since + LATE) });
        }
    }
    Ok(out)
}

/// A place to back up to, as its Vault item says: one in Everyone that the node is switched on for.
struct PlaceItem {
    id: String,
    title: String,
    place: Result<Place>,
    /// For a folder: the device whose disk it's on, and where it is there, as that device said.
    device: Option<Key>,
    shown: String,
    /// When it was added.
    made: u64,
}

/// The setting that names the S3 places the install backs up to: each item's id, and the hash of its fields
/// (`place_hash`), as an admin's device wrote it. A Vault item is anyone's to write; this is only admins'.
const PINNED: &str = "backups";

/// The hash of a place's fields, as `PINNED` names it.
fn place_hash(fields: &BTreeMap<String, String>) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(serde_json::to_vec(fields).unwrap_or_default()))
}

/// The S3 places `PINNED` names: id → hash.
fn pinned(m: &Members) -> BTreeMap<String, String> {
    m.settings.get(PINNED).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default()
}

/// What a place's item says, read from global's items. A folder's item names only its device: where the folder is
/// comes from that device's own file (`Install::folders`), never from an item, which anyone may write. An S3 place
/// counts only as `PINNED` names it.
fn places(i: &Install) -> Vec<PlaceItem> {
    let pinned = pinned(&i.r.members);
    i.vault
        .iter()
        .filter(|(_, it)| it.apps.contains(members::APP) && it.kind == b::KIND)
        .filter(|(id, it)| it.fields.contains_key("device") || pinned.get(*id).is_some_and(|h| *h == place_hash(&it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect())))
        .map(|(id, it)| {
            let mut fields: BTreeMap<String, String> = it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect();
            let device = fields.get("device").and_then(|s| unhex(s).ok());
            let shown = fields.remove("where").unwrap_or_default();
            fields.remove("path");
            let place = match device {
                Some(d) if d == i.r.id => i.folders.get(id).map(|path| Place::Folder(path.into())).context("this device keeps no folder for it"),
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
        .collect()
}

/// The devices that back up a scope to an S3 place, in the order they take it: those that run its calls, in an order
/// the place and scope pick. Every device works it out alike.
fn line(m: &Members, scope: &Scope, place: &str) -> Vec<Key> {
    let mut devices: Vec<([u8; 32], Key)> = m
        .computes()
        .filter(|s| m.runs_for(&s.key, scope))
        .map(|s| {
            let mut h = blake3::Hasher::new();
            h.update(&s.key);
            h.update(format!("{scope}/backups/{place}").as_bytes());
            (*h.finalize().as_bytes(), s.key)
        })
        .collect();
    devices.sort();
    devices.into_iter().map(|s| s.1).collect()
}

/// One scope to back up to one place in a run.
struct Due {
    scope: String,
    key: Option<[u8; 32]>,
}

impl Node {
    /// Now and then, backs up what's due.
    pub(super) async fn backup_loop(self) {
        let started = now_ms();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = self.0.backup_now.notified() => {}
            }
            let force = std::mem::take(&mut *self.0.backup_forced.lock().unwrap());
            if self.has_install() && self.ready() {
                if let Err(err) = self.back_up(force).await {
                    tracing::warn!("backups: {err:#}");
                }
                if let Err(err) = self.check_due(started).await {
                    tracing::warn!("checking backups: {err:#}");
                }
                if let Err(err) = self.tell_admins().await {
                    tracing::warn!("telling admins about backups: {err:#}");
                }
            }
        }
    }

    /// Pushes a place's problem to the install's admins once it's worth it, through the install's push. Every device
    /// watches, so a device that's gone is noticed too; the first in line tells them, each after it only once the
    /// ones before have had time to, and the `backup-alert` it writes keeps the rest quiet until the next problem.
    async fn tell_admins(&self) -> Result<()> {
        let now = now_ms();
        let me = self.0.id;
        let (due, admins, push) = self.with(|i| {
            let m = &i.r.members;
            let push = crate::state::settings(m).remove("push").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            if push.is_empty() || !i.views.contains_key("global") {
                return Ok((Vec::new(), Vec::new(), push));
            }
            let at = line(m, &Scope::Global, "alerts").iter().position(|k| *k == me).unwrap_or(usize::MAX / 2) as u64;
            let due: Vec<Problem> = problems(i, now)?
                .into_iter()
                .filter(|p| p.push.is_some_and(|t| now >= t + at * STAGGER) && i.status.get("global").and_then(|s| s.alerted.get(&p.place)).is_none_or(|t| *t < p.since))
                .collect();
            let admins: Vec<String> = m.users.values().filter(|u| !u.removed && m.is_admin(&u.id)).map(|u| u.id.clone()).collect();
            Ok((due, admins, push))
        })?;
        for p in due {
            self.say(&Scope::Global, Said::Alerted { place: p.place.clone() }).await?;
            for admin in &admins {
                let input = json!({ "title": p.title, "body": format!("{} {}", p.text, p.fix) });
                let call = CallRequest { app: push.clone(), service: Some(push.clone()), op: "send".into(), input, client: Some("app".into()), ..Default::default() };
                match Box::pin(self.route(call, Caller { key: me, user: admin.clone(), agent: None, token: None })).await {
                    Ok(sent) => tracing::info!("told {admin} about backups: {sent}"),
                    Err(err) => tracing::warn!("couldn't tell {admin} about backups: {err:#}"),
                }
            }
        }
        Ok(())
    }

    /// The first of the backups' problems, for the badge on Settings: only an admin's to see.
    pub(super) fn backup_problem(&self, i: &Install, user: &str) -> Option<Value> {
        if !i.r.members.is_admin(user) {
            return None;
        }
        problems(i, now_ms()).ok()?.into_iter().min_by_key(|p| p.since).map(|p| p.json())
    }

    /// Backs up now, everything this device backs up, whether or not an hour has passed.
    pub fn back_up_now(&self) {
        *self.0.backup_forced.lock().unwrap() = true;
        self.0.backup_now.notify_one();
    }

    /// Backs up each scope to each place this device backs it up to, if an hour passed and anything changed (or
    /// `force`), and writes how each place's run went.
    async fn back_up(&self, force: bool) -> Result<()> {
        let now = now_ms();
        let me = self.0.id;
        let keys = self.held_keys().await;
        let (install, plan) = self.with(|i| {
            let m = &i.r.members;
            if !i.views.contains_key("global") {
                return Ok((String::new(), Vec::new()));
            }
            let install = i.r.root.map(|r| r.to_hex()[..16].to_string()).context("no install")?;
            m.recovery.context("the install has no recovery key")?;
            let frontier = i.r.store.frontier()?;
            let cache = self.0.backed.lock().unwrap();
            let failed = self.0.backup_failed.lock().unwrap();
            let pruned = self.0.pruned.lock().unwrap();
            let mut plan = Vec::new();
            for p in places(i) {
                if !force && failed.get(&p.id).is_some_and(|t| now - t < b::EVERY) {
                    continue;
                }
                let folder = p.device.is_some();
                if folder && p.device != Some(me) {
                    continue;
                }
                // Every device prunes each scope it holds the key of, so pruning needs no one device.
                let prune = (force || pruned.get(&p.id).is_none_or(|t| now - t >= PRUNE_EVERY)).then(|| -> Result<_> {
                    Ok((keys.clone(), releases(&i.r.store.scope("global")?)))
                });
                let prune = prune.transpose()?;
                let mut due = Vec::new();
                // Every scope it holds, as one may have databases (its Vault's among them) and no entries yet.
                let mut logs = i.r.store.logs()?;
                for scope in m.scopes().iter().map(Scope::to_string) {
                    if !logs.contains(&scope) {
                        logs.push(scope);
                    }
                }
                logs.sort_by_key(|l| l != "global");
                for log in logs {
                    let Ok(scope) = Scope::parse(&log) else { continue };
                    if !i.r.holds(&scope) {
                        continue;
                    }
                    if !folder {
                        let line = line(m, &scope, &p.id);
                        let Some(at) = line.iter().position(|k| *k == me) else { continue };
                        // A device further down the line takes a scope's changes only once they've waited for each
                        // device before it.
                        if at > 0 {
                            let covered = i.status.get("global").and_then(|s| s.covered.get(&(p.id.clone(), log.clone()))).map_or(0, |c| c.0);
                            if !b::takes_over(at, i.r.store.first_change(&log, covered)?, p.made, now) {
                                continue;
                            }
                        }
                    }
                    let key = keys.get(&log).copied();
                    let known = cache.get(&(p.id.clone(), log.clone())).filter(|s| key.is_some_and(|k| s.1 == b::key_id(&k)));
                    let changed = match known {
                        None => true,
                        Some((s, _)) => {
                            let since = s.time + b::EVERY <= now || force;
                            // Its databases change without entries: the bucket is looked at once an hour.
                            let looked = self.0.backup_looked.lock().unwrap().get(&(p.id.clone(), log.clone())).is_some_and(|t| t + b::EVERY > now);
                            let entries = frontier.iter().any(|(a, l, next)| *l == log && s.frontier.get(&hex(a)).copied().unwrap_or(0) < *next) && i.r.store.changed_since(&log, &s.frontier)?;
                            since && (entries || !looked || force)
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
            Ok((install, plan))
        })?;
        for (p, due, prune) in plan {
            let place = match p.place {
                Ok(place) => place,
                Err(err) => {
                    if !due.is_empty() {
                        self.0.backup_failed.lock().unwrap().insert(p.id.clone(), now);
                        self.backed_up(&p.id, now, Some(err), Vec::new()).await?;
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
                match self.back_up_scope(&place, &p.id, &install, &d, now, force).await {
                    Ok(Some(s)) => snapshots.push(s),
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!("backing up {} to {}: {err:#}", d.scope, p.title);
                        error.get_or_insert(err);
                    }
                }
            }
            match &error {
                Some(_) => self.0.backup_failed.lock().unwrap().insert(p.id.clone(), now),
                None => self.0.backup_failed.lock().unwrap().remove(&p.id),
            };
            let failed = error.is_some();
            if error.is_some() || !snapshots.is_empty() {
                self.backed_up(&p.id, now, error, snapshots).await?;
            }
            if let (Some((scopes, fetchable)), false) = (prune, failed) {
                self.0.pruned.lock().unwrap().insert(p.id.clone(), now);
                let (place, install, title) = (place.clone(), install.clone(), p.title.clone());
                match tokio::task::spawn_blocking(move || b::prune(&place, &install, &scopes, &fetchable, now_ms())).await? {
                    Ok(pruned) if pruned != b::Pruned::default() => tracing::info!("pruned {title}: {} snapshots, {} objects, {} bytes", pruned.snapshots, pruned.objects, pruned.bytes),
                    Ok(_) => {}
                    Err(err) => tracing::warn!("pruning {title}: {err:#}"),
                }
            }
        }
        Ok(())
    }

    /// The key of the newest epoch of each scope this device holds, by its name, of those it opens.
    async fn held_keys(&self) -> BTreeMap<String, [u8; 32]> {
        self.held_epochs().await.into_iter().filter_map(|(s, k)| Some((s, *k.last_key_value()?.1))).collect()
    }

    /// Every epoch's key of each scope this device holds, by its name, of those it opens.
    async fn held_epochs(&self) -> BTreeMap<String, crate::db::Keys> {
        let scopes = self.with(|i| Ok(i.r.members.scopes().into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>())).unwrap_or_default();
        let mut out = BTreeMap::new();
        for scope in scopes {
            match self.epoch_keys(&scope, false).await {
                Ok(keys) if !keys.is_empty() => drop(out.insert(scope.to_string(), keys)),
                _ => {}
            }
        }
        out
    }

    /// Backs up one scope to one place, with its newest epoch's key: sends what the place lacks, unless (but for `force`) the
    /// place has a snapshot of it from within the hour, which a device that just started doesn't know of yet.
    async fn back_up_scope(&self, place: &Place, place_id: &str, install: &str, d: &Due, now: u64, force: bool) -> Result<Option<Snapshot>> {
        let scope = Scope::parse(&d.scope)?;
        let (n, key) = self.epoch_keys(&scope, false).await?.pop_last().context("no epoch")?;
        ensure!(d.key.is_none_or(|k| k == key), "{scope}'s newest epoch changed: it's backed up on the next run");
        let name = crate::keys::epoch_name(&d.scope, n, "recovery");
        let sealed = crate::bucket::Bucket::get(&self.bucket()?, &name).await?.with_context(|| format!("{name} is missing"))?.bytes;
        let cached = self.0.backed.lock().unwrap().get(&(place_id.to_string(), d.scope.clone())).filter(|s| s.1 == b::key_id(&key)).map(|s| s.0.clone());
        let (entries, blobs) = self.with(|i| {
            let entries = i.r.store.scope(&d.scope)?;
            let forgotten = i.views.get(&d.scope).map(|(_, s)| s.forgotten.clone()).unwrap_or_default();
            Ok((entries.clone(), backed_blobs(&entries, &forgotten)))
        })?;
        let (node, runtime) = (self.clone(), tokio::runtime::Handle::current());
        let bucket = self.bucket().ok().map(|b| b.place);
        let (place, install, scope_name, device) = (place.clone(), install.to_string(), d.scope.clone(), self.0.id);
        let snap = tokio::task::spawn_blocking(move || -> Result<(Option<Snapshot>, Option<Snapshot>)> {
            // What the place holds: the newest snapshot there, which another device may have written since.
            let newest = b::snapshots(&place, &install, &scope_name)?.into_iter().next();
            let previous = match (cached, newest) {
                (Some(c), Some((t, _))) if c.time >= t => Some(c),
                (_, None) => None,
                _ => b::latest(&place, &install, &scope_name, &[key])?.map(|(s, _)| s),
            };
            if !force && previous.as_ref().is_some_and(|p| p.time + b::EVERY > now) {
                return Ok((None, previous));
            }
            let job = b::Job { place: &place, install: &install, scope: &scope_name, key, sealed, device, now, previous: previous.clone(), entries, blobs, bucket: bucket.as_ref() };
            // A blob this device doesn't keep is read from the bucket, where the device that wrote it put it.
            let written = b::back_up(job, |hash| runtime.block_on(node.blob(hash)).inspect_err(|err| tracing::debug!("backing up blob {hash}: {err:#}")).ok())?;
            Ok((written.clone(), written.or(previous)))
        })
        .await??;
        let (written, known) = snap;
        self.0.backup_looked.lock().unwrap().insert((place_id.to_string(), d.scope.clone()), now);
        if let Some(known) = known {
            self.0.backed.lock().unwrap().insert((place_id.to_string(), d.scope.clone()), (known, b::key_id(&key)));
        }
        Ok(written)
    }

    /// Says how a run to a place went, in `global`.
    async fn backed_up(&self, place: &str, now: u64, error: Option<anyhow::Error>, snapshots: Vec<Snapshot>) -> Result<()> {
        let snapshots = snapshots.iter().map(|s| BackedUp { scope: s.scope.clone(), upto: s.upto, entries: s.entries, blobs: s.blobs.len() as u64, bytes: s.bytes }).collect();
        let trouble = error.as_ref().and_then(crate::bucket::Trouble::of).map(|t| t.name().to_string());
        let error = error.map(|e| format!("{e:#}").chars().take(500).collect::<String>());
        self.say(&Scope::Global, Said::BackedUp(BackupRun { place: place.into(), device: self.0.id, time: now, error, trouble, snapshots })).await
    }

    /// Backups as a person sees them: each place, how its last run went, the recent snapshots, and the disks here.
    pub fn backups(&self) -> Result<Value> {
        self.with(|i| {
            let m = &i.r.members;
            let name = |k: &Key| m.machine(k).map(|s| s.name.clone()).unwrap_or_else(|| short(k));
            let runs = i.status.get("global").map(|s| s.backups.clone()).unwrap_or_default();
            let checks = i.status.get("global").map(|s| s.checks.clone()).unwrap_or_default();
            let problems = problems(i, now_ms())?;
            let mut out = Vec::new();
            for p in places(i) {
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
                    "device": p.device.map(|k| name(&k)),
                    "problem": problems.iter().find(|q| q.place == p.id).map(Problem::json),
                    "last": last_ok.map(|r| json!({ "time": r.time, "device": name(&r.device) })),
                    "checked": checks.iter().rev().find(|c| c.place == p.id).map(|c| json!({ "time": c.time, "device": name(&c.device), "error": c.error })),
                }));
            }
            let titles: BTreeMap<String, String> = places(i).into_iter().map(|p| (p.id, p.title)).collect();
            // The install's snapshots, newest first: the runs of each minute, whichever devices and places, as one.
            let mut moments: Vec<(u64, BTreeSet<String>, BTreeSet<String>, BTreeSet<String>)> = Vec::new();
            for r in runs.iter().rev().filter(|r| !r.snapshots.is_empty() && titles.contains_key(&r.place)) {
                let minute = r.time / 60_000;
                if moments.last().is_none_or(|m| m.0 / 60_000 != minute) {
                    if moments.len() == 20 {
                        break;
                    }
                    moments.push((r.time, BTreeSet::new(), BTreeSet::new(), BTreeSet::new()));
                }
                let m = moments.last_mut().expect("just pushed");
                m.1.insert(name(&r.device));
                m.2.insert(titles[&r.place].clone());
                m.3.extend(r.snapshots.iter().map(|s| scope_name(i, &s.scope)));
            }
            let snapshots: Vec<Value> = moments
                .into_iter()
                .map(|(time, devices, places, scopes)| {
                    let (devices, places): (Vec<String>, Vec<String>) = (devices.into_iter().collect(), places.into_iter().collect());
                    // `device` and `place`, one each, for apps built before a snapshot was the install's.
                    json!({ "time": time, "device": devices.join(", "), "place": places.join(", "), "devices": devices, "places": places, "scopes": scopes })
                })
                .collect();
            Ok(json!({ "places": out, "snapshots": snapshots, "here": self.device_name(), "disks": disks() }))
        })
    }

    /// Tries a place (writes, reads and deletes a test file), and if that works, keeps it as a Vault item in
    /// Everyone and backs up to it now. → each step tried, and the item's id if it was kept.
    pub async fn add_place(&self, caller: &Caller, body: &Value) -> Result<Value> {
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, &Scope::Global), "only admins add places to back up to");
            Ok(())
        })?;
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
            tokio::task::spawn_blocking(move || place.try_it(false)).await?
        };
        let steps: Vec<Value> = tried.iter().map(|(step, error)| json!({ "step": step, "error": error })).collect();
        if tried.iter().any(|(_, e)| e.is_some()) || body["test"] == true {
            return Ok(json!({ "tried": steps }));
        }
        let (title, item_fields, hosts) = match &place {
            Place::Folder(path) => {
                let title = text("title").unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string()));
                (title, json!({ "where": { "value": path.display().to_string() }, "device": { "value": hex(&self.0.id) } }), json!([]))
            }
            Place::S3(s3) => {
                let host = s3.endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
                let crate::bucket::Auth::Keys { key_id, secret } = &s3.auth else { bail!("a place is added with its keys") };
                let title = text("title").unwrap_or_else(|| format!("{} on {host}", s3.bucket));
                let fields = json!({
                    "endpoint": { "value": s3.endpoint }, "bucket": { "value": s3.bucket }, "region": { "value": s3.region },
                    "key_id": { "value": key_id }, "secret": { "value": secret, "secret": true },
                });
                (title, fields, json!([host]))
            }
        };
        let data = json!({ "title": title, "type": b::KIND, "fields": item_fields, "hosts": hosts, "apps": [members::APP] });
        let id = self.set_item(caller, &Scope::Global, data).await?;
        match &place {
            Place::Folder(path) => self.keep_folder(&id, Some(path))?,
            Place::S3(_) => {
                let fields = item_fields.as_object().into_iter().flatten().map(|(n, f)| (n.clone(), f["value"].as_str().unwrap_or("").to_string())).collect();
                self.pin_place(caller, &id, Some(&fields)).await?;
            }
        }
        self.back_up_now();
        Ok(json!({ "tried": steps, "id": id }))
    }

    /// Asks the person for an S3 place's details in a sheet on their device, so an agent never sees its key.
    pub async fn request_place(&self, caller: &Caller, body: &Value) -> Result<Value> {
        let endpoint = body["endpoint"].as_str().map(str::trim).filter(|e| e.starts_with("https://") || e.starts_with("http://")).context("give the endpoint, like https://<account>.r2.cloudflarestorage.com")?;
        let host = endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
        let fields = json!({ "endpoint": { "secret": false }, "bucket": { "secret": false }, "key_id": { "secret": false }, "secret": { "secret": true } });
        let ask = json!({ "title": body["title"].as_str().unwrap_or("Backups"), "fields": fields, "hosts": [host] });
        let why = format!("Backups go to {endpoint}, encrypted: its bucket, access key ID and secret key.");
        self.request_item(caller, &Scope::Global, members::APP, Some(b::KIND), &ask, &why).await
    }

    /// Stops backing up to a place: deletes its item. What's in the place stays there.
    pub async fn remove_place(&self, caller: &Caller, id: &str) -> Result<()> {
        let known = self.with(|i| Ok(places(i).iter().any(|p| p.id == id)))?;
        ensure!(known, "That place was already removed: there's no place {id}");
        self.set_item(caller, &Scope::Global, json!({ "id": id, "deleted": true })).await?;
        self.keep_folder(id, None)?;
        if self.with(|i| Ok(pinned(&i.r.members).contains_key(id)))? {
            self.pin_place(caller, id, None).await?;
        }
        Ok(())
    }

    /// Names an S3 place the install backs up to in `PINNED` (or with `None`, stops naming it), for an admin.
    pub(super) async fn pin_place(&self, caller: &Caller, id: &str, fields: Option<&BTreeMap<String, String>>) -> Result<()> {
        let mut now = self.with(|i| Ok(pinned(&i.r.members)))?;
        match fields {
            Some(fields) => drop(now.insert(id.to_string(), place_hash(fields))),
            None => drop(now.remove(id)),
        }
        self.set_setting(caller, PINNED, Some(&serde_json::to_string(&now)?)).await
    }

    /// Restores each place's newest backup that's due its weekly check (`started`: when this device started), on the
    /// device that checks it: a folder's own device, and for S3 the first in a line of the devices that run `global`,
    /// each after it once the ones before have had time to. Not before a place's first backup.
    async fn check_due(&self, started: u64) -> Result<()> {
        let now = now_ms();
        if now < started + CHECK_SETTLE {
            return Ok(());
        }
        let me = self.0.id;
        let due: Vec<String> = self.with(|i| {
            let m = &i.r.members;
            let Some(status) = i.status.get("global") else { return Ok(Vec::new()) };
            Ok(places(i)
                .into_iter()
                .filter(|p| status.covered.keys().any(|(place, _)| *place == p.id))
                .filter(|p| {
                    let last = status.checks.iter().rev().find(|c| c.place == p.id);
                    let next = last.map_or(0, |c| c.time + if c.error.is_some() { CHECK_AGAIN } else { CHECK_EVERY });
                    match p.device {
                        Some(d) => d == me && now >= next,
                        None => line(m, &Scope::Global, &format!("{}/check", p.id)).iter().position(|k| *k == me).is_some_and(|at| now >= next && b::takes_over(at, Some(next), 0, now)),
                    }
                })
                .map(|p| p.id)
                .collect())
        })?;
        if !due.is_empty() {
            self.check_places(Some(&due)).await?;
        }
        Ok(())
    }

    /// Restores each scope's newest snapshot from each place into a throwaway store and throwaway databases, checks
    /// them (`backups::check`, `backups::check_databases`), and says what came back.
    pub async fn check_backups(&self) -> Result<Value> {
        self.check_places(None).await
    }

    /// Checks the places `only` names, or every one this device reaches, and writes how each went in `global`.
    async fn check_places(&self, only: Option<&[String]>) -> Result<Value> {
        let epochs = self.held_epochs().await;
        let (install, places, root, live) = self.with(|i| {
            ensure!(i.views.contains_key("global"), "no install");
            let places: Vec<(String, String, Place)> = places(i).into_iter().filter(|p| only.is_none_or(|o| o.contains(&p.id))).filter_map(|p| Some((p.id, p.title, p.place.ok()?))).filter(|(_, _, p)| p.present()).collect();
            let mut live = BTreeMap::new();
            for scope in epochs.keys() {
                live.insert(scope.clone(), i.r.store.scope(scope)?);
            }
            Ok((i.r.root.map(|r| r.to_hex()[..16].to_string()).unwrap_or_default(), places, i.r.root, live))
        })?;
        let (live, bucket) = (Arc::new(live), self.bucket().ok());
        let mut out = Vec::new();
        for (id, title, place) in places {
            let now = now_ms();
            let checked = {
                let keys: BTreeMap<String, Vec<[u8; 32]>> = epochs.iter().map(|(s, k)| (s.clone(), k.values().copied().collect())).collect();
                let (place, install, live) = (place.clone(), install.clone(), live.clone());
                tokio::task::spawn_blocking(move || b::check(&b::Check { place: &place, install: &install, keys: &keys, root, live: Some(&live) })).await?
            };
            let (mut report, error) = match checked {
                Ok((mut report, restored)) => {
                    let mut problems: Vec<Value> = report["problems"].as_array().cloned().unwrap_or_default();
                    for r in &restored {
                        let Some(keys) = epochs.get(&r.snapshot.scope) else { continue };
                        problems.extend(b::check_databases(&place, &install, &r.snapshot, keys, bucket.as_ref()).await.into_iter().map(Value::from));
                    }
                    // Each says whose data it's about by its scope; people read their names.
                    let named: Vec<(String, String)> = self.with(|i| Ok(epochs.keys().map(|s| (s.clone(), scope_name(i, s))).collect()))?;
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
                    report["unchecked"] = json!(self.with(|i| Ok(unchecked.iter().map(|s| scope_name(i, s)).collect::<Vec<_>>()))?);
                    for s in report["scopes"].as_array_mut().into_iter().flatten() {
                        s["name"] = json!(self.with(|i| Ok(scope_name(i, s["scope"].as_str().unwrap_or(""))))?);
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
            self.say(&Scope::Global, Said::Checked(CheckRun { place: id, device: self.0.id, time: now, error: error500, report: kept })).await?;
            report["place"] = json!(title);
            report["databases"] = json!(true);
            if let Some(error) = error {
                report["error"] = json!(error);
            }
            out.push(report);
        }
        Ok(json!(out))
    }

    /// The backups' routes, `/v1/backups…`, for the local API.
    pub async fn backups_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        let me = &Caller { agent: body["agent"].as_str().map(str::to_string).or(me.agent.clone()), ..me.clone() };
        Ok(Some(match (method, path) {
            ("GET", "/v1/backups") => self.backups()?,
            ("POST", "/v1/backups/now") => {
                self.with(|i| {
                    ensure!(i.r.members.may_use(&me.user, &Scope::Global), "You aren't a member here, so you can't back up.");
                    Ok(())
                })?;
                self.back_up_now();
                json!({})
            }
            ("POST", "/v1/backups/place") => match body["deleted"] == true {
                true => {
                    self.remove_place(me, body["id"].as_str().unwrap_or("")).await?;
                    json!({})
                }
                false => {
                    ensure!(me.agent.is_none() || body["secret"].is_null(), "agents never handle a place's key: ask the person with request");
                    self.add_place(me, body).await?
                }
            },
            ("POST", "/v1/backups/request") => self.request_place(me, body).await?,
            ("POST", "/v1/backups/check") => self.check_backups().await?,
            _ => return Ok(None),
        }))
    }
}

/// Whether an entry is a release of the node's or a service's.
fn is_release(e: &Entry) -> bool {
    e.content.app == release::APP && (e.content.kind == "node-release" || e.content.kind == release::SERVICE_KIND)
}

/// The node's releases' blobs: what a backup needn't keep but the newest of, since releases can be fetched again from
/// where they're published.
fn releases(entries: &[Entry]) -> BTreeSet<String> {
    entries.iter().filter(|e| is_release(e)).flat_map(|e| e.content.blobs.iter().cloned()).collect()
}

/// The blobs a scope's backup keeps: every blob its entries name but those its apps forgot, and of the node's releases
/// only the newest for each platform (and each service's), so a white label with no published releases can still
/// recover.
fn backed_blobs(entries: &[Entry], forgotten: &BTreeSet<String>) -> BTreeSet<String> {
    let mut newest: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut out = BTreeSet::new();
    for e in entries {
        if is_release(e) {
            let target = format!("{} {}", e.content.data["target"].as_str().unwrap_or(""), e.content.data["service"].as_str().unwrap_or(""));
            if let Some(blob) = e.content.blobs.first() {
                if newest.get(&target).is_none_or(|(t, _)| e.time() > *t) {
                    newest.insert(target, (e.time(), blob.clone()));
                }
            }
            continue;
        }
        out.extend(e.content.blobs.iter().filter(|b| (e.content.app == members::APP && e.content.kind != "keep") || !forgotten.contains(*b)).cloned());
    }
    out.extend(newest.into_values().map(|(_, b)| b));
    out
}

/// The disks plugged into this machine, where a folder place could go: each one's name, folder and free space.
fn disks() -> Vec<Value> {
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
            out.push(json!({ "name": e.file_name().to_string_lossy(), "path": path.join(format!("{} Backups", crate::config::NAME)).display().to_string(), "free": free }));
        }
    }
    out
}

fn free_space(path: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}
