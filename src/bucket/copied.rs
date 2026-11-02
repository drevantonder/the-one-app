//! The install's bucket as a server on a machine reads it: every database step and copy it reads or writes is kept on
//! its own disk too, sealed as it is in the bucket, so it reads each from the bucket once. Each is written once, at a
//! name its seal binds it to, so what's kept here can only be behind the bucket, never different from it; holding it
//! opens nothing without the key. Everything else (keys, hints, leases) is read from the bucket each time.

use std::path::PathBuf;

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
}

impl<B: Bucket + Clone> Copied<B> {
    pub fn new(place: B, dir: PathBuf) -> Copied<B> {
        Copied { place, here: Place::Folder(dir), reach: true }
    }

    /// Only this server's copy: what it lacks isn't there, and nothing is written.
    pub fn here_only(&self) -> Copied<B> {
        Copied { reach: false, ..self.clone() }
    }

    /// Keeps an object another server read or wrote, unless one with its name is kept already.
    pub fn keep(&self, name: &str, bytes: &[u8]) -> Result<()> {
        ensure!(kept(name), "{name} isn't a step or copy of a database");
        self.here.create(name, bytes)?;
        Ok(())
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
        let got = Bucket::get(&self.place, name).await?;
        if let Some(o) = got.as_ref().filter(|_| kept(name)) {
            if let Err(err) = self.here.create(name, &o.bytes) {
                tracing::debug!("keeping {name}: {err:#}");
            }
        }
        Ok(got)
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        if !self.reach {
            bail!("this server's copy of the bucket is only read");
        }
        tracing::trace!(target: "bucket", "create {name}");
        let created = Bucket::create(&self.place, name, bytes).await?;
        if created && kept(name) {
            if let Err(err) = self.here.create(name, bytes) {
                tracing::debug!("keeping {name}: {err:#}");
            }
        }
        Ok(created)
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        if !self.reach {
            bail!("this server's copy of the bucket is only read");
        }
        tracing::trace!(target: "bucket", "replace {name}");
        Bucket::replace(&self.place, name, bytes, tag).await
    }
}
