//! Backups, as a server runs them: which scopes it backs up to which places, hourly when anything changed, and how
//! each run went, written in `global` so every device shows it. The format and the places are `crate::backups`.

use super::*;
use super::status::{BackedUp, BackupRun, Said};
use std::collections::BTreeSet;

use crate::backups::{self as b, Snapshot};
use crate::bucket::Place;

/// How often the node looks for backups that are due.
const TICK: Duration = Duration::from_secs(60);
/// How often a server prunes each place it backs up to: after its first run, then daily.
const PRUNE_EVERY: u64 = 24 * 3_600_000;
/// How long changes may wait for a place before admins hear of it, which catches a server that's gone.
const LATE: u64 = 24 * 3_600_000;
/// How long each server after the first in line waits before telling admins itself, in case those before it are down.
const STAGGER: u64 = 10 * 60_000;

/// What's wrong with a place, for admins: what, its fix, since when, and from when it's worth a push.
#[derive(Clone, Debug)]
pub struct Problem {
    pub place: String,
    pub text: String,
    pub fix: String,
    pub since: u64,
    pub push: Option<u64>,
}

impl Problem {
    pub fn json(&self) -> Value {
        json!({ "place": self.place, "text": self.text, "fix": self.fix, "since": self.since })
    }
}

/// Each place's problem, as this server sees it: its runs failing (worth a push from the second failure in a row),
/// or a scope this server holds with changes that have waited a day for it, whoever should have backed them up.
/// Both end by themselves once a run works.
fn problems(i: &Install, now: u64) -> Result<Vec<Problem>> {
    let m = &i.r.members;
    let Some((_, global)) = i.views.get("global") else { return Ok(Vec::new()) };
    let status = i.status.get("global").cloned().unwrap_or_default();
    let name = |k: &Key| m.machine(k).map(|s| s.name.clone()).unwrap_or_else(|| short(k));
    let mut out = Vec::new();
    for p in places(global) {
        let runs: Vec<&BackupRun> = status.backups.iter().filter(|r| r.place == p.id).collect();
        let failing: Vec<&&BackupRun> = runs.iter().rev().take_while(|r| r.error.is_some()).collect();
        if let Some(last) = failing.first() {
            let error = last.error.clone().unwrap_or_default();
            let trouble = last.trouble.as_deref().and_then(crate::bucket::Trouble::parse);
            let text = match trouble {
                Some(_) => format!("{} {error}.", p.title),
                None => format!("{}: {error}.", p.title),
            };
            let server = name(&last.server);
            let fix = match (trouble, &p.place) {
                (_, Err(_)) => "Fix it in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Key), _) => "Change its key in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Bucket), _) => "Check the bucket's name in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Refused), _) => "Give the key permission to write there, or change it in the Vault.".to_string(),
                (Some(crate::bucket::Trouble::Unreachable), _) => format!("Check {server} is online. It tries again every hour."),
                (Some(crate::bucket::Trouble::Disk), _) => format!("Check the disk on {server} is plugged in and has room."),
                (None, _) => "It tries again every hour.".to_string(),
            };
            let since = failing.last().map_or(last.time, |r| r.time);
            out.push(Problem { place: p.id.clone(), text, fix, since, push: failing.iter().rev().nth(1).map(|r| r.time) });
            continue;
        }
        let mut late: Option<(u64, String)> = None;
        for log in i.r.store.logs()? {
            let Ok(scope) = Scope::parse(&log) else { continue };
            let backer = match &p.place {
                Ok(Place::Folder(_)) => p.server.filter(|k| m.holds(k, &scope)),
                _ => line(m, &scope, &p.id).first().copied(),
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
            let fix = match &p.place {
                Ok(Place::Folder(_)) => format!("Check {backer} is on, with the disk plugged in."),
                _ => format!("{backer} backs it up: check it's on."),
            };
            out.push(Problem { place: p.id.clone(), text: format!("Nothing has gone to {} for a day.", p.title), fix, since, push: Some(since + LATE) });
        }
    }
    Ok(out)
}

/// A place to back up to, as its Vault item says: one in Everyone that the node is switched on for.
struct PlaceItem {
    id: String,
    title: String,
    place: Result<Place>,
    /// For a folder: the server whose disk it's on.
    server: Option<Key>,
    /// When it was added.
    made: u64,
}

/// What a place's item says, read from global's items.
fn places(state: &ScopeState) -> Vec<PlaceItem> {
    state
        .items
        .iter()
        .filter(|(_, it)| it.apps.contains(members::APP) && it.kind != super::storage::STORAGE)
        .map(|(id, it)| {
            let fields: BTreeMap<String, String> = it.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect();
            let place = Place::from_fields(&fields).and_then(|p| match &p {
                // Secrets stay put: the key goes only to the item's own hosts.
                Place::S3(s3) => {
                    let host = s3.endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
                    ensure!(it.hosts.iter().any(|h| h == host), "its key is for {}, not {host}", it.hosts.join(", "));
                    Ok(p)
                }
                Place::Folder(_) => Ok(p),
            });
            PlaceItem { id: id.clone(), title: it.title.clone(), place, server: fields.get("server").and_then(|s| unhex(s).ok()), made: it.made }
        })
        .collect()
}

/// The servers that back up a scope to an S3 place, in the order they take it: privileged servers first, then the
/// scope's user's own, then the rest, each group in an order the place and scope pick. Every server works it out alike.
fn line(m: &Members, scope: &Scope, place: &str) -> Vec<Key> {
    let user = match scope {
        Scope::User(u) => Some(Principal::User(u.clone())),
        _ => None,
    };
    let mut servers: Vec<(bool, bool, [u8; 32], Key)> = m
        .servers()
        .filter(|s| m.holds(&s.key, scope))
        .map(|s| {
            let own = user.as_ref().is_some_and(|u| m.owners(&s.key).contains(u));
            let mut h = blake3::Hasher::new();
            h.update(&s.key);
            h.update(format!("{scope}/backups/{place}").as_bytes());
            (!m.privileged(&s.key), !own, *h.finalize().as_bytes(), s.key)
        })
        .collect();
    servers.sort();
    servers.into_iter().map(|s| s.3).collect()
}

/// One scope to back up to one place in a run.
struct Due {
    scope: String,
    key: Option<[u8; 32]>,
}

impl Node {
    /// Now and then, backs up what's due.
    pub(super) async fn backup_loop(self) {
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
                if let Err(err) = self.tell_admins().await {
                    tracing::warn!("telling admins about backups: {err:#}");
                }
            }
        }
    }

    /// Pushes a place's problem to the install's admins once it's worth it, through the install's push. Every server
    /// watches, so a server that's gone is noticed too; the first in line tells them, each after it only once the
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
                let input = json!({ "title": "Backups", "body": format!("{} {}", p.text, p.fix) });
                let call = CallRequest { app: push.clone(), sidecar: Some(push.clone()), op: "send".into(), input, client: Some("app".into()), ..Default::default() };
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

    /// Backs up now, everything this server backs up, whether or not an hour has passed.
    pub fn back_up_now(&self) {
        *self.0.backup_forced.lock().unwrap() = true;
        self.0.backup_now.notify_one();
    }

    /// Backs up each scope to each place this server backs it up to, if an hour passed and anything changed (or
    /// `force`), and writes how each place's run went.
    async fn back_up(&self, force: bool) -> Result<()> {
        let now = now_ms();
        let me = self.0.id;
        let keys = self.held_keys().await;
        let (install, recovery, plan) = self.with(|i| {
            let m = &i.r.members;
            let Some(global) = i.views.get("global").map(|(_, s)| s) else { return Ok((String::new(), [0; 32], Vec::new())) };
            let install = i.r.root.map(|r| r.to_hex()[..16].to_string()).context("no install")?;
            let recovery = m.recovery.context("the install has no recovery key")?;
            let frontier = i.r.store.frontier()?;
            let cache = self.0.backed.lock().unwrap();
            let failed = self.0.backup_failed.lock().unwrap();
            let pruned = self.0.pruned.lock().unwrap();
            let mut plan = Vec::new();
            for p in places(global) {
                if !force && failed.get(&p.id).is_some_and(|t| now - t < b::EVERY) {
                    continue;
                }
                let folder = matches!(p.place, Ok(Place::Folder(_)));
                if folder && p.server != Some(me) {
                    continue;
                }
                // Every server prunes each scope it holds the key of, so pruning needs no one server.
                let prune = (force || pruned.get(&p.id).is_none_or(|t| now - t >= PRUNE_EVERY)).then(|| -> Result<_> {
                    Ok((keys.clone(), releases(&i.r.store.scope("global")?)))
                });
                let prune = prune.transpose()?;
                let mut due = Vec::new();
                let mut logs = i.r.store.logs()?;
                if !logs.iter().any(|l| l == "global") {
                    logs.insert(0, "global".into());
                }
                for log in logs {
                    let Ok(scope) = Scope::parse(&log) else { continue };
                    if !i.r.holds(&scope) {
                        continue;
                    }
                    if !folder {
                        let line = line(m, &scope, &p.id);
                        let Some(at) = line.iter().position(|k| *k == me) else { continue };
                        // A server further down the line takes a scope's changes only once they've waited for each
                        // server before it.
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
            Ok((install, recovery, plan))
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
                match self.back_up_scope(&place, &p.id, &install, recovery, &d, now).await {
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

    /// The key of each scope this server holds, by its name, of those it can have.
    async fn held_keys(&self) -> BTreeMap<String, [u8; 32]> {
        let scopes = self.with(|i| Ok(i.r.members.scopes().into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>())).unwrap_or_default();
        let mut out = BTreeMap::new();
        for scope in scopes {
            if let Ok(key) = self.scope_key(&scope).await {
                out.insert(scope.to_string(), key);
            }
        }
        out
    }

    /// Backs up one scope to one place, with the scope's key: sends what the place lacks.
    async fn back_up_scope(&self, place: &Place, place_id: &str, install: &str, recovery: Key, d: &Due, now: u64) -> Result<Option<Snapshot>> {
        let scope = Scope::parse(&d.scope)?;
        let key = match d.key {
            Some(key) => key,
            None => self.scope_key(&scope).await?,
        };
        let cached = self.0.backed.lock().unwrap().get(&(place_id.to_string(), d.scope.clone())).filter(|s| s.1 == b::key_id(&key)).map(|s| s.0.clone());
        let (entries, blobs) = self.with(|i| {
            let entries = i.r.store.scope(&d.scope)?;
            let forgotten = i.views.get(&d.scope).map(|(_, s)| s.forgotten.clone()).unwrap_or_default();
            Ok((entries.clone(), backed_blobs(&entries, &forgotten)))
        })?;
        let dir = self.0.dir.clone();
        let bucket = self.bucket().ok().map(|b| b.place);
        let (place, install, scope_name, server) = (place.clone(), install.to_string(), d.scope.clone(), self.0.id);
        let snap = tokio::task::spawn_blocking(move || -> Result<(Option<Snapshot>, Option<Snapshot>)> {
            // What the place holds: the newest snapshot there, which another server may have written since.
            let newest = b::snapshots(&place, &install, &scope_name)?.into_iter().next();
            let previous = match (cached, newest) {
                (Some(c), Some((t, _))) if c.time >= t => Some(c),
                (_, None) => None,
                _ => b::latest(&place, &install, &scope_name, &[key])?.map(|(s, _)| s),
            };
            let job = b::Job { place: &place, install: &install, scope: &scope_name, key, recovery, server, now, previous: previous.clone(), entries, blobs, bucket: bucket.as_ref() };
            let written = b::back_up(job, |hash| std::fs::read(release::blob_path(&dir, hash).ok()?).ok())?;
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
        self.say(&Scope::Global, Said::BackedUp(BackupRun { place: place.into(), server: self.0.id, time: now, error, trouble, snapshots })).await
    }

    /// Backups as a person sees them: each place, how its last run went, the recent snapshots, and the disks here.
    pub fn backups(&self) -> Result<Value> {
        self.with(|i| {
            let m = &i.r.members;
            let empty = ScopeState::default();
            let global = i.views.get("global").map(|(_, s)| s).unwrap_or(&empty);
            let name = |k: &Key| m.machine(k).map(|s| s.name.clone()).unwrap_or_else(|| short(k));
            let runs = i.status.get("global").map(|s| s.backups.clone()).unwrap_or_default();
            let problems = problems(i, now_ms())?;
            let mut out = Vec::new();
            for p in places(global) {
                let mine: Vec<&BackupRun> = runs.iter().filter(|r| r.place == p.id).collect();
                let last_ok = mine.iter().rev().find(|r| r.error.is_none());
                let (kind, place_where, endpoint) = match &p.place {
                    Ok(Place::Folder(path)) => ("folder", path.display().to_string(), None),
                    Ok(Place::S3(s3)) => ("s3", s3.bucket.clone(), Some(s3.endpoint.clone())),
                    Err(_) => ("s3", String::new(), None),
                };
                out.push(json!({
                    "id": p.id,
                    "title": p.title,
                    "kind": kind,
                    "where": place_where,
                    "endpoint": endpoint,
                    "server": p.server.map(|k| name(&k)),
                    "problem": problems.iter().find(|q| q.place == p.id).map(Problem::json),
                    "last": last_ok.map(|r| json!({ "time": r.time, "server": name(&r.server) })),
                }));
            }
            let titles: BTreeMap<String, String> = places(global).into_iter().map(|p| (p.id, p.title)).collect();
            // The install's snapshots, newest first: the runs of each minute, whichever servers and places, as one.
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
                m.1.insert(name(&r.server));
                m.2.insert(titles[&r.place].clone());
                m.3.extend(r.snapshots.iter().map(|s| scope_name(i, &s.scope)));
            }
            let snapshots: Vec<Value> = moments
                .into_iter()
                .map(|(time, servers, places, scopes)| {
                    let (servers, places): (Vec<String>, Vec<String>) = (servers.into_iter().collect(), places.into_iter().collect());
                    // `server` and `place`, one each, for apps built before a snapshot was the install's.
                    json!({ "time": time, "server": servers.join(", "), "place": places.join(", "), "servers": servers, "places": places, "scopes": scopes })
                })
                .collect();
            Ok(json!({ "places": out, "snapshots": snapshots, "here": self.server_name(), "disks": disks() }))
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
                (title, json!({ "path": { "value": path.display().to_string() }, "server": { "value": hex(&self.0.id) } }), json!([]))
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
        self.request_item(caller, &Scope::Global, members::APP, None, &ask, &why).await
    }

    /// Stops backing up to a place: deletes its item. What's in the place stays there.
    pub async fn remove_place(&self, caller: &Caller, id: &str) -> Result<()> {
        let known = self.with(|i| Ok(i.views.get("global").is_some_and(|(_, s)| places(s).iter().any(|p| p.id == id))))?;
        ensure!(known, "no place {id}");
        self.set_item(caller, &Scope::Global, json!({ "id": id, "deleted": true })).await?;
        Ok(())
    }

    /// Restores each scope's newest snapshot from each place into a throwaway store, checking every entry and blob,
    /// and says what came back.
    pub async fn check_backups(&self) -> Result<Value> {
        let keys = self.held_keys().await;
        let (install, places, root) = self.with(|i| {
            let global = &i.views.get("global").context("no install")?.1;
            let places: Vec<(String, Place)> = places(global).into_iter().filter_map(|p| Some((p.title, p.place.ok()?))).filter(|(_, p)| p.present()).collect();
            Ok((i.r.root.map(|r| r.to_hex()[..16].to_string()).unwrap_or_default(), places, i.r.root))
        })?;
        let mut out = Vec::new();
        for (title, place) in places {
            let (install, keys) = (install.clone(), keys.clone());
            let report = tokio::task::spawn_blocking(move || -> Result<Value> {
                let keys: BTreeMap<String, Vec<[u8; 32]>> = keys.into_iter().map(|(s, k)| (s, vec![k])).collect();
                b::check(&place, &install, &keys, root)
            })
            .await?;
            out.push(match report {
                Ok(mut r) => {
                    r["place"] = json!(title);
                    r
                }
                Err(err) => json!({ "place": title, "error": format!("{err:#}") }),
            });
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
                    ensure!(i.r.members.may_use(&me.user, &Scope::Global), "not in this install");
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

/// The node's releases' blobs: what a backup needn't keep but the newest of, since releases can be fetched again from
/// where they're published.
fn releases(entries: &[Entry]) -> BTreeSet<String> {
    entries.iter().filter(|e| e.content.app == release::APP && e.content.kind == "node-release").flat_map(|e| e.content.blobs.iter().cloned()).collect()
}

/// The blobs a scope's backup keeps: every blob its entries name but those its apps forgot, and of the node's releases
/// only the newest for each platform, so a white label with no published releases can still recover.
fn backed_blobs(entries: &[Entry], forgotten: &BTreeSet<String>) -> BTreeSet<String> {
    let mut newest: BTreeMap<String, (u64, String)> = BTreeMap::new();
    let mut out = BTreeSet::new();
    for e in entries {
        if e.content.app == release::APP && e.content.kind == "node-release" {
            let target = e.content.data["target"].as_str().unwrap_or("").to_string();
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

/// A scope as people call it: Everyone, a person's name, or a group's.
fn scope_name(i: &Install, scope: &str) -> String {
    let m = &i.r.members;
    match Scope::parse(scope) {
        Ok(Scope::Global) => "Everyone".to_string(),
        Ok(Scope::User(u)) => m.user(&u).map(|u| u.name.clone()).unwrap_or(u),
        Ok(Scope::Group(g)) => m.groups.get(&g).map(|g| g.name.clone()).unwrap_or(g),
        Err(_) => scope.to_string(),
    }
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
