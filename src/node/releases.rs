//! Releases of the node and of services in other languages: publishing one, and each device taking the newest it
//! trusts.

use super::*;

impl Node {
    /// Publishes `binary` (a build of this node for `target`, or of `service`'s program, with the release key's
    /// `signature` over it) as the install's newest release of it.
    pub async fn publish(&self, binary: &Path, service: Option<&str>, commit: &str, target: &str, committed: u64, signature: &str) -> Result<Release> {
        ensure!(service.is_none_or(crate::members::valid_app_name), "a service's name is lowercase words joined by -");
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        release::verify(crate::config::RELEASE_KEY, &bytes, signature).context("no device would take it")?;
        let blob = self.put_blob(&bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into(), service: service.map(str::to_string) };
        self.append(&Scope::Global, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node whenever a newer build for its platform is published, signed with the release key. A download
    /// that drops, or a release that fails to install, is tried again, waiting twice as long each time up to ten minutes.
    pub(super) async fn update_loop(self) {
        let mut wait = Duration::from_secs(2);
        loop {
            let Some(r) = self.wanted_release() else {
                wait = Duration::from_secs(2);
                self.0.released.notified().await;
                continue;
            };
            match self.apply(&r).await {
                Ok(exe) => {
                    let _ = std::fs::write(self.0.dir.join("release"), &r.blob);
                    tracing::info!("updated {} to {}; restarting", exe.display(), r.commit);
                    crate::handoff::restart();
                    return;
                }
                Err(err) => tracing::warn!("couldn't take the release of {}, trying again in {}s: {err:#}", r.commit, wait.as_secs()),
            }
            let _ = tokio::time::timeout(wait, self.0.released.notified()).await;
            wait = (wait * 2).min(Duration::from_secs(600));
        }
    }

    /// The newest release for this node's platform, if it's one this node should take and hasn't yet.
    pub(super) fn wanted_release(&self) -> Option<Release> {
        let (r, _) = self.with(|i| Ok(release::latest(crate::config::RELEASE_KEY, &i.r.store.scope("global")?, crate::config::TARGET))).ok()??;
        let taken = std::fs::read_to_string(self.0.dir.join("release")).is_ok_and(|m| m.trim() == r.blob);
        (release::wanted(&r, crate::config::BUILD, crate::config::COMMITTED) && !taken).then_some(r)
    }

    pub(super) async fn apply(&self, r: &Release) -> Result<PathBuf> {
        let path = release::blob_path(&self.0.dir, &r.blob)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => self.blob(&r.blob).await?,
        };
        release::verify(crate::config::RELEASE_KEY, &bytes, &r.signature)?;
        release::install(r, &bytes)
    }

    /// The newest release of `service`'s program for this device's platform, signed with the release key.
    pub fn service_release(&self, service: &str) -> Option<Release> {
        let (r, _) = self.with(|i| Ok(release::latest_of(crate::config::RELEASE_KEY, &i.r.store.scope("global")?, crate::config::TARGET, Some(service)))).ok()??;
        Some(r)
    }

    /// `r`'s program, fetched if it isn't here, once its signature is the release key's.
    pub async fn service_program(&self, r: &Release) -> Result<Vec<u8>> {
        let bytes = match std::fs::read(release::blob_path(&self.0.dir, &r.blob)?) {
            Ok(b) => b,
            Err(_) => self.blob(&r.blob).await?,
        };
        release::verify(crate::config::RELEASE_KEY, &bytes, &r.signature)?;
        Ok(bytes)
    }

    /// The newest release of each service's program for this device's platform, signed with the release key.
    pub fn service_releases(&self) -> BTreeMap<String, Release> {
        let entries = self.with(|i| i.r.store.scope("global")).unwrap_or_default();
        let names: std::collections::BTreeSet<String> = entries.iter().filter(|e| e.content.app == release::APP && e.content.kind == release::SERVICE_KIND).filter_map(|e| e.content.data["service"].as_str().map(str::to_string)).collect();
        names.into_iter().filter_map(|n| Some((n.clone(), release::latest_of(crate::config::RELEASE_KEY, &entries, crate::config::TARGET, Some(&n))?.0))).collect()
    }
}
