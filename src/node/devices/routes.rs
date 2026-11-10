//! Devices' routes this node serves itself: pairing a machine, its own name, erasing what one keeps, and letting one in
//! with the network's recovery key.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use crate::api::openapi::Op;
use crate::api::{Body, Me, Reply, Routes, Who, done, given};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/pair", Who::Clients, post(pair))
        .says(Op::post("A pairing code for a new device or someone new").body::<Pair>().answers_as(json!({
            "type": "object",
            "required": ["code", "user"],
            "properties": { "code": { "type": "string", "description": "to give the new device" }, "user": { "type": "string", "description": "whose it will be" } }
        })))
        .route("/api/name", Who::ThisMachine, post(name))
        .route("/api/erase", Who::Clients, post(erase))
        .says(Op::post("Erase what a device keeps").body::<Device>().answers_as(json!({ "type": "object", "required": ["erasing"], "properties": { "erasing": { "type": "string", "description": "the device's name" } } })))
        .route("/api/devices/approve-with-recovery", Who::ThisMachine, post(approve_with_recovery))
        .route("/api/devices/recovery-key/replace", Who::ThisMachine, post(replace_recovery))
        .route("/api/devices/recovery-holders", Who::Clients, get(recovery_holders))
        .route("/api/devices/recovery-holders/seal", Who::ThisMachine, post(seal_recovery))
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Pair {
    user: Option<String>,
}

async fn pair(State(node): State<Node>, me: Me, Body(body): Body<Pair>) -> Reply {
    let (code, user) = node.pair_as(&me.caller, body.user.as_deref())?;
    Ok(Json(json!({ "code": code, "user": user })))
}

/// Renames this device, for this machine's own programs: another is renamed as any device's user does
/// (`settings::answer`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Name {
    name: String,
}

async fn name(State(node): State<Node>, Body(body): Body<Name>) -> Reply {
    node.rename_device(given(&body.name, "name")?).await?;
    done()
}

/// A device, by its key.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Device {
    key: String,
}

/// Erases what a device keeps, asked of it directly (`Node::erase`), before it's removed.
async fn erase(State(node): State<Node>, me: Me, Body(body): Body<Device>) -> Reply {
    Ok(Json(node.erase(&me.caller, given(&body.key, "key")?).await?))
}

/// Lets in the device showing `code` with `recovery`, the network's recovery key (hex), read by the command on this
/// machine (`devices::cli`) and never kept.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    code: String,
    #[serde(default)]
    confirm: bool,
    recovery: String,
}

async fn approve_with_recovery(State(node): State<Node>, Body(body): Body<Approve>) -> Reply {
    let secret: [u8; 32] = crate::keys::unhex(body.recovery.trim()).map_err(|_| anyhow::anyhow!("a recovery key is 64 hex characters"))?;
    let key = ed25519_dalek::SigningKey::from_bytes(&secret);
    let mut approved = node.approve_with_recovery(given(&body.code, "code")?, &key, body.confirm).await?;
    approved["said"] = json!(crate::settings::said_approved(&approved));
    Ok(Json(approved))
}

/// Replaces the recovery key with `next`, both read by the command on this machine (`devices::cli`) and never kept.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceRecovery {
    recovery: String,
    next: String,
    #[serde(default)]
    confirm: bool,
}

async fn replace_recovery(State(node): State<Node>, Body(body): Body<ReplaceRecovery>) -> Reply {
    let key = |hex: &str, what: &str| crate::keys::unhex(hex.trim()).map_err(|_| anyhow::anyhow!("{what} is 64 hex characters")).map(|k| ed25519_dalek::SigningKey::from_bytes(&k));
    Ok(Json(node.replace_recovery(&key(&body.recovery, "a recovery key")?, &key(&body.next, "the next recovery key")?, body.confirm).await?))
}

async fn recovery_holders(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(node.recovery_holders(&me.caller).await?))
}

/// The recovery key, sealed on this machine by the command that read its file (`devices::cli`), for a device that may
/// hold it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealRecovery {
    to: String,
    /// What the recovery key signed and sealed for it (`keys::held_copy`).
    copy: String,
}

async fn seal_recovery(State(node): State<Node>, Body(body): Body<SealRecovery>) -> Reply {
    node.seal_recovery(given(&body.to, "to")?, given(&body.copy, "copy")?).await?;
    done()
}
