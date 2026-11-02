//! The bucket: where an install keeps what it holds, as objects by name, in a place: a folder on one machine's disk,
//! or S3-compatible storage. The install's own bucket must write an object only if none has its name, and replace one
//! only while it's unchanged, so two devices writing at once never both win; backups' places only need to keep what
//! they're sent.

use anyhow::Result;

/// An object as it was read: its bytes, and the tag `replace` names it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub tag: String,
}

/// What everything that keeps an install's data reads and writes it through, here or on another runtime.
pub trait Bucket {
    /// The object with this name, if there is one.
    fn get(&self, name: &str) -> impl Future<Output = Result<Option<Object>>>;
    /// Writes an object only if none has its name: false if one does.
    fn create(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<bool>>;
    /// Replaces an object only while it's the one `tag` names, or with none, only if there's none: false if not.
    fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> impl Future<Output = Result<bool>>;
    /// Waits before asking again, on the clock of the runtime it's reached from.
    fn wait(&self, ms: u64) -> impl Future<Output = ()>;
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`, which every disk takes.
pub fn folder(scope: &str) -> String {
    scope.replacen(':', "-", 1)
}

/// Where a blob named in `scope` is kept in the bucket, by its hash: under the scope's node folder, sealed with the key
/// of the node's database `blobs` there. A deploy's bundle and a release's binary are `global`'s.
pub fn blob_name(scope: &str, hash: &str) -> String {
    format!("{}/{}/{BLOBS}/{hash}", folder(scope), crate::members::APP)
}

pub const BLOBS: &str = "blobs";

/// The scope a folder is.
pub fn scope_of(folder: &str) -> String {
    folder.replacen('-', ":", 1)
}

#[cfg(not(target_family = "wasm"))]
mod copied;
#[cfg(not(target_family = "wasm"))]
mod place;
#[cfg(not(target_family = "wasm"))]
pub use copied::Copied;
#[cfg(not(target_family = "wasm"))]
pub use place::*;

/// A bucket in memory that can lose a race on purpose.
#[cfg(test)]
#[derive(Default)]
pub struct Memory {
    pub objects: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    /// Names another device takes just before this one writes them.
    pub taken: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
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
