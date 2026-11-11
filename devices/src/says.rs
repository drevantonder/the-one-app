//! What the devices' routes say of themselves (`api::says`).

use serde::Deserialize;
use serde_json::json;

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/pair", Who::Clients, Op::post("A pairing code for a new device or someone new").body::<Pair>().answers_as(json!({
            "type": "object",
            "required": ["code", "user"],
            "properties": { "code": { "type": "string", "description": "to give the new device" }, "user": { "type": "string", "description": "whose it will be" } }
        }))),
        ("/api/devices/approve-with-recovery", Who::ThisMachine, Op::post("Let in someone who lost every device, with the network's recovery key").body::<ApproveWithRecovery>().answers_as(json!({ "type": "object", "required": ["said", "device", "user"], "properties": { "said": { "type": "string" }, "device": { "type": "string", "description": "the device's name" }, "user": { "type": "string", "description": "whose it is" } } }))),
        ("/api/devices/recovery-key/replace", Who::ThisMachine, Op::post("Replace the network's recovery key").body::<ReplaceRecovery>().answers_as(json!({ "type": "object", "required": ["said"], "properties": { "said": { "type": "string" } } }))),
        ("/api/devices/recovery-holders/seal", Who::ThisMachine, Op::post("Keep a copy of the recovery key sealed for a device of someone who holds it").body::<SealRecovery>().done()),
        ("/api/erase", Who::Clients, Op::post("Erase what a device keeps").body::<Device>().answers_as(json!({ "type": "object", "required": ["erasing"], "properties": { "erasing": { "type": "string", "description": "the device's name" } } }))),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub user: Option<String>,
}

/// A device, by its key.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub key: String,
}

/// Lets in the device showing `code` with `recovery`, the network's recovery key (hex), read by the command on this
/// computer (`devices::cli`) and never kept.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveWithRecovery {
    pub code: String,
    #[serde(default)]
    pub confirm: bool,
    pub recovery: String,
}

/// Replaces the recovery key with `next`, both read by the command on this computer (`devices::cli`) and never kept.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceRecovery {
    pub recovery: String,
    pub next: String,
    #[serde(default)]
    pub confirm: bool,
}

/// The recovery key, sealed on this computer by the command that read its file (`devices::cli`), for a device that may
/// hold it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SealRecovery {
    pub to: String,
    /// What the recovery key signed and sealed for it (`keys::held_copy`).
    pub copy: String,
}

/// A recovery key, as the command that read its file sends it: 64 hex characters.
pub fn recovery_key(hex: &str, what: &str) -> anyhow::Result<key::SigningKey> {
    key::unhex(hex.trim()).map_err(|_| anyhow::anyhow!("{what} is 64 hex characters")).map(|k| key::SigningKey::from_bytes(&k))
}
