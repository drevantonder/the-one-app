//! Each device taking its own updates from the signed record at the network's address (`crate::latest`): the node's, and the
//! services' builds, and the builds of a service a deploy names, which a device runs instead of any release.

use super::*;
use crate::insights::{Attrs, Fault};

pub(crate) mod routes;

/// How often a device checks the record (`crate::latest`). A device that can't speak anything else reaches the network
/// only by this, so it stays dumb: a timer, not a wake-up over anything that could stop working.
const PULL_EVERY: Duration = Duration::from_secs(30);

/// How long after asking to hand over to a build it installed a node that still runs asks again: longer than a node
/// taking over takes to warm.
const HAND_OVER_AGAIN: Duration = Duration::from_secs(150);

impl Node {
    /// Checks the record at the network's address now and then and, if it says there's a newer build of this node, takes it:
    /// nothing else of the network has to work for that. A bad or missing record means staying on what it has and saying
    /// why (`updating`). A node in an app bundle reads the record for `status` but doesn't take it: `keep-mac-current` builds and installs it.
    pub(super) async fn pull_loop(self) {
        if crate::mode::here().release_key().is_empty() {
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
        if let Err(err) = crate::update::keep_address(&self.0.dir, &address) {
            tracing::warn!("couldn't note the network's address for {} update: {err}", crate::config::CLI);
        }
        let fetched = {
            let address = address.clone();
            tokio::task::spawn_blocking(move || crate::latest::fetch(&address)).await??
        };
        let Some(text) = fetched else { return Ok(()) };
        let latest = crate::latest::Latest::read(crate::mode::here().release_key(), &text).context("the latest record")?;
        *self.0.latest.lock().unwrap() = latest.clone();
        let part = crate::latest::Thing::node_name(crate::config::TARGET);
        let Some(thing) = latest.get(&part).cloned() else { return Ok(()) };
        let newer = thing.commit != crate::config::BUILD;
        if newer && self.note_update_seen(&thing.commit) {
            self.device_signal("update.seen", Attrs::default().part(&part), true, None);
        }
        if release::in_app_bundle() {
            if newer {
                self.device_signal("update.not_taken", Attrs::default().part(&part).reason("app-bundle"), true, None);
            }
            return Ok(());
        }
        let taken = crate::update::taken(&self.0.dir);
        if !crate::latest::wanted(&thing, crate::config::BUILD, crate::config::COMMITTED, &taken) {
            // Already installed: asked again now and then to hand over to it, as long as this build still runs.
            if thing.commit != crate::config::BUILD && taken == thing.sha256 && handed.elapsed() > HAND_OVER_AGAIN {
                *handed = Instant::now();
                crate::handoff::restart();
            }
            return Ok(());
        }
        let not_taken = |reason: &'static str, err: anyhow::Error| {
            self.device_signal("update.not_taken", Attrs::default().part(&part).reason(reason), false, Some(Fault::Internal));
            err
        };
        let bytes = {
            let (address, thing) = (address, thing.clone());
            tokio::task::spawn_blocking(move || crate::latest::download(&address, &thing)).await?.map_err(|e| not_taken("download", e))?
        };
        let exe = release::install_commit(&thing.commit, &bytes).map_err(|e| not_taken("install", e))?;
        crate::update::took(&self.0.dir, &thing.sha256).map_err(|e| not_taken("install", e.into()))?;
        self.device_signal("update.taken", Attrs::default().part(&part), true, None);
        tracing::info!("updated {} to {}; restarting", exe.display(), thing.commit);
        *handed = Instant::now();
        crate::handoff::restart();
        Ok(())
    }

    /// Whether this is the first time this process has seen `commit` as the newest: `update.seen` is said once for each.
    fn note_update_seen(&self, commit: &str) -> bool {
        let mut seen = self.0.update_seen.lock().unwrap();
        (seen.as_deref() != Some(commit)).then(|| seen.replace(commit.to_string())).is_some()
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

    /// Keeps the file `one-dev update` publishes: unsealed in the bucket's `updates/`, by its hash. → the hash.
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
        let was = self.latest();
        let replaced = Bucket::replace(&bucket.place, &crate::latest::newest_name(), text.as_bytes(), tag).await?;
        if replaced && let Ok(new) = crate::latest::Latest::read(crate::mode::here().release_key(), text) {
            for thing in new.things.iter().filter(|t| was.get(&t.name).is_none_or(|w| w.sha256 != t.sha256)) {
                self.device_signal("update.sent", Attrs::default().part(&thing.name), true, None);
            }
        }
        Ok(replaced)
    }

    /// The file a service's or the node's build is, fetched from the network's address, once it's the one the record names.
    pub async fn thing_bytes(&self, thing: &crate::latest::Thing) -> Result<Vec<u8>> {
        let (address, thing) = (self.address()?, thing.clone());
        tokio::task::spawn_blocking(move || crate::latest::download(&address, &thing)).await?
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

    /// The newest build of each service's program for this device's platform, as the record names it.
    pub fn service_releases(&self) -> BTreeMap<String, crate::latest::Thing> {
        let (prefix, suffix) = ("service/", format!("/{}", crate::config::TARGET));
        self.latest().things.into_iter().filter_map(|t| Some((t.name.strip_prefix(prefix)?.strip_suffix(&suffix)?.to_string(), t.clone()))).filter(|(n, _)| !n.contains('/')).collect()
    }
}

/// What a device runs of a service on its platform.
#[derive(Clone, Debug, PartialEq)]
pub enum Build {
    /// The newest build of its program, as the record at the network's address names it.
    Released(crate::latest::Thing),
    /// The build, by its hash, its app's latest deploy names.
    Deployed(String),
    /// Nothing: its app's latest deploy names builds, none for this platform.
    None,
}
