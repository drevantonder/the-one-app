//! The routes of the network itself: its state, its keys, creating or joining it, and signed steps.

use axum::Json;
use axum::routing::{get, post};
use axum::extract::{Query, State};
use serde_json::json;


use crate::Host;
use ::api::says::network::{Create, Join};

use ::api::Me;
use super::Routes;
use ::api::Who;
use ::api::reply::{Body, Reply, done, given};

pub(super) fn routes() -> Routes {
    use Who::*;
    Routes::default()
        .route("/api/state", Clients, get(state))
        .route("/api/keys", Clients, get(keys_get).post(keys_post))
        .route("/api/create", ThisMachine, post(create))
        .route("/api/join", ThisMachine, post(join))
        .route("/api/joined", ThisMachine, post(joined))
        .route("/api/steps", Clients, post(steps))
        .route("/api/steps/global", Clients, get(global_steps))
        .route("/api/relay", ThisMachine, get(relay_heard).post(relay))
}

/// What the relay answered this computer's onet (`/relay/computes`), given as it hears it.
async fn relay(State(host): State<Host>, Body(said): Body<serde_json::Value>) -> Reply {
    host.heard_relay(&said);
    done()
}

/// What the relay says now, for an app runtime this one holds its device's key for, as onet answers it: read first.
async fn relay_heard(State(host): State<Host>) -> Reply {
    host.read_relay().await;
    Ok(Json(host.relay_heard()))
}

/// `?scope=&to=<endpoint key>` or `?scope=&app=`: a scope's keys, sealed (`Host::sealed_keys`).
async fn keys_get(State(host): State<Host>, me: Me, Query(q): Query<std::collections::HashMap<String, String>>) -> Reply {
    let scope = scope::scopes::Scope::parse(q.get("scope").map(String::as_str).unwrap_or(""))?;
    Ok(Json(host.sealed_keys(&me.caller, &scope, q.get("to").map(String::as_str), q.get("app").map(String::as_str)).await?))
}

/// A key sealed by the caller's device, for a device (`to`), an app, or an app's key for a device (`Host::seal_key`).
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

async fn keys_post(State(host): State<Host>, me: Me, Body(body): Body<SealedKey>) -> Reply {
    let scope = scope::scopes::Scope::parse(given(&body.scope, "scope")?)?;
    Ok(Json(host.seal_key(&me.caller, &scope, body.n, body.to.as_deref(), body.app.as_deref(), body.vault, given(&body.sealed, "sealed")?).await?))
}

async fn state(State(host): State<Host>, client: Option<axum::Extension<Me>>) -> Reply {
    Ok(Json(json!(match client {
        Some(me) => host.state_for_client(&me.caller, me.client.as_deref().unwrap_or_default())?,
        None => host.state()?,
    })))
}

async fn create(State(host): State<Host>, Body(body): Body<Create>) -> Reply {
    let recovery = host.create_with(given(&body.name, "name")?, given(&body.user_name, "user_name")?, body.user.as_deref(), crate::built_in::STARTERS).await?;
    Ok(Json(json!({ "recovery": recovery })))
}

async fn joined(State(host): State<Host>) -> Reply {
    host.joined()?;
    done()
}

async fn join(State(host): State<Host>, Body(body): Body<Join>) -> Reply {
    ::api::git_wire::wire_here(host.join(given(&body.code, "code")?).await?);
    done()
}

/// A step as `POST /api/steps` takes it, each part base64url.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedStep {
    header: String,
    signature: String,
    body: String,
}

/// Steps of `global`'s scope database the caller's own device signed, each for the next position, which this device
/// checks count and writes (`database::steps`).
async fn steps(State(host): State<Host>, me: Me, Body(list): Body<Vec<SignedStep>>) -> Reply {
    let b = |v: &str, k: &str| data_encoding::BASE64URL_NOPAD.decode(v.as_bytes()).map_err(|_| anyhow::anyhow!("{k} isn't base64url"));
    let list = list.iter().map(|s| scope::steps::Step::decode(&b(&s.header, "header")?, &b(&s.signature, "signature")?, &b(&s.body, "body")?)).collect::<anyhow::Result<Vec<_>>>()?;
    host.put_signed_steps(&me.caller, list).await?;
    done()
}

/// `?from=<n>`: `global`'s scope database's steps after position n (`Host::global_steps`).
async fn global_steps(State(host): State<Host>, me: Me, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let from = q.get("from").map(|f| f.parse()).transpose()?.unwrap_or(0);
    let welcome = q.get("welcome").and_then(|w| scope::steps::Welcome::parse(w));
    Ok(Json(host.global_steps(&me.caller, from, welcome).await?))
}
