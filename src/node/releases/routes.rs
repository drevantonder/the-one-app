//! Publishing a build of the node, or of a service's program, from this machine.

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

use crate::api::{Body, Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/node-release", Who::ThisMachine, post(release))
        .route("/v1/service-release", Who::ThisMachine, post(release))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseRequest {
    path: String,
    /// For a service's program, which.
    #[serde(default)]
    service: Option<String>,
    commit: String,
    target: String,
    #[serde(default)]
    committed: u64,
    signature: String,
}

/// A build of the node, or with `service` of a service's program (at its own path, so a node too old to know them
/// refuses it rather than taking it for its own), for `target` and signed: published as its newest release.
async fn release(State(node): State<Node>, Body(req): Body<ReleaseRequest>) -> Reply {
    let r = node.publish(std::path::Path::new(&req.path), req.service.as_deref(), &req.commit, &req.target, req.committed, &req.signature).await?;
    Ok(Json(json!(r)))
}
