//! Files kept in the network: putting one, and reading one back.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::json;

use api::runtime::Rt;
use api::{Error, Me, Reply, Routes, Who};

pub fn routes() -> Routes<Rt> {
    Routes::default().route("/api/blobs", Who::Clients, post(put)).route("/api/blobs/{hash}", Who::Clients, get(blob))
}

/// A file to keep, read as it comes: from elsewhere, within the room `put_from` gives its user.
async fn put(State(rt): State<Rt>, me: Me, req: axum::extract::Request) -> Reply {
    let (hash, size) = super::put_from(&rt, &me.caller, req.into_body()).await?;
    Ok(Json(json!({ "blob": hash, "size": size })))
}

async fn blob(State(rt): State<Rt>, me: Me, Path(hash): Path<String>) -> Result<Response, Error> {
    let bytes = super::blob_for(&rt, &me.caller, &hash).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}
