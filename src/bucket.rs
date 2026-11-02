//! The bucket: where an install keeps what it holds, as objects by name, in a place: a folder on one machine's disk,
//! or S3-compatible storage. The install's own bucket must write an object only if none has its name, and replace one
//! only while it's unchanged, so two servers writing at once never both win; backups' places only need to keep what
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
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`, which every disk takes.
pub fn folder(scope: &str) -> String {
    scope.replacen(':', "-", 1)
}

/// The scope a folder is.
pub fn scope_of(folder: &str) -> String {
    folder.replacen('-', ":", 1)
}

#[cfg(not(target_family = "wasm"))]
mod place;
#[cfg(not(target_family = "wasm"))]
pub use place::*;
