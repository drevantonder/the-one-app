//! Backups, as a server runs them: which scopes it backs up to which places, hourly when anything changed, and how
//! each run went, written in `global` so every device shows it. The format and the places are `crate::backups`.

use super::*;
use std::collections::BTreeSet;

use crate::backups::{self as b, Place, Snapshot};

/// How often the node looks for backups that are due.
const TICK: Duration = Duration::from_secs(60);

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
        .filter(|(_, it)| it.apps.contains(members::APP))
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

/// A user this server may write a scope's node entries as: the scope's own user, else its local user, else anyone
/// who trusts it and may use the scope.
fn writer(i: &Install, me: &Key, scope: &Scope) -> Option<String> {
    let m = &i.r.members;
    let ok = |u: &str| m.may_use(u, scope) && m.trusts(u, me);
    match scope {
        Scope::User(u) => ok(u).then(|| u.clone()),
        _ if ok(&i.user) => Some(i.user.clone()),
        _ => m.users.keys().find(|u| ok(u)).cloned(),
    }
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
    writer: String,
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
            }
        }
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
        let (install, recovery, plan) = self.with(|i| {
            let m = &i.r.members;
            let Some(global) = i.views.get("global").map(|(_, s)| s) else { return Ok((String::new(), [0; 32], Vec::new())) };
            let install = i.r.root.map(|r| r.to_hex()[..16].to_string()).context("no install")?;
            let recovery = m.recovery.context("the install has no recovery key")?;
            let frontier = i.r.store.frontier()?;
            let cache = self.0.backed.lock().unwrap();
            let failed = self.0.backup_failed.lock().unwrap();
            let mut plan = Vec::new();
            for p in places(global) {
                if !force && failed.get(&p.id).is_some_and(|t| now - t < b::EVERY) {
                    continue;
                }
                let folder = matches!(p.place, Ok(Place::Folder(_)));
                if folder && p.server != Some(me) {
                    continue;
                }
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
                            let covered = global.backups.iter().filter(|r| r.place == p.id && r.error.is_none()).flat_map(|r| &r.snapshots).filter(|s| s.scope == log).map(|s| s.upto).max().unwrap_or(0);
                            if !b::takes_over(at, i.r.store.first_change(&log, covered)?, p.made, now) {
                                continue;
                            }
                        }
                    }
                    let state = i.views.get(&log).map(|(_, s)| s);
                    let key = state.and_then(|s| s.backup_key);
                    let known = cache.get(&(p.id.clone(), log.clone())).filter(|s| key.is_some_and(|k| s.1 == b::key_id(&k)));
                    let changed = match known {
                        None => true,
                        Some((s, _)) => {
                            let since = s.time + b::EVERY <= now || force;
                            since && frontier.iter().any(|(a, l, next)| *l == log && s.frontier.get(&hex(a)).copied().unwrap_or(0) < *next) && i.r.store.changed_since(&log, &s.frontier)?
                        }
                    };
                    if !changed {
                        continue;
                    }
                    let Some(writer) = writer(i, &me, &scope) else { continue };
                    due.push(Due { scope: log, key, writer });
                }
                if !due.is_empty() {
                    plan.push((p, due));
                }
            }
            Ok((install, recovery, plan))
        })?;
        for (p, due) in plan {
            let place = match p.place {
                Ok(place) => place,
                Err(err) => {
                    self.backed_up(&p.id, now, Some(format!("{err:#}")), Vec::new()).await?;
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
                        error.get_or_insert(format!("{err:#}"));
                    }
                }
            }
            match &error {
                Some(_) => self.0.backup_failed.lock().unwrap().insert(p.id.clone(), now),
                None => self.0.backup_failed.lock().unwrap().remove(&p.id),
            };
            if error.is_some() || !snapshots.is_empty() {
                self.backed_up(&p.id, now, error, snapshots).await?;
            }
        }
        Ok(())
    }

    /// Backs up one scope to one place: makes the scope's key if it has none, then sends what the place lacks.
    async fn back_up_scope(&self, place: &Place, place_id: &str, install: &str, recovery: Key, d: &Due, now: u64) -> Result<Option<Snapshot>> {
        let scope = Scope::parse(&d.scope)?;
        let key = match d.key {
            Some(key) => key,
            None => {
                let key: [u8; 32] = rand::random();
                self.append_as(&scope, &d.writer, None, members::node("backup-key", json!({ "key": hex(&key) }))).await?;
                // Two servers may make one at once: the first written is the scope's, and the other's backups go again.
                self.with(|i| Ok(i.views.get(&d.scope).and_then(|(_, s)| s.backup_key)))?.unwrap_or(key)
            }
        };
        let cached = self.0.backed.lock().unwrap().get(&(place_id.to_string(), d.scope.clone())).filter(|s| s.1 == b::key_id(&key)).map(|s| s.0.clone());
        let (entries, blobs) = self.with(|i| {
            let entries = i.r.store.scope(&d.scope)?;
            Ok((entries.clone(), backed_blobs(&entries)))
        })?;
        let dir = self.0.dir.clone();
        let (place, install, scope_name, server) = (place.clone(), install.to_string(), d.scope.clone(), self.0.id);
        let snap = tokio::task::spawn_blocking(move || -> Result<(Option<Snapshot>, Option<Snapshot>)> {
            // What the place holds: the newest snapshot there, which another server may have written since.
            let newest = b::snapshots(&place, &install, &scope_name)?.into_iter().next();
            let previous = match (cached, newest) {
                (Some(c), Some((t, _))) if c.time >= t => Some(c),
                (_, None) => None,
                _ => b::latest(&place, &install, &scope_name, &[key])?.map(|(s, _)| s),
            };
            let job = b::Job { place: &place, install: &install, scope: &scope_name, key, recovery, server, now, previous: previous.clone(), entries, blobs };
            let written = b::back_up(job, |hash| std::fs::read(release::blob_path(&dir, hash).ok()?).ok())?;
            Ok((written.clone(), written.or(previous)))
        })
        .await??;
        let (written, known) = snap;
        if let Some(known) = known {
            self.0.backed.lock().unwrap().insert((place_id.to_string(), d.scope.clone()), (known, b::key_id(&key)));
        }
        Ok(written)
    }

    /// Writes how a run to a place went, in `global`.
    async fn backed_up(&self, place: &str, now: u64, error: Option<String>, snapshots: Vec<Snapshot>) -> Result<()> {
        let snapshots: Vec<Value> = snapshots.iter().map(|s| json!({ "scope": s.scope, "upto": s.upto, "entries": s.entries, "blobs": s.blobs.len(), "bytes": s.bytes })).collect();
        let mut data = json!({ "place": place, "time": now, "snapshots": snapshots });
        if let Some(error) = error {
            data["error"] = json!(error.chars().take(500).collect::<String>());
        }
        self.append(&Scope::Global, members::node("backup", data)).await?;
        Ok(())
    }

    /// Backups as a person sees them: each place, how its last run went, the recent snapshots, and the disks here.
    pub fn backups(&self) -> Result<Value> {
        self.with(|i| {
            let m = &i.r.members;
            let empty = ScopeState::default();
            let global = i.views.get("global").map(|(_, s)| s).unwrap_or(&empty);
            let name = |k: &Key| m.machine(k).map(|s| s.name.clone()).unwrap_or_else(|| short(k));
            let runs = &global.backups;
            let mut out = Vec::new();
            for p in places(global) {
                let mine: Vec<&members::BackupRun> = runs.iter().filter(|r| r.place == p.id).collect();
                let last_ok = mine.iter().rev().find(|r| r.error.is_none());
                // A place fails while its latest run since its last good one failed.
                let failing = mine.iter().rev().take_while(|r| r.error.is_some()).last().map(|first| (first.time, mine.last().and_then(|r| r.error.clone())));
                let (kind, place_where) = match &p.place {
                    Ok(Place::Folder(path)) => ("folder", path.display().to_string()),
                    Ok(Place::S3(s3)) => ("s3", format!("{} · {}", s3.bucket, s3.endpoint.split("://").nth(1).unwrap_or(""))),
                    Err(_) => ("s3", String::new()),
                };
                let bytes: u64 = {
                    let mut latest: BTreeMap<&str, u64> = BTreeMap::new();
                    for r in &mine {
                        for s in &r.snapshots {
                            latest.insert(&s.scope, s.bytes);
                        }
                    }
                    latest.values().sum()
                };
                out.push(json!({
                    "id": p.id,
                    "title": p.title,
                    "kind": kind,
                    "where": place_where,
                    "server": p.server.map(|k| name(&k)),
                    "last": last_ok.map(|r| json!({ "time": r.time, "server": name(&r.server) })),
                    "failing": failing.map(|(since, error)| json!({ "since": since, "error": error.or(p.place.as_ref().err().map(|e| format!("{e:#}"))) })),
                    "bytes": bytes,
                }));
            }
            let titles: BTreeMap<String, String> = places(global).into_iter().map(|p| (p.id, p.title)).collect();
            // Each run that wrote snapshots, newest first: when, by which server, to which place.
            let snapshots: Vec<Value> = runs.iter().rev().filter(|r| r.error.is_none() && titles.contains_key(&r.place)).take(20).map(|r| json!({ "time": r.time, "server": name(&r.server), "place": titles[&r.place], "scopes": r.snapshots.iter().map(|s| s.scope.clone()).collect::<Vec<_>>() })).collect();
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
            tokio::task::spawn_blocking(move || place.try_it()).await?
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
                let title = text("title").unwrap_or_else(|| format!("{} on {host}", s3.bucket));
                let fields = json!({
                    "endpoint": { "value": s3.endpoint }, "bucket": { "value": s3.bucket }, "region": { "value": s3.region },
                    "key_id": { "value": s3.key_id }, "secret": { "value": s3.secret, "secret": true },
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
        let (install, places, keys, root) = self.with(|i| {
            let global = &i.views.get("global").context("no install")?.1;
            let keys: BTreeMap<String, [u8; 32]> = i.views.iter().filter_map(|(l, (_, s))| Some((l.clone(), s.backup_key?))).collect();
            let places: Vec<(String, Place)> = places(global).into_iter().filter_map(|p| Some((p.title, p.place.ok()?))).filter(|(_, p)| p.present()).collect();
            Ok((i.r.root.map(|r| r.to_hex()[..16].to_string()).unwrap_or_default(), places, keys, i.r.root))
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

    /// The backups' routes, `/v1/backups…`, for the local API and phones alike.
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

/// The blobs a scope's backup keeps: every blob its entries name, but of the node's releases only the newest for each
/// platform, so a white label with no published releases can still recover.
fn backed_blobs(entries: &[Entry]) -> BTreeSet<String> {
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
        out.extend(e.content.blobs.iter().cloned());
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
