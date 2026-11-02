//! The install's bucket as a server on a machine reads it: every database step and copy it reads or writes is kept on
//! its own disk too, sealed as it is in the bucket, so it reads each from the bucket once. Each is written once, at a
//! name its seal binds it to, so what's kept here can only be behind the bucket, never different from it; holding it
//! opens nothing without the key. Everything else (keys, hints, leases) is read from the bucket each time.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, bail, ensure};

use super::{Bucket, Object, Place};

#[derive(Clone, Debug)]
pub struct Copied<B = Place> {
    /// The install's bucket.
    pub place: B,
    /// This server's copy: a folder of the same names.
    here: Place,
    /// Whether a name this server's copy lacks is looked for in the bucket.
    reach: bool,
    /// Where the install's data went, if it moved since `place` was taken: held to write, and taken whole while it
    /// moves, so a write waits for the move and then goes where the data is, never only to where it was.
    moved: Arc<tokio::sync::RwLock<Option<B>>>,
}

impl<B: Bucket + Clone> Copied<B> {
    pub fn new(place: B, dir: PathBuf) -> Copied<B> {
        Copied::moving(place, dir, Arc::default())
    }

    /// The bucket `place` and this server's copy in `dir`, following each move of the install's data in `moved`.
    pub fn moving(place: B, dir: PathBuf, moved: Arc<tokio::sync::RwLock<Option<B>>>) -> Copied<B> {
        Copied { place, here: Place::Folder(dir), reach: true, moved }
    }

    /// Only this server's copy: what it lacks isn't there, and nothing is written.
    pub fn here_only(&self) -> Copied<B> {
        Copied { reach: false, ..self.clone() }
    }

    /// A kept object, without asking the bucket.
    pub fn kept(&self, name: &str) -> Result<Option<Vec<u8>>> {
        if !kept(name) {
            return Ok(None);
        }
        self.here.get(name)
    }

    /// Forgets what's kept of a folder: a scope this server no longer holds, or an app erased.
    pub fn forget(&self, folder: &str) -> Result<()> {
        ensure!(!folder.is_empty() && folder.split('/').all(segment), "{folder} isn't a folder of the bucket");
        let Place::Folder(dir) = &self.here else { unreachable!("a copy is a folder") };
        match std::fs::remove_dir_all(dir.join(folder)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// The folders of scopes kept here.
    pub fn folders(&self) -> Result<Vec<String>> {
        self.here.list("")
    }

    /// Fetches into this copy a database's steps after `at` that the bucket has: how many.
    pub async fn fill(&self, prefix: &str, at: u64) -> Result<u64> {
        let mut n = at;
        while Bucket::get(self, &crate::db::step_name(prefix, n + 1)).await?.is_some() {
            n += 1;
        }
        Ok(n - at)
    }
}

/// Whether an object is written once and kept here: a database's step or copy, named by its 12-digit position.
fn kept(name: &str) -> bool {
    let mut segments = name.rsplit('/');
    segments.next().is_some_and(|last| last.len() == 12 && last.bytes().all(|b| b.is_ascii_digit())) && name.split('/').count() > 1 && name.split('/').all(segment)
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
        let got = Bucket::get(moved.as_deref().and_then(Option::as_ref).unwrap_or(&self.place), name).await?;
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
        let created = Bucket::create(moved.as_ref().unwrap_or(&self.place), name, bytes).await?;
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
        Bucket::replace(moved.as_ref().unwrap_or(&self.place), name, bytes, tag).await
    }

    async fn wait(&self, ms: u64) {
        self.place.wait(ms).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write made while the install's data moves waits, then goes where the data went: none is left behind.
    #[tokio::test]
    async fn a_write_during_a_move_goes_to_the_new_bucket() {
        let dir = std::env::temp_dir().join(format!("copied-move-{}", rand::random::<u64>()));
        let (old, new) = (Place::Folder(dir.join("old")), Place::Folder(dir.join("new")));
        let moved: Arc<tokio::sync::RwLock<Option<Place>>> = Arc::default();
        let copied = Copied::moving(old.clone(), dir.join("copy"), moved.clone());
        let name = "global/node/log/000000000001";
        let mut moving = moved.write().await;
        let write = tokio::spawn({
            let copied = copied.clone();
            async move { Bucket::create(&copied, name, b"written while it moved").await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!write.is_finished(), "a write waits while the data moves");
        *moving = Some(new.clone());
        drop(moving);
        assert!(write.await.unwrap().unwrap());
        assert!(new.get(name).unwrap().is_some() && old.get(name).unwrap().is_none(), "it went to the new bucket only");
        assert!(Bucket::get(&copied.here_only(), name).await.unwrap().is_some(), "and is kept in this server's copy");
        let _ = std::fs::remove_dir_all(dir);
    }
}
