//! Backups' routes, `/v1/backups…`, each with who may ask it.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::api::{Me, Reply, Routes, Who};
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/backups", Who::SignedIn, get(status))
        .route("/v1/backups/now", Who::SignedIn, post(now))
        .route("/v1/backups/place", Who::SignedIn, post(place))
        .route("/v1/backups/request", Who::SignedIn, post(request))
        .route("/v1/backups/check", Who::SignedIn, post(check))
}

/// Who asked, crediting the agent the body names, if any.
fn caller(me: Me, body: &Value) -> Caller {
    Caller { agent: body["agent"].as_str().map(str::to_string).or(me.caller.agent.clone()), ..me.caller }
}

async fn status(State(node): State<Node>) -> Reply {
    Ok(Json(node.backups()?))
}

async fn now(State(node): State<Node>, me: Me) -> Reply {
    node.back_up_now_for(&me.caller)?;
    Ok(Json(json!({})))
}

/// `{ path | endpoint…, title? }` adds a place, after trying it; `{ id, deleted: true }` removes one.
async fn place(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    if body["deleted"] == true {
        node.remove_place(&me, body["id"].as_str().unwrap_or("")).await?;
        return Ok(Json(json!({})));
    }
    if me.agent.is_some() && !body["secret"].is_null() {
        return Err(anyhow::anyhow!("agents never handle a place's key: ask the person with request").into());
    }
    Ok(Json(node.add_place(&me, &body).await?))
}

/// `{ endpoint, title? }`: asks the person for a bucket and its key, on their own device.
async fn request(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let me = caller(me, &body);
    Ok(Json(node.request_place(&me, &body).await?))
}

async fn check(State(node): State<Node>) -> Reply {
    Ok(Json(node.check_backups().await?))
}
