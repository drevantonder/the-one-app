//! Releases of the node itself, and of the services in other languages it runs as its parts.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};

use crate::database::content::Content;

use crate::keys::Key;

pub const APP: &str = "node";
/// The kind of entry, or step, that says a release of the node.
pub const NODE_KIND: &str = "node-release";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub commit: String,
    pub target: String,
    pub blob: String,
    pub size: u64,
    /// When the commit was made, in seconds since 1970. A device never takes a release older than its build.
    #[serde(default)]
    pub committed: u64,
    /// The publisher's signature over the binary, as `ssh-keygen -Y sign -n release` makes it.
    #[serde(default)]
    pub signature: String,
    /// For a service in another language's release, which (`whatsapp`): its program, not the node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

/// The namespace release signatures are made in, so nothing else the key signs passes for a release.
pub const NAMESPACE: &str = "release";

/// A service's release is an entry of its own kind, which a device that doesn't know it ignores.
pub const SERVICE_KIND: &str = "service-release";

pub fn content(release: &Release) -> Content {
    let kind = if release.service.is_some() { SERVICE_KIND } else { NODE_KIND };
    Content { blobs: vec![release.blob.clone()], ..Content::new(APP, kind, json!(release)) }
}

/// Signs a binary as a release.
pub fn sign(key: &PrivateKey, bytes: &[u8]) -> Result<String> {
    Ok(key.sign(NAMESPACE, HashAlg::Sha512, bytes)?.to_pem(LineEnding::LF)?)
}

/// Checks that `signature` is `key`'s, over `bytes`.
pub fn verify(key: &str, bytes: &[u8], signature: &str) -> Result<()> {
    ensure!(signed_by(key, signature), "it isn't signed by this build's release key");
    let sig = SshSig::from_pem(signature)?;
    PublicKey::from_openssh(key)?.verify(NAMESPACE, bytes, &sig).map_err(|_| anyhow!("its signature doesn't match the binary"))
}

/// Whether a signature claims to be `key`'s, which can be told before the binary it's over is here.
fn signed_by(key: &str, signature: &str) -> bool {
    let (Ok(key), Ok(sig)) = (PublicKey::from_openssh(key), SshSig::from_pem(signature)) else { return false };
    sig.public_key() == key.key_data() && sig.namespace() == NAMESPACE
}

/// What `global` said of releases, oldest first, as each release's content and who published it, from its scope
/// database's steps (`ScopeDb::releases`).
pub fn said(steps: &[(crate::database::content::Content, Key, u64)]) -> Vec<(crate::database::content::Content, Key)> {
    steps.iter().map(|(c, k, _)| (c.clone(), *k)).collect()
}

/// The newest release for `target` signed with `key`, of the node or the service `service`, and the device that
/// published it, of what `said` gives. Any device may publish or pass on a release: only the signature counts. The
/// newest of any newer ones without it is logged.
pub fn latest_said(key: &str, said: &[(crate::database::content::Content, Key)], target: &str, service: Option<&str>) -> Option<(Release, Key)> {
    let kind = if service.is_some() { SERVICE_KIND } else { NODE_KIND };
    let releases = said.iter().rev().filter(|(c, _)| c.app == APP && c.kind == kind);
    let mut logged = false;
    for (content, author) in releases {
        let e = Said { content, author: *author };
        let Ok(r) = serde_json::from_value::<Release>(e.content.data.clone()) else { continue };
        if r.target != target || r.service.as_deref() != service {
            continue;
        }
        if signed_by(key, &r.signature) {
            return Some((r, e.author));
        }
        if !std::mem::replace(&mut logged, true) {
            tracing::warn!("ignoring the release of {} by {}: it isn't signed by this build's release key", r.commit, &crate::keys::hex(&e.author)[..8]);
        }
    }
    None
}

/// A release as `global` said it.
struct Said<'a> {
    content: &'a crate::database::content::Content,
    author: Key,
}

/// Whether a node running `build`, committed at `committed`, takes `r`: a different build, and not an older one.
pub fn wanted(r: &Release, build: &str, committed: u64) -> bool {
    r.commit != build && r.committed >= committed
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

/// Replaces the running binary with `bytes`, `r`'s signed binary, once it says it's that build and no older than this one.
/// The service manager starts the new one when this one exits.
pub fn install(r: &Release, bytes: &[u8]) -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    ensure!(!exe.to_string_lossy().contains(".app/Contents/"), "this node lives in an app bundle, which updates with the app");
    let tmp = exe.with_extension("new");
    std::fs::write(&tmp, bytes).context("write the new binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    // The entry's commit and date aren't signed; the binary's own are.
    let version = Version::of(&tmp);
    if !version.as_ref().is_some_and(|v| v.commit == r.commit && v.target == crate::config::TARGET && v.committed >= crate::config::COMMITTED) {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("the binary says it's {}, not {} for {}", version.map_or("nothing".into(), |v| v.json()), r.commit, crate::config::TARGET);
    }
    std::fs::rename(&tmp, &exe).context("replace the binary")?;
    Ok(exe)
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Running {
    pub commit: String,
    #[serde(default)]
    pub target: String,
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

    fn release(commit: &str, committed: u64, signature: String) -> Release {
        Release { commit: commit.into(), target: "t".into(), blob: "b".repeat(64), size: 3, committed, signature, service: None }
    }

    /// One device publishing each release in turn.
    fn said(releases: &[Release]) -> Vec<(Content, Key)> {
        releases.iter().map(|r| (content(r), [1; 32])).collect()
    }

    #[test]
    fn unsigned_and_wrongly_signed_releases_dont() {
        let (publisher, someone) = (key(1), key(2));
        let good = release("good", 20, sign(&publisher, b"bin").unwrap());
        let unsigned = release("unsigned", 30, String::new());
        let theirs = release("theirs", 40, sign(&someone, b"bin").unwrap());
        let garbled = release("garbled", 50, "-----BEGIN SSH SIGNATURE-----\nnope\n-----END SSH SIGNATURE-----\n".into());
        let all = said(&[good.clone(), unsigned, theirs.clone(), garbled]);
        assert_eq!(latest_said(&public(&publisher), &all, "t", None).unwrap().0.commit, "good", "newer releases without its signature don't count");
        assert!(latest_said(&public(&publisher), &all, "other target", None).is_none());
        // A service's release is its program's, never the node's, nor another service's.
        let whatsapp = Release { service: Some("whatsapp".into()), ..release("whatsapp", 60, sign(&publisher, b"go").unwrap()) };
        let all = [all, said(std::slice::from_ref(&whatsapp))].concat();
        assert_eq!(latest_said(&public(&publisher), &all, "t", None).unwrap().0.commit, "good");
        assert_eq!(latest_said(&public(&publisher), &all, "t", Some("whatsapp")).unwrap().0, whatsapp);
        assert!(latest_said(&public(&publisher), &all, "t", Some("push")).is_none());
        assert!(verify(&public(&publisher), b"bin", &theirs.signature).is_err());
        assert!(verify(&public(&publisher), b"something else", &good.signature).is_err(), "a signature over another binary");
        assert!(verify(&public(&publisher), b"bin", "").is_err());
        let elsewhere = key(1).sign("file", HashAlg::Sha512, b"bin").unwrap().to_pem(LineEnding::LF).unwrap();
        assert!(verify(&public(&publisher), b"bin", &elsewhere).is_err(), "the key's signatures for anything but releases");
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
