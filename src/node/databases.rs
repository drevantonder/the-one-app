//! Databases: each scope's epochs this device opens, and each app's databases in a scope, as this device reads and
//! writes them in the bucket.

use super::*;

/// What databases keep while the node runs.
#[derive(Default)]
pub(super) struct State {
    /// Each database of each scope, by its app and name (`tally/main`), as this device last read it.
    dbs: Mutex<HashMap<(Scope, String), Arc<tokio::sync::Mutex<Db>>>>,
    /// The databases another device said it wrote whose step isn't in this device's copy yet, which a read catches up
    /// from the bucket.
    pub(super) behind: Mutex<std::collections::HashSet<(Scope, String)>>,
    /// Each scope's epochs this device opens, by number, once it has looked.
    pub(super) epochs: Mutex<HashMap<Scope, crate::database::stored::Keys>>,
    /// Each database's keys by epoch as this device opens them (`database_keys`), with the newest epoch they're as of.
    database_keys: Mutex<HashMap<(Scope, String), (u32, crate::database::stored::Keys)>>,
    /// Positions of each database (by prefix) holding a step under an epoch no entry made, which every read passes over.
    junk: Mutex<HashMap<String, std::collections::BTreeSet<u64>>>,
    /// When this device last tidied each database (by prefix), ms.
    tidied: Mutex<HashMap<String, u64>>,
    /// The newest hint each database's last write wants moved on to, by its prefix, while one is waiting its turn
    /// (`hint_after`): (last step, newest copy, its epoch).
    hints: Mutex<HashMap<String, (u64, u64, u32)>>,
}

/// How many times a database is opened again after finding junk or a newer epoch before it gives up.
const TRIES: usize = 20;
/// How often each database open here is tidied (`stored::tidy`).
const TIDY_EVERY: u64 = 24 * 60 * 60 * 1000;

/// A scope this device wasn't given the newest key of: a device of someone who opens it seals it to this one.
#[derive(Debug)]
pub struct NotGiven(pub Scope, pub String);

impl std::fmt::Display for NotGiven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} wasn't given {}'s newest key: a device of someone who opens it gives it when it's next on", self.1, self.0)
    }
}

impl std::error::Error for NotGiven {}

/// The keys a call was handed for a database, with the bucket as that call reaches it, and until when they're kept (and
/// whether app schedules handed them).
pub(super) struct Given<'a> {
    pub keys: crate::database::stored::Keys,
    pub bucket: &'a Copied,
    pub until: u64,
    pub app_schedules: bool,
}

impl Node {
    /// The network's bucket, as this device reaches it, with its copy of the databases' steps.
    pub(super) fn bucket(&self) -> Result<Copied> {
        let place = self.with(|i| i.bucket.as_ref().map(|(p, _)| p.clone()).context("this device can't reach the network's bucket, where apps keep their data: use another device"))?;
        Ok(Copied::moving(place, self.network_dir().join("copy"), self.0.moved.clone()))
    }

    /// This device as an endpoint: the key it signs with, and the key what's for it is sealed to.
    pub fn endpoint(&self) -> crate::keys::Endpoint {
        crate::keys::Endpoint::of(&self.signing_key())
    }

    pub(super) fn signing_key(&self) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes())
    }

    /// The secret that opens what's sealed to this device: epochs, and the keys a call is handed.
    pub(super) fn seal_secret(&self) -> [u8; 32] {
        crate::keys::seal_secret(&self.signing_key())
    }

    /// `app`'s `database` in `scope`'s keys by epoch, as this device opens them for its own person's work: from the
    /// app's key in each epoch, sealed under the epoch's. The app's key in the newest epoch is made if it isn't yet.
    /// With `make`, a scope with no epoch yet gets its first (`epoch_keys`).
    pub(super) async fn database_keys(&self, scope: &Scope, app: &str, database: &str, make: bool) -> Result<crate::database::stored::Keys> {
        let epochs = self.epoch_keys(scope, make).await?;
        let newest = epochs.keys().next_back().copied().unwrap_or(0);
        let id = (scope.clone(), format!("{app}/{database}"));
        if let Some((n, keys)) = self.0.databases.database_keys.lock().unwrap().get(&id)
            && *n == newest
        {
            return Ok(keys.clone());
        }
        let out = crate::keys::database_keys(&self.bucket()?, &scope.to_string(), app, database, &epochs, true).await?;
        self.0.databases.database_keys.lock().unwrap().insert(id, (newest, out.clone()));
        Ok(out)
    }

    /// The epochs of a scope this device opens, by number: each one an `epoch` entry that counts made (or for
    /// `global`'s first, the network's first freeze), from its copy in the bucket sealed to this device (`keys::epoch_name`) whose
    /// hash is the one the entry names. A copy that isn't is someone's junk, passed over; nothing in the bucket makes an
    /// epoch but its entry. The newest must be one of them. With `make`, a scope with no epoch yet gets its first here,
    /// if this device's person may make it: an admin of the group, or the person whose scope it is.
    pub(crate) async fn epoch_keys(&self, scope: &Scope, make: bool) -> Result<crate::database::stored::Keys> {
        let (keys, newest) = self.epochs_held(scope).await?;
        if newest == 0 {
            if make && *scope != Scope::Global {
                return Box::pin(self.make_first_epoch(scope)).await;
            }
            bail!(NotGiven(scope.clone(), self.device_name()));
        }
        ensure!(keys.contains_key(&newest), NotGiven(scope.clone(), self.device_name()));
        self.0.databases.epochs.lock().unwrap().insert(scope.clone(), keys.clone());
        Ok(keys)
    }

    /// The epochs of a scope this device opens, as `epoch_keys` finds them, even while it wasn't given the newest (as
    /// between its own new epoch's entry and its seals), and the newest's number: 0 for none.
    pub(super) async fn epochs_held(&self, scope: &Scope) -> Result<(crate::database::stored::Keys, u32)> {
        let kept = self.0.databases.epochs.lock().unwrap().get(scope).cloned().unwrap_or_default();
        // Before this device has read `global`'s freeze, those it was given as it made the network, or checked against
        // the steps as it first read them (`first_global_read`): what it reads the freeze with.
        if *scope == Scope::Global && !self.frozen() {
            let newest = kept.keys().next_back().copied().unwrap_or(0);
            return Ok((kept, newest));
        }
        let hashes = self.with(|i| Ok(if *scope == Scope::Global { i.r.global_epochs() } else { i.r.db().epoch_hashes(scope) }))?;
        let newest = hashes.keys().next_back().copied().unwrap_or(0);
        // Kept only while each is still the one its entry made.
        let mut keys: crate::database::stored::Keys = kept.into_iter().filter(|(n, k)| hashes.get(n) == Some(&crate::keys::epoch_hash(k))).collect();
        if newest == 0 || keys.contains_key(&newest) {
            return Ok((keys, newest));
        }
        for (n, hash) in &hashes {
            if !keys.contains_key(n)
                && let Some(key) = self.sealed_epoch(scope, *n, hash).await?
            {
                keys.insert(*n, key);
            }
        }
        Ok((keys, newest))
    }

    /// Epoch `n` of a scope as it's sealed to this device in the bucket, if it's there and its hash is `hash`.
    async fn sealed_epoch(&self, scope: &Scope, n: u32, hash: &str) -> Result<Option<[u8; 32]>> {
        let name = crate::keys::epoch_name(&scope.to_string(), n, &hex(&self.0.id));
        let Some(o) = crate::bucket::Bucket::get(&self.bucket()?, &name).await? else { return Ok(None) };
        let key = crate::keys::open_sealed(&self.seal_secret(), &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok());
        match key.filter(|k| crate::keys::epoch_hash(k) == hash) {
            Some(key) => Ok(Some(key)),
            None => {
                tracing::warn!("{name} isn't the key {scope}'s epoch {n} entry made: passed over");
                Ok(None)
            }
        }
    }

    /// Forgets the epochs this device knows of a scope, so the next use looks in the bucket for newer ones: after a
    /// step under one it wasn't given.
    pub(super) fn forget_epochs(&self, scope: &Scope) {
        self.0.databases.epochs.lock().unwrap().remove(scope);
        self.0.databases.database_keys.lock().unwrap().retain(|(s, _), _| s != scope);
    }

    /// `global`'s epochs, for a device that has read nothing of who's who yet, so knows no epoch's hash: those sealed
    /// to it in the bucket, each kept only once who's who, from the network's first freeze, says it's theirs
    /// (`steps::read_sealed_to`).
    pub(super) async fn first_global_read(&self) -> Result<crate::database::stored::Keys> {
        let mut reader = crate::database::steps::Reader::new(self.with(|i| Ok(i.r.root()))?);
        let (epochs, _) = crate::database::steps::read_sealed_to(&self.bucket()?, &mut reader, &hex(&self.0.id), &self.seal_secret(), &mut None, now_ms()).await?;
        ensure!(!epochs.is_empty(), NotGiven(Scope::Global, self.device_name()));
        self.0.databases.epochs.lock().unwrap().insert(Scope::Global, epochs.clone());
        Ok(epochs)
    }

    /// A scope's first epoch, made here: its `epoch` entry first, naming its key's hash, then the key sealed to the
    /// recovery key and to each client device of someone who uses the scope, and only then used. The recovery key's
    /// copy is made first, so two devices rarely make it at once: one that finds it there waits for the other's entry.
    /// If two are made at once all the same, the one first in causal order counts, and the other's key is dropped.
    async fn make_first_epoch(&self, scope: &Scope) -> Result<crate::database::stored::Keys> {
        let _making = self.0.making_keys.lock().await;
        if self.with(|i| Ok(i.r.db().epoch(scope)))? > 0 {
            return Box::pin(self.epoch_keys(scope, false)).await;
        }
        // A scope's first keys are its admins' to make, as they choose where its apps' schedules run.
        let (may, recovery) = self.with(|i| Ok((i.r.db().may(&i.user, "choose-devices", &scopes::object(scope)), i.r.db().recovery())))?;
        ensure!(may, NotGiven(scope.clone(), self.device_name()));
        let recovery = recovery.context("the network has no recovery key to seal its keys to")?;
        let key: [u8; 32] = rand::random();
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let sealed = crate::keys::seal_to(&crate::keys::twin_public(&recovery)?, &key)?;
        if !crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, 1, "recovery"), &sealed).await? {
            // Another device is making it: its entry says so soon. If none does, what's there is junk, and this one's made.
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                self.read_scope(&Scope::Global).await?;
                if self.with(|i| Ok(i.r.db().epoch(scope)))? > 0 {
                    return self.settled_first_epoch(scope).await;
                }
            }
            tracing::warn!("{scope}'s first epoch was begun but no entry made it: making it here");
        }
        let to = self.with(|i| Ok(i.r.db().openers(scope)))?;
        let hash = crate::keys::epoch_hash(&key);
        self.append(&Scope::Global, content::node("epoch", json!({ "scope": name, "n": 1, "to": to.iter().map(|c| hex(&c.key)).collect::<Vec<_>>(), "hash": hash }))).await?;
        if self.with(|i| Ok(i.r.db().epoch_hash(scope, 1).as_deref() != Some(hash.as_str())))? {
            tracing::info!("another device made {scope}'s first epoch first: its key is used");
            return self.settled_first_epoch(scope).await;
        }
        for c in &to {
            crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, 1, &hex(&c.key)), &crate::keys::seal_to(&c.seal, &key)?).await?;
        }
        tracing::info!("made {scope}'s first epoch");
        let keys = crate::database::stored::Keys::from([(1, key)]);
        self.0.databases.epochs.lock().unwrap().insert(scope.clone(), keys.clone());
        Ok(keys)
    }

    /// A first epoch another device made: its copy for this one, which it seals right after its entry.
    async fn settled_first_epoch(&self, scope: &Scope) -> Result<crate::database::stored::Keys> {
        for _ in 0..50 {
            match Box::pin(self.epoch_keys(scope, false)).await {
                Err(err) if err.is::<NotGiven>() => tokio::time::sleep(Duration::from_millis(100)).await,
                other => return other,
            }
        }
        Box::pin(self.epoch_keys(scope, false)).await
    }

    /// Seals each epoch this device opens of each scope to each client device of someone who uses the scope that
    /// lacks it: a device added since it was made. Compute is never sealed one: it's handed what a call needs.
    pub(super) fn seal_epochs(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let known: Vec<(Scope, crate::database::stored::Keys)> = node.0.databases.epochs.lock().unwrap().iter().map(|(s, k)| (s.clone(), k.clone())).collect();
            for (scope, keys) in known {
                if let Err(err) = node.seal_epoch(&scope, &keys).await {
                    tracing::warn!("sealing {scope}'s epochs: {err:#}");
                }
            }
        });
    }

    async fn seal_epoch(&self, scope: &Scope, keys: &crate::database::stored::Keys) -> Result<()> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        let to = self.with(|i| Ok(i.r.db().openers(scope).into_iter().filter(|c| c.key != self.0.id).collect::<Vec<_>>()))?;
        for (n, key) in keys {
            for c in &to {
                let name = crate::keys::epoch_name(&scope.to_string(), *n, &hex(&c.key));
                if Bucket::get(&bucket, &name).await?.is_none() {
                    Bucket::create(&bucket, &name, &crate::keys::seal_to(&c.seal, key)?).await?;
                }
            }
        }
        Ok(())
    }

    /// Brings a database up to date for a call that reads it, as far as this device's copy goes: a `changed` from another
    /// device's write fetches its step into the copy, and `fill_copies` keeps up with the bucket. Only while a `changed`'s
    /// step isn't here yet is the bucket asked first, and if it doesn't answer, the copy is read as it is: behind,
    /// never wrong.
    pub(super) async fn read_ready(&self, id: &(Scope, String), db: &mut Db, bucket: &Copied) -> Result<()> {
        if self.0.databases.behind.lock().unwrap().remove(id) {
            // A `changed` said another device wrote it: the bucket is asked first, for what that write was.
            let started = Instant::now();
            let result = match tokio::time::timeout(BUCKET_WAIT, db.catch_up(bucket)).await {
                Ok(Ok(_)) => {
                    trace::note(json!({ "step": "behind", "result": "caught up" }), started.elapsed());
                    return Ok(());
                }
                Ok(Err(err)) => {
                    tracing::debug!("{}: reading this device's copy, as the bucket said {err:#}", db.prefix());
                    "failed"
                }
                Err(_) => {
                    tracing::debug!("{}: reading this device's copy, as the bucket didn't answer", db.prefix());
                    "no answer"
                }
            };
            trace::note(json!({ "step": "behind", "result": result }), started.elapsed());
        }
        for _ in 0..TRIES {
            match db.catch_up(&bucket.here_only()).await {
                Err(err) => match err.downcast_ref::<crate::database::stored::Ahead>() {
                    Some(ahead) if !self.judge_ahead(&id.0, db.prefix(), ahead).await? => db.pass_over(ahead.at),
                    _ => return Err(err),
                },
                Ok(_) => return Ok(()),
            }
        }
        bail!("{} has too much junk in it to read", db.prefix())
    }

    /// Writes to a database as `Db::write` does, passing over a step it meets under an epoch no entry made, as a read
    /// does (`judge_ahead`).
    ///
    /// The newest hint (`hint`), which only saves readers work, is moved on after the write returns (`hint_after`), not
    /// before.
    pub(super) async fn write_in<R>(&self, scope: &Scope, db: &mut Db, bucket: &Copied, mut f: impl FnMut(&rusqlite::Connection) -> Result<R>) -> Result<R> {
        db.leave_hint(true);
        let mut out = None;
        for _ in 0..TRIES {
            match db.write(bucket, &mut f).await {
                Err(err) => match err.downcast_ref::<crate::database::stored::Ahead>() {
                    Some(ahead) if !self.judge_ahead(scope, db.prefix(), ahead).await? => db.pass_over(ahead.at),
                    _ => {
                        out = Some(Err(err));
                        break;
                    }
                },
                ok => {
                    out = Some(ok);
                    break;
                }
            }
        }
        db.leave_hint(false);
        if let Some(hint) = db.take_hint() {
            self.hint_after(bucket.clone(), hint);
        }
        out.unwrap_or_else(|| Err(anyhow!("{} has too much junk in it to write to", db.prefix())))
    }

    /// Moves a database's newest hint on to `hint` (its prefix, last step, newest copy and that copy's epoch) in the
    /// background: one already waiting for its turn for the same database is only raised, so a burst of writes asks the
    /// bucket about it once or twice. A hint that fails is the next write's to make.
    fn hint_after(&self, bucket: Copied, (prefix, at, copy, epoch): (String, u64, u64, u32)) {
        let waiting = {
            let mut hints = self.0.databases.hints.lock().unwrap();
            hints.insert(prefix.clone(), (at, copy, epoch)).is_some()
        };
        if waiting {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            loop {
                let Some((at, copy, epoch)) = node.0.databases.hints.lock().unwrap().get(&prefix).copied() else { return };
                if let Err(err) = crate::database::stored::hint(&bucket, &prefix, at, copy, epoch).await {
                    tracing::debug!("{prefix}'s hint: {err:#}");
                }
                let mut hints = node.0.databases.hints.lock().unwrap();
                if hints.get(&prefix) == Some(&(at, copy, epoch)) {
                    hints.remove(&prefix);
                    return;
                }
            }
        });
    }

    /// Brings a database to `schema` as `Db::migrate` does.
    async fn migrate_in(&self, db: &mut Db, bucket: &Copied, schema: &[String]) -> Result<()> {
        db.migrate(bucket, schema).await
    }

    /// A step a database's reader stopped at, under an epoch above all its keys: once `global` is read again, a new
    /// epoch's (true), since its entry is written before anything is sealed under it; or, if no entry made it, junk
    /// (false), which every read of it here passes over from now on.
    async fn judge_ahead(&self, scope: &Scope, prefix: &str, ahead: &crate::database::stored::Ahead) -> Result<bool> {
        if self.with(|i| Ok(i.r.epoch_made(scope)))? < ahead.epoch {
            tokio::time::timeout(Duration::from_secs(10), self.read_scope(&Scope::Global)).await.map_err(|_| anyhow!("global's entries didn't come in time to say what {prefix}'s step {} is", ahead.at))??;
        }
        if self.with(|i| Ok(i.r.epoch_made(scope)))? >= ahead.epoch {
            return Ok(true);
        }
        tracing::warn!("{prefix}'s step {} is under epoch {}, which no entry made: passed over", ahead.at, ahead.epoch);
        self.0.databases.junk.lock().unwrap().entry(prefix.to_string()).or_default().insert(ahead.at);
        Ok(false)
    }

    /// Fetches into this device's copy what the bucket has of each database open here that it lacks, every tick: what
    /// a lost `changed` would have said. A database a call is using is left for the next.
    pub(super) async fn fill_copies(&self) {
        let Ok(bucket) = self.bucket() else { return };
        let mut changed = Vec::new();
        for ((_, name), db) in self.open_dbs() {
            let Ok((prefix, at)) = db.try_lock().map(|db| (db.prefix().to_string(), db.at())) else { continue };
            match bucket.fill(&prefix, at).await {
                Ok(0) => {}
                Ok(_) => changed.extend(name.split('/').next().filter(|app| *app != scopes::APP).map(str::to_string)),
                Err(err) => tracing::debug!("{prefix}: {err:#}"),
            }
        }
        if !changed.is_empty() {
            self.emit(Event::Changed { apps: changed });
        }
    }

    /// Deletes from the bucket what no reader of each database open here needs any more (`stored::tidy`), once a day. A
    /// device that may not delete from the bucket (only an admin's may) just finds it can't.
    pub(super) async fn tidy_databases(&self) {
        let Ok(bucket) = self.bucket() else { return };
        let now = now_ms();
        for (_, db) in self.open_dbs() {
            let Ok((prefix, keys)) = db.try_lock().map(|db| (db.prefix().to_string(), db.keys().clone())) else { continue };
            if self.0.databases.tidied.lock().unwrap().get(&prefix).is_some_and(|t| now < t + TIDY_EVERY) {
                continue;
            }
            self.0.databases.tidied.lock().unwrap().insert(prefix.clone(), now);
            let junk = self.0.databases.junk.lock().unwrap().get(&prefix).cloned().unwrap_or_default();
            match crate::database::stored::tidy(&bucket, &prefix, &keys, &junk, now).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("{prefix}: deleted {n} objects no reader needs"),
                Err(err) => tracing::debug!("{prefix}: tidying: {err:#}"),
            }
        }
    }

    /// Every database open here, by (scope, `app/database`): with this device's own keys, and with keys calls were
    /// handed.
    pub(super) fn open_dbs(&self) -> Vec<((Scope, String), Arc<tokio::sync::Mutex<Db>>)> {
        let mut open: Vec<_> = self.0.databases.dbs.lock().unwrap().iter().map(|(id, db)| (id.clone(), db.clone())).collect();
        open.extend(self.0.sessions.opened.lock().unwrap().iter().flat_map(|(id, all)| all.iter().map(|o| (id.clone(), o.db.clone()))));
        open
    }

    /// Forgets the open databases `close` names, by scope and `app/database`, whichever keys opened them: read again
    /// when next used.
    pub(super) fn close_dbs(&self, close: impl Fn(&Scope, &str) -> bool) {
        self.0.databases.dbs.lock().unwrap().retain(|(scope, db), _| !close(scope, db));
        self.0.sessions.opened.lock().unwrap().retain(|(scope, db), _| !close(scope, db));
    }

    /// `app`'s `database` of `scope`, opened from the bucket the first time and brought to `schema`, or else to the
    /// app's deploy's: with this device's own keys, or `given`, keys a call was handed. Either way, one already open
    /// is used only if the keys open everything it read and it writes under the same newest epoch; one opened with
    /// handed keys is kept open only for calls handed exactly those keys, until they run out.
    pub(super) async fn database(&self, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>, given: Option<Given<'_>>) -> Result<Arc<tokio::sync::Mutex<Db>>> {
        let own = given.is_none();
        let (keys, bucket, until, app_schedules) = match given {
            Some(g) => (g.keys, g.bucket.clone(), g.until, g.app_schedules),
            None => (self.database_keys(scope, app, database, true).await?, self.bucket()?, 0, false),
        };
        let schema = match schema {
            Some(schema) => schema,
            None => {
                let kind = match scope {
                    Scope::Global => "global",
                    Scope::Group(_) => "group",
                    Scope::User(_) => "user",
                };
                serde_json::from_value(self.0.apps.describe(app).await?["schema"][kind].clone()).unwrap_or_default()
            }
        };
        let id = (scope.clone(), format!("{app}/{database}"));
        let handed = (!own).then(|| self.opened_with(&id, &keys, until)).flatten();
        let cached = self.0.databases.dbs.lock().unwrap().get(&id).cloned();
        for db in handed.into_iter().chain(cached) {
            let mut open = db.lock().await;
            if open.opens_for(&keys) {
                // This device's own keys of a newer epoch, which it writes under from now on.
                if own {
                    open.add_keys(&keys);
                }
                if open.keys().last_key_value() == keys.last_key_value() {
                    // A deploy since it opened may have a schema change it hasn't had.
                    if open.version()? < schema.len() {
                        self.migrate_in(&mut open, &bucket, &schema).await?;
                    }
                    drop(open);
                    return Ok(db);
                }
            }
        }
        let prefix = crate::bucket::database_prefix(&scope.to_string(), app, database);
        let opening = Instant::now();
        let (mut keys, mut tries) = (keys, 0);
        let mut db = loop {
            tries += 1;
            let junk = self.0.databases.junk.lock().unwrap().get(&prefix).cloned().unwrap_or_default();
            let err = match Db::open_passing(&bucket, &prefix, keys.clone(), junk).await {
                Ok(db) => break db,
                Err(err) if tries > TRIES => return Err(err),
                Err(err) => err,
            };
            let new = match err.downcast_ref::<crate::database::stored::Ahead>() {
                Some(ahead) => self.judge_ahead(scope, &prefix, ahead).await?,
                // A step under an epoch this device hasn't looked for yet: it looks once.
                None => err.is::<crate::database::stored::Missing>() && tries == 1,
            };
            match new {
                false if err.is::<crate::database::stored::Ahead>() => {}
                true if own => {
                    self.forget_epochs(scope);
                    keys = self.database_keys(scope, app, database, true).await?;
                }
                _ => return Err(err),
            }
        };
        db.sign_as(self.signing_key());
        self.migrate_in(&mut db, &bucket, &schema).await?;
        trace::note(json!({ "step": "open", "app": app, "steps": db.at() }), opening.elapsed());
        let db = Arc::new(tokio::sync::Mutex::new(db));
        if !own {
            self.keep_opened(id, keys, until, app_schedules, db.clone());
            return Ok(db);
        }
        let mut dbs = self.0.databases.dbs.lock().unwrap();
        match dbs.get(&id) {
            Some(open) if open.try_lock().is_ok_and(|d| d.opens_for(&keys) && d.keys().last_key_value() == keys.last_key_value()) => Ok(open.clone()),
            _ => Ok(dbs.entry(id).insert_entry(db).get().clone()),
        }
    }

    /// Erases an app's data in every scope by deleting its keys: its databases stay in the bucket, which nothing opens
    /// again.
    pub(super) async fn destroy_keys(&self, app: &str) -> Result<()> {
        let bucket = self.bucket()?;
        self.close_dbs(|_, db| db.split('/').next() == Some(app));
        self.0.databases.database_keys.lock().unwrap().retain(|(_, db), _| db.split('/').next() != Some(app));
        for folder in bucket.folders()? {
            bucket.forget(&format!("{folder}/{app}"))?;
        }
        let (bucket, app) = (bucket.place, app.to_string());
        tokio::task::spawn_blocking(move || {
            for scope in bucket.list("")?.iter().map(|f| crate::bucket::scope_of(f)).filter(|s| Scope::parse(s).is_ok()) {
                let folder = crate::keys::app_keys_folder(&scope, &app);
                for n in bucket.list(&folder)? {
                    bucket.delete(&format!("{folder}/{n}"))?;
                }
            }
            Ok(())
        })
        .await?
    }
}

