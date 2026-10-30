//! Releases of the node itself. See SPEC.md, "Releases" and "Blobs".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::entry::{Content, Entry, Key};
use crate::space::{Members, Role};

pub const APP: &str = "node";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub commit: String,
    pub target: String,
    pub blob: String,
    pub size: u64,
    /// When the commit was made, in seconds since 1970. A device never takes a release older than its build.
    #[serde(default)]
    pub committed: u64,
}

pub fn content(release: &Release) -> Content {
    Content { app: APP.into(), kind: "release".into(), data: json!(release) }
}

/// The newest release an admin published for `target`, and the device that published it.
pub fn latest(members: &Members, entries: &[Entry], target: &str) -> Option<(Release, Key)> {
    entries
        .iter()
        .filter(|e| e.content.app == APP && e.content.kind == "release")
        .filter(|e| members.active(&e.author(), e.order()).is_some_and(|m| m.role == Role::Admin))
        .filter_map(|e| serde_json::from_value::<Release>(e.content.data.clone()).ok().map(|r| (e.order(), r, e.author())))
        .filter(|(_, r, _)| r.target == target)
        .max_by_key(|(order, _, _)| *order)
        .map(|(_, r, author)| (r, author))
}

/// Blobs are files named by their BLAKE3 hash.
pub fn blob_path(dir: &Path, hash: &str) -> Result<PathBuf> {
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "not a blob hash");
    Ok(dir.join("blobs").join(hash))
}

pub fn put_blob(dir: &Path, bytes: &[u8]) -> Result<String> {
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

/// Replaces the running binary with `bytes`. The service manager starts the new one when this one exits.
pub fn install(bytes: &[u8]) -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    ensure!(!exe.to_string_lossy().contains(".app/Contents/"), "this node lives in an app bundle, which updates with the app");
    let tmp = exe.with_extension("new");
    std::fs::write(&tmp, bytes).context("write the new binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &exe).context("replace the binary")?;
    Ok(exe)
}

/// Says which build a device runs, so everyone can see when a release has landed.
pub fn running() -> Content {
    Content { app: APP.into(), kind: "running".into(), data: json!({ "commit": crate::brand::BUILD, "target": crate::brand::TARGET }) }
}

/// The build each device last said it runs.
pub fn builds(entries: &[Entry]) -> std::collections::HashMap<Key, String> {
    let mut out = std::collections::HashMap::new();
    for e in entries.iter().filter(|e| e.content.app == APP && e.content.kind == "running") {
        if let Some(c) = e.content.data["commit"].as_str() {
            out.insert(e.author(), c.to_string());
        }
    }
    out
}
