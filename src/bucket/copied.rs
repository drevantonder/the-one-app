//! The network's bucket as a device on a machine reads it: every database step and copy it reads or writes is kept on
//! its own disk too, sealed as it is in the bucket, so it reads each from the bucket once. Each is written once, at a
//! name its seal binds it to, so what's kept here can only be behind the bucket, never different from it; holding it
//! opens nothing without the key. Everything else (keys, hints, leases) is read from the bucket each time.
//!
//! Devices tell each other a "something changed" notice over iroh, which is only a hint: each reads what changed from
//! the bucket, and every 10 seconds fills its copy from the bucket anyway, for a notice that was lost. A read uses the
//! copy, and asks the bucket first only after a notice whose step it hasn't fetched yet. A notice of a write carries its
//! step (up to a megabyte), which a device keeps only once the bucket confirms it holds those bytes (`take_sent`), so no
//! device can show another anything but the bucket's.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, bail, ensure};

use super::{Bucket, Object, Place};

#[derive(Clone, Debug)]
pub struct Copied<B = Place> {
    pub place: B,
    /// This device's copy: a folder of the same names.
    here: Place,
    /// Whether a name this device's copy lacks is looked for in the bucket.
    reach: bool,
    /// Where the network's data went, if it moved since `place` was taken: held to write, and taken whole while it
    /// moves, so a write waits for the move and then goes where the data is, never only to where it was.
    moved: Arc<tokio::sync::RwLock<Option<B>>>,
}

impl<B: Bucket + Clone> Copied<B> {
    pub fn new(place: B, dir: PathBuf) -> Copied<B> {
        Copied::moving(place, dir, Arc::default())
    }

    pub fn moving(place: B, dir: PathBuf, moved: Arc<tokio::sync::RwLock<Option<B>>>) -> Copied<B> {
        Copied { place, here: Place::Folder(dir), reach: true, moved }
    }

    /// Only this device's copy: what it lacks isn't there, and nothing is written.
    pub fn here_only(&self) -> Copied<B> {
        Copied { reach: false, ..self.clone() }
    }

    /// Forgets the steps other devices sent before they stopped sending them, whose notes `.sent/` keeps, unchecked
    /// against the bucket: each is read from it again when it's next needed.
    pub fn forget_sent(&self) -> Result<()> {
        let Place::Folder(dir) = &self.here else { unreachable!("a copy is a folder") };
        let sent = Place::Folder(dir.join(".sent"));
        if !dir.join(".sent").exists() {
            return Ok(());
        }
        for name in sent.names()? {
            self.here.delete(&name)?;
        }
        std::fs::remove_dir_all(dir.join(".sent"))?;
        Ok(())
    }

    /// A kept object, without asking the bucket.
    pub fn kept(&self, name: &str) -> Result<Option<Vec<u8>>> {
        if !kept(name) {
            return Ok(None);
        }
        self.here.get(name)
    }

    /// Keeps the steps other devices sent, each a name and its bytes, that the bucket confirms it holds as sent, asking
    /// it about all of them at once: → which are kept. One it doesn't confirm is never kept, so no device can show
    /// another anything but the bucket's: it's read from the bucket when it's next needed.
    pub async fn take_sent(&self, sent: &[(String, Vec<u8>)]) -> Result<Vec<bool>> {
        let mut out: Vec<bool> = sent.iter().map(|(name, _)| self.kept(name).is_ok_and(|k| k.is_some())).collect();
        let ask: Vec<(String, Vec<u8>)> = sent.iter().zip(&out).filter(|((name, _), here)| !**here && kept(name)).map(|(s, _)| s.clone()).collect();
        if ask.is_empty() {
            return Ok(out);
        }
        let mut confirmed = Bucket::confirm(self, &ask).await?.into_iter();
        for ((name, bytes), took) in sent.iter().zip(out.iter_mut()) {
            if !*took && kept(name) && confirmed.next() == Some(true) {
                // Kept already if not created: from the bucket too.
                self.here.create(name, bytes)?;
                *took = true;
            }
        }
        Ok(out)
    }

    /// Forgets what's kept of a folder: a scope this device no longer holds, or an app erased.
    pub fn forget(&self, folder: &str) -> Result<()> {
        ensure!(!folder.is_empty() && folder.split('/').all(segment), "{folder} isn't a folder of the bucket");
        let Place::Folder(dir) = &self.here else { unreachable!("a copy is a folder") };
        match std::fs::remove_dir_all(dir.join(folder)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn folders(&self) -> Result<Vec<String>> {
        self.here.list("")
    }

    /// Fetches into this copy a database's steps after `at` that the bucket has, and the signature beside each: how
    /// many steps.
    pub async fn fill(&self, prefix: &str, at: u64) -> Result<u64> {
        let mut n = at;
        while Bucket::get(self, &crate::database::stored::step_name(prefix, n + 1)).await?.is_some() {
            if crate::database::stored::signed_beside(prefix) {
                Bucket::get(self, &crate::database::stored::sig_name(prefix, n + 1)).await?;
            }
            n += 1;
        }
        Ok(n - at)
    }
}

/// Whether an object is written once and kept here: a database's step, the signature beside one, or a copy, named by
/// its 12-digit position.
fn kept(name: &str) -> bool {
    let mut segments = name.rsplit('/');
    let position = |last: &str| last.len() == crate::protocol::STEP_DIGITS && last.bytes().all(|b| b.is_ascii_digit());
    segments.next().is_some_and(|last| position(last.strip_suffix(".sig").unwrap_or(last))) && name.split('/').count() > 1 && name.split('/').all(segment)
}

/// A name's part that's safe as a file's.
fn segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

impl<B: Bucket> Bucket for Copied<B> {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        if kept(name) {
            if let Some(o) = Bucket::get(&self.here, name).await? {
                return Ok(Some(o));
            }
        }
        if !self.reach {
            return Ok(None);
        }
        tracing::trace!(target: "bucket", "get {name}");
        // While the data moves, what it moves from is still all of it.
        let moved = self.moved.try_read().ok();
        let asking = std::time::Instant::now();
        let got = Bucket::get(moved.as_deref().and_then(Option::as_ref).unwrap_or(&self.place), name).await?;
        crate::node::trace::bucket("read", asking.elapsed(), got.is_some());
        if let Some(o) = got.as_ref().filter(|_| kept(name)) {
            if let Err(err) = self.here.create(name, &o.bytes) {
                tracing::debug!("keeping {name}: {err:#}");
            }
        }
        Ok(got)
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        if !self.reach {
            bail!("this device's copy of the bucket is only read");
        }
        tracing::trace!(target: "bucket", "create {name}");
        let moved = self.moved.read().await;
        let asking = std::time::Instant::now();
        let created = Bucket::create(moved.as_ref().unwrap_or(&self.place), name, bytes).await?;
        crate::node::trace::bucket("create", asking.elapsed(), created);
        if created && kept(name) {
            if let Err(err) = self.here.create(name, bytes) {
                tracing::debug!("keeping {name}: {err:#}");
            }
        }
        Ok(created)
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        if !self.reach {
            bail!("this device's copy of the bucket is only read");
        }
        tracing::trace!(target: "bucket", "replace {name}");
        let moved = self.moved.read().await;
        let asking = std::time::Instant::now();
        let replaced = Bucket::replace(moved.as_ref().unwrap_or(&self.place), name, bytes, tag).await?;
        crate::node::trace::bucket("replace", asking.elapsed(), replaced);
        Ok(replaced)
    }

    async fn wait(&self, ms: u64) {
        self.place.wait(ms).await
    }

    fn now(&self) -> u64 {
        self.place.now()
    }

    async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
        let moved = self.moved.read().await;
        Bucket::objects(moved.as_ref().unwrap_or(&self.place), folder).await
    }

    /// From the bucket, and from this device's copy.
    async fn delete(&self, name: &str) -> Result<()> {
        if !self.reach {
            bail!("this device's copy of the bucket is only read");
        }
        let moved = self.moved.read().await;
        Bucket::delete(moved.as_ref().unwrap_or(&self.place), name).await?;
        self.here.delete(name)
    }

    /// Asks the bucket, never this copy.
    async fn confirm(&self, objects: &[(String, Vec<u8>)]) -> Result<Vec<bool>> {
        let moved = self.moved.try_read().ok();
        Bucket::confirm(moved.as_deref().and_then(Option::as_ref).unwrap_or(&self.place), objects).await
    }
}
