//! The Vault's routes for its Settings screens, each with who may ask it. Its tools are routes of their own
//! (`crate::vault::tools`).


use axum::Json;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::openapi::Op;
use crate::api::{Body, Error, Me, Reply, Routes, Who};
use crate::database::scopes::Scope;
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/vault", Who::SignedIn, get(list).post(set))
        .says(Op::get("The items the caller may change, in one scope or all").query::<Items>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } }, "additionalProperties": true } })))
        .says(Op::post("Change an item").body::<Change>().answers_as(json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string", "description": "the item's" } } })))
        .route("/v1/vault/requests", Who::SignedIn, get(requests))
        .says(Op::get("The requests for items the caller may answer, or one").query::<Asked>().answers_as(json!({ "type": "array", "items": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } }, "additionalProperties": true } })))
        .route("/v1/vault/answer", Who::SignedIn, post(answer))
        .says(Op::post("Answer a request for an item").body::<Answer>().done())
}

/// Who asked, crediting the `agent` the body (or query) names, if any.
fn caller(me: Me, agent: Option<&str>) -> Caller {
    Caller { agent: agent.map(str::to_string).or(me.caller.agent.clone()), ..me.caller }
}

/// The scope a body names, `user` by default.
fn scope(node: &Node, me: &Caller, scope: Option<&str>) -> Result<Scope, Error> {
    Ok(node.scope_for(me, scope.unwrap_or("user"))?)
}

/// The agent it's for, and the scope, or every one the caller may change.
#[derive(Deserialize, schemars::JsonSchema)]
struct Items {
    agent: Option<String>,
    scope: Option<String>,
}

/// As `Items`, and one request by its `id`.
#[derive(Deserialize, schemars::JsonSchema)]
struct Asked {
    agent: Option<String>,
    scope: Option<String>,
    id: Option<String>,
}

/// `?scope=`: the items the caller may change, in one scope or all.
async fn list(State(node): State<Node>, me: Me, Query(q): Query<Items>) -> Reply {
    let me = caller(me, q.agent.as_deref());
    let scope = q.scope.as_deref().map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.vault(&me, scope.as_ref()).await?)))
}

/// An item's changes (`vault::apply`), in `scope`: → `{ id }`.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Change {
    #[serde(skip_serializing)]
    scope: Option<String>,
    #[serde(skip_serializing)]
    agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hosts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    apps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deleted: Option<bool>,
}

async fn set(State(node): State<Node>, me: Me, Body(body): Body<Change>) -> Reply {
    let me = caller(me, body.agent.as_deref());
    let scope = scope(&node, &me, body.scope.as_deref())?;
    Ok(Json(json!({ "id": node.set_item(&me, &scope, serde_json::to_value(body)?).await? })))
}

/// `?scope=&id=`: the requests for items the caller may answer, or one.
async fn requests(State(node): State<Node>, me: Me, Query(q): Query<Asked>) -> Reply {
    let me = caller(me, q.agent.as_deref());
    let scope = q.scope.as_deref().map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.item_requests(&me, scope.as_ref(), q.id.as_deref()).await?)))
}

/// Answers request `id`: with one of the caller's items, the `fields` (and `title`) of a new one, or `declined`.
#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Answer {
    #[serde(skip_serializing)]
    scope: Option<String>,
    #[serde(skip_serializing)]
    agent: Option<String>,
    id: String,
    item: Option<String>,
    title: Option<String>,
    fields: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    declined: bool,
}

async fn answer(State(node): State<Node>, me: Me, Body(body): Body<Answer>) -> Reply {
    let me = caller(me, body.agent.as_deref());
    let scope = scope(&node, &me, body.scope.as_deref())?;
    node.answer_request(&me, &scope, &body.id, &serde_json::to_value(&body)?).await?;
    Ok(Json(json!({})))
}
