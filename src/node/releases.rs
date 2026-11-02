//! Releases of the node: publishing one, and each server taking the newest it trusts.

use super::*;

impl Node {
    /// Publishes `binary` (a build of this node for `target`, with the release key's `signature` over it) as the
    /// install's newest release.
    pub async fn publish(&self, binary: &Path, commit: &str, target: &str, committed: u64, signature: &str) -> Result<Release> {
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        release::verify(crate::config::RELEASE_KEY, &bytes, signature).context("no node would take it")?;
        let blob = self.put_blob(&bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into() };
        self.append(&Scope::Global, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node whenever a newer build for its platform is published, signed with the release key. A download
    /// that drops, or a release that fails to install, is tried again, waiting twice as long each time up to ten minutes.
    /// The first wait is a restart's: the servers it would come from take the release too, and restart at once.
    pub(super) async fn update_loop(self) {
        let mut wait = Duration::from_secs(2);
        loop {
            let Some((r, author)) = self.wanted_release() else {
                wait = Duration::from_secs(2);
                self.0.released.notified().await;
                continue;
            };
            match self.apply(&r, author).await {
                Ok(exe) => {
                    // The servers taking a release take it from the one that published it first: it gives them a moment.
                    if author == self.0.id {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    let _ = std::fs::write(self.0.dir.join("release"), &r.blob);
                    tracing::info!("updated {} to {}; restarting", exe.display(), r.commit);
                    // Closing says so to every server connected, which would otherwise wait half a minute to notice:
                    // those taking this release from here try again, and get it once this server is back.
                    let _ = tokio::time::timeout(Duration::from_secs(2), self.0.endpoint.close()).await;
                    std::process::exit(0);
                }
                Err(err) => tracing::warn!("couldn't take the release of {}, trying again in {}s: {err:#}", r.commit, wait.as_secs()),
            }
            let _ = tokio::time::timeout(wait, self.0.released.notified()).await;
            wait = (wait * 2).min(Duration::from_secs(600));
        }
    }

    /// The newest release for this node's platform, if it's one this node should take and hasn't yet.
    pub(super) fn wanted_release(&self) -> Option<(Release, Key)> {
        let (r, author) = self.with(|i| Ok(release::latest(crate::config::RELEASE_KEY, &i.r.store.scope("global")?, crate::config::TARGET))).ok()??;
        let taken = std::fs::read_to_string(self.0.dir.join("release")).is_ok_and(|m| m.trim() == r.blob);
        (release::wanted(&r, crate::config::BUILD, crate::config::COMMITTED) && !taken).then_some((r, author))
    }

    pub(super) async fn apply(&self, r: &Release, author: Key) -> Result<PathBuf> {
        let path = release::blob_path(&self.0.dir, &r.blob)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                let mut peers = vec![author];
                peers.extend(self.peers().into_iter().filter(|p| *p != author));
                self.fetch_from(peers, &r.blob, r.size).await?
            }
        };
        release::verify(crate::config::RELEASE_KEY, &bytes, &r.signature)?;
        release::install(r, &bytes)
    }
}
