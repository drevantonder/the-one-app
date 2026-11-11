//! Databases: each scope's SQLite databases, every one kept in the bucket as signed steps and copies (`stored`),
//! and where the keys that open them are kept (`keys`). The scope database is `scope`'s.

pub mod keys;
pub mod stored;

use std::path::{Path, PathBuf};

/// Blobs are files named by their BLAKE3 hash, in a device's `dir`.
pub fn blob_path(dir: &Path, hash: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "not a blob hash");
    Ok(dir.join("blobs").join(hash))
}

pub fn put_blob(dir: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    let hash = blake3::hash(bytes).to_hex().to_string();
    let path = blob_path(dir, &hash)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    if !path.exists() {
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(hash)
}

/// What a client said it reads, in `protocol::header::READS`' value: its first number, 0 for nothing.
pub fn reads_of(said: Option<&str>) -> u32 {
    said.and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
