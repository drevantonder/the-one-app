//! What the Vault's routes for its Settings screens say of themselves (src/says.rs).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/vault", Who::Clients, Op::get("The items the caller may change, in one scope or all").query::<Items>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } }, "additionalProperties": true } }))),
        ("/api/vault", Who::Clients, Op::post("Change an item").body::<Change>().answers_as(json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the item's" } } }))),
        ("/api/vault/requests", Who::Clients, Op::get("The requests for items the caller may answer, or one").query::<Asked>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } }, "additionalProperties": true } }))),
        ("/api/vault/answer", Who::Clients, Op::post("Answer a request for an item").body::<Answer>().done()),
    ]
}

/// The agent it's for, and the scope, or every one the caller may change.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Items {
    pub agent: Option<String>,
    pub scope: Option<String>,
}

/// As `Items`, and one request by its `id`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Asked {
    pub agent: Option<String>,
    pub scope: Option<String>,
    pub id: Option<String>,
}

/// An item's changes (`vault::apply`), in `scope`: → `{ id }`.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Change {
    #[serde(skip_serializing)]
    pub scope: Option<String>,
    #[serde(skip_serializing)]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
}

/// Answers request `id`: with one of the caller's items, the `fields` (and `title`) of a new one, or `declined`.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    #[serde(skip_serializing)]
    pub scope: Option<String>,
    #[serde(skip_serializing)]
    pub agent: Option<String>,
    pub id: String,
    pub item: Option<String>,
    pub title: Option<String>,
    pub fields: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub declined: bool,
}
