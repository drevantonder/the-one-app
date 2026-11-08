//! Devices' routes this node serves itself: pairing a machine, its own name, erasing what one keeps, and letting one in
//! with the network's recovery key.

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

use crate::api::{Body, Me, Reply, Routes, Who, done, given};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/pair", Who::SignedIn, post(pair))
        .route("/v1/name", Who::ThisMachine, post(name))
        .route("/v1/erase", Who::SignedIn, post(erase))
        .route("/v1/devices/approve-with-recovery", Who::ThisMachine, post(approve_with_recovery))
}

#[derive(Deserialize)]
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Device {
    key: String,
}

/// Erases what a device keeps, asked of it directly (`Node::erase`), before it's removed.
async fn erase(State(node): State<Node>, me: Me, Body(body): Body<Device>) -> Reply {
    Ok(Json(node.erase(&me.caller, given(&body.key, "key")?).await?))
}

/// Lets in the device showing `code` with `recovery`, the network's recovery key (hex), read from a file here by the
/// CLI and never kept.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    code: String,
    #[serde(default)]
    confirm: bool,
    recovery: String,
}

async fn approve_with_recovery(State(node): State<Node>, Body(body): Body<Approve>) -> Reply {
    let (code, recovery) = (given(&body.code, "code")?, &body.recovery);
    let secret: [u8; 32] = crate::keys::unhex(recovery.trim()).map_err(|_| anyhow::anyhow!("a recovery key is 64 hex characters"))?;
    let key = ed25519_dalek::SigningKey::from_bytes(&secret);
    Ok(Json(node.approve_with_recovery(code, &key, body.confirm).await?))
}
