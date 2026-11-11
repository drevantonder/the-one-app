//! Releases of the app runtime itself, and of the services in other languages it runs as its parts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// Whether this program runs from inside an app bundle: the copy of it a Mac's window carries only because builds before
/// it check it's there as they update (`native-apps/mac/build.sh`). Nothing runs it, and nothing updates it.
pub fn in_app_bundle() -> bool {
    std::env::current_exe().and_then(|e| e.canonicalize()).is_ok_and(|e| e.to_string_lossy().contains(".app/Contents/"))
}

/// Replaces the running binary with `bytes`, once it says it's `commit` and no older than this one. By a rename, so the
/// path holds the whole old program or the whole new one, however this ends.
pub fn install_commit(commit: &str, bytes: &[u8]) -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    // Linux names a running program's file that was replaced since it started "<path> (deleted)": the path is still
    // where it's installed.
    let exe = exe.to_str().and_then(|p| p.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe).canonicalize().context("find where this program is installed")?;
    ensure!(!in_app_bundle(), "this is the copy of {} inside the Mac app, which runs nothing: open the app, which sets up {} on this Mac, then update with that", custom_build::config::CLI, custom_build::config::CLI);
    // Named for this process, since `one update` and a device's own check can both be installing at once.
    let tmp = exe.with_extension(format!("new.{}", std::process::id()));
    let placed = placed_at(&tmp, commit, bytes).and_then(|()| platform::os::replace_program(&tmp, &exe).context("replace the binary"));
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed?;
    Ok(exe)
}

/// Writes `bytes` to `tmp` as a program, on disk before it's renamed, and checks it says it's `commit`.
fn placed_at(tmp: &Path, commit: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(tmp).context("write the new binary")?;
    file.write_all(bytes).context("write the new binary")?;
    // A power cut after the rename must not leave an empty file, as a filesystem may do with data not yet on disk.
    file.sync_all().context("write the new binary")?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    // The release step's commit and date aren't signed; the binary's own are.
    let version = Version::of(tmp);
    if !version.as_ref().is_some_and(|v| v.commit == commit && v.target == custom_build::config::TARGET && v.committed >= custom_build::config::COMMITTED) {
        anyhow::bail!("the binary says it's {}, not {} for {}", version.map_or("nothing".into(), |v| v.json()), commit, custom_build::config::TARGET);
    }
    Ok(())
}

/// What a build says it is: `version --json`, which publishing, an update's check and installing a release all read.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Version {
    pub commit: String,
    pub target: String,
    /// When the commit was made, in seconds since 1970.
    pub committed: u64,
}

impl Version {
    /// This build.
    pub fn this() -> Version {
        Version { commit: custom_build::config::BUILD.into(), target: custom_build::config::TARGET.into(), committed: custom_build::config::COMMITTED }
    }

    /// What the program at `path` says it is: none if it can't run here (another platform's) or isn't one of ours.
    pub fn of(path: &Path) -> Option<Version> {
        let out = std::process::Command::new(path).args(["version", "--json"]).output().ok().filter(|o| o.status.success())?;
        serde_json::from_slice(&out.stdout).ok()
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
