//! The Vault's routes for its Settings screens, each with who may ask it. Its tools are routes of their own
//! (`crate::vault::tools`).

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{Body, Error, Me, Reply, Routes, Who};
use crate::database::scopes::Scope;
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/vault", Who::SignedIn, get(list).post(set))
        .route("/v1/vault/requests", Who::SignedIn, get(requests))
        .route("/v1/vault/answer", Who::SignedIn, post(answer))
}

/// Who asked, crediting the `agent` the body (or query) names, if any.
fn caller(me: Me, agent: Option<&str>) -> Caller {
    Caller { agent: agent.map(str::to_string).or(me.caller.agent.clone()), ..me.caller }
}

/// The scope a body names, `user` by default.
fn scope(node: &Node, me: &Caller, scope: Option<&str>) -> Result<Scope, Error> {
    Ok(node.scope_for(me, scope.unwrap_or("user"))?)
}

/// `?scope=`: the items the caller may change, in one scope or all.
async fn list(State(node): State<Node>, me: Me, Query(q): Query<HashMap<String, String>>) -> Reply {
    let me = caller(me, q.get("agent").map(String::as_str));
    let scope = q.get("scope").map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.vault(&me, scope.as_ref()).await?)))
}

/// An item's changes (`vault::apply`), in `scope`: → `{ id }`.
#[derive(Deserialize, Serialize)]
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
async fn requests(State(node): State<Node>, me: Me, Query(q): Query<HashMap<String, String>>) -> Reply {
    let me = caller(me, q.get("agent").map(String::as_str));
    let scope = q.get("scope").map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.item_requests(&me, scope.as_ref(), q.get("id").map(String::as_str)).await?)))
}

/// Answers request `id`: with one of the caller's items, the `fields` (and `title`) of a new one, or `declined`.
#[derive(Deserialize, Serialize)]
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
