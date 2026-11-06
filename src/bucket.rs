//! The bucket: where a network keeps what it holds, as objects by name, in a place: a folder on one machine's disk,
//! or S3-compatible storage. The network's own bucket must write an object only if none has its name, and replace one
//! only while it's unchanged, so two devices writing at once never both win; backups' places only need to keep what
//! they're sent.

use anyhow::Result;

/// An object as it was read: its bytes, and the tag `replace` names it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub tag: String,
}

/// What everything that keeps a network's data reads and writes it through, here or on another runtime.
pub trait Bucket {
    fn get(&self, name: &str) -> impl Future<Output = Result<Option<Object>>>;
    /// Writes an object only if none has its name: false if one does.
    fn create(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<bool>>;
    /// Replaces an object only while it's the one `tag` names, or with none, only if there's none: false if not.
    fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> impl Future<Output = Result<bool>>;
    /// Waits before asking again, on the clock of the runtime it's reached from.
    fn wait(&self, ms: u64) -> impl Future<Output = ()>;
    /// Now, in ms since 1970, on the clock of the runtime it's reached from; 0 where it has none, and what a reader
    /// stamped with it then never counts as recent.
    fn now(&self) -> u64 {
        0
    }
    /// The objects directly in a folder, each its name in it and when the bucket wrote it (ms): for tidying, where a
    /// place can be listed.
    fn objects(&self, _folder: &str) -> impl Future<Output = Result<Vec<(String, u64)>>> {
        async { anyhow::bail!("this bucket isn't listed from here") }
    }
    fn delete(&self, _name: &str) -> impl Future<Output = Result<()>> {
        async { anyhow::bail!("nothing is deleted from this bucket from here") }
    }
    /// Which of `objects` the bucket holds exactly as given, each a name and its bytes: read and compared, unless the
    /// place can tell in one trip.
    fn confirm(&self, objects: &[(String, Vec<u8>)]) -> impl Future<Output = Result<Vec<bool>>> {
        async move {
            let mut out = Vec::new();
            for (name, bytes) in objects {
                out.push(self.get(name).await?.is_some_and(|o| o.bytes == *bytes));
            }
            Ok(out)
        }
    }
}

/// What went wrong at a place, so a person can be told how to fix it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The place rejected the key.
    Key,
    /// It has no such bucket.
    Bucket,
    /// The key may not write there.
    Refused,
    /// The place couldn't be reached.
    Unreachable,
    /// A folder's disk couldn't be written or read.
    Disk,
}

impl Trouble {
    pub fn name(self) -> &'static str {
        match self {
            Trouble::Key => "key",
            Trouble::Bucket => "bucket",
            Trouble::Refused => "refused",
            Trouble::Unreachable => "unreachable",
            Trouble::Disk => "disk",
        }
    }

    pub fn parse(name: &str) -> Option<Trouble> {
        [Trouble::Key, Trouble::Bucket, Trouble::Refused, Trouble::Unreachable, Trouble::Disk].into_iter().find(|t| t.name() == name)
    }

    /// The trouble behind an error, if a place said what it was.
    pub fn of(err: &anyhow::Error) -> Option<Trouble> {
        err.chain().find_map(|e| e.downcast_ref::<Failed>()).map(|f| f.0)
    }
}

/// A place's failure, in words that follow the place's name ("rejected the key"), and what kind it is.
#[derive(Debug)]
pub struct Failed(pub Trouble, pub String);

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

impl std::error::Error for Failed {}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`, which every disk takes.
pub fn folder(scope: &str) -> String {
    scope.replacen(':', "-", 1)
}

/// Where a database of an app's in a scope keeps its steps, copies and hint (`database::stored`).
pub fn database_prefix(scope: &str, app: &str, database: &str) -> String {
    format!("{}/{app}/{database}", folder(scope))
}

/// Where a blob named in `scope` is kept in the bucket, by its hash: under the scope's node folder, sealed with the key
/// of the node's database `blobs` there. A deploy's bundle and a release's binary are `global`'s.
pub fn blob_name(scope: &str, hash: &str) -> String {
    format!("{}/{hash}", database_prefix(scope, crate::database::scopes::APP, BLOBS))
}

pub const BLOBS: &str = "blobs";

pub fn scope_of(folder: &str) -> String {
    folder.replacen('-', ":", 1)
}

#[cfg(feature = "node")]
mod copied;
#[cfg(feature = "node")]
mod place;
#[cfg(not(target_family = "wasm"))]
mod signed;
#[cfg(feature = "node")]
pub use copied::Copied;
#[cfg(not(target_family = "wasm"))]
pub use signed::{Address, Ask, Signer, signer_requests};
#[cfg(feature = "node")]
pub use place::*;

/// A bucket in memory that can lose a race on purpose.
#[cfg(test)]
#[derive(Default)]
pub struct Memory {
    pub objects: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    /// Names another device takes just before this one writes them.
    pub taken: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    /// What `now` says, in ms: 0, no clock, unless a test sets it.
    pub clock: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
impl Bucket for Memory {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        Ok(self.objects.lock().unwrap().get(name).map(|b| Object { bytes: b.clone(), tag: blake3::hash(b).to_hex().to_string() }))
    }
    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        let mut objects = self.objects.lock().unwrap();
        let mut taken = self.taken.lock().unwrap();
        if let Some(i) = taken.iter().position(|(n, _)| n == name) {
            let (n, b) = taken.remove(i);
            objects.insert(n, b);
        }
        if objects.contains_key(name) {
            return Ok(false);
        }
        objects.insert(name.into(), bytes.to_vec());
        Ok(true)
    }
    async fn wait(&self, _: u64) {}
    fn now(&self) -> u64 {
        self.clock.load(std::sync::atomic::Ordering::Relaxed)
    }
    async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
        let within = format!("{folder}/");
        Ok(self.objects.lock().unwrap().keys().filter_map(|n| n.strip_prefix(&within)).filter(|n| !n.contains('/')).map(|n| (n.to_string(), 0)).collect())
    }
    async fn delete(&self, name: &str) -> Result<()> {
        self.objects.lock().unwrap().remove(name);
        Ok(())
    }
    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        let mut objects = self.objects.lock().unwrap();
        let now = objects.get(name).map(|b| blake3::hash(b).to_hex().to_string());
        if now.as_deref() != tag {
            return Ok(false);
        }
        objects.insert(name.into(), bytes.to_vec());
        Ok(true)
    }
}
