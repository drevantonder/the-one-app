//! This device's network as its home keeps it: whose it is and its bucket (found by the object at its root); making
//! a network, and joining one with a pairing code (`onboarding_runtime` makes the code and admits who comes with it).

use super::*;

impl Host {
    /// Joins the network a pairing code is from → that network's address.
    pub async fn join(&self, code: &str) -> Result<String> {
        self.ensure_no_network()?;
        let joined = ::onboarding::join::join_with(&self.0.key, &self.device_name(), code).await?;
        self.save_network(&joined.network)?;
        self.joined()?;
        Ok(joined.address)
    }

    /// Opens the network its home names now that it's joined it: by itself (`join`), or on a computer, where its onet
    /// joins as the device, as the app runtime asked by it (`/api/joined`). Open already, it's nothing.
    pub fn joined(&self) -> Result<()> {
        if self.has_network() {
            return Ok(());
        }
        self.open_network()?;
        let host = self.clone();
        tokio::spawn(async move {
            if let Err(err) = host.read_scopes(None).await {
                tracing::debug!("reading the bucket after joining: {err:#}");
            }
        });
        self.emit(Event::Network);
        Ok(())
    }

    pub(super) fn network_dir(&self) -> PathBuf {
        self.0.dir.join(NETWORK_DIR)
    }

    /// The network's bucket from the fields this device keeps of it: a folder here, or S3 through the URL signer at the
    /// network's address, signing with this device's key.
    /// A device that joined keeps the address it joined at with them (`address`), until it has read the network's.
    pub(super) fn network_place(&self, fields: &BTreeMap<String, String>) -> Result<Place> {
        let host = Arc::downgrade(&self.0);
        let joined = fields.get("address").cloned();
        let address: bucket::Address = Arc::new(move || {
            let host = Host(host.upgrade().context("this device stopped")?);
            host.with(|i| Ok(i.r.db().setting("address")?))?.or_else(|| joined.clone()).with_context(|| format!("{YOURS} has no address yet, where its bucket is reached: an admin sets it with cloudflare use"))
        });
        Place::network(fields, address, ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes()))
    }

    /// Keeps the network's data in `to` from now on, once `copy` is done: writes wait while it copies, then go to the
    /// new bucket, so none is left only in the old one.
    pub(super) async fn keep_in(&self, to: Place, fields: BTreeMap<String, String>, copy: ::api::runtime::Fut<'_, ()>) -> Result<()> {
        let mut moved = self.0.moved.write().await;
        copy.await?;
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(self.network_dir().join(NETWORK_FILE))?)?;
        file.bucket = Some(fields.clone());
        self.save_network(&file)?;
        self.with(|i| {
            i.bucket = Some((to.clone(), fields));
            Ok(())
        })?;
        *moved = Some(to);
        drop(moved);
        // What it read of the old bucket stays true, but is read again from the new one.
        self.close_dbs(|_, _| true);
        Ok(())
    }

    pub(super) fn open_network(&self) -> Result<()> {
        let dir = self.network_dir();
        let file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        let root: [u8; 32] = data_encoding::HEXLOWER.decode(file.root.as_bytes())?.try_into().map_err(|_| anyhow!("bad root"))?;
        let r = Replica::new(self.0.key.clone(), Some(Hash::from_bytes(root)), now_ms());
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((self.network_place(&f)?, f))).transpose()?;
        let network = Network { r, user: file.user, bucket, said: HashMap::new(), exchanged: HashMap::new(), described: HashMap::new(), stated: HashMap::new(), parts: HashMap::new(), last_seen: HashMap::new(), versions: serde_json::Value::Null, workers: serde_json::Value::Null, moved: None, connected: Default::default(), scope_dbs: HashMap::new(), views: HashMap::new(), nearest: HashMap::new(), far: HashMap::new(), read: false };
        *self.0.network.lock()? = Some(network);
        self.start_kept();
        Ok(())
    }

    /// A network made before it kept its data in a bucket, with no device but this one: it keeps it in a folder here.
    pub(super) fn keep_alone_here(&self) -> Result<()> {
        let alone = self.with(|i| Ok(i.bucket.is_none() && i.r.db().computes()?.iter().all(|s| s.key == self.0.id)))?;
        if !alone {
            return Ok(());
        }
        let dir = self.network_dir();
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        let fields: BTreeMap<String, String> = [("path".to_string(), dir.join("bucket").display().to_string())].into();
        let place = Place::from_fields(&fields)?;
        place.create(NETWORK_OBJECT, file.root.as_bytes())?;
        file.bucket = Some(fields.clone());
        self.save_network(&file)?;
        self.with(|i| {
            i.bucket = Some((place, fields));
            Ok(())
        })
    }

    pub(super) fn save_network(&self, file: &NetworkFile) -> Result<()> {
        file.write(&self.0.dir)
    }

    /// Creates the network, with every built-in app added (owned by admins), so anyone may add one to their own page,
    /// and these added to `global`. Its first user is `user` if given (who they were in a network this one
    /// replaces, so their account at its sign-in is still theirs), else someone new. Who's who is its scope database from
    /// its first freeze on, which its new recovery key signs as it starts (`steps::first_freeze`).
    pub async fn create_with(&self, name: &str, user_name: &str, user: Option<&str>, apps: &[&str]) -> Result<String> {
        self.ensure_no_network()?;
        ensure!(!name.trim().is_empty() && !user_name.trim().is_empty(), "give the network a name, and yourself one");
        ensure!(user.is_none_or(|u| u.len() == 16 && u.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))), "a user's id is 16 hex digits, like 976091fe8c23a057");
        let user = user.map_or_else(new_id, str::to_string);
        let recovery = SigningKey::from_bytes(&rand::random());
        // Its first device is the device this app runtime is, or the runtime of, whose onet then lists that runtime.
        let me = key::Endpoint { key: self.0.holds.device(), seal: key::Opens::public(&self.0.holds) };
        // The network's id, which every device of it pins, and `global`'s first epoch, named by its first freeze before
        // anything is sealed under it.
        let root = Hash::from_bytes(rand::random());
        let global: [u8; 32] = rand::random();
        let start = content::first(name.trim(), &user, user_name.trim(), &me, &self.device_name(), onboarding_runtime::MACHINE, recovery.verifying_key().as_bytes(), &key::epoch_hash(&global));
        // Until it's moved to S3-compatible storage, the network's bucket is a folder here.
        let bucket: BTreeMap<String, String> = [("path".to_string(), self.network_dir().join("bucket").display().to_string())].into();
        Place::from_fields(&bucket)?.create(NETWORK_OBJECT, root.to_hex().as_bytes())?;
        self.save_network(&NetworkFile { root: root.to_hex(), user: user.clone(), bucket: Some(bucket) })?;
        self.open_network()?;
        let bucket = self.bucket()?;
        let recovery_seal = key::twin_public(recovery.verifying_key().as_bytes())?;
        for (to, seal) in [("recovery".to_string(), recovery_seal), (hex(&me.key), me.seal)] {
            database::stored::created(&bucket, &database::keys::epoch_name("global", 1, &to), &key::seal_to(&seal, &global)?).await?;
        }
        self.0.databases.epochs.lock().unwrap().insert(Scope::Global, database::stored::Keys::from([(1, global)]));
        // Who's who, as the freeze its recovery key signs at step 1 of its scope database.
        let keys = self.step_keys(&Scope::Global).await?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })?;
        let first = scope::steps::first_freeze(&recovery, root, &start, &user, now_ms())?;
        let name = bucket::names::step_name(&scope::steps::prefix("global"), 1);
        let sealed = scope::steps::seal(&keys, &name, &scope::steps::encode(&first.step, first.copy.as_deref()))?;
        ensure!(bucket::Bucket::create(&bucket, &name, &sealed).await?, "the new network's bucket isn't empty");
        self.read_steps().await?;
        ensure!(self.frozen(), "the new network's freeze didn't count");
        let me = Caller::own(self.0.id, user);
        // Its Vault and its first user's own open on this machine, so a new network fills items with no other step.
        for scope in [Scope::Global, Scope::User(me.user.clone())] {
            scope::settings::new_vault_key(self, &scope, 1, &[self.0.holds.device()]).await?;
        }
        // Written first, in one write by whatever holds the device's key, and run together: as each is written, settling
        // would run them all again, holding up the next.
        let settling = self.0.settling.lock().await;
        let mut steps = Vec::new();
        for (app, _) in crate::built_in::BUILT_IN {
            steps.extend(self.built_in_steps(&me, app)?);
        }
        steps.extend(apps.iter().map(|app| content::added(app, true)));
        self.write_steps(&me.user, steps).await?;
        drop(settling);
        self.settle().await?;
        self.emit(Event::Network);
        Ok(hex(&recovery.to_bytes()))
    }
}
