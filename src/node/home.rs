//! This device's home on disk: its key and name, and the token its own programs ask with.

use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct DeviceFile {
    pub(super) secret: String,
    pub(super) name: String,
}

/// Writes a file only this account can read: device keys, pairing secrets.
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The key of the device whose home is `dir`, for what this machine signs as itself without its node (`cloudflare invite`).
pub fn device_key(dir: &Path) -> Result<ed25519_dalek::SigningKey> {
    let bytes = std::fs::read(dir.join("device.json")).with_context(|| format!("{} has no device: {} serve starts one", dir.display(), crate::config::CLI))?;
    let file: DeviceFile = serde_json::from_slice(&bytes)?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&unhex(&file.secret)?))
}

/// The key (hex) and name of the device whose home is `dir`, as it last was.
pub fn device_here(dir: &Path) -> Result<(String, String)> {
    let file = load_device(dir)?;
    let key = ed25519_dalek::SigningKey::from_bytes(&unhex(&file.secret)?);
    Ok((hex(&key.verifying_key().to_bytes()), file.name))
}

pub(super) fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let file = DeviceFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
    Ok(file)
}

/// This machine's own programs' token, made once: only its user reads the file, which a program in a VM or emulator
/// here, or a page, can't.
pub(super) fn load_local_token(dir: &Path) -> Result<String> {
    let path = dir.join(crate::api::LOCAL_TOKEN);
    match std::fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_string()),
        _ => {
            let token = hex(&rand::random::<[u8; 32]>());
            write_private(&path, token.as_bytes())?;
            Ok(token)
        }
    }
}

pub(super) fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".into())
}
