//! Push's routes, `/v1/push/…`: for anyone signed in, each to their own devices.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};

use crate::api::{Body, Me, Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/push/send", Who::SignedIn, post(send))
        .route("/v1/push/devices", Who::SignedIn, get(devices))
        .route("/v1/push/status", Who::SignedIn, get(status))
}

/// A note to the caller's devices: → how many it went to, and how many of theirs get pushes.
async fn send(State(node): State<Node>, me: Me, Body(send): Body<crate::push::Send>) -> Reply {
    Ok(Json(node.push_note(&me.caller.user, &send.note("")?).await?))
}

async fn devices(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(node.push_devices(&me.caller.user).await?))
}

async fn status(State(node): State<Node>) -> Reply {
    Ok(Json(node.push_status().await?))
}
