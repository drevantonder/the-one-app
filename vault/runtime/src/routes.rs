//! The Vault's routes for its Settings screens, each with who may ask it. Its tools are routes of their own
//! (`tools`).

use axum::Json;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use serde_json::json;

use api::runtime::prelude::*;
use api::{Body, Caller, Error, Me, Reply, Routes, Who};
use scope::scopes::Scope;
use vault::says::{Answer, Asked, Change, Items};

use crate::{items, requests};

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default()
        .route("/api/vault", Who::Clients, get(list).post(set))
        .route("/api/vault/requests", Who::Clients, get(asked))
        .route("/api/vault/answer", Who::Clients, post(answer))
}

/// The scope a body names, `user` by default.
fn scope(rt: &Rt, me: &Caller, scope: Option<&str>) -> Result<Scope, Error> {
    Ok(rt.read(|r| r.scope_for(&me.user, scope.unwrap_or("user")))?)
}

/// `?scope=`: the items the caller may change, in one scope or all.
async fn list(State(rt): State<Rt>, me: Me, Query(q): Query<Items>) -> Reply {
    let me = me.caller.for_agent(q.agent.as_deref());
    let scope = q.scope.as_deref().map(|s| rt.read(|r| r.scope_for(&me.user, s))).transpose()?;
    Ok(Json(json!(items::list(&rt, &me, scope.as_ref()).await?)))
}

async fn set(State(rt): State<Rt>, me: Me, Body(body): Body<Change>) -> Reply {
    let me = me.caller.for_agent(body.agent.as_deref());
    let scope = scope(&rt, &me, body.scope.as_deref())?;
    Ok(Json(json!({ "id": items::set_item(&rt, &me, &scope, serde_json::to_value(body)?).await? })))
}

/// `?scope=&id=`: the requests for items the caller may answer, or one.
async fn asked(State(rt): State<Rt>, me: Me, Query(q): Query<Asked>) -> Reply {
    let me = me.caller.for_agent(q.agent.as_deref());
    let scope = q.scope.as_deref().map(|s| rt.read(|r| r.scope_for(&me.user, s))).transpose()?;
    Ok(Json(json!(requests::item_requests(&rt, &me, scope.as_ref(), q.id.as_deref()).await?)))
}

async fn answer(State(rt): State<Rt>, me: Me, Body(body): Body<Answer>) -> Reply {
    let me = me.caller.for_agent(body.agent.as_deref());
    let scope = scope(&rt, &me, body.scope.as_deref())?;
    requests::answer_request(&rt, &me, &scope, &body.id, &serde_json::to_value(&body)?).await?;
    Ok(Json(json!({})))
}
