//! Where the install keeps its data: its bucket. It starts as a folder on the device that made the install, which no
//! other machine reaches; an admin moves it to S3-compatible storage, whose credentials only the kernel's URL signer
//! holds, with the `storage` setting (an entry only an admin's device writes) that says only where it is, and each
//! device follows: it reaches it through the URL signer, copies there what it reaches of the old one, and keeps its
//! data there from then on. A folder is only ever a machine's own choice, never taken from what others write.

use super::*;

/// The setting that says where the install keeps its data: S3's `endpoint`, `bucket` and `region`, as JSON.
pub const STORAGE: &str = "storage";

/// The install's bucket as its `storage` setting says, if it has one: S3's address alone, never a folder's path.
fn storage_setting(m: &ScopeDb) -> Result<Option<BTreeMap<String, String>>> {
    let Some(value) = m.setting(STORAGE) else { return Ok(None) };
    let fields: BTreeMap<String, String> = serde_json::from_str(&value).context("the storage setting isn't an S3 address")?;
    ensure!(fields.keys().all(|k| ["endpoint", "bucket", "region"].contains(&k.as_str())), "the storage setting names only S3's endpoint, bucket and region");
    ensure!(fields.contains_key("endpoint") && fields.contains_key("bucket"), "the storage setting names S3's endpoint and bucket");
    Ok(Some(fields))
}

impl Node {
    /// Where this device keeps the install's data, as a person sees it.
    pub fn storage(&self) -> Result<Value> {
        self.with(|i| {
            Ok(match &i.bucket {
                Some((Place::Folder(path), _)) => json!({ "kind": "folder", "where": path.display().to_string(), "device": self.device_name() }),
                Some((Place::S3(s3), _)) => json!({ "kind": "s3", "where": s3.bucket, "endpoint": s3.endpoint }),
                None => json!({ "kind": null }),
            })
        })
    }

    /// The install's bucket from the fields this device keeps of it: a folder here, or S3 through the URL signer at the
    /// install's address, signing with this device's key.
    /// A device that joined keeps the address it joined at with them (`address`), until it has read the install's.
    pub(super) fn install_place(&self, fields: &BTreeMap<String, String>) -> Result<Place> {
        let node = Arc::downgrade(&self.0);
        let joined = fields.get("address").cloned();
        let address: crate::bucket::Address = Arc::new(move || {
            let node = Node(node.upgrade().context("this device stopped")?);
            node.with(|i| Ok(i.r.db().setting("address")))?.or_else(|| joined.clone()).context("your One App has no address yet, where its bucket is reached: an admin sets one with kernel use")
        });
        Place::install(fields, address, ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes()))
    }

    /// Moves the install's data, for an admin, to S3-compatible storage (`endpoint`, `bucket`, `region`), whose key the
    /// kernel's URL signer holds: reaches it through the signer first, and makes it the install's `storage` setting if
    /// it can.
    pub async fn set_storage(&self, caller: &Caller, body: &Value) -> Result<Value> {
        self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "change-settings", scopes::INSTALL), "only admins move the install's data");
            Ok(())
        })?;
        let fields: BTreeMap<String, String> = ["endpoint", "bucket", "region"]
            .into_iter()
            .filter_map(|k| Some((k.to_string(), body[k].as_str().map(str::trim).filter(|v| !v.is_empty())?.to_string())))
            .collect();
        let place = self.install_place(&fields)?;
        let Place::S3(_) = &place else { bail!("the install's data moves to S3-compatible storage, given its endpoint") };
        place.can_hold_an_install()?;
        let reached = {
            let place = place.clone();
            tokio::task::spawn_blocking(move || place.get(INSTALL_OBJECT)).await?
        };
        let steps = vec![json!({ "step": "Reached it through the URL signer", "error": reached.as_ref().err().map(|e| format!("{e:#}")) })];
        if reached.is_err() {
            return Ok(json!({ "tried": steps }));
        }
        let setting = serde_json::to_string(&fields)?;
        // A device of an install made before its data was in a bucket keeps it there now, with its entries, which
        // takes a while: it runs no calls until it's done.
        if self.bucket().is_err() {
            let (node, caller) = (self.clone(), caller.clone());
            tokio::spawn(async move {
                let adopted = node.adopt(fields).await;
                if let Err(err) = adopted.and(node.set_setting(&caller, STORAGE, Some(&setting)).await) {
                    tracing::warn!("moving the install's data: {err:#}");
                }
            });
            return Ok(json!({ "tried": steps, "moving": true }));
        }
        self.set_setting(caller, STORAGE, Some(&setting)).await?;
        Ok(json!({ "tried": steps, "kept": true }))
    }

    /// Follows the install's `storage` setting, when this device keeps its data anywhere else: in the background.
    pub(super) fn follow_storage(&self) {
        let wanted = self.with(|i| Ok(storage_setting(i.r.db())?.filter(|f| i.bucket.as_ref().is_none_or(|(_, now)| now.iter().filter(|(k, _)| k.as_str() != "address").ne(f.iter())))));
        let fields = match wanted {
            Ok(Some(fields)) => fields,
            Ok(None) => return,
            Err(err) => return tracing::warn!("storage: {err:#}"),
        };
        if std::mem::replace(&mut *self.0.moving.lock().unwrap(), true) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.move_to(fields).await {
                tracing::warn!("moving the install's data: {err:#}");
            }
            *node.0.moving.lock().unwrap() = false;
        });
    }

    /// Moves this device to the bucket `fields` says, once it reaches it: copies there each object of the bucket it had
    /// that the new one lacks, then keeps the install's data there.
    async fn move_to(&self, fields: BTreeMap<String, String>) -> Result<()> {
        let to = self.install_place(&fields)?;
        let from = self.bucket().ok().map(|b| b.place);
        let root = self.with(|i| i.r.root.map(|r| r.to_hex()).context("no install"))?;
        let (place, theirs) = (to.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            place.can_hold_an_install()?;
            if let Some(other) = place.get(INSTALL_OBJECT)?.filter(|r| r != theirs.as_bytes()) {
                bail!("that bucket holds another install's data ({})", String::from_utf8_lossy(&other));
            }
            Ok(())
        })
        .await??;
        // Writes wait while it copies, then go to the new bucket, so none is left only in the old one.
        let mut moved = self.0.moved.write().await;
        let (place, theirs) = (to.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(from) = from.filter(|f| f.present()) {
                for name in from.names()? {
                    if place.get(&name)?.is_none() {
                        if let Some(bytes) = from.get(&name)? {
                            place.create(&name, &bytes)?;
                        }
                    }
                }
            }
            place.create(INSTALL_OBJECT, theirs.as_bytes())?;
            Ok(())
        })
        .await??;
        let dir = self.install_dir();
        let mut file: InstallFile = serde_json::from_slice(&std::fs::read(dir.join("install.json"))?)?;
        file.bucket = Some(fields.clone());
        self.save_install(&file)?;
        self.with(|i| {
            i.bucket = Some((to.clone(), fields));
            Ok(())
        })?;
        *moved = Some(to);
        drop(moved);
        // What it read of the old bucket stays true, but is read again from the new one.
        self.close_dbs(|_, _| true);
        tracing::info!("the install's data is kept in {} now", self.storage()?["where"].as_str().unwrap_or(""));
        self.emit(Event::Changed { apps: Vec::new() });
        Ok(())
    }

    /// Takes the bucket `fields` say as this device's own, for an install whose devices had none: puts every entry
    /// it holds there (another device may have put some already), so every device reads them from then on.
    async fn adopt(&self, fields: BTreeMap<String, String>) -> Result<()> {
        let place = self.install_place(&fields)?;
        let root = self.with(|i| i.r.root.map(|r| r.to_hex()).context("no install"))?;
        let (p, theirs) = (place.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(other) = p.get(INSTALL_OBJECT)?.filter(|r| r != theirs.as_bytes()) {
                bail!("that bucket holds another install's data ({})", String::from_utf8_lossy(&other));
            }
            p.create(INSTALL_OBJECT, theirs.as_bytes())?;
            Ok(())
        })
        .await??;
        let logs = self.with(|i| {
            i.bucket = Some((place, fields.clone()));
            i.read = false;
            Ok(i.r.store.logs()?)
        })?;
        let _write = self.0.writes.lock().await;
        let put = async {
            for log in logs {
                let scope = Scope::parse(&log)?;
                let entries = self.with(|i| i.r.store.scope(&log))?;
                for entry in &entries {
                    self.put_entry(&scope, entry).await?;
                }
                tracing::info!("put {} of {scope}'s entries in the bucket", entries.len());
            }
            anyhow::Ok(())
        };
        if let Err(err) = put.await {
            // Back as it was, with no bucket, for moving again to finish.
            self.with(|i| {
                i.bucket = None;
                i.read = true;
                Ok(())
            })?;
            return Err(err);
        }
        // Kept as this device's bucket only once its entries are there: stopped before, it starts with none again, and
        // moving again puts what's missing.
        let dir = self.install_dir();
        let mut file: InstallFile = serde_json::from_slice(&std::fs::read(dir.join("install.json"))?)?;
        file.bucket = Some(fields);
        self.save_install(&file)?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })
    }

    pub async fn storage_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        Ok(Some(match (method, path) {
            ("GET", "/v1/storage") => self.storage()?,
            ("POST", "/v1/storage") => self.set_storage(me, body).await?,
            _ => return Ok(None),
        }))
    }
}
