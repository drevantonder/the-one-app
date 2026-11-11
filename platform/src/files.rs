//! A small JSON file an app runtime keeps beside its data (pairings, offered services, hook times). Absent is what a first run
//! has, so it reads as the default; a file that is there and can't be read says so in the log, never as empty.

use std::path::Path;

use serde::{Serialize, de::DeserializeOwned};

/// `path`'s JSON, or `T::default()` if there's no such file. A file that can't be read or parsed is logged and also
/// reads as the default, since what keeps it writes it again.
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            tracing::warn!("{} isn't what it should be, so it's read as empty: {err}", path.display());
            T::default()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(err) => {
            tracing::warn!("couldn't read {}: {err}", path.display());
            T::default()
        }
    }
}

/// Writes `value` as `path`'s JSON. A file that can't be written is logged: what asked has nothing to do about it.
pub fn write_json(path: &Path, value: &impl Serialize) {
    let written = serde_json::to_vec(value).map_err(std::io::Error::other).and_then(|bytes| std::fs::write(path, bytes));
    if let Err(err) = written {
        tracing::warn!("couldn't write {}: {err}", path.display());
    }
}
