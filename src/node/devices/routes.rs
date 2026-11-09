//! Devices' routes this node serves itself: pairing a machine, its own name, and erasing what one keeps.

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
