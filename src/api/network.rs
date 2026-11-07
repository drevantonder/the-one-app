//! The routes of the network itself: its state, its keys, creating or joining it, and signed steps.

use axum::Json;
use axum::routing::{get, post};
use axum::extract::{Query, State};
use serde_json::json;


use crate::node::Node;

use super::guard::Me;
use super::{Routes, Who};
use super::reply::{Body, Reply, done, given};

pub(super) fn routes() -> Routes {
    use Who::*;
    Routes::default()
        .route("/v1/state", SignedIn, get(state))
        .route("/v1/keys", SignedIn, get(keys_get).post(keys_post))
        .route("/v1/create", ThisMachine, post(create))
        .route("/v1/join", ThisMachine, post(join))
        .route("/v1/steps", SignedIn, post(steps))
        .route("/v1/steps/global", SignedIn, get(global_steps))
}

/// `?scope=&to=<endpoint key>` or `?scope=&app=`: a scope's keys, sealed (`Node::sealed_keys`).
async fn keys_get(State(node): State<Node>, me: Me, Query(q): Query<std::collections::HashMap<String, String>>) -> Reply {
    let scope = crate::database::scopes::Scope::parse(q.get("scope").map(String::as_str).unwrap_or(""))?;
    Ok(Json(node.sealed_keys(&me.caller, &scope, q.get("to").map(String::as_str), q.get("app").map(String::as_str)).await?))
}

/// A key sealed by the caller's device, for a device (`to`), an app, or an app's key for a device (`Node::seal_key`).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedKey {
    scope: String,
    /// The epoch.
    n: u32,
    to: Option<String>,
    app: Option<String>,
    /// A Vault's key, sealed to a machine that opens it or to the recovery key.
    #[serde(default)]
    vault: bool,
    sealed: String,
}

async fn keys_post(State(node): State<Node>, me: Me, Body(body): Body<SealedKey>) -> Reply {
    let scope = crate::database::scopes::Scope::parse(given(&body.scope, "scope")?)?;
    Ok(Json(node.seal_key(&me.caller, &scope, body.n, body.to.as_deref(), body.app.as_deref(), body.vault, given(&body.sealed, "sealed")?).await?))
}

async fn state(State(node): State<Node>, signed_in: Option<axum::Extension<Me>>) -> Reply {
    Ok(Json(json!(match signed_in {
        Some(me) => node.state_for_sign_in(&me.caller, me.sign_in.as_deref().unwrap_or_default())?,
        None => node.state()?,
    })))
}

/// A new network, named `name`, whose first person is `user_name` (with `user`, their id).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    user_name: String,
    user: Option<String>,
}

async fn create(State(node): State<Node>, Body(body): Body<Create>) -> Reply {
    let recovery = node.create_with(given(&body.name, "name")?, given(&body.user_name, "user_name")?, body.user.as_deref(), crate::config::STARTERS).await?;
    Ok(Json(json!({ "recovery": recovery })))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    code: String,
}

async fn join(State(node): State<Node>, Body(body): Body<Join>) -> Reply {
    node.join(given(&body.code, "code")?).await?;
    done()
}

/// A step as `POST /v1/steps` takes it, each part base64url.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedStep {
    header: String,
    signature: String,
    body: String,
}

/// Steps of `global`'s scope database the caller's own device signed, each for the next position, which this device
/// checks count and writes (`database::steps`).
async fn steps(State(node): State<Node>, me: Me, Body(list): Body<Vec<SignedStep>>) -> Reply {
    let b = |v: &str, k: &str| data_encoding::BASE64URL_NOPAD.decode(v.as_bytes()).map_err(|_| anyhow::anyhow!("{k} isn't base64url"));
    let list = list.iter().map(|s| crate::database::steps::Step::decode(&b(&s.header, "header")?, &b(&s.signature, "signature")?, &b(&s.body, "body")?)).collect::<anyhow::Result<Vec<_>>>()?;
    node.put_signed_steps(&me.caller, list).await?;
    done()
}

/// `?from=<n>`: `global`'s scope database's steps after position n (`Node::global_steps`).
async fn global_steps(State(node): State<Node>, me: Me, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let from = q.get("from").map(|f| f.parse()).transpose()?.unwrap_or(0);
    let welcome = q.get("welcome").and_then(|w| crate::database::steps::Welcome::parse(w));
    Ok(Json(node.global_steps(&me.caller, from, welcome).await?))
}
