//! Databases: each scope's epochs this device opens, and each app's databases in a scope, as this device reads and
//! writes them in the bucket.

use super::*;

/// A scope this device wasn't given the newest key of: a device of someone who opens it seals it to this one.
#[derive(Debug)]
pub struct NotGiven(pub Scope, pub String);

impl std::fmt::Display for NotGiven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} wasn't given {}'s newest key: a device of someone who opens it gives it when it's next on", self.1, self.0)
    }
}

impl std::error::Error for NotGiven {}

impl Node {
    /// The install's bucket, as this device reaches it, with its copy of the databases' steps.
    pub(super) fn bucket(&self) -> Result<Copied> {
        let place = self.with(|i| i.bucket.as_ref().map(|(p, _)| p.clone()).context("this device can't reach the install's bucket, where apps keep their data: use another device"))?;
        Ok(Copied::moving(place, self.install_dir().join("copy"), self.0.moved.clone()))
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
    pub(super) async fn database_keys(&self, scope: &Scope, app: &str, database: &str) -> Result<crate::db::Keys> {
        let epochs = self.epoch_keys(scope).await?;
        let newest = epochs.keys().next_back().copied().unwrap_or(0);
        let id = (scope.clone(), format!("{app}/{database}"));
        if let Some((n, keys)) = self.0.database_keys.lock().unwrap().get(&id)
            && *n == newest
        {
            return Ok(keys.clone());
        }
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let mut out = crate::db::Keys::new();
        for (n, epoch) in &epochs {
            if let Some(app_key) = crate::db::app_key(&bucket, epoch, &name, app, *n, *n == newest).await? {
                out.insert(*n, crate::kernel::database_key(&app_key, database));
            }
        }
        self.0.database_keys.lock().unwrap().insert(id, (newest, out.clone()));
        Ok(out)
    }

    /// The epochs of a scope this device opens, by number: those sealed to it in the bucket (`keys::epoch_name`), up to
    /// the scope's newest, which it must be one of. A scope with no epoch yet gets its first here, if this device's person
    /// may make it: an admin of `global` or of the group, or the person whose scope it is.
    pub(crate) async fn epoch_keys(&self, scope: &Scope) -> Result<crate::db::Keys> {
        let known = self.with(|i| Ok(i.r.members.epoch(scope)))?;
        let fresh = |kept: &HashMap<Scope, crate::db::Keys>| kept.get(scope).filter(|k| k.keys().next_back().is_some_and(|n| *n >= known)).cloned();
        if let Some(keys) = fresh(&self.0.epochs.lock().unwrap()) {
            return Ok(keys);
        }
        // One at a time here, so this device never makes two first epochs of a scope.
        let _making = self.0.making_keys.lock().await;
        if let Some(keys) = fresh(&self.0.epochs.lock().unwrap()) {
            return Ok(keys);
        }
        let bucket = self.bucket()?;
        let (me, secret, name) = (hex(&self.0.id), self.seal_secret(), scope.to_string());
        let mut keys = self.0.epochs.lock().unwrap().get(scope).cloned().unwrap_or_default();
        let mut n = keys.keys().next_back().map_or(1, |n| n + 1);
        let mut newest = n - 1;
        let mut waited = 0;
        loop {
            if let Some(o) = crate::bucket::Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, &me)).await? {
                let key = crate::kernel::open_sealed(&secret, &o.bytes).with_context(|| format!("{scope}'s epoch {n} sealed to {} doesn't open", self.device_name()))?;
                keys.insert(n, key.try_into().map_err(|_| anyhow!("an epoch's key is 32 bytes"))?);
                (newest, n) = (n, n + 1);
                continue;
            }
            // An epoch sealed to others: there's always a copy for the recovery key, sealed first.
            if crate::bucket::Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, "recovery")).await?.is_some() {
                // One made just now, by a device that seals it to this one next.
                if keys.is_empty() && n == 1 && waited < 50 {
                    waited += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
                (newest, n) = (n, n + 1);
                continue;
            }
            break;
        }
        if newest == 0
            && let Some(key) = self.make_first_epoch(scope).await?
        {
            (keys, newest) = ([(1, key)].into(), 1);
        }
        ensure!(keys.keys().next_back().is_some_and(|n| *n >= newest.max(known)), NotGiven(scope.clone(), self.device_name()));
        self.0.epochs.lock().unwrap().insert(scope.clone(), keys.clone());
        Ok(keys)
    }

    /// Forgets the epochs this device knows of a scope, so the next use looks in the bucket for newer ones: after a
    /// step under one it wasn't given.
    pub(super) fn forget_epochs(&self, scope: &Scope) {
        self.0.epochs.lock().unwrap().remove(scope);
        self.0.database_keys.lock().unwrap().retain(|(s, _), _| s != scope);
    }

    /// A scope's first epoch, made here: a random key, sealed to the recovery key (first, which says the epoch is
    /// made, so two devices can't both make it) and to each device that holds the scope, then an `epoch` entry in
    /// `global`. None when this device's person may not make it, or another device made it first.
    async fn make_first_epoch(&self, scope: &Scope) -> Result<Option<[u8; 32]>> {
        let me = self.endpoint();
        let (may, recovery, to) = self.with(|i| {
            let m = &i.r.members;
            let mut to: Vec<(Key, [u8; 32])> = vec![(me.key, me.seal)];
            to.extend(m.computes().filter(|d| d.key != me.key && m.runs_for(&d.key, scope)).filter_map(|d| Some((d.key, m.seal_key(&d.key)?))));
            Ok((m.role_in(&i.user, scope) == Some(Role::Admin), m.recovery, to))
        })?;
        if !may {
            return Ok(None);
        }
        let recovery = recovery.context("the install has no recovery key to seal its keys to")?;
        let key: [u8; 32] = rand::random();
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let sealed = crate::kernel::seal_to(&crate::kernel::twin_public(&recovery)?, &key)?;
        if !crate::db::created(&bucket, &crate::keys::epoch_name(&name, 1, "recovery"), &sealed).await? {
            return Ok(None);
        }
        for (device, seal) in &to {
            crate::bucket::Bucket::create(&bucket, &crate::keys::epoch_name(&name, 1, &hex(device)), &crate::kernel::seal_to(seal, &key)?).await?;
        }
        tracing::info!("made {scope}'s first epoch");
        // Written after: writing it needs `global`'s keys, which may be what's being made.
        let to: Vec<String> = to.iter().map(|(k, _)| hex(k)).collect();
        self.0.epoch_entries.lock().unwrap().push(members::node("epoch", json!({ "scope": name, "n": 1, "to": to, "hash": crate::keys::epoch_hash(&key) })));
        Ok(Some(key))
    }

    /// Writes the `epoch` entries of the epochs this device made, in `global`.
    pub(super) async fn write_epoch_entries(&self) {
        let made = std::mem::take(&mut *self.0.epoch_entries.lock().unwrap());
        for (i, entry) in made.iter().enumerate() {
            if let Err(err) = self.append(&Scope::Global, entry.clone()).await {
                tracing::warn!("writing an epoch's entry, to try again: {err:#}");
                self.0.epoch_entries.lock().unwrap().extend(made[i..].iter().cloned());
                return;
            }
        }
    }

    /// Seals the newest epoch of each scope this device opens to each device that holds the scope and lacks it: a
    /// device added since it was made.
    pub(super) fn seal_epochs(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let known: Vec<(Scope, u32, [u8; 32])> = node.0.epochs.lock().unwrap().iter().filter_map(|(s, k)| k.last_key_value().map(|(n, k)| (s.clone(), *n, *k))).collect();
            for (scope, n, key) in known {
                if let Err(err) = node.seal_epoch(&scope, n, &key).await {
                    tracing::warn!("sealing {scope}'s epoch {n}: {err:#}");
                }
            }
        });
    }

    async fn seal_epoch(&self, scope: &Scope, n: u32, key: &[u8; 32]) -> Result<()> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        let to = self.with(|i| {
            let m = &i.r.members;
            Ok(m.computes().filter(|d| d.key != self.0.id && m.runs_for(&d.key, scope)).filter_map(|d| Some((d.key, m.seal_key(&d.key)?))).collect::<Vec<_>>())
        })?;
        for (device, seal) in to {
            let name = crate::keys::epoch_name(&scope.to_string(), n, &hex(&device));
            if Bucket::get(&bucket, &name).await?.is_none() {
                Bucket::create(&bucket, &name, &crate::kernel::seal_to(&seal, key)?).await?;
            }
        }
        Ok(())
    }

    /// Brings a database up to date for a call that reads it, as far as this device's copy goes: a notice of another
    /// device's write fetches its step into the copy, and `fill_copies` keeps up with the bucket. Only while a notice's
    /// step isn't here yet is the bucket asked first, and if it doesn't answer, the copy is read as it is: behind,
    /// never wrong.
    pub(super) async fn read_ready(&self, id: &(Scope, String), db: &mut Db, bucket: &Copied) -> Result<()> {
        if self.0.behind.lock().unwrap().remove(id) {
            match tokio::time::timeout(BUCKET_WAIT, db.catch_up(bucket)).await {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(err)) => tracing::debug!("{}: reading this device's copy, as the bucket said {err:#}", db.prefix()),
                Err(_) => tracing::debug!("{}: reading this device's copy, as the bucket didn't answer", db.prefix()),
            }
        }
        db.catch_up(&bucket.here_only()).await?;
        Ok(())
    }

    /// Fetches into this device's copy what the bucket has of each database open here that it lacks, every tick: what
    /// a lost notice would have said. A database a call is using is left for the next.
    pub(super) async fn fill_copies(&self) {
        let Ok(bucket) = self.bucket() else { return };
        let mut changed = Vec::new();
        let open: Vec<_> = self.0.dbs.lock().unwrap().iter().map(|(id, db)| (id.clone(), db.clone())).collect();
        for ((_, name), db) in open {
            let Ok((prefix, at)) = db.try_lock().map(|db| (db.prefix().to_string(), db.at())) else { continue };
            match bucket.fill(&prefix, at).await {
                Ok(0) => {}
                Ok(_) => changed.extend(name.split('/').next().filter(|app| *app != members::APP).map(str::to_string)),
                Err(err) => tracing::debug!("{prefix}: {err:#}"),
            }
        }
        if !changed.is_empty() {
            self.emit(Event::Changed { apps: changed });
        }
    }

    /// `app`'s `database` of `scope`, opened from the bucket the first time and brought to `schema`, or else to the
    /// app's deploy's: with this device's own keys, or `given`, keys a call was handed, with the bucket as that call
    /// reaches it. Either way, one already open is used only if the keys open everything it read and it writes under
    /// the same newest epoch.
    pub(super) async fn database(&self, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>, given: Option<(crate::db::Keys, &Copied)>) -> Result<Arc<tokio::sync::Mutex<Db>>> {
        let own = given.is_none();
        let (keys, bucket) = match given {
            Some((keys, bucket)) => (keys, bucket.clone()),
            None => (self.database_keys(scope, app, database).await?, self.bucket()?),
        };
        let id = (scope.clone(), format!("{app}/{database}"));
        let cached = self.0.dbs.lock().unwrap().get(&id).cloned();
        if let Some(db) = cached {
            let mut open = db.lock().await;
            if open.opens_for(&keys) {
                // This device's own keys of a newer epoch, which it writes under from now on.
                if own {
                    open.add_keys(&keys);
                }
                if open.keys().last_key_value() == keys.last_key_value() {
                    drop(open);
                    return Ok(db);
                }
            }
        }
        let prefix = format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()));
        let mut db = match Db::open(&bucket, &prefix, keys.clone()).await {
            // A step under an epoch this device hasn't looked for yet: it looks once.
            Err(err) if own && err.is::<crate::db::Missing>() => {
                self.forget_epochs(scope);
                Db::open(&bucket, &prefix, self.database_keys(scope, app, database).await?).await?
            }
            other => other?,
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
        db.migrate(&bucket, &schema).await?;
        let db = Arc::new(tokio::sync::Mutex::new(db));
        let mut dbs = self.0.dbs.lock().unwrap();
        match dbs.get(&id) {
            Some(open) if open.try_lock().is_ok_and(|d| d.opens_for(&keys) && d.keys().last_key_value() == keys.last_key_value()) => Ok(open.clone()),
            _ => Ok(dbs.entry(id).insert_entry(db).get().clone()),
        }
    }

    /// Erases an app's data in every scope by deleting its keys: its databases stay in the bucket, which nothing opens
    /// again.
    pub(super) async fn destroy_keys(&self, app: &str) -> Result<()> {
        let bucket = self.bucket()?;
        self.0.dbs.lock().unwrap().retain(|(_, db), _| db.split('/').next() != Some(app));
        self.0.database_keys.lock().unwrap().retain(|(_, db), _| db.split('/').next() != Some(app));
        for folder in bucket.folders()? {
            bucket.forget(&format!("{folder}/{app}"))?;
        }
        let (bucket, app) = (bucket.place, app.to_string());
        tokio::task::spawn_blocking(move || {
            for scope in bucket.list("")?.iter().map(|f| crate::bucket::scope_of(f)).filter(|s| Scope::parse(s).is_ok()) {
                let folder = format!("{}/{app}/_key", crate::bucket::folder(&scope));
                for n in bucket.list(&folder)? {
                    bucket.delete(&format!("{folder}/{n}"))?;
                }
            }
            Ok(())
        })
        .await?
    }
}
