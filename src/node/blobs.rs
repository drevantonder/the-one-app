//! Blobs: keeping them, reading the ones a person may, and fetching them from other servers.

use super::*;

impl Node {
    /// Keeps `bytes` as a blob.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        release::put_blob(&self.0.dir, bytes)
    }

    /// A blob from this server, else from a server it syncs with.
    pub async fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) {
            return Ok(bytes);
        }
        self.fetch_from(self.peers(), hash, 1 << 30).await
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

    /// Fetches a blob from the first of `peers` that has it, and keeps it.
    pub(super) async fn fetch_from(&self, peers: Vec<Key>, hash: &str, size: u64) -> Result<Vec<u8>> {
        let mut last = anyhow!("no device had it");
        for peer in peers {
            match self.fetch_blob(peer, hash, size).await {
                Ok(bytes) => {
                    release::put_blob(&self.0.dir, &bytes)?;
                    return Ok(bytes);
                }
                Err(err) => last = err,
            }
        }
        Err(last)
    }

    /// The servers this server syncs with now, then the rest.
    pub(super) fn peers(&self) -> Vec<Key> {
        self.with(|i| {
            let mut peers: Vec<Key> = i.r.members.servers().map(|m| m.key).filter(|d| *d != self.0.id).collect();
            peers.sort_by_key(|k| !i.online(k));
            Ok(peers)
        })
        .unwrap_or_default()
    }

    pub(super) async fn fetch_blob(&self, peer: Key, hash: &str, size: u64) -> Result<Vec<u8>> {
        let conn = tokio::time::timeout(Duration::from_secs(10), self.0.endpoint.connect(EndpointId::from_bytes(&peer)?, BLOB_ALPN)).await.context("timed out")??;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(hash.as_bytes()).await?;
        send.finish()?;
        let bytes = recv.read_to_end(size as usize).await?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "the blob didn't match its hash");
        Ok(bytes)
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

    /// Deletes the blobs here that scopes forgot, once no entry this server holds still needs them.
    pub(super) fn delete_forgotten(&self, hashes: Vec<String>) {
        for hash in hashes {
            if self.0.built_in.values().any(|b| *b == hash) || self.blob_named(&hash, |_| true).unwrap_or(true) {
                continue;
            }
            if let Ok(path) = release::blob_path(&self.0.dir, &hash) {
                if std::fs::remove_file(&path).is_ok() {
                    tracing::info!("deleted blob {} that nothing needs any more", &hash[..12]);
                }
            }
        }
    }

    pub(super) fn blob_named_for(&self, hash: &str, user: &str) -> Result<bool> {
        let m = self.with(|i| Ok(i.r.members.clone()))?;
        self.blob_named(hash, |s| m.may_use(user, s))
    }

    pub(super) async fn accept_blob(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let m = self.with(|i| Ok(i.r.members.clone()))?;
        ensure!(m.server(&remote).is_some(), "not a device of this install");
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hash = String::from_utf8(recv.read_to_end(64).await?)?;
        ensure!(self.blob_named(&hash, |s| m.holds(&remote, s))?, "no such blob for you");
        let bytes = std::fs::read(release::blob_path(&self.0.dir, &hash)?).context("no such blob here")?;
        send.write_all(&bytes).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}
