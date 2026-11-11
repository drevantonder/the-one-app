//! What the route Settings' Backups screen adds a place with says of itself (src/says.rs).

use serde::{Deserialize, Serialize};
use serde_json::json;

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![("/api/backups/place", Who::Clients, Op::post("Add a place backups go, after trying it").body::<Place>().answers_as(json!({
        "type": "object",
        "required": ["tried"],
        "properties": {
            "tried": { "type": "array", "items": { "type": "object", "required": ["step"], "properties": { "step": { "type": "string" }, "error": { "type": ["string", "null"] } } }, "description": "what trying it did, each step and why it failed, if it did" },
            "id": { "type": "string", "description": "the place's, once added" }
        },
        "additionalProperties": true
    })))]
}

/// A place to add, a folder (`path`) or S3-compatible storage (`endpoint…`), after trying it (only trying it, with
/// `test`).
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Place {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub test: bool,
}
