//! Files kept in the network: putting one, and reading one back.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::json;

use crate::api::openapi::Op;
use crate::api::{Error, Me, Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/blobs", Who::Clients, post(put_blob))
        .says(Op::post("Keep a file, read as it comes").takes_file().answers_as(json!({ "type": "object", "required": ["blob", "size"], "properties": { "blob": { "type": "string", "description": "its hash" }, "size": { "type": "integer", "description": "bytes" } } })))
        .route("/api/blobs/{hash}", Who::Clients, get(blob))
        .says(Op::get("Read back a kept file").file())
}

/// A file to keep, read as it comes: from elsewhere, within the room `Node::put_blob_from` gives its user.
async fn put_blob(State(node): State<Node>, me: Me, req: axum::extract::Request) -> Reply {
    let (hash, size) = node.put_blob_from(&me.caller, req.into_body()).await?;
    Ok(Json(json!({ "blob": hash, "size": size })))
}

async fn blob(State(node): State<Node>, me: Me, Path(hash): Path<String>) -> Result<Response, Error> {
    let bytes = node.blob_for(&me.caller, &hash).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}
