//! What a computer's app runtime asks the holder of its device's key (its onet) at `/api/holds/*`, and what that
//! holder signs for it: the runtime opens and signs nothing of the device's itself (ARCHITECTURE.md, Trust).

use anyhow::{Context, Result, ensure};
use key::SigningKey;
use scope::content::Content;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Signs `bytes` with the device's `key`, for its app runtime, if they're a request to the relay or auth
/// (`keys::signed_request_by`): never a step, which only `/api/holds/write` writes, nor anything else.
pub fn sign_request(key: &SigningKey, bytes: &[u8]) -> Result<[u8; 64]> {
    let (domain, fields) = bytes.split_at(bytes.iter().position(|b| *b == 0).context("that isn't a signed request")?);
    let domain = std::str::from_utf8(domain)?;
    ensure!([custom_build::protocol::RELAY, custom_build::protocol::AUTH_DOMAIN].contains(&domain), "the device signs only its requests to the relay and auth for its app runtime, not {domain}");
    serde_json::from_slice::<Value>(&fields[1..]).context("that isn't a signed request")?;
    Ok(ed25519_dalek::Signer::sign(key, bytes).to_bytes())
}

/// What `/api/holds/shared` takes: answered by whatever holds the device's key, its app runtime or its onet.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldShare {
    /// An X25519 public key, base64url.
    pub share: String,
}

/// What `/api/holds/write` takes: answered by whatever holds the device's key, its app runtime or its onet. Its
/// contents are written as that many steps, in order.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldWrite {
    pub user: String,
    pub contents: Vec<Content>,
}

/// What `/api/holds/moved` takes: the fields of the network's bucket its app runtime keeps the network's data in from
/// now on (`::bucket::Place::network`), which the device's holder then reads and writes too.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldMoved {
    pub bucket: std::collections::BTreeMap<String, String>,
}

/// What `/api/holds/sign` takes: answered by whatever holds the device's key, its app runtime or its onet.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldSign {
    /// What a request to the relay or auth signs (`keys::signed_request_by`), base64url.
    pub bytes: String,
}
