//! The Vault's routes for its Settings screens, each with who may ask it. Its tools are routes of their own
//! (`crate::vault::tools`).


use axum::Json;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use serde_json::json;

use crate::api::{Body, Error, Me, Reply, Routes, Who};
use ::scope::scopes::Scope;
use crate::node::{Caller, Node};
use crate::says::vault::{Answer, Asked, Change, Items};

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/vault", Who::Clients, get(list).post(set))
        .route("/api/vault/requests", Who::Clients, get(requests))
        .route("/api/vault/answer", Who::Clients, post(answer))
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
async fn list(State(node): State<Node>, me: Me, Query(q): Query<Items>) -> Reply {
    let me = caller(me, q.agent.as_deref());
    let scope = q.scope.as_deref().map(|s| node.scope_for(&me, s)).transpose()?;
    Ok(Json(json!(node.vault(&me, scope.as_ref()).await?)))
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

async fn answer(State(node): State<Node>, me: Me, Body(body): Body<Answer>) -> Reply {
    let me = caller(me, body.agent.as_deref());
    let scope = scope(&node, &me, body.scope.as_deref())?;
    node.answer_request(&me, &scope, &body.id, &serde_json::to_value(&body)?).await?;
    Ok(Json(json!({})))
}
