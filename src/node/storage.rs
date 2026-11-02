//! Where the install keeps its data: its bucket. It starts as a folder on the server that made the install, which no
//! other machine reaches; an admin moves it to S3-compatible storage with a Storage item in Everyone's Vault, and each
//! server follows: it tries the place, copies there what it reaches of the old one, and keeps its data there from then on.

use super::*;

/// The Vault item type that says where the install keeps its data.
pub const STORAGE: &str = "Storage";

impl Node {
    /// Where this server keeps the install's data, as a person sees it.
    pub fn storage(&self) -> Result<Value> {
        self.with(|i| {
            Ok(match &i.bucket {
                Some((Place::Folder(path), _)) => json!({ "kind": "folder", "where": path.display().to_string(), "server": self.server_name() }),
                Some((Place::S3(s3), _)) => json!({ "kind": "s3", "where": s3.bucket, "endpoint": s3.endpoint }),
                None => json!({ "kind": null }),
            })
        })
    }

    /// Asks the person, for an admin, for the S3-compatible storage to move the install's data to: its bucket and key,
    /// typed into a sheet on their own device.
    pub async fn request_storage(&self, caller: &Caller, body: &Value) -> Result<Value> {
        let endpoint = body["endpoint"].as_str().map(str::trim).filter(|e| e.starts_with("https://") || e.starts_with("http://")).context("give the endpoint, like https://<account>.r2.cloudflarestorage.com")?;
        let probe: BTreeMap<String, String> = [("endpoint", endpoint), ("bucket", "-"), ("key_id", "-"), ("secret", "-")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Place::from_fields(&probe)?.can_hold_an_install()?;
        let host = endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("");
        let fields = json!({ "endpoint": { "secret": false }, "bucket": { "secret": false }, "key_id": { "secret": false }, "secret": { "secret": true } });
        let ask = json!({ "title": "Storage", "fields": fields, "hosts": [host] });
        let why = format!("The install's data moves to a bucket of its own at {endpoint}, sealed: its name, access key ID and secret key.");
        self.request_item(caller, &Scope::Global, members::APP, Some(STORAGE), &ask, &why).await
    }

    /// Moves the install's data, for an admin, to S3-compatible storage given with its key (`endpoint`, `bucket`,
    /// `region`, `key_id`, `secret`): tries it first, and makes it the install's Storage item if it works.
    pub async fn set_storage(&self, caller: &Caller, body: &Value) -> Result<Value> {
        self.with(|i| {
            ensure!(members::manages_scope(&i.r.members, &caller.user, &Scope::Global), "only admins move the install's data");
            Ok(())
        })?;
        let fields: BTreeMap<String, String> = ["endpoint", "bucket", "region", "key_id", "secret"]
            .into_iter()
            .filter_map(|k| Some((k.to_string(), body[k].as_str().map(str::trim).filter(|v| !v.is_empty())?.to_string())))
            .collect();
        let place = Place::from_fields(&fields)?;
        let Place::S3(s3) = &place else { bail!("the install's data moves to S3-compatible storage, given its endpoint") };
        place.can_hold_an_install()?;
        let host = s3.endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
        let tried = {
            let place = place.clone();
            tokio::task::spawn_blocking(move || place.try_it(true)).await?
        };
        let steps: Vec<Value> = tried.iter().map(|(step, error)| json!({ "step": step, "error": error })).collect();
        if tried.iter().any(|(_, e)| e.is_some()) {
            return Ok(json!({ "tried": steps }));
        }
        let item_fields: serde_json::Map<String, Value> = fields.iter().map(|(k, v)| (k.clone(), json!({ "value": v, "secret": k == "secret" }))).collect();
        let data = json!({ "title": "Storage", "type": STORAGE, "fields": item_fields, "hosts": [host], "apps": [members::APP] });
        // A server of an install made before its data was in a bucket keeps it there now, with its entries, which
        // takes a while: it runs no calls until it's done.
        if self.bucket().is_err() {
            let (node, caller) = (self.clone(), caller.clone());
            tokio::spawn(async move {
                let adopted = node.adopt(fields).await;
                if let Err(err) = adopted.and(node.set_item(&caller, &Scope::Global, data).await.map(|_| ())) {
                    tracing::warn!("moving the install's data: {err:#}");
                }
            });
            return Ok(json!({ "tried": steps, "moving": true }));
        }
        let id = self.set_item(caller, &Scope::Global, data).await?;
        Ok(json!({ "tried": steps, "id": id }))
    }

    /// Follows the install's newest Storage item, when this server keeps its data anywhere else: in the background.
    pub(super) fn follow_storage(&self) {
        let wanted = self.with(|i| {
            let Some((_, global)) = i.views.get("global") else { return Ok(None) };
            let Some(item) = global.items.values().filter(|it| it.kind == STORAGE && it.apps.contains(members::APP)).max_by_key(|it| it.made) else { return Ok(None) };
            let fields: BTreeMap<String, String> = item.fields.iter().map(|(n, f)| (n.clone(), f.value.clone())).collect();
            // Secrets stay put: the key goes only to the item's own hosts.
            if let Some(endpoint) = fields.get("endpoint") {
                let host = endpoint.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
                ensure!(item.hosts.contains(&host), "the Storage item's key is for {}, not {host}", item.hosts.join(", "));
            }
            Ok(Some(fields).filter(|f| i.bucket.as_ref().is_none_or(|(_, now)| now != f)))
        });
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

    /// Moves this server to the bucket `fields` says, once it works as an install's bucket must: copies there each
    /// object of the bucket it had that the new one lacks, then keeps the install's data there.
    async fn move_to(&self, fields: BTreeMap<String, String>) -> Result<()> {
        let to = Place::from_fields(&fields)?;
        let from = self.bucket().ok().map(|b| b.place);
        let root = self.with(|i| i.r.root.map(|r| r.to_hex()).context("no install"))?;
        let (place, theirs) = (to.clone(), root.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            place.can_hold_an_install()?;
            if let Some((step, Some(err))) = place.try_it(true).into_iter().find(|(_, e)| e.is_some()) {
                bail!("{step}: {err}");
            }
            if let Some(other) = place.get(INSTALL_OBJECT)?.filter(|r| r != theirs.as_bytes()) {
                bail!("that bucket holds another install's data ({})", String::from_utf8_lossy(&other));
            }
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
            i.bucket = Some((to, fields));
            Ok(())
        })?;
        // What it read of the old bucket stays true, but is read again from the new one.
        self.0.dbs.lock().unwrap().clear();
        tracing::info!("the install's data is kept in {} now", self.storage()?["where"].as_str().unwrap_or(""));
        self.emit(Event::Changed { apps: Vec::new() });
        Ok(())
    }

    /// Takes the bucket `fields` say as this server's own, for an install whose servers had none: puts every entry
    /// it holds there (another server may have put some already), so every server reads them from then on.
    async fn adopt(&self, fields: BTreeMap<String, String>) -> Result<()> {
        let place = Place::from_fields(&fields)?;
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
        let dir = self.install_dir();
        let mut file: InstallFile = serde_json::from_slice(&std::fs::read(dir.join("install.json"))?)?;
        file.bucket = Some(fields.clone());
        self.save_install(&file)?;
        let logs = self.with(|i| {
            i.bucket = Some((place, fields));
            i.read = false;
            Ok(i.r.store.logs()?)
        })?;
        let _write = self.0.writes.lock().await;
        for log in logs {
            let scope = Scope::parse(&log)?;
            let entries = self.with(|i| i.r.store.scope(&log))?;
            for entry in &entries {
                self.put_entry(&scope, entry).await?;
            }
            tracing::info!("put {} of {scope}'s entries in the bucket", entries.len());
        }
        self.with(|i| {
            i.read = true;
            Ok(())
        })
    }

    pub async fn storage_route(&self, me: &Caller, method: &str, path: &str, body: &Value) -> Result<Option<Value>> {
        let me = &Caller { agent: body["agent"].as_str().map(str::to_string).or(me.agent.clone()), ..me.clone() };
        Ok(Some(match (method, path) {
            ("GET", "/v1/storage") => self.storage()?,
            ("POST", "/v1/storage") => {
                ensure!(me.agent.is_none(), "agents never handle the storage's key: ask the person with request");
                self.set_storage(me, body).await?
            }
            ("POST", "/v1/storage/request") => self.request_storage(me, body).await?,
            _ => return Ok(None),
        }))
    }
}
