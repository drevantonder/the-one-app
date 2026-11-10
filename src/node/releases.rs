//! What's published at the network's address, as a node reads the signed record there (`crate::latest`): the
//! services' builds, and the builds of a service a deploy names, which a device runs instead of any release. A
//! computer's oned takes its own build from the record (`oned`'s pull loop), needing nothing of a node.

use super::*;
use crate::insights::{Attrs, Status};

pub(crate) mod routes;

/// How often a node reads the record (`crate::latest`).
const PULL_EVERY: Duration = Duration::from_secs(30);

impl Node {
    /// Reads the record at the network's address now and then, for what's published for this platform and its
    /// services, and keeps the network's address in its home for oned and `one update`, which can read nothing else.
    pub(super) async fn pull_loop(self) {
        if crate::mode::here().release_key().is_empty() {
            return;
        }
        loop {
            if let Err(err) = self.pull_once().await {
                tracing::debug!("reading the record at the network's address: {err:#}");
            }
            tokio::time::sleep(PULL_EVERY).await;
        }
    }

    async fn pull_once(&self) -> Result<()> {
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
        if let Some(thing) = latest.get(&part)
            && thing.commit != crate::config::BUILD
            && self.note_update_seen(&thing.commit)
        {
            self.device_signal("update.seen", Attrs::default().part(&part), Status::Unset);
        }
        Ok(())
    }

    /// Whether this is the first time this process has seen `commit` as the newest: `update.seen` is said once for each.
    fn note_update_seen(&self, commit: &str) -> bool {
        let mut seen = self.0.update_seen.lock().unwrap();
        (seen.as_deref() != Some(commit)).then(|| seen.replace(commit.to_string())).is_some()
    }

    /// Whether this device can't update, and why, as its hello says and `status` shows: what its oned, or on a Mac its
    /// app's install, last said.
    pub fn updating(&self) -> Option<String> {
        crate::update::why(&self.0.dir)
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
                self.device_signal("update.sent", Attrs::default().part(&thing.name), Status::Unset);
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
