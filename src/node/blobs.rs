//! Blobs: keeping them here and in the bucket, and reading the ones a user may. Each blob a scope database names is in
//! the bucket too, sealed with its scope's key, where a device that lacks one reads it: a deploy's bundle and a release's
//! binary among them.

use super::*;

pub(crate) mod routes;
pub mod cli;

/// How long a blob a scope forgot stays in the bucket, in case an entry another device wrote names it again.
const FORGOTTEN_KEPT: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct State {
    /// The blobs sign-ins put here (their files in an MCP answer, say), by hash: for whom, so they may read them back.
    pub(super) put_by: Mutex<HashMap<String, std::collections::HashSet<String>>>,
    /// The files put from elsewhere this last hour, which count against their user's room (`put_blob_from`).
    remote_puts: Mutex<Vec<blobs::RemotePut>>,
    /// Held while `network/forgotten` changes.
    forgotten: Mutex<()>,
    /// The blobs this device has put in the bucket, as `<scope> <hash>`, kept in `network/shared` too.
    shared: Mutex<std::collections::HashSet<String>>,
    /// The blobs of entries this device wrote that the bucket didn't take then, tried again each minute.
    unshared: Mutex<Vec<(Scope, String)>>,
    /// `<scope> <hash>` of blobs a call put in the bucket before writing the steps that keep them, which don't again.
    pub(super) put_first: Mutex<std::collections::HashSet<String>>,
}

impl Node {
    /// Keeps `bytes` as a blob.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        release::put_blob(&self.0.dir, bytes)
    }

    /// A blob from this device, else from the bucket, in a scope whose database names it.
    pub async fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) {
            return Ok(bytes);
        }
        use ::bucket::Bucket;
        let bucket = self.bucket()?;
        for scope in self.scopes_naming(hash)? {
            let name = ::bucket::names::blob_name(&scope.to_string(), hash);
            let Some(sealed) = Bucket::get(&bucket.place, &name).await? else { continue };
            let keys = self.database_keys(&scope, key::APP, key::CoreDb::Blobs.name(), false).await?;
            let epoch = key::epoch_of(&sealed.bytes)?;
            let key = keys.get(&epoch).with_context(|| format!("{} wasn't given {scope}'s epoch {epoch}", self.device_name()))?;
            let bytes = key::open(key, &name, &sealed.bytes)?;
            ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "the blob kept as {hash} isn't it");
            release::put_blob(&self.0.dir, &bytes)?;
            return Ok(bytes);
        }
        bail!("no device has put that blob in the bucket")
    }

    /// Puts a blob this device has in the bucket, in `scope`, sealed under that scope's newest epoch, where it isn't: for a new
    /// step, always, since another device may have taken it out after a scope forgot it, unless a call just put it
    /// there before writing (`Node::prepare_appends`); else (`noted`) unless this device notes it put it there.
    pub(super) async fn share_blob(&self, scope: &Scope, hash: &str, noted: bool) -> Result<()> {
        use ::bucket::Bucket;
        let marker = format!("{scope} {hash}");
        if (noted && self.0.blobs.shared.lock().unwrap().contains(&marker)) || self.0.blobs.put_first.lock().unwrap().contains(&marker) {
            return Ok(());
        }
        let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) else { return Ok(()) };
        let keys = self.database_keys(scope, key::APP, key::CoreDb::Blobs.name(), false).await?;
        let name = ::bucket::names::blob_name(&scope.to_string(), hash);
        Bucket::create(&self.bucket()?.place, &name, &scope::steps::seal(&keys, &name, &bytes)?).await?;
        self.0.blobs.shared.lock().unwrap().insert(marker.clone());
        self.note_shared(&marker)
    }

    /// Notes in `network/shared` a blob put in the bucket, `<scope> <hash>`, or with `- ` before it, one taken out.
    fn note_shared(&self, line: &str) -> Result<()> {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(self.network_dir().join("shared"))?;
        std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes())?;
        Ok(())
    }

    /// Tries again to put in the bucket the blobs it didn't take when their steps were written.
    pub(super) async fn share_unshared(&self) {
        let unshared = std::mem::take(&mut *self.0.blobs.unshared.lock().unwrap());
        for (scope, hash) in unshared {
            if let Err(err) = self.share_blob(&scope, &hash, false).await {
                tracing::debug!("putting blob {} in the bucket: {err:#}", &hash[..12.min(hash.len())]);
                self.0.blobs.unshared.lock().unwrap().push((scope, hash));
            }
        }
    }

    /// Puts in the bucket each blob this device has that a scope it holds names, and it hasn't put there yet.
    pub(super) async fn share_blobs(self) {
        let noted = std::fs::read_to_string(self.network_dir().join("shared")).unwrap_or_default();
        {
            let mut shared = self.0.blobs.shared.lock().unwrap();
            for line in noted.lines() {
                match line.strip_prefix("- ") {
                    Some(gone) => drop(shared.remove(gone)),
                    None => drop(shared.insert(line.to_string())),
                }
            }
        }
        let Ok(named) = self.with(|i| {
            let mut named: Vec<(Scope, String)> = i.r.db().blobs()?.into_iter().map(|b| (Scope::Global, b)).collect();
            for (scope, db) in i.scope_dbs.iter().map(|(s, r)| (s, &r.db)) {
                named.extend(db.blobs()?.into_iter().map(|b| (scope.clone(), b)));
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

    fn scopes_naming(&self, hash: &str) -> Result<Vec<Scope>> {
        self.with(|i| steps_naming(i, hash))
    }

    /// Keeps what `body` sends as a blob, read as it comes, up to `most` bytes and with at most `IDLE` between its
    /// parts: → its hash and size. Nothing is kept of one that fails.
    async fn put_blob_streamed(&self, body: axum::body::Body, most: u64) -> Result<(String, u64)> {
        use futures_util::StreamExt;
        use std::io::Write;
        let dir = self.0.dir.join("blobs");
        std::fs::create_dir_all(&dir)?;
        let part = dir.join(format!("{}.part", new_id()));
        let wrote = async {
            let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
            let mut hasher = blake3::Hasher::new();
            let mut size = 0u64;
            let mut stream = body.into_data_stream();
            loop {
                let chunk = match tokio::time::timeout(PUT_IDLE, stream.next()).await {
                    Ok(Some(chunk)) => chunk.map_err(|e| anyhow!("the file didn't arrive whole: {e}"))?,
                    Ok(None) => break,
                    Err(_) => bail!("the file stopped arriving for {} s", PUT_IDLE.as_secs()),
                };
                size += chunk.len() as u64;
                ensure!(size <= most, Full(most));
                hasher.update(&chunk);
                file.write_all(&chunk)?;
            }
            file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            Ok((hasher.finalize().to_hex().to_string(), size))
        };
        let (hash, size) = match wrote.await {
            Ok(done) => done,
            Err(err) => {
                let _ = std::fs::remove_file(&part);
                return Err(err);
            }
        };
        let path = release::blob_path(&self.0.dir, &hash)?;
        match path.exists() {
            true => std::fs::remove_file(&part)?,
            false => std::fs::rename(&part, &path)?,
        }
        Ok((hash, size))
    }

    /// Keeps a file `caller` put, read as it comes. This machine's own programs put up to `LOCAL_PUT`. Someone else (a
    /// client device, or a sign-in on another machine) may then read theirs back: up to `REMOTE_PUT`, and only while what
    /// they put that nothing names yet stays under `REMOTE_WAITING`, since anyone signed in could fill this machine's
    /// disk. One nothing names an hour on is deleted (`sweep_remote_puts`). → its hash and size.
    pub async fn put_blob_from(&self, caller: &Caller, body: axum::body::Body) -> Result<(String, u64)> {
        if self.this_machine(caller) {
            return self.put_blob_streamed(body, LOCAL_PUT).await;
        }
        let waiting: u64 = self.0.blobs.remote_puts.lock().unwrap().iter().filter(|p| p.user == caller.user).map(|p| p.size).sum();
        let room = REMOTE_WAITING.saturating_sub(waiting);
        ensure!(room > 0, "you've put {} MiB of files from your other devices that nothing kept yet: wait an hour, when they're cleared, and try again", waiting >> 20);
        let (hash, size) = self.put_blob_streamed(body, room.min(REMOTE_PUT)).await.map_err(|err| match err.downcast_ref::<Full>() {
            Some(_) if room < REMOTE_PUT => anyhow!("you've put {} MiB of files from your other devices that nothing kept yet, and this one doesn't fit under {} MiB: wait an hour, when they're cleared, and try again", waiting >> 20, REMOTE_WAITING >> 20),
            _ => err,
        })?;
        self.0.blobs.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(caller.user.clone());
        self.0.blobs.remote_puts.lock().unwrap().push(RemotePut { hash: hash.clone(), user: caller.user.clone(), size, at: now_ms() });
        Ok((hash, size))
    }

    /// Deletes a file put for one request (an app's `_http` body) once it's answered, unless something names it.
    pub fn drop_put(&self, hash: &str) {
        if self.0.built_in.values().any(|b| b == hash) || self.blob_named(hash, |_| true).unwrap_or(true) {
            return;
        }
        self.0.blobs.remote_puts.lock().unwrap().retain(|p| p.hash != hash);
        self.0.blobs.put_by.lock().unwrap().remove(hash);
        if let Ok(path) = release::blob_path(&self.0.dir, hash) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Deletes the files put from elsewhere an hour ago that nothing names: their room is theirs again.
    pub(super) fn sweep_remote_puts(&self) {
        let due = now_ms().saturating_sub(REMOTE_KEPT.as_millis() as u64);
        let old: Vec<RemotePut> = {
            let mut puts = self.0.blobs.remote_puts.lock().unwrap();
            let (old, young) = std::mem::take(&mut *puts).into_iter().partition(|p| p.at <= due);
            *puts = young;
            old
        };
        for put in old {
            if !self.0.blobs.remote_puts.lock().unwrap().iter().any(|p| p.hash == put.hash) {
                self.drop_put(&put.hash);
            }
        }
    }

    /// Keeps `bytes` as a blob a sign-in put, which its user may then read back.
    pub fn put_blob_for(&self, caller: &Caller, bytes: &[u8]) -> Result<String> {
        let hash = self.put_blob(bytes)?;
        self.0.blobs.put_by.lock().unwrap().entry(hash.clone()).or_default().insert(caller.user.clone());
        Ok(hash)
    }

    /// A blob for `caller`: for someone who isn't this machine's own programs, one they put, or a scope they may use names.
    pub async fn blob_for(&self, caller: &Caller, hash: &str) -> Result<Vec<u8>> {
        ensure!(self.this_machine(caller) || self.may_read_blob(&caller.user, hash)?, "no such blob for you");
        self.blob(hash).await
    }

    pub(super) fn may_read_blob(&self, user: &str, hash: &str) -> Result<bool> {
        Ok(self.0.blobs.put_by.lock().unwrap().get(hash).is_some_and(|users| users.contains(user)) || self.blob_named_for(hash, user)?)
    }

    /// Whether the database of a scope `holds` says yes to names the blob, and that scope still needs it: a blob a
    /// scope keeps stops counting once it forgets it; one another step names (a deploy's bundle, a release's binary)
    /// never does (`ScopeDb::names_blob`).
    pub(super) fn blob_named(&self, hash: &str, holds: impl Fn(&Scope) -> bool) -> Result<bool> {
        self.with(|i| Ok(steps_naming(i, hash)?.into_iter().any(|s| holds(&s))))
    }

    /// Notes in `network/forgotten` each blob a scope forgot, `(scope, hash)`s, to take from the bucket once
    /// `FORGOTTEN_KEPT` is up, and deletes the blobs here that no scope this device holds still needs.
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

    /// Takes from the bucket each forgotten blob whose time is up that its scope no longer needs, by when a step
    /// another device wrote naming it again is read here. Kept in a file, so a restart loses none; one the bucket didn't
    /// take is tried again next time.
    pub(super) async fn take_forgotten(&self) {
        if !self.removes_from_bucket() {
            return;
        }
        let now = now_ms();
        let noted = std::fs::read_to_string(self.network_dir().join("forgotten")).unwrap_or_default();
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
                Err(err) => {
                    tracing::warn!("whether {blob} is still named in {log}: {err:#}");
                    continue;
                }
            }
            let Ok(bucket) = self.bucket() else { continue };
            let name = ::bucket::names::blob_name(log, blob);
            if let Err(err) = tokio::task::spawn_blocking(move || bucket.place.delete(&name)).await.map_err(anyhow::Error::from).and_then(|r| r) {
                tracing::warn!("taking a forgotten blob from the bucket: {err:#}");
                continue;
            }
            let marker = format!("{log} {blob}");
            self.0.blobs.shared.lock().unwrap().remove(&marker);
            let _ = self.note_shared(&format!("- {marker}"));
            done.push(line.to_string());
        }
        if !done.is_empty() {
            if let Err(err) = self.note_forgotten(&[], &done) {
                tracing::warn!("noting forgotten blobs taken: {err:#}");
            }
        }
    }

    /// Adds lines to `network/forgotten`, `<due> <scope> <hash>`, and takes out the ones `done`.
    fn note_forgotten(&self, add: &[String], done: &[String]) -> Result<()> {
        let _file = self.0.blobs.forgotten.lock().unwrap();
        let path = self.network_dir().join("forgotten");
        let was = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: String = was.lines().filter(|l| !done.iter().any(|d| d == l)).chain(add.iter().map(String::as_str)).map(|l| format!("{l}\n")).collect();
        let tmp = path.with_extension("new");
        std::fs::write(&tmp, lines)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub(super) fn blob_named_for(&self, hash: &str, user: &str) -> Result<bool> {
        let usable: Vec<Scope> = self.with(|i| Ok(i.r.db().scopes()?.into_iter().filter(|s| i.r.db().may(user, "use", &scopes::object(s))).collect()))?;
        self.blob_named(hash, |s| usable.contains(s))
    }
}

/// The most this machine's own programs put in one file.
const LOCAL_PUT: u64 = 1 << 30;
/// The most one file put from elsewhere may be.
pub const REMOTE_PUT: u64 = 512 << 20;
/// The most of one user's files put from elsewhere that nothing names yet this machine keeps at once.
pub const REMOTE_WAITING: u64 = 2 << 30;
/// How long a file put from elsewhere that nothing names is kept.
const REMOTE_KEPT: Duration = Duration::from_secs(3600);
/// How long a file being put may send nothing.
const PUT_IDLE: Duration = Duration::from_secs(60);

/// A file someone put from elsewhere, until it's an hour old.
pub(super) struct RemotePut {
    hash: String,
    user: String,
    size: u64,
    at: u64,
}

#[derive(Debug)]
struct Full(u64);

impl std::fmt::Display for Full {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "that's over {} MiB, the most this takes at once", self.0 >> 20)
    }
}

impl std::error::Error for Full {}

/// The scopes whose databases name a blob the scope still needs (`ScopeDb::names_blob`): `global`'s, once this device
/// has read who's who, and each other scope's database it reads.
fn steps_naming(i: &Network, hash: &str) -> Result<Vec<Scope>> {
    let mut named = Vec::new();
    if i.r.freeze().is_some() && i.r.db().names_blob(hash)? {
        named.push(Scope::Global);
    }
    for (scope, db) in i.scope_dbs.iter().map(|(s, r)| (s, &r.db)) {
        if db.names_blob(hash)? {
            named.push(scope.clone());
        }
    }
    Ok(named)
}
