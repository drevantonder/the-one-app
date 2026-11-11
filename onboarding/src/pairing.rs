//! A pairing code that lets a new device join the network: what it says, the one that's out, and what a computer joining
//! with it asks and is told.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use custom_build::protocol::{Coded, code};
use key::Key;
use serde::{Deserialize, Serialize};

/// A code a device made, which it keeps until it's used or has expired.
#[derive(Serialize, Deserialize)]
pub struct Pairing {
    pub user: String,
    /// The client that asked for it, if one did.
    #[serde(default)]
    pub by: Option<String>,
    /// The user the machine joins as, when it isn't the asker's: an admin's code for someone else's machine.
    #[serde(default)]
    pub admits: Option<String>,
    /// Milliseconds since 1970 after which it no longer works.
    pub until: u64,
    /// The machine whose join with it didn't finish (hex), which alone may use it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried: Option<String>,
}

/// What a pairing code says: where the device that made it is reached, and the secret that lets one machine in.
#[derive(Serialize, Deserialize)]
pub struct PairingCode {
    /// The network's address, whose relay reaches the device that made it.
    pub address: String,
    pub from: Key,
    pub secret: [u8; 16],
}

impl PairingCode {
    /// The code as it's shown and typed.
    pub fn text(&self) -> Result<String> {
        let mut bytes = Vec::new();
        ciborium::into_writer(self, &mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// The code a person copied, as they may have copied it.
    pub fn parse(text: &str) -> Result<PairingCode> {
        let bad = || Coded::error(code::BAD_PAIRING_CODE, "that isn't a pairing code: copy it again, or ask for a new one");
        let bytes = data_encoding::BASE32_NOPAD.decode(text.trim().to_uppercase().as_bytes()).map_err(|_| bad())?;
        ciborium::from_reader(bytes.as_slice()).map_err(|_| bad())
    }
}

#[derive(Serialize, Deserialize)]
pub struct JoinRequest {
    pub secret: [u8; 16],
    pub name: String,
    #[serde(rename = "as")]
    pub as_: String,
}

#[derive(Serialize, Deserialize)]
pub struct JoinReply {
    pub root: String,
    pub user: String,
    /// For a device: the network's bucket, as a place's fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BTreeMap<String, String>>,
}

/// The codes a device made that were kept in its home (`pairings.json`), so one still works after it restarts.
pub fn pairings(dir: &Path) -> HashMap<[u8; 16], Pairing> {
    let list: Vec<(String, Pairing)> = platform::files::read_json(&dir.join("pairings.json"));
    list.into_iter()
        .filter_map(|(k, v)| Some((data_encoding::HEXLOWER.decode(k.as_bytes()).ok()?.try_into().ok()?, v)))
        .collect()
}
