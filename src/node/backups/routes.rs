//! Backups' routes, `/v1/backups…`, each with who may ask it.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::{Body, Me, Reply, Routes, Who};
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/backups", Who::SignedIn, get(status))
        .route("/v1/backups/now", Who::SignedIn, post(now))
        .route("/v1/backups/place", Who::SignedIn, post(place))
        .route("/v1/backups/request", Who::SignedIn, post(request))
        .route("/v1/backups/check", Who::SignedIn, post(check))
}

/// Who asked, crediting the `agent` the body names, if any.
fn caller(me: Me, agent: Option<String>) -> Caller {
    Caller { agent: agent.or(me.caller.agent.clone()), ..me.caller }
}

async fn status(State(node): State<Node>, me: Me) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    Ok(Json(node.backups()?))
}

async fn now(State(node): State<Node>, me: Me) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    node.back_up_now();
    Ok(Json(json!({})))
}

/// A place to add, a folder (`path`) or S3-compatible storage (`endpoint…`), after trying it (only trying it, with
/// `test`); or with `id` and `deleted: true`, one to remove.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Place {
    #[serde(skip_serializing)]
    agent: Option<String>,
    #[serde(skip_serializing)]
    id: Option<String>,
    #[serde(default, skip_serializing)]
    deleted: bool,
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

async fn place(State(node): State<Node>, me: Me, Body(mut body): Body<Place>) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    let me = caller(me, body.agent.take());
    if body.deleted {
        node.remove_place(&me, body.id.as_deref().unwrap_or("")).await?;
        return Ok(Json(json!({})));
    }
    if me.agent.is_some() && body.secret.is_some() {
        return Err(anyhow::anyhow!("agents never handle a place's key: ask the user with request").into());
    }
    Ok(Json(node.add_place(&me, &serde_json::to_value(body)?).await?))
}

/// S3-compatible storage at `endpoint` to ask the user for a bucket and its key of, on their own device.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(skip_serializing)]
    agent: Option<String>,
    endpoint: Option<String>,
    title: Option<String>,
}

async fn request(State(node): State<Node>, me: Me, Body(mut body): Body<Request>) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    let me = caller(me, body.agent.take());
    Ok(Json(node.request_place(&me, &serde_json::to_value(body)?).await?))
}

async fn check(State(node): State<Node>, me: Me) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    Ok(Json(node.check_backups().await?))
}
