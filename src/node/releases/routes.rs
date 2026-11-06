//! Publishing a build of the node, or of a service's program, from this machine.

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

use crate::api::{Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/node-release", Who::ThisMachine, post(release))
        .route("/v1/service-release", Who::ThisMachine, post(service_release))
}

#[derive(Deserialize)]
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

async fn release(State(node): State<Node>, Json(req): Json<ReleaseRequest>) -> Reply {
    let r = node.publish(std::path::Path::new(&req.path), None, &req.commit, &req.target, req.committed, &req.signature).await?;
    Ok(Json(json!(r)))
}

/// A service's program, built for `target` and signed, published as its newest release (its own path, so a node too
/// old to know them refuses it, rather than taking it for its own).
async fn service_release(State(node): State<Node>, Json(req): Json<ReleaseRequest>) -> Reply {
    let service = req.service.as_deref().ok_or_else(|| anyhow::anyhow!("say which service"))?;
    let r = node.publish(std::path::Path::new(&req.path), Some(service), &req.commit, &req.target, req.committed, &req.signature).await?;
    Ok(Json(json!(r)))
}
