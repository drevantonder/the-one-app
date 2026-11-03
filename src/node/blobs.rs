//! Blobs: keeping them here and in the bucket, and reading the ones a person may.

use super::*;

impl Node {
    /// Keeps `bytes` as a blob.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        release::put_blob(&self.0.dir, bytes)
    }

    /// A blob from this device, else from the bucket, in a scope whose entries name it.
    pub async fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) {
            return Ok(bytes);
        }
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        for scope in self.scopes_naming(hash)? {
            let name = crate::bucket::blob_name(&scope.to_string(), hash);
            let Some(sealed) = Bucket::get(&bucket.place, &name).await? else { continue };
            let keys = self.database_keys(&scope, members::APP, crate::bucket::BLOBS).await?;
            let epoch = crate::keys::epoch_of(&sealed.bytes)?;
            let key = keys.get(&epoch).with_context(|| format!("{} wasn't given {scope}'s epoch {epoch}", self.device_name()))?;
            let bytes = crate::keys::open(key, &name, &sealed.bytes)?;
            ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "the blob kept as {hash} isn't it");
            release::put_blob(&self.0.dir, &bytes)?;
            return Ok(bytes);
        }
        bail!("no device has put that blob in the bucket")
    }

    /// Puts a blob this device has in the bucket, in `scope`, sealed under that scope's newest epoch, where it isn't: for a new
    /// entry, always, since another device may have taken it out after a scope forgot it; else (`noted`) unless this
    /// device notes it put it there.
    pub(super) async fn share_blob(&self, scope: &Scope, hash: &str, noted: bool) -> Result<()> {
        use crate::bucket::Bucket;
        let marker = format!("{scope} {hash}");
        if noted && self.0.shared.lock().unwrap().contains(&marker) {
            return Ok(());
        }
        let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) else { return Ok(()) };
        let keys = self.database_keys(scope, members::APP, crate::bucket::BLOBS).await?;
        let name = crate::bucket::blob_name(&scope.to_string(), hash);
        Bucket::create(&self.bucket()?.place, &name, &crate::log::seal(&keys, &name, &bytes)?).await?;
        self.0.shared.lock().unwrap().insert(marker.clone());
        self.note_shared(&marker)
    }

    /// Notes in `install/shared` a blob put in the bucket, `<scope> <hash>`, or with `- ` before it, one taken out.
    fn note_shared(&self, line: &str) -> Result<()> {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.install_dir().join("shared"))?;
        std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes())?;
        Ok(())
    }

    /// Tries again to put in the bucket the blobs it didn't take when their entries were written.
    pub(super) async fn share_unshared(&self) {
        let unshared = std::mem::take(&mut *self.0.unshared.lock().unwrap());
        for (scope, hash) in unshared {
            if let Err(err) = self.share_blob(&scope, &hash, false).await {
                tracing::debug!("putting blob {} in the bucket: {err:#}", &hash[..12.min(hash.len())]);
                self.0.unshared.lock().unwrap().push((scope, hash));
            }
        }
    }

    /// Puts in the bucket each blob this device has that an entry it holds names, and it hasn't put there yet: those
    /// from before blobs were kept in the bucket.
    pub(super) async fn share_blobs(self) {
        let noted = std::fs::read_to_string(self.install_dir().join("shared")).unwrap_or_default();
        {
            let mut shared = self.0.shared.lock().unwrap();
            for line in noted.lines() {
                match line.strip_prefix("- ") {
                    Some(gone) => drop(shared.remove(gone)),
                    None => drop(shared.insert(line.to_string())),
                }
            }
        }
        let Ok(named) = self.with(|i| {
            let mut named = Vec::new();
            for log in i.r.store.logs()? {
                let Ok(scope) = Scope::parse(&log) else { continue };
                for e in i.r.store.scope(&log)? {
                    named.extend(e.content.blobs.iter().map(|b| (scope.clone(), b.clone())));
                }
            }
            Ok(named)
        }) else {
            return;
        };
        for (scope, hash) in named {
            if let Err(err) = self.share_blob(&scope, &hash, true).await {
                tracing::debug!("putting blob {} in the bucket: {err:#}", &hash[..12.min(hash.len())]);
            }
        }
    }

    /// The scopes this device holds whose entries name a blob.
    fn scopes_naming(&self, hash: &str) -> Result<Vec<Scope>> {
        self.with(|i| {
            let mut out = Vec::new();
            for log in i.r.store.logs()? {
                if i.r.store.scope(&log)?.iter().any(|e| e.content.blobs.iter().any(|b| b == hash)) {
                    out.extend(Scope::parse(&log));
                }
            }
            Ok(out)
        })
    }

    /// Keeps `bytes` as a blob a sign-in put, which its person may then read back.
    pub fn put_blob_for(&self, caller: &Caller, bytes: &[u8]) -> Result<String> {
        let hash = self.put_blob(bytes)?;
        self.0.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(caller.user.clone());
        Ok(hash)
    }

    /// A blob for someone who isn't this machine's own programs: one they put, or an entry names in a scope they may use.
    pub async fn blob_for(&self, caller: &Caller, hash: &str) -> Result<Vec<u8>> {
        ensure!(self.may_read_blob(&caller.user, hash)?, "no such blob for you");
        self.blob(hash).await
    }

    pub(super) fn may_read_blob(&self, user: &str, hash: &str) -> Result<bool> {
        Ok(self.0.put_by.lock().unwrap().get(hash).is_some_and(|users| users.contains(user)) || self.blob_named_for(hash, user)?)
    }

    /// Whether an entry in a scope `holds` says yes to names the blob, and that scope still needs it: a blob a scope
    /// keeps stops counting once it forgets it; one another node entry names (a deploy's bundle, a release's binary)
    /// never does.
    pub(super) fn blob_named(&self, hash: &str, holds: impl Fn(&Scope) -> bool) -> Result<bool> {
        self.with(|i| {
            for log in i.r.store.logs()? {
                let forgotten = i.views.get(&log).is_some_and(|(_, s)| s.forgotten.contains(hash));
                let names = |e: &Entry| e.content.blobs.iter().any(|b| b == hash) && (!forgotten || (e.content.app == members::APP && e.content.kind != "keep"));
                if Scope::parse(&log).is_ok_and(|s| holds(&s)) && i.r.store.scope(&log)?.iter().any(names) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    /// Notes in `install/forgotten` each blob a scope forgot, `(scope, hash)`s, to take from the bucket once
    /// `FORGOTTEN_KEPT` is up, and deletes the blobs here that no entry this device holds still needs.
    pub(super) fn delete_forgotten(&self, forgotten: Vec<(String, String)>) {
        let due = now_ms() + FORGOTTEN_KEPT.as_millis() as u64;
        for (log, hash) in forgotten {
            if self.0.built_in.values().any(|b| *b == hash) {
                continue;
            }
            if let Err(err) = self.note_forgotten(&[format!("{due} {log} {hash}")], &[]) {
                tracing::warn!("noting a forgotten blob: {err:#}");
            }
            if self.blob_named(&hash, |_| true).unwrap_or(true) {
                continue;
            }
            if let Ok(path) = release::blob_path(&self.0.dir, &hash) {
                if std::fs::remove_file(&path).is_ok() {
                    tracing::info!("deleted blob {} that nothing needs any more", &hash[..12]);
                }
            }
        }
    }

    /// Takes from the bucket each forgotten blob whose time is up that no entry of its scope needs, by when an entry
    /// another device wrote naming it again is read here. Kept in a file, so a restart loses none; one the bucket didn't
    /// take is tried again next time.
    pub(super) async fn take_forgotten(&self) {
        let now = now_ms();
        let noted = std::fs::read_to_string(self.install_dir().join("forgotten")).unwrap_or_default();
        let mut done = Vec::new();
        for line in noted.lines() {
            let mut parts = line.splitn(3, ' ');
            let (Some(Ok(due)), Some(log), Some(blob)) = (parts.next().map(str::parse::<u64>), parts.next(), parts.next()) else {
                done.push(line.to_string());
                continue;
            };
            if due > now {
                continue;
            }
            match self.blob_named(blob, |s| s.to_string() == log) {
                Ok(false) => {}
                Ok(true) => {
                    done.push(line.to_string());
                    continue;
                }
                Err(_) => continue,
            }
            let Ok(bucket) = self.bucket() else { continue };
            let name = crate::bucket::blob_name(log, blob);
            if let Err(err) = tokio::task::spawn_blocking(move || bucket.place.delete(&name)).await.map_err(anyhow::Error::from).and_then(|r| r) {
                tracing::warn!("taking a forgotten blob from the bucket: {err:#}");
                continue;
            }
            let marker = format!("{log} {blob}");
            self.0.shared.lock().unwrap().remove(&marker);
            let _ = self.note_shared(&format!("- {marker}"));
            done.push(line.to_string());
        }
        if !done.is_empty() {
            if let Err(err) = self.note_forgotten(&[], &done) {
                tracing::warn!("noting forgotten blobs taken: {err:#}");
            }
        }
    }

    /// Adds lines to `install/forgotten`, `<due> <scope> <hash>`, and takes out the ones `done`.
    fn note_forgotten(&self, add: &[String], done: &[String]) -> Result<()> {
        let _file = self.0.forgotten.lock().unwrap();
        let path = self.install_dir().join("forgotten");
        let was = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: String = was.lines().filter(|l| !done.iter().any(|d| d == l)).chain(add.iter().map(String::as_str)).map(|l| format!("{l}\n")).collect();
        let tmp = path.with_extension("new");
        std::fs::write(&tmp, lines)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub(super) fn blob_named_for(&self, hash: &str, user: &str) -> Result<bool> {
        let m = self.with(|i| Ok(i.r.members.clone()))?;
        self.blob_named(hash, |s| m.may_use(user, s))
    }
}
