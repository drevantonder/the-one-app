//! A computer's home on disk, as its onet, oned and app runtime all read it: the device's key and name, the token
//! its own programs ask with, its app runtime's key, and the network it's in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use key::{SigningKey, hex, unhex};
use bucket::Place;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct DeviceFile {
    pub secret: String,
    pub name: String,
}

/// Writes a file only this account can read: device keys, pairing secrets.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The key (hex) and name of the device whose home is `dir`, as it last was.
pub fn device_here(dir: &Path) -> Result<(String, String)> {
    let (key, name) = device_key(dir)?;
    Ok((hex(&key.verifying_key().to_bytes()), name))
}

/// The key and name of the device whose home is `dir`, made the first time.
pub fn device_key(dir: &Path) -> Result<(SigningKey, String)> {
    let file = load_device(dir)?;
    Ok((SigningKey::from_bytes(&unhex(&file.secret)?), file.name))
}

pub fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let file = DeviceFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
    Ok(file)
}

/// The folder of the app runtime of the device whose home is `dir`: its key, and the token its device's own programs
/// ask it with, as a service's folder holds them (`services::folder`).
pub fn runtime_folder(dir: &Path) -> PathBuf {
    dir.join("services").join(scope::scopes::APPS_PART)
}

/// The app runtime's own key, in its folder, made the first time: by whichever of it and its device's onet, which lists
/// it, asks first, and then the same for both.
pub fn load_runtime_key(folder: &Path) -> Result<SigningKey> {
    let path = folder.join("key");
    if let Ok(secret) = std::fs::read_to_string(&path) {
        return Ok(SigningKey::from_bytes(&unhex(secret.trim()).context("the app runtime's key file isn't a key")?));
    }
    std::fs::create_dir_all(folder)?;
    let key = SigningKey::from_bytes(&rand::random());
    let made = folder.join(format!("key.{}", std::process::id()));
    write_private(&made, hex(&key.to_bytes()).as_bytes())?;
    // Put in place only if it isn't there yet, so a key the other one made at once is the one both keep.
    let placed = std::fs::hard_link(&made, &path);
    let _ = std::fs::remove_file(&made);
    match placed {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => load_runtime_key(folder),
        placed => placed.map(|()| key).map_err(Into::into),
    }
}

/// The endpoint of the app runtime of the device whose home is `dir` (`runtime_folder`), its key made the first time.
pub fn runtime_endpoint(dir: &Path) -> Result<key::Endpoint> {
    Ok(key::Endpoint::of(&load_runtime_key(&runtime_folder(dir))?))
}

/// This machine's own programs' token (in its app runtime's folder, the one they ask that with), made once: only its
/// user reads the file, which a program in a VM or emulator here, or a page, can't.
pub fn load_local_token(dir: &Path) -> Result<String> {
    let path = dir.join(crate::local::LOCAL_TOKEN);
    match std::fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_string()),
        _ => {
            let token = hex(&rand::random::<[u8; 32]>());
            write_private(&path, token.as_bytes())?;
            Ok(token)
        }
    }
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".into())
}

pub const NETWORK_DIR: &str = "network";

pub const NETWORK_FILE: &str = "network.json";

/// The network a computer is in, as its home keeps it (`<home>/network/network.json`): read by its app runtime and its onet.
#[derive(Serialize, Deserialize)]
pub struct NetworkFile {
    pub root: String,
    pub user: String,
    /// The network's bucket, as a place's fields: a folder's `path`, or S3's. None if this device can't reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BTreeMap<String, String>>,
}

impl NetworkFile {
    /// The network the computer whose home is `home` is in, if it's in one.
    pub fn read(home: &Path) -> Result<Option<NetworkFile>> {
        match std::fs::read(home.join(NETWORK_DIR).join(NETWORK_FILE)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn write(&self, home: &Path) -> Result<()> {
        let dir = home.join(NETWORK_DIR);
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join(NETWORK_FILE), &serde_json::to_vec_pretty(self)?)
    }
}

/// The object in a network's bucket that says whose it is: the network's root.
pub const NETWORK_OBJECT: &str = "network";

/// Whose the bucket at `place` is, as its `NETWORK_OBJECT` says: or as `install` says, the name a build from before it
/// was called a network gave it (a bucket then, or a backup of one put back), which it puts under the new name too.
pub fn bucket_root(place: &Place) -> Result<Option<Vec<u8>>> {
    if let Some(root) = place.get(NETWORK_OBJECT)? {
        return Ok(Some(root));
    }
    let old = place.get("install")?;
    if let Some(root) = &old {
        place.create(NETWORK_OBJECT, root)?;
    }
    Ok(old)
}
