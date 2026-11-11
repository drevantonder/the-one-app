//! What's published at the network's address, as a node reads the signed record there (`crate::latest`): the
//! services' builds, and the builds of a service a deploy names, which a device runs instead of any release. A device
//! takes its own build only by `one update`, which reads the record itself, needing nothing of a node.

use super::*;
use crate::insights::{Attrs, Status};

pub(crate) mod routes;

/// How often a node reads the record (`crate::latest`).
const PULL_EVERY: Duration = Duration::from_secs(30);

/// What each record kept in `updates/` beside the newest is named after, then its publish and when it was written (ms).
const PAST: &str = "past-";
/// How many publishes before the newest record still have their files: a device that read one a while ago still
/// finds what it names.
const KEPT_PUBLISHES: usize = 5;
/// How long a file in `updates/` stays, whatever names it (ms): a publisher writes its files before its record names
/// them, and another may publish meanwhile.
const UPLOADING: u64 = 60 * 60 * 1000;

impl Node {
    /// Reads the record at the network's address now and then, for what's published for this platform and its
    /// services, and keeps the network's address in its home for `one update`, which can read nothing else.
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
            tracing::warn!("couldn't note the network's address for {} update: {err}", custom_build::config::CLI);
        }
        let fetched = {
            let address = address.clone();
            tokio::task::spawn_blocking(move || crate::latest::fetch(&address)).await??
        };
        let Some(text) = fetched else { return Ok(()) };
        let latest = crate::latest::Latest::read(crate::mode::here().release_key(), &text).context("the latest record")?;
        *self.0.latest.lock().unwrap() = latest;
        Ok(())
    }

    /// Whether this device couldn't update, and why, as its answers say and `status` shows: what a Mac's app's install
    /// last said.
    pub fn updating(&self) -> Option<String> {
        crate::update::why(&self.0.dir)
    }

    /// What the record last said, as this node read it.
    pub fn latest(&self) -> crate::latest::Latest {
        self.0.latest.lock().unwrap().clone()
    }

    /// Keeps the file `one-dev update` publishes: unsealed in the bucket's `updates/`, by its hash. → the hash.
    pub async fn publish_file(&self, path: &Path) -> Result<(String, u64)> {
        use ::bucket::Bucket;
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let sha256 = crate::latest::sha256(&bytes);
        let bucket = self.bucket()?;
        Bucket::create(&bucket.place, &format!("{}/{sha256}", crate::latest::FOLDER), &bytes).await?;
        Ok((sha256, bytes.len() as u64))
    }

    /// The record as it is in the bucket, with its tag, to replace.
    pub async fn latest_record(&self) -> Result<Option<(String, String)>> {
        use ::bucket::Bucket;
        let bucket = self.bucket()?;
        Ok(Bucket::get(&bucket.place, &crate::latest::newest_name()).await?.map(|o| (String::from_utf8_lossy(&o.bytes).into_owned(), o.tag)))
    }

    /// Replaces the record, if it's still the one `tag` names (none: if there's none). → whether it did.
    pub async fn replace_latest(&self, text: &str, tag: Option<&str>) -> Result<bool> {
        use ::bucket::Bucket;
        let bucket = self.bucket()?;
        let was = self.latest();
        let replaced = Bucket::replace(&bucket.place, &crate::latest::newest_name(), text.as_bytes(), tag).await?;
        if replaced && let Ok(new) = crate::latest::Latest::read(crate::mode::here().release_key(), text) {
            for thing in new.things.iter().filter(|t| was.get(&t.name).is_none_or(|w| w.sha256 != t.sha256)) {
                self.device_signal("update.sent", Attrs::default().part(&thing.name), Status::Unset);
            }
        }
        if replaced {
            tidy(&bucket, crate::mode::here().release_key(), text).await.context("the record is published, but the builds no record names weren't removed from the bucket's updates/")?;
        }
        Ok(replaced)
    }

    /// The build of each service a device on this platform runs: the one its app's latest deploy names
    /// (`bundle::SERVICES`), else its newest release signed with the release key, as built-in apps' services come.
    pub async fn service_builds(&self) -> BTreeMap<String, crate::oned::Build> {
        use crate::oned::Build;
        let mut builds: BTreeMap<String, Build> = self.service_releases().into_iter().map(|(n, r)| (n, Build::Released(r))).collect();
        let bundles = self.with_or(Default::default(), |i| Ok(i.r.db().apps()?.into_iter().filter(|a| a.deleted.is_none()).filter_map(|a| Some((a.name, a.bundle?))).collect::<Vec<_>>()));
        for (app, bundle) in bundles {
            match self.blob(&bundle).await {
                Ok(bytes) => match crate::app::bundle::services(&bytes).remove(custom_build::config::TARGET) {
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
        let (prefix, suffix) = ("service/", format!("/{}", custom_build::config::TARGET));
        self.latest().things.into_iter().filter_map(|t| Some((t.name.strip_prefix(prefix)?.strip_suffix(&suffix)?.to_string(), t.clone()))).filter(|(n, _)| !n.contains('/')).collect()
    }
}

/// Once `text` is the record, keeps it as its publish's latest record, and removes from `updates/` each file that
/// neither the newest record nor the latest record of each of the `KEPT_PUBLISHES` publishes before it name, unless it
/// was written in the last `UPLOADING`. → how many objects went.
async fn tidy(bucket: &impl ::bucket::Bucket, key: &str, text: &str) -> Result<usize> {
    use crate::latest::{FOLDER, Latest};
    let read = |text: &[u8]| Latest::read(key, &String::from_utf8_lossy(text));
    let record = read(text.as_bytes()).context("the record published doesn't verify")?;
    let newest = bucket.get(&crate::latest::newest_name()).await?.context("the record isn't there")?;
    let mut keep: BTreeSet<String> = record.things.iter().chain(&read(&newest.bytes).context("the record there doesn't verify")?.things).map(|t| t.sha256.clone()).collect();
    let now = bucket.now();
    // A publish writes the record once for each thing it sends: its records are the ones whose newest thing is its commit.
    let publish = record.things.iter().max_by_key(|t| t.committed).map_or(String::new(), |t| format!("{:010}-{}", t.committed, t.commit));
    bucket.create(&format!("{FOLDER}/{PAST}{publish}-{now:016}"), text.as_bytes()).await?;
    let objects = bucket.objects(FOLDER).await?;
    let mut past: Vec<(&str, &str)> = objects.iter().filter_map(|(n, _)| Some((n.strip_prefix(PAST)?.rsplit_once('-')?.0, n.as_str()))).collect();
    past.sort();
    let mut latest: Vec<(&str, &str)> = past.iter().rev().copied().collect();
    latest.dedup_by_key(|(p, _)| *p);
    let kept: Vec<&str> = latest.iter().take(KEPT_PUBLISHES + 1).map(|(_, n)| *n).collect();
    for name in &kept {
        if let Some(record) = bucket.get(&format!("{FOLDER}/{name}")).await? {
            keep.extend(read(&record.bytes).with_context(|| format!("{FOLDER}/{name} doesn't verify"))?.things.into_iter().map(|t| t.sha256));
        }
    }
    let built = |n: &str| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit());
    let files = objects.iter().filter(|(n, written)| built(n) && !keep.contains(n) && written + UPLOADING <= now).map(|(n, _)| n.as_str());
    let records = past.iter().map(|(_, n)| *n).filter(|n| !kept.contains(n));
    let gone: Vec<String> = files.chain(records).map(|n| format!("{FOLDER}/{n}")).collect();
    for name in &gone {
        bucket.delete(name).await?;
    }
    Ok(gone.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::bucket::Place;
    use crate::latest::{Latest, Thing};
    use ssh_key::private::{Ed25519Keypair, PrivateKey};

    /// Publishing many times keeps what the newest record and the last record of each of the five publishes before
    /// it name, and what's just been uploaded for a record still to come, and removes the rest.
    #[tokio::test]
    async fn a_publish_removes_the_builds_no_recent_publish_names() {
        let dir = std::env::temp_dir().join(format!("one-tidy-{}", rand::random::<u64>()));
        let bucket = Place::Folder(dir.clone());
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        let public = key.public_key().to_openssh().unwrap();
        let file = |n: u8, old: bool| {
            let name = crate::latest::sha256(&[n]);
            let path = dir.join("updates").join(&name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [n]).unwrap();
            if old {
                std::fs::File::options().write(true).open(&path).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60)).unwrap();
            }
            name
        };
        let (mut record, mut tag) = (Latest::default(), None);
        for n in 0..10u8 {
            // Each publish sends two things, writing the record once for each.
            for (thing, at) in [("node/x", n), ("web", 50 + n)] {
                record = record.merged([Thing { name: thing.into(), commit: format!("c{n}"), committed: n as u64, sha256: file(at, true), size: 1, config: "-".into() }]);
                let text = record.signed(&key).unwrap();
                assert!(bucket.replace(&crate::latest::newest_name(), text.as_bytes(), tag.as_deref()).unwrap());
                tag = bucket.get_object(&crate::latest::newest_name()).unwrap().map(|o| o.tag);
                let (unnamed, uploading) = (file(100 + n, true), file(200 + n, false));
                tidy(&bucket, &public, &text).await.unwrap();
                assert!(!dir.join("updates").join(unnamed).exists(), "an old file no record names goes");
                assert!(dir.join("updates").join(uploading).exists(), "a file just uploaded stays: a record may yet name it");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }
        let left = |n: u8| dir.join("updates").join(crate::latest::sha256(&[n])).exists();
        assert!((4..10).all(|n| left(n) && left(50 + n)), "what the newest publish and the five before it sent stays");
        assert!(!(0..4).any(|n| left(n) || left(50 + n)), "what older ones sent goes");
        let past = std::fs::read_dir(dir.join("updates")).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(PAST)).count();
        assert_eq!(past, KEPT_PUBLISHES + 1, "one record of each publish still kept is");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
