//! Releases of the node itself. See SPEC.md, "Releases" and "Blobs".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};

use crate::entry::{Content, Entry, Key};

pub const APP: &str = "node";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub commit: String,
    pub target: String,
    pub blob: String,
    pub size: u64,
    /// When the commit was made, in seconds since 1970. A server never takes a release older than its build.
    #[serde(default)]
    pub committed: u64,
    /// The publisher's signature over the binary, as `ssh-keygen -Y sign -n release` makes it.
    #[serde(default)]
    pub signature: String,
}

/// The namespace release signatures are made in, so nothing else the key signs passes for a release.
pub const NAMESPACE: &str = "release";

pub fn content(release: &Release) -> Content {
    Content { blobs: vec![release.blob.clone()], ..Content::new(APP, "node-release", json!(release)) }
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

/// The newest release for `target` signed with `key`, and the server that published it. Any server may publish or pass
/// on a release: only the signature counts. The newest of any newer ones without it is logged.
pub fn latest(key: &str, entries: &[Entry], target: &str) -> Option<(Release, Key)> {
    let releases = entries.iter().rev().filter(|e| e.content.app == APP && e.content.kind == "node-release");
    let mut logged = false;
    for e in releases {
        let Ok(r) = serde_json::from_value::<Release>(e.content.data.clone()) else { continue };
        if r.target != target {
            continue;
        }
        if signed_by(key, &r.signature) {
            return Some((r, e.author()));
        }
        if !std::mem::replace(&mut logged, true) {
            tracing::warn!("ignoring the release of {} by {}: it isn't signed by this build's release key", r.commit, &crate::entry::hex(&e.author())[..8]);
        }
    }
    None
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
    let version = std::process::Command::new(&tmp).args(["version", "--json"]).output().ok().and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok());
    let is = |v: &serde_json::Value| v["commit"] == r.commit.as_str() && v["target"] == crate::brand::TARGET && v["committed"].as_u64().is_some_and(|c| c >= crate::brand::COMMITTED);
    if !version.as_ref().is_some_and(is) {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("the binary says it's {}, not {} for {}", version.map_or("nothing".into(), |v| v.to_string()), r.commit, crate::brand::TARGET);
    }
    std::fs::rename(&tmp, &exe).context("replace the binary")?;
    Ok(exe)
}

/// Says which build a server runs, so everyone can see when a release has landed.
pub fn running() -> Content {
    Content::new(APP, "running", json!({ "commit": crate::brand::BUILD, "target": crate::brand::TARGET }))
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Running {
    pub commit: String,
    #[serde(default)]
    pub target: String,
}

/// The build each server last said it runs.
pub fn builds(entries: &[Entry]) -> std::collections::HashMap<Key, Running> {
    let mut out = std::collections::HashMap::new();
    for e in entries.iter().filter(|e| e.content.app == APP && e.content.kind == "running") {
        if let Ok(r) = serde_json::from_value::<Running>(e.content.data.clone()) {
            out.insert(e.author(), r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2panda_core::SigningKey;
    use ssh_key::private::Ed25519Keypair;

    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    fn public(key: &PrivateKey) -> String {
        key.public_key().to_openssh().unwrap()
    }

    fn release(commit: &str, committed: u64, signature: String) -> Release {
        Release { commit: commit.into(), target: "t".into(), blob: "b".repeat(64), size: 3, committed, signature }
    }

    /// One server publishing each release in turn.
    fn entries(releases: &[Release]) -> Vec<Entry> {
        let device = SigningKey::from([1; 32]);
        let mut out: Vec<Entry> = Vec::new();
        for (i, r) in releases.iter().enumerate() {
            let ext = crate::entry::Ext { log: "global".into(), time: i as u64 + 1, user: "a000000000000001".into(), by: None };
            let prev = out.last().map(Entry::head);
            out.push(Entry::create(&device, ext, prev, content(r)));
        }
        out
    }

    #[test]
    fn a_correctly_signed_release_installs() {
        let publisher = key(1);
        let r = release("new", 20, sign(&publisher, b"bin").unwrap());
        assert_eq!(latest(&public(&publisher), &entries(&[r.clone()]), "t").unwrap().0, r);
        assert!(wanted(&r, "old", 10));
        verify(&public(&publisher), b"bin", &r.signature).unwrap();
    }

    #[test]
    fn unsigned_and_wrongly_signed_releases_dont() {
        let (publisher, someone) = (key(1), key(2));
        let good = release("good", 20, sign(&publisher, b"bin").unwrap());
        let unsigned = release("unsigned", 30, String::new());
        let theirs = release("theirs", 40, sign(&someone, b"bin").unwrap());
        let garbled = release("garbled", 50, "-----BEGIN SSH SIGNATURE-----\nnope\n-----END SSH SIGNATURE-----\n".into());
        let all = entries(&[good.clone(), unsigned, theirs.clone(), garbled]);
        assert_eq!(latest(&public(&publisher), &all, "t").unwrap().0.commit, "good", "newer releases without its signature don't count");
        assert!(latest(&public(&publisher), &all, "other target").is_none());
        assert!(verify(&public(&publisher), b"bin", &theirs.signature).is_err());
        assert!(verify(&public(&publisher), b"something else", &good.signature).is_err(), "a signature over another binary");
        assert!(verify(&public(&publisher), b"bin", "").is_err());
        let elsewhere = key(1).sign("file", HashAlg::Sha512, b"bin").unwrap().to_pem(LineEnding::LF).unwrap();
        assert!(verify(&public(&publisher), b"bin", &elsewhere).is_err(), "the key's signatures for anything but releases");
    }

    #[test]
    fn ssh_keygen_checks_them_as_the_installer_does() {
        let publisher = key(1);
        let dir = std::env::temp_dir().join(format!("one-release-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bin"), b"bin").unwrap();
        std::fs::write(dir.join("bin.sig"), sign(&publisher, b"bin").unwrap()).unwrap();
        std::fs::write(dir.join("signers"), format!("release namespaces=\"release\" {}\n", public(&publisher))).unwrap();
        let check = |file: &str| {
            let sh = format!("ssh-keygen -Y verify -f signers -I release -n release -s bin.sig < {file}");
            std::process::Command::new("sh").current_dir(&dir).args(["-c", &sh]).output().unwrap().status.success()
        };
        assert!(check("bin"));
        std::fs::write(dir.join("tampered"), b"bim").unwrap();
        assert!(!check("tampered"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nor_do_older_ones() {
        let r = release("older", 10, sign(&key(1), b"bin").unwrap());
        assert!(!wanted(&r, "mine", 20));
        assert!(!wanted(&r, "older", 10), "nor the build it runs");
        assert!(wanted(&r, "mine", 10), "another build of the same moment is fine");
    }
}
