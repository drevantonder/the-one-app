//! Where the network keeps its data, `/api/storage`: this machine's own programs' alone.

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::api::Routes;
use ::api::Body;
use ::api::Reply;
use ::api::Who;
use crate::Node;

pub fn routes() -> Routes {
    Routes::default().route("/api/storage", Who::ThisMachine, get(storage).post(set))
}

async fn storage(State(node): State<Node>) -> Reply {
    Ok(Json(node.storage()?))
}

/// S3-compatible storage to move the network's data to.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Storage {
    endpoint: String,
    bucket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
}

async fn set(State(node): State<Node>, Body(body): Body<Storage>) -> Reply {
    Ok(Json(node.set_storage(&node.caller()?, &serde_json::to_value(body)?).await?))
}
