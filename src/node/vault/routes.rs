//! The Vault's routes, `/v1/vault…`, each with who may ask it.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::api::{Error, Me, Reply, Routes, Who};
use crate::database::scopes::Scope;
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/vault", Who::SignedIn, get(list).post(set))
        .route("/v1/vault/requests", Who::SignedIn, get(requests))
        .route("/v1/vault/request", Who::SignedIn, post(request))
        .route("/v1/vault/answer", Who::SignedIn, post(answer))
        .route("/v1/vault/fetch", Who::SignedIn, post(fetch))
        // Which machines open a Vault's secrets is for its admins' own devices to say.
        .route("/v1/vault/grant", Who::ThisMachine, post(grant))
}

/// Who asked, crediting the agent the body (or query) names, if any.
fn caller(me: Me, body: &Value) -> Caller {
    Caller { agent: body["agent"].as_str().map(str::to_string).or(me.caller.agent.clone()), ..me.caller }
}

/// The scope a body names, `user` by default.
fn scope(node: &Node, me: &Caller, body: &Value) -> Result<Scope, Error> {
    Ok(node.scope_for(me, body["scope"].as_str().unwrap_or("user"))?)
}

fn query(q: HashMap<String, String>) -> Value {
    Value::Object(q.into_iter().map(|(k, v)| (k, json!(v))).collect())
}

/// `?scope=`: the items the caller may change, in one scope or all.
async fn list(State(node): State<Node>, me: Me, Query(q): Query<HashMap<String, String>>) -> Reply {
    let q = query(q);
    let me = caller(me, &q);
    let scope = q["scope"].as_str().map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.vault(&me, scope.as_ref()).await?)))
}

/// An item's changes, `{ scope?, id?, title?, fields?, hosts?, apps?, deleted? }`: → `{ id }`.
async fn set(State(node): State<Node>, me: Me, Json(mut body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    let scope = scope(&node, &me, &body)?;
    body.as_object_mut().ok_or_else(|| anyhow::anyhow!("an item is an object"))?.remove("scope");
    Ok(Json(json!({ "id": node.set_item(&me, &scope, body).await? })))
}

/// `?scope=&id=`: the requests for items the caller may answer, or one.
async fn requests(State(node): State<Node>, me: Me, Query(q): Query<HashMap<String, String>>) -> Reply {
    let q = query(q);
    let me = caller(me, &q);
    let scope = q["scope"].as_str().map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.item_requests(&me, scope.as_ref(), q["id"].as_str()).await?)))
}

/// `{ scope?, app, need?, title?, fields?, hosts?, why? }`: asks the person for an item.
async fn request(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    let scope = scope(&node, &me, &body)?;
    Ok(Json(node.request_item(&me, &scope, body["app"].as_str().unwrap_or(""), body["need"].as_str(), &body, body["why"].as_str().unwrap_or("")).await?))
}

/// `{ scope?, id, item? | fields? | declined? }`: answers a request.
async fn answer(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    let scope = scope(&node, &me, &body)?;
    node.answer_request(&me, &scope, body["id"].as_str().unwrap_or(""), &body).await?;
    Ok(Json(json!({})))
}

/// `{ url, method?, headers?, body? }`: a request with the items agents may use filled in.
async fn fetch(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    Ok(Json(node.vault_fetch(&me, &body).await?))
}

/// `{ scope?, device, on }`: lets a machine open a scope's Vault, or no longer.
async fn grant(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    let scope = scope(&node, &me, &body)?;
    let device = body["device"].as_str().ok_or_else(|| anyhow::anyhow!("give device"))?;
    Ok(Json(node.vault_grant(&me, &scope, device, body["on"] != false).await?))
}
