//! Databases: each scope's keys, and each app's databases in a scope, as this server reads and writes them in the bucket.

use super::*;

impl Node {
    /// The install's bucket, as this server reaches it, with its copy of the databases' steps.
    pub(super) fn bucket(&self) -> Result<Copied> {
        let place = self.with(|i| i.bucket.as_ref().map(|(p, _)| p.clone()).context("this device can't reach the install's bucket, where apps keep their data: use another device"))?;
        Ok(Copied::moving(place, self.install_dir().join("copy"), self.0.moved.clone()))
    }

    /// A database's key, `app`'s `database` in `scope`, as this server opens it with its own keys: from the app's key,
    /// in the bucket sealed under the scope's. A call that came with a token gets its keys from the vault instead
    /// (`sessions`); this is for this server's own work.
    pub(super) async fn database_key(&self, scope: &Scope, app: &str, database: &str) -> Result<[u8; 32]> {
        tracing::debug!("opening {scope}'s {app}/{database} with this server's own key");
        let scope_key = self.scope_key(scope).await?;
        let app_key = crate::db::app_key(&self.bucket()?, &scope_key, &scope.to_string(), app, database).await?;
        Ok(crate::kernel::database_key(&app_key, database))
    }

    /// A scope's key, which its databases and backups are sealed with. The first server to need one makes it, and the
    /// bucket says whose is the scope's (its id), so two made at once can't both seal its data. Each server that
    /// holds the scope finds it in the bucket sealed to its own key, as the recovery key does.
    pub(crate) async fn scope_key(&self, scope: &Scope) -> Result<[u8; 32]> {
        use crate::bucket::Bucket;
        if let Some(key) = self.0.scope_keys.lock().unwrap().get(scope) {
            return Ok(*key);
        }
        // One at a time here, so this server never makes two keys for a scope.
        let _making = self.0.making_keys.lock().await;
        if let Some(key) = self.0.scope_keys.lock().unwrap().get(scope) {
            return Ok(*key);
        }
        let bucket = self.bucket()?;
        let folder = crate::bucket::folder(&scope.to_string());
        let given = format!("{folder}/{KEYS}/{}", hex(&self.0.id));
        let key = match Bucket::get(&bucket, &given).await? {
            Some(o) => crate::backups::unseal(self.0.key.as_bytes(), &o.bytes)?,
            None => {
                // One its backups made before the bucket kept keys.
                let earlier = self.with(|i| Ok(i.views.get(&scope.to_string()).and_then(|(_, s)| s.backup_key)))?;
                let key = earlier.unwrap_or_else(rand::random);
                let id = crate::backups::key_id(&key);
                let name = format!("{folder}/{KEY_OBJECT}");
                if !crate::db::created(&bucket, &name, id.as_bytes()).await? {
                    // Another server made it just now: it gives it to this one next, once it's sealed it.
                    let mut found = None;
                    for _ in 0..50 {
                        found = Bucket::get(&bucket, &given).await?;
                        if found.is_some() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    match found {
                        Some(o) => crate::backups::unseal(self.0.key.as_bytes(), &o.bytes)?,
                        None => bail!("{} doesn't have {scope}'s key yet: a device that has it gives it when it's next on", self.server_name()),
                    }
                } else {
                    key
                }
            }
        };
        self.0.scope_keys.lock().unwrap().insert(scope.clone(), key);
        self.give_key(scope, &key).await?;
        Ok(key)
    }

    /// Seals a scope's key, in the bucket, to each server that holds the scope and to the recovery key, where it isn't
    /// yet.
    pub(super) async fn give_key(&self, scope: &Scope, key: &[u8; 32]) -> Result<()> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        let folder = crate::bucket::folder(&scope.to_string());
        let to = self.with(|i| {
            let m = &i.r.members;
            let mut to: Vec<(String, Key)> = m.servers().filter(|s| m.holds(&s.key, scope)).map(|s| (hex(&s.key), s.key)).collect();
            to.extend(m.recovery.map(|r| ("recovery".to_string(), r)));
            Ok(to)
        })?;
        for (name, public) in to {
            let name = format!("{folder}/{KEYS}/{name}");
            if Bucket::get(&bucket, &name).await?.is_none() {
                Bucket::create(&bucket, &name, &crate::backups::seal(&public, key)?).await?;
            }
        }
        Ok(())
    }

    /// Gives each scope's key this server has to the servers that hold the scope now.
    pub(super) fn give_keys(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let keys: Vec<(Scope, [u8; 32])> = node.0.scope_keys.lock().unwrap().iter().map(|(s, k)| (s.clone(), *k)).collect();
            for (scope, key) in keys {
                if let Err(err) = node.give_key(&scope, &key).await {
                    tracing::warn!("giving {scope}'s key: {err:#}");
                }
            }
        });
    }

    /// What a node that tells no other server what it wrote (the serverless one) said it wrote, as someone who may use
    /// `scope`: step `at` of `app`'s `database`, which this server takes as another server's word, and passes on to the
    /// others that hold the scope as if it wrote it.
    pub async fn written_elsewhere(&self, me: &Caller, scope: &Scope, app: &str, database: &str, at: u64) -> Result<()> {
        ensure!(self.with(|i| Ok(i.r.members.may_use(&me.user, scope)))?, "you can't use {scope}");
        self.changed_there(scope, Some((app, database, at)));
        self.ping(scope, Some((app, database, at)));
        Ok(())
    }

    /// Brings a database up to date for a call that reads it, as far as this server's copy goes: a notice of another
    /// server's write fetches its step into the copy, and `fill_copies` keeps up with the bucket. Only while a notice's
    /// step isn't here yet is the bucket asked first, and if it doesn't answer, the copy is read as it is: behind,
    /// never wrong.
    pub(super) async fn read_ready(&self, id: &(Scope, String), db: &mut Db, bucket: &Copied) -> Result<()> {
        if self.0.behind.lock().unwrap().remove(id) {
            match tokio::time::timeout(BUCKET_WAIT, db.catch_up(bucket)).await {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(err)) => tracing::debug!("{}: reading this server's copy, as the bucket said {err:#}", db.prefix()),
                Err(_) => tracing::debug!("{}: reading this server's copy, as the bucket didn't answer", db.prefix()),
            }
        }
        db.catch_up(&bucket.here_only()).await?;
        Ok(())
    }

    /// Fetches into this server's copy what the bucket has of each database open here that it lacks, every tick: what
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
    /// app's deploy's: with this server's own key, or `given`, a key the vault gave a call's session, with the bucket
    /// as that call reaches it.
    pub(super) async fn database(&self, scope: &Scope, app: &str, database: &str, schema: Option<Vec<String>>, given: Option<([u8; 32], &Copied)>) -> Result<Arc<tokio::sync::Mutex<Db>>> {
        let id = (scope.clone(), format!("{app}/{database}"));
        let cached = self.0.dbs.lock().unwrap().get(&id).cloned();
        if let Some(db) = cached {
            match given {
                None => return Ok(db),
                Some((key, _)) if db.lock().await.opens_with(&key) => return Ok(db),
                Some(_) => {}
            }
        }
        let (key, bucket) = match given {
            Some((key, bucket)) => (key, bucket.clone()),
            None => (self.database_key(scope, app, database).await?, self.bucket()?),
        };
        let mut db = Db::open(&bucket, &format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string())), key).await?;
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
            Some(open) if open.try_lock().is_ok_and(|d| d.opens_with(&key)) => Ok(open.clone()),
            _ => Ok(dbs.entry(id).insert_entry(db).get().clone()),
        }
    }

    /// Erases an app's data in every scope by deleting its keys: its databases stay in the bucket, which nothing opens
    /// again.
    pub(super) async fn destroy_keys(&self, app: &str) -> Result<()> {
        let bucket = self.bucket()?;
        self.0.dbs.lock().unwrap().retain(|(_, db), _| db.split('/').next() != Some(app));
        for folder in bucket.folders()? {
            bucket.forget(&format!("{folder}/{app}"))?;
        }
        let (bucket, app) = (bucket.place, app.to_string());
        tokio::task::spawn_blocking(move || {
            for scope in bucket.list("")?.iter().map(|f| crate::bucket::scope_of(f)).filter(|s| Scope::parse(s).is_ok()) {
                bucket.delete(&crate::kernel::app_key_name(&scope, &app))?;
            }
            Ok(())
        })
        .await?
    }
}
