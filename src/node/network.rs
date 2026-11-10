//! This device's network as its home keeps it: whose it is, its bucket (found by the object at its root), and the
//! folders it backs up to; and making a network.

use super::*;

/// The object in a network's bucket that says whose it is: the network's root.
pub(super) const NETWORK_OBJECT: &str = "network";

/// Whose the bucket at `place` is, as its `NETWORK_OBJECT` says: or as `install` says, the name a build from before it
/// was called a network gave it (a bucket then, or a backup of one put back), which it puts under the new name too.
pub(super) fn bucket_root(place: &Place) -> Result<Option<Vec<u8>>> {
    if let Some(root) = place.get(NETWORK_OBJECT)? {
        return Ok(Some(root));
    }
    let old = place.get("install")?;
    if let Some(root) = &old {
        place.create(NETWORK_OBJECT, root)?;
    }
    Ok(old)
}

pub(super) const NETWORK_DIR: &str = "network";

pub(super) const NETWORK_FILE: &str = "network.json";

#[derive(Serialize, Deserialize)]
pub(super) struct NetworkFile {
    pub(super) root: String,
    pub(super) user: String,
    /// The network's bucket, as a place's fields: a folder's `path`, or S3's. None if this device can't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) bucket: Option<BTreeMap<String, String>>,
    /// The folders on this device it backs up to, by their place's id: a folder is only ever this device's own choice.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) folders: BTreeMap<String, String>,
}

impl Node {
    pub(super) fn network_dir(&self) -> PathBuf {
        self.0.dir.join(NETWORK_DIR)
    }

    pub(super) fn open_network(&self) -> Result<()> {
        let dir = self.network_dir();
        let file: NetworkFile = serde_json::from_slice(&std::fs::read(dir.join(NETWORK_FILE))?)?;
        let root: [u8; 32] = data_encoding::HEXLOWER.decode(file.root.as_bytes())?.try_into().map_err(|_| anyhow!("bad root"))?;
        let r = Replica::new(self.0.key.clone(), Some(Hash::from_bytes(root)), now_ms());
        let bucket = file.bucket.map(|f| Ok::<_, anyhow::Error>((self.network_place(&f)?, f))).transpose()?;
        let network = Network { r, user: file.user, bucket, folders: file.folders, peers: HashMap::new(), versions: serde_json::Value::Null, workers: serde_json::Value::Null, connected: Default::default(), builds: HashMap::new(), scope_dbs: HashMap::new(), status: HashMap::new(), views: HashMap::new(), changed: HashMap::new(), vault: Default::default(), nearest: HashMap::new(), far: HashMap::new(), read: false };
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

    pub(super) fn keep_folder(&self, id: &str, path: Option<&std::path::Path>) -> Result<()> {
        let mut file: NetworkFile = serde_json::from_slice(&std::fs::read(self.network_dir().join(NETWORK_FILE))?)?;
        match path {
            Some(path) => drop(file.folders.insert(id.to_string(), path.display().to_string())),
            None => drop(file.folders.remove(id)),
        }
        self.save_network(&file)?;
        self.with(|i| {
            i.folders = file.folders;
            Ok(())
        })
    }

    pub(super) fn save_network(&self, file: &NetworkFile) -> Result<()> {
        let dir = self.network_dir();
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join(NETWORK_FILE), &serde_json::to_vec_pretty(file)?)?;
        Ok(())
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
        let me = crate::keys::Endpoint::of(&self.0.key);
        // The network's id, which every device of it pins, and `global`'s first epoch, named by its first freeze before
        // anything is sealed under it.
        let root = Hash::from_bytes(rand::random());
        let global: [u8; 32] = rand::random();
        let start = content::first(name.trim(), &user, user_name.trim(), &me, &self.device_name(), MACHINE, recovery.verifying_key().as_bytes(), &crate::keys::epoch_hash(&global));
        // Until it's moved to S3-compatible storage, the network's bucket is a folder here.
        let bucket: BTreeMap<String, String> = [("path".to_string(), self.network_dir().join("bucket").display().to_string())].into();
        Place::from_fields(&bucket)?.create(NETWORK_OBJECT, root.to_hex().as_bytes())?;
        self.save_network(&NetworkFile { root: root.to_hex(), user: user.clone(), bucket: Some(bucket), folders: BTreeMap::new() })?;
        self.open_network()?;
        let bucket = self.bucket()?;
        let recovery_seal = crate::keys::twin_public(recovery.verifying_key().as_bytes())?;
        for (to, seal) in [("recovery".to_string(), recovery_seal), (hex(&me.key), me.seal)] {
            crate::database::stored::created(&bucket, &crate::keys::epoch_name("global", 1, &to), &crate::keys::seal_to(&seal, &global)?).await?;
        }
        self.0.databases.epochs.lock().unwrap().insert(Scope::Global, crate::database::stored::Keys::from([(1, global)]));
        // Who's who, as the freeze its recovery key signs at step 1 of its scope database.
        let keys = self.step_keys(&Scope::Global).await?;
        self.with(|i| {
            i.read = true;
            Ok(())
        })?;
        let first = crate::database::steps::first_freeze(&recovery, root, &start, &user, now_ms())?;
        let name = crate::database::stored::step_name(&crate::database::steps::prefix("global"), 1);
        let sealed = crate::database::steps::seal(&keys, &name, &crate::database::steps::encode(&first.step, first.copy.as_deref()))?;
        ensure!(crate::bucket::Bucket::create(&bucket, &name, &sealed).await?, "the new network's bucket isn't empty");
        self.read_steps().await?;
        ensure!(self.frozen(), "the new network's freeze didn't count");
        let me = Caller::own(self.0.id, user);
        // Its Vault and its first user's own open on this machine, so a new network fills items with no other step.
        for scope in [Scope::Global, Scope::User(me.user.clone())] {
            crate::settings::new_vault_key(self, &scope, 1, &[self.0.id]).await?;
        }
        // Written first and run together: running them one by one, each runs them all again.
        for (app, _) in crate::config::BUILT_IN {
            self.write_built_in(&me, app).await?;
        }
        for app in apps {
            self.write_built_in(&me, app).await?;
            self.append(&Scope::Global, content::added(app, true)).await?;
        }
        self.settle().await?;
        self.emit(Event::Network);
        Ok(hex(&recovery.to_bytes()))
    }
}
