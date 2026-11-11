//! Devices' routes this node serves itself: pairing a machine, its own name, erasing what one keeps, and letting one in
//! with the network's recovery key.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use crate::api::{Body, Me, Reply, Routes, Who, done, given};
use crate::node::Node;
use crate::says::devices::{ApproveWithRecovery, Device, Pair, ReplaceRecovery, SealRecovery, recovery_key};

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/pair", Who::Clients, post(pair))
        .route("/api/name", Who::ThisMachine, post(name))
        .route("/api/erase", Who::Clients, post(erase))
        .route("/api/devices/approve-with-recovery", Who::ThisMachine, post(approve_with_recovery))
        .route("/api/devices/recovery-key/replace", Who::ThisMachine, post(replace_recovery))
        .route("/api/devices/recovery-holders", Who::Clients, get(recovery_holders))
        .route("/api/devices/recovery-holders/seal", Who::ThisMachine, post(seal_recovery))
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

/// Erases what a device keeps, asked of it directly (`Node::erase`), before it's removed.
async fn erase(State(node): State<Node>, me: Me, Body(body): Body<Device>) -> Reply {
    Ok(Json(node.erase(&me.caller, given(&body.key, "key")?).await?))
}

async fn approve_with_recovery(State(node): State<Node>, Body(body): Body<ApproveWithRecovery>) -> Reply {
    let key = recovery_key(&body.recovery, "a recovery key")?;
    Ok(Json(scope::settings::recovery::approve(&node, given(&body.code, "code")?, &key, body.confirm).await?))
}

async fn replace_recovery(State(node): State<Node>, Body(body): Body<ReplaceRecovery>) -> Reply {
    Ok(Json(scope::settings::recovery::replace(&node, &recovery_key(&body.recovery, "a recovery key")?, &recovery_key(&body.next, "the next recovery key")?, body.confirm).await?))
}

async fn recovery_holders(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(node.recovery_holders(&me.caller).await?))
}

async fn seal_recovery(State(node): State<Node>, Body(body): Body<SealRecovery>) -> Reply {
    scope::settings::recovery::seal(&node, given(&body.to, "to")?, given(&body.copy, "copy")?).await?;
    done()
}
