//! Publishing a build of the node, or of a service's program, from this machine.

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use serde_json::json;

use crate::api::Routes;
use ::api::Body;
use ::api::Reply;
use ::api::Who;
use crate::Host;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/latest/put", Who::ThisMachine, post(put))
        .route("/api/latest", Who::ThisMachine, axum::routing::get(latest).post(replace))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutRequest {
    path: String,
}

async fn put(State(host): State<Host>, Body(req): Body<PutRequest>) -> Reply {
    let (sha256, size) = host.publish_file(std::path::Path::new(&req.path)).await?;
    Ok(Json(json!({ "sha256": sha256, "size": size })))
}

async fn latest(State(host): State<Host>) -> Reply {
    let record = host.latest_record().await?;
    Ok(Json(json!({ "text": record.as_ref().map(|(t, _)| t), "tag": record.as_ref().map(|(_, t)| t) })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceRequest {
    text: String,
    #[serde(default)]
    tag: Option<String>,
}

async fn replace(State(host): State<Host>, Body(req): Body<ReplaceRequest>) -> Reply {
    Ok(Json(json!({ "replaced": host.replace_latest(&req.text, req.tag.as_deref()).await? })))
}
