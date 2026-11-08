//! Releases of the node and of services in other languages: publishing one, and each device taking the newest it
//! trusts. And the builds of a service a deploy names, which a device runs instead of any release.

use super::*;

pub(crate) mod routes;

/// How often a device checks the record (`crate::latest`). A device that can't speak anything else reaches the network
/// only by this, so it stays dumb: a timer, not a wake-up over anything that could stop working.
const PULL_EVERY: Duration = Duration::from_secs(30);

/// How long after asking to hand over to a build it installed a node that still runs asks again: longer than a node
/// taking over takes to warm.
const HAND_OVER_AGAIN: Duration = Duration::from_secs(150);

impl Node {
    /// Publishes `binary` (a build of this node for `target`, or of `service`'s program, with the release key's
    /// `signature` over it) as the network's newest release of it.
    pub async fn publish(&self, binary: &Path, service: Option<&str>, commit: &str, target: &str, committed: u64, signature: &str) -> Result<Release> {
        ensure!(service.is_none_or(crate::database::scopes::valid_app_name), "a service's name is lowercase words joined by -");
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        release::verify(crate::config::RELEASE_KEY, &bytes, signature).context("no device would take it")?;
        let blob = self.put_blob(&bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into(), service: service.map(str::to_string) };
        self.append(&Scope::Global, release::content(&r)).await?;
        Ok(r)
    }

    /// Checks the record at the network's address now and then and, if it says there's a newer build of this node, takes it:
    /// nothing else of the network has to work for that. A bad or missing record means staying on what it has and saying
    /// why (`updating`). A node in an app bundle reads the record for `status` but doesn't take it: the Mac's job does.
    pub(super) async fn pull_loop(self) {
        if crate::config::RELEASE_KEY.is_empty() {
            return;
        }
        let mut handed = Instant::now() - HAND_OVER_AGAIN;
        loop {
            match self.pull_once(&mut handed).await {
                Ok(()) => self.say_updating(None),
                Err(err) => {
                    tracing::warn!("couldn't update: {err:#}");
                    self.say_updating(Some(format!("{err:#}")));
                }
            }
            tokio::time::sleep(PULL_EVERY).await;
        }
    }

    async fn pull_once(&self, handed: &mut Instant) -> Result<()> {
        let Ok(address) = self.address() else { return Ok(()) };
        let fetched = {
            let address = address.clone();
            tokio::task::spawn_blocking(move || crate::latest::fetch(&address)).await??
        };
        let Some(text) = fetched else { return Ok(()) };
        let latest = crate::latest::Latest::read(crate::config::RELEASE_KEY, &text).context("the latest record")?;
        *self.0.latest.lock().unwrap() = latest.clone();
        let Some(thing) = latest.get(&crate::latest::Thing::node_name(crate::config::TARGET)).cloned() else { return Ok(()) };
        if release::in_app_bundle() {
            return Ok(());
        }
        let file = self.0.dir.join("release");
        let taken = std::fs::read_to_string(&file).unwrap_or_default();
        if !crate::latest::wanted(&thing, crate::config::BUILD, crate::config::COMMITTED, taken.trim()) {
            // Already installed: asked again now and then to hand over to it, as long as this build still runs.
            if thing.commit != crate::config::BUILD && taken.trim() == thing.sha256 && handed.elapsed() > HAND_OVER_AGAIN {
                *handed = Instant::now();
                crate::handoff::restart();
            }
            return Ok(());
        }
        let bytes = {
            let (address, thing) = (address, thing.clone());
            tokio::task::spawn_blocking(move || crate::latest::download(&address, &thing)).await??
        };
        let exe = release::install_commit(&thing.commit, &bytes)?;
        std::fs::write(&file, &thing.sha256)?;
        tracing::info!("updated {} to {}; restarting", exe.display(), thing.commit);
        *handed = Instant::now();
        crate::handoff::restart();
        Ok(())
    }

    /// Whether this device can't update, and why, as its hello says and `status` shows: on a Mac, what the job beside
    /// the app last said, since the node's inside the thing being replaced.
    pub fn updating(&self) -> Option<String> {
        if release::in_app_bundle() {
            return std::fs::read_to_string(crate::user_home().join("Library/Caches").join(crate::config::CLI).join("why")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }
        self.0.updating.lock().unwrap().clone()
    }

    fn say_updating(&self, why: Option<String>) {
        *self.0.updating.lock().unwrap() = why;
    }

    /// What the record last said, as this node read it.
    pub fn latest(&self) -> crate::latest::Latest {
        self.0.latest.lock().unwrap().clone()
    }

    /// Keeps the file a `one update` publishes: unsealed in the bucket's `updates/`, by its hash. → the hash.
    pub async fn publish_file(&self, path: &Path) -> Result<(String, u64)> {
        use crate::bucket::Bucket;
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let sha256 = crate::latest::sha256(&bytes);
        let bucket = self.bucket()?;
        Bucket::create(&bucket.place, &format!("{}/{sha256}", crate::latest::FOLDER), &bytes).await?;
        Ok((sha256, bytes.len() as u64))
    }

    /// The record as it is in the bucket, with its tag, to replace.
    pub async fn latest_record(&self) -> Result<Option<(String, String)>> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        Ok(Bucket::get(&bucket.place, &crate::latest::newest_name()).await?.map(|o| (String::from_utf8_lossy(&o.bytes).into_owned(), o.tag)))
    }

    /// Replaces the record, if it's still the one `tag` names (none: if there's none). → whether it did.
    pub async fn replace_latest(&self, text: &str, tag: Option<&str>) -> Result<bool> {
        use crate::bucket::Bucket;
        let bucket = self.bucket()?;
        Bucket::replace(&bucket.place, &crate::latest::newest_name(), text.as_bytes(), tag).await
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
        let (r, _) = self.with_or(None, |i| Ok(release::latest_said(crate::config::RELEASE_KEY, &release::said(&i.r.db().releases()?), crate::config::TARGET, None)))?;
        let taken = std::fs::read_to_string(self.0.dir.join("release")).is_ok_and(|m| m.trim() == r.blob);
        (release::wanted(&r, crate::config::BUILD, crate::config::COMMITTED) && !taken).then_some(r)
    }

    pub(super) async fn apply(&self, r: &Release) -> Result<PathBuf> {
        release::install(r, &self.release_bytes(r).await?)
    }

    /// The newest release of `service`'s program for this device's platform, signed with the release key.
    pub fn service_release(&self, service: &str) -> Option<Release> {
        let (r, _) = self.with_or(None, |i| Ok(release::latest_said(crate::config::RELEASE_KEY, &release::said(&i.r.db().releases()?), crate::config::TARGET, Some(service))))?;
        Some(r)
    }

    /// `r`'s program, fetched if it isn't here, once its signature is the release key's.
    pub async fn release_bytes(&self, r: &Release) -> Result<Vec<u8>> {
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
        let bundles = self.with_or(Default::default(), |i| Ok(i.r.db().apps()?.into_iter().filter(|a| a.deleted.is_none()).filter_map(|a| Some((a.name, a.bundle?))).collect::<Vec<_>>()));
        for (app, bundle) in bundles {
            match self.blob(&bundle).await {
                Ok(bytes) => match crate::app::bundle::services(&bytes).remove(crate::config::TARGET) {
                    Some(build) => drop(builds.insert(app, Build::Deployed(build))),
                    // A deploy naming builds for other platforms only runs on none here, release or not.
                    None if !crate::app::bundle::services(&bytes).is_empty() => drop(builds.insert(app, Build::None)),
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
        let said = self.with_or(Default::default(), |i| Ok(release::said(&i.r.db().releases()?)));
        let names: std::collections::BTreeSet<String> = said.iter().filter(|(c, _)| c.is(crate::database::content::Change::ServiceRelease)).filter_map(|(c, _)| c.data["service"].as_str().map(str::to_string)).collect();
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
