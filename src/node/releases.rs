//! Releases of the node and of services in other languages: publishing one, and each device taking the newest it
//! trusts. And the builds of a service a deploy names, which a device runs instead of any release.

use super::*;

/// How long after asking to hand over to a build it installed a node that still runs asks again: longer than a node
/// taking over takes to warm.
const HAND_OVER_AGAIN: Duration = Duration::from_secs(150);

impl Node {
    /// Publishes `binary` (a build of this node for `target`, or of `service`'s program, with the release key's
    /// `signature` over it) as the network's newest release of it.
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
    /// that drops, or a release that fails to install, is tried again, waiting twice as long each time up to ten minutes;
    /// and so is handing over to one installed, while this build still runs, until it hands over or a newer one comes.
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
                    let mut after = HAND_OVER_AGAIN;
                    loop {
                        if !crate::handoff::restart() {
                            tracing::info!("{} hands over to it once it serves", crate::config::BUILD);
                        }
                        let _ = tokio::time::timeout(after, self.0.released.notified()).await;
                        if self.wanted_release().is_some() {
                            break;
                        }
                        tracing::warn!("{} still runs, though {} is installed: handing over again", crate::config::BUILD, r.commit);
                        after = (after * 2).min(Duration::from_secs(600));
                    }
                    wait = Duration::from_secs(2);
                    continue;
                }
                Err(err) => tracing::warn!("couldn't take the release of {}, trying again in {}s: {err:#}", r.commit, wait.as_secs()),
            }
            let _ = tokio::time::timeout(wait, self.0.released.notified()).await;
            wait = (wait * 2).min(Duration::from_secs(600));
        }
    }

    /// The newest release for this node's platform, if it's one this node should take and hasn't yet.
    pub(super) fn wanted_release(&self) -> Option<Release> {
        let (r, _) = self.with(|i| Ok(release::latest_said(crate::config::RELEASE_KEY, &release::said(&i.r.store.scope("global")?, &i.r.db().releases()), crate::config::TARGET, None))).ok()??;
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
        let (r, _) = self.with(|i| Ok(release::latest_said(crate::config::RELEASE_KEY, &release::said(&i.r.store.scope("global")?, &i.r.db().releases()), crate::config::TARGET, Some(service)))).ok()??;
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

    /// The build of each service a device on this platform runs: the one its app's latest deploy names
    /// (`bundle::SERVICES`), else its newest release signed with the release key, as built-in apps' services come.
    pub async fn service_builds(&self) -> BTreeMap<String, Build> {
        let mut builds: BTreeMap<String, Build> = self.service_releases().into_iter().map(|(n, r)| (n, Build::Released(r))).collect();
        let bundles = self.with(|i| Ok(i.r.db().apps().into_iter().filter(|a| a.deleted.is_none()).filter_map(|a| Some((a.name, a.bundle?))).collect::<Vec<_>>())).unwrap_or_default();
        for (app, bundle) in bundles {
            match self.blob(&bundle).await {
                Ok(bytes) => match crate::bundle::services(&bytes).remove(crate::config::TARGET) {
                    Some(build) => drop(builds.insert(app, Build::Deployed(build))),
                    // A deploy naming builds for other platforms only runs on none here, release or not.
                    None if !crate::bundle::services(&bytes).is_empty() => drop(builds.insert(app, Build::None)),
                    None => {}
                },
                Err(err) => tracing::debug!("reading {app}'s bundle for its service: {err:#}"),
            }
        }
        builds
    }

    /// A build of a service its deploy names, fetched if it isn't here, once it's the one by that hash.
    pub async fn service_build(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes = self.blob(hash).await?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "it isn't the build its deploy names");
        Ok(bytes)
    }

    /// The newest release of each service's program for this device's platform, signed with the release key.
    pub fn service_releases(&self) -> BTreeMap<String, Release> {
        let said = self.with(|i| Ok(release::said(&i.r.store.scope("global")?, &i.r.db().releases()))).unwrap_or_default();
        let names: std::collections::BTreeSet<String> = said.iter().filter(|(c, _)| c.kind == release::SERVICE_KIND).filter_map(|(c, _)| c.data["service"].as_str().map(str::to_string)).collect();
        names.into_iter().filter_map(|n| Some((n.clone(), release::latest_said(crate::config::RELEASE_KEY, &said, crate::config::TARGET, Some(&n))?.0))).collect()
    }
}

/// What a device runs of a service on its platform.
#[derive(Clone, Debug, PartialEq)]
pub enum Build {
    /// The newest release of its program, signed with the release key.
    Released(Release),
    /// The build, by its hash, its app's latest deploy names.
    Deployed(String),
    /// Nothing: its app's latest deploy names builds, none for this platform.
    None,
}
