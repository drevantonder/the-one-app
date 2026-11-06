//! Files kept in the network: putting one, and reading one back.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::json;

use crate::api::{Error, LOCAL_PUT, Me, Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/blobs", Who::SignedIn, post(put_blob))
        .route("/v1/blobs/{hash}", Who::SignedIn, get(blob))
}

/// A file to keep, read as it comes: from elsewhere, within the room `Node::put_blob_from` gives its person.
async fn put_blob(State(node): State<Node>, me: Me, req: axum::extract::Request) -> Reply {
    let (hash, size) = match me.sign_in {
        Some(_) => node.put_blob_from(&me.caller, req.into_body()).await?,
        None => node.put_blob_streamed(req.into_body(), LOCAL_PUT).await?,
    };
    Ok(Json(json!({ "blob": hash, "size": size })))
}

async fn blob(State(node): State<Node>, me: Me, Path(hash): Path<String>) -> Result<Response, Error> {
    let bytes = match me.sign_in {
        Some(_) => node.blob_for(&me.caller, &hash).await?,
        None => node.blob(&hash).await?,
    };
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}
