//! The web client as one signed file, which the native apps load from the network in place of the copy built into them.
//! Whoever carries it (the Workers, the bucket) only carries it: an app takes a bundle only if the release key built
//! into it signed it, and only if it's newer than what it has. See native-apps/src/bundle.rs for what the app does.

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use ssh_key::{PublicKey, SshSig};
use std::io::Read;

/// The namespace bundle signatures are made in, so a signature on anything else the key signs doesn't pass for one.
pub const NAMESPACE: &str = "web";

#[derive(Serialize, Deserialize)]
struct Header {
    commit: String,
    committed: u64,
    shell: u32,
    files: Vec<(String, u64)>,
}

/// The web client of one commit, as an app takes it.
#[derive(Debug, PartialEq, Clone)]
pub struct Bundle {
    pub commit: String,
    /// When the commit was made, in seconds since 1970: an app never takes a bundle older than what it has.
    pub committed: u64,
    /// The shell it needs (`web::SHELL`).
    pub shell: u32,
    pub files: Vec<(String, Vec<u8>)>,
}

impl Bundle {
    /// This build's web client, ready to sign.
    #[cfg(feature = "node")]
    pub fn of_this_build(commit: &str, committed: u64) -> Bundle {
        Bundle { commit: commit.to_string(), committed, shell: super::SHELL, files: super::with_client() }
    }

    /// What says whether two are the same web client: their files, whatever commit each was made at.
    #[cfg(feature = "node")]
    pub fn files_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for (path, bytes) in &self.files {
            hash.update(format!("{path}\0{}\0", bytes.len()));
            hash.update(bytes);
        }
        data_encoding::HEXLOWER.encode(&hash.finalize())
    }

    /// As gzip, to sign and send.
    #[cfg(feature = "node")]
    pub fn packed(&self) -> Result<Vec<u8>> {
        use std::io::Write;
        let header = Header { commit: self.commit.clone(), committed: self.committed, shell: self.shell, files: self.files.iter().map(|(p, b)| (p.clone(), b.len() as u64)).collect() };
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&serde_json::to_vec(&header)?)?;
        gz.write_all(b"\n")?;
        for (_, bytes) in &self.files {
            gz.write_all(bytes)?;
        }
        Ok(gz.finish()?)
    }

    /// What `packed` made, whose signature was already checked, or whose hash the signed record names: never call this
    /// on bytes the release key hasn't vouched for.
    pub fn unpacked(packed: &[u8]) -> Result<Bundle> {
        let mut all = Vec::new();
        flate2::read::GzDecoder::new(packed).read_to_end(&mut all).context("it isn't a bundle")?;
        let at = all.iter().position(|&b| b == b'\n').context("it isn't a bundle")?;
        let header: Header = serde_json::from_slice(&all[..at]).context("it isn't a bundle")?;
        let mut rest = &all[at + 1..];
        let mut files = Vec::with_capacity(header.files.len());
        for (path, size) in header.files {
            ensure!(!path.split('/').any(|p| p == ".." || p.is_empty()) && !path.starts_with('/'), "it names a file outside itself");
            let size = usize::try_from(size)?;
            ensure!(size <= rest.len(), "it's cut short");
            let (bytes, after) = rest.split_at(size);
            files.push((path, bytes.to_vec()));
            rest = after;
        }
        ensure!(rest.is_empty(), "it has more in it than it says");
        Ok(Bundle { commit: header.commit, committed: header.committed, shell: header.shell, files })
    }
}

/// `packed` signed by the release key's private half, as an app takes it: its signature's length, the signature, then it.
#[cfg(feature = "node")]
pub fn signed(key: &ssh_key::PrivateKey, packed: &[u8]) -> Result<Vec<u8>> {
    let signature = key.sign(NAMESPACE, ssh_key::HashAlg::Sha512, packed)?.to_pem(ssh_key::LineEnding::LF)?;
    let mut file = (signature.len() as u32).to_le_bytes().to_vec();
    file.extend_from_slice(signature.as_bytes());
    file.extend_from_slice(packed);
    Ok(file)
}

/// The bundle in `file`, once the signature in it is `key`'s over the rest, in this namespace.
pub fn open(key: &str, file: &[u8]) -> Result<Bundle> {
    let split = |bytes: &[u8]| -> Option<(String, Vec<u8>)> {
        let len = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        Some((String::from_utf8(bytes.get(4..4 + len)?.to_vec()).ok()?, bytes.get(4 + len..)?.to_vec()))
    };
    let (signature, packed) = split(file).context("it isn't a signed bundle")?;
    let sig = SshSig::from_pem(&signature).map_err(|_| anyhow!("its signature can't be read"))?;
    let key = PublicKey::from_openssh(key).map_err(|_| anyhow!("this build's release key can't be read"))?;
    ensure!(sig.public_key() == key.key_data(), "it isn't signed by this build's release key");
    key.verify(NAMESPACE, &packed, &sig).map_err(|_| anyhow!("its signature doesn't match it"))?;
    Bundle::unpacked(&packed)
}

#[cfg(all(test, feature = "node"))]
mod tests {
    use super::*;
    use ssh_key::PrivateKey;
    use ssh_key::private::Ed25519Keypair;

    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    fn bundle() -> Bundle {
        Bundle { commit: "abc".into(), committed: 5, shell: 1, files: vec![("index.html".into(), b"<p>hi".to_vec()), ("a/b.js".into(), vec![0, 1, 2])] }
    }

    #[test]
    fn only_what_the_release_key_signed_opens() {
        let (ours, theirs) = (key(1), key(2));
        let public = ours.public_key().to_openssh().unwrap();
        let packed = bundle().packed().unwrap();
        assert_eq!(open(&public, &signed(&ours, &packed).unwrap()).unwrap(), bundle());
        assert!(open(&public, &signed(&theirs, &packed).unwrap()).is_err(), "someone else's key");
        let mut altered = signed(&ours, &packed).unwrap();
        *altered.last_mut().unwrap() ^= 1;
        assert!(open(&public, &altered).is_err(), "a change after signing");
        assert!(open(&public, &packed).is_err(), "no signature");
        let release = ours.sign("release", ssh_key::HashAlg::Sha512, &packed).unwrap().to_pem(ssh_key::LineEnding::LF).unwrap();
        let mut file = (release.len() as u32).to_le_bytes().to_vec();
        file.extend_from_slice(release.as_bytes());
        file.extend_from_slice(&packed);
        assert!(open(&public, &file).is_err(), "a signature the key made for a release, not a bundle");
    }

    #[test]
    fn a_bundle_cant_name_a_file_outside_itself() {
        let bad = Bundle { files: vec![("../evil.js".into(), vec![])], ..bundle() };
        let packed = bad.packed().unwrap();
        let k = key(1);
        assert!(open(&k.public_key().to_openssh().unwrap(), &signed(&k, &packed).unwrap()).is_err());
    }
}
