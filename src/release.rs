//! Releases of the node itself, and of the services in other languages it runs as its parts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use ssh_key::{HashAlg, LineEnding, PrivateKey};

/// The namespace a release's GitHub assets are signed in, which `install.sh` checks.
pub const NAMESPACE: &str = "release";

pub fn sign(key: &PrivateKey, bytes: &[u8]) -> Result<String> {
    Ok(key.sign(NAMESPACE, HashAlg::Sha512, bytes)?.to_pem(LineEnding::LF)?)
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

/// Whether this program runs from inside an app bundle, which updates as a whole, by what `keep-mac-current` installs.
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
    ensure!(!in_app_bundle(), "this node lives in an app bundle, which updates with the app");
    // Named for this process, since `one update` and a device's own check can both be installing at once.
    let tmp = exe.with_extension(format!("new.{}", std::process::id()));
    let placed = placed_at(&tmp, commit, bytes).and_then(|()| crate::os::replace_program(&tmp, &exe).context("replace the binary"));
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
    if !version.as_ref().is_some_and(|v| v.commit == commit && v.target == crate::config::TARGET && v.committed >= crate::config::COMMITTED) {
        anyhow::bail!("the binary says it's {}, not {} for {}", version.map_or("nothing".into(), |v| v.json()), commit, crate::config::TARGET);
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
        Version { commit: crate::config::BUILD.into(), target: crate::config::TARGET.into(), committed: crate::config::COMMITTED }
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


#[cfg(test)]
mod tests {
    use super::*;
    use ssh_key::private::Ed25519Keypair;

    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    fn public(key: &PrivateKey) -> String {
        key.public_key().to_openssh().unwrap()
    }

    /// install.sh's own `fetch`, from a folder of files (curl reads file:// too), as the installer runs it.
    #[test]
    fn the_installers_fetch_checks_them() {
        let publisher = key(1);
        let dir = std::env::temp_dir().join(format!("one-release-{}", rand::random::<u32>()));
        std::fs::create_dir_all(dir.join("served")).unwrap();
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        std::fs::write(dir.join("served/bin"), b"bin").unwrap();
        std::fs::write(dir.join("served/bin.sig"), sign(&publisher, b"bin").unwrap()).unwrap();
        let installer = include_str!("../install.sh");
        let function = &installer[installer.find("fetch() {").unwrap()..];
        let function = &function[..function.find("\n}\n").unwrap() + 3];
        let fetch = |file: &str| {
            let sh = format!("{function}\ntmp={tmp}; url=file://{served}; key='{key}'; fetch {file}", tmp = dir.join("tmp").display(), served = dir.join("served").display(), key = public(&publisher));
            std::process::Command::new("sh").args(["-c", &sh]).output().unwrap().status.success()
        };
        assert!(fetch("bin"));
        std::fs::write(dir.join("served/bin"), b"bim").unwrap();
        assert!(!fetch("bin"), "a download that isn't what was signed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

}
