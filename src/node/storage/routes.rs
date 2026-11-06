//! Where the network keeps its data, `/v1/storage`: this machine's own programs' alone.

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use serde_json::Value;

use crate::api::{Reply, Routes, Who};
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default().route("/v1/storage", Who::ThisMachine, get(storage).post(set))
}

async fn storage(State(node): State<Node>) -> Reply {
    Ok(Json(node.storage()?))
}

/// `{ endpoint, bucket, region? }`: moves the network's data there.
async fn set(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    Ok(Json(node.set_storage(&Caller::local(&node)?, &body).await?))
}
