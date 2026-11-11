//! Where the network keeps its data, `/api/storage`: this machine's own programs' alone.

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use api::runtime::prelude::*;
use api::{Body, Reply, Routes, Who};

pub fn routes() -> Routes<Rt> {
    Routes::default().route("/api/storage", Who::ThisMachine, get(storage).post(set))
}

async fn storage(State(rt): State<Rt>) -> Reply {
    Ok(Json(super::storage(&rt)))
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

async fn set(State(rt): State<Rt>, Body(body): Body<Storage>) -> Reply {
    Ok(Json(super::set(&rt, &rt.caller()?, &serde_json::to_value(body)?).await?))
}
