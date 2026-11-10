//! The route Settings' Backups screen adds a place with, which takes the key its user types and so is not a tool
//! (`crate::backups::tools` are the others): who may ask it.

use axum::Json;
use axum::extract::State;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::openapi::Op;
use crate::api::{Body, Me, Reply, Routes, Who};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default().route("/api/backups/place", Who::Clients, post(place)).says(Op::post("Add a place backups go, after trying it").body::<Place>().answers_as(json!({
        "type": "object",
        "required": ["tried"],
        "properties": {
            "tried": { "type": "array", "items": { "type": "object", "required": ["step"], "properties": { "step": { "type": "string" }, "error": { "type": ["string", "null"] } } }, "description": "what trying it did, each step and why it failed, if it did" },
            "id": { "type": "string", "description": "the place's, once added" }
        },
        "additionalProperties": true
    })))
}

/// A place to add, a folder (`path`) or S3-compatible storage (`endpoint…`), after trying it (only trying it, with
/// `test`).
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Place {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default)]
    test: bool,
}

async fn place(State(node): State<Node>, me: Me, Body(body): Body<Place>) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    if me.caller.agent.is_some() && body.secret.is_some() {
        return Err(anyhow::anyhow!("agents never handle a place's key: ask the user with backups_add_place").into());
    }
    Ok(Json(node.add_place(&me.caller, &serde_json::to_value(body)?).await?))
}
