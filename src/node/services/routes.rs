//! Services' routes: switching one on or off on a device, what a computer's oned asks of its device to keep its
//! services running (`oned::keep_parts`), through its onet, which passes it to the device's own app runtime, and what
//! the device's parts ask that runtime directly: their sockets (`carry`), their own asks, and who's who.

use axum::Json;
use axum::response::IntoResponse;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::Routes;
use ::api::Body;
use ::api::Me;
use ::api::Reply;
use ::api::Who;
use ::api::done;
use ::api::given;
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/service", Who::ThisMachine, post(service))
        .route("/api/services", Who::ThisMachine, get(wanted).post(said))
        .route("/api/services/builds", Who::ThisMachine, get(builds))
        .route("/api/services/builds/{hash}", Who::ThisMachine, get(build))
        .route("/api/services/declared", Who::ThisMachine, post(declared))
        .route("/api/services/up", Who::ThisMachine, post(up))
        .route("/api/services/list", Who::ThisMachine, post(list))
        .route("/api/services/carry/{name}", Who::ThisMachine, get(next_ask).post(register))
        .route("/api/services/carry/{name}/{id}", Who::ThisMachine, post(answered))
        .route("/api/services/ask", Who::ThisMachine, post(asked))
        .route("/api/services/steps", Who::ThisMachine, get(steps))
}

/// Removes a service from a device (this one by default, for this machine's own programs), or with `on` lets it
/// register there again.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Service {
    name: String,
    device: Option<String>,
    #[serde(default)]
    on: bool,
}

async fn service(State(node): State<Node>, me: Me, Body(body): Body<Service>) -> Reply {
    node.set_service(&me.caller, body.device.as_deref(), given(&body.name, "name")?, body.on).await?;
    done()
}

async fn wanted(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.parts_wanted()?)))
}

/// What this device's oned runs, as its onet passes it on: `{ parts }`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Said {
    parts: Value,
}

async fn said(State(node): State<Node>, Body(body): Body<Said>) -> Reply {
    node.parts_from(&node.0.holds.device(), body.parts);
    done()
}

async fn builds(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.service_builds().await)))
}

async fn build(State(node): State<Node>, Path(hash): Path<String>) -> Result<axum::response::Response, ::api::Error> {
    let bytes = node.service_build(&hash).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

/// The service a request is about.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

async fn declared(State(node): State<Node>, Body(body): Body<Named>) -> Reply {
    Ok(Json(json!(node.service_declared(&body.name).await?)))
}

/// Whether a part of this device is registered at its runtime: started, and reached.
async fn up(State(node): State<Node>, Body(body): Body<Named>) -> Reply {
    Ok(Json(json!({ "up": node.0.sockets.has(&body.name) })))
}

/// A part registering at its runtime, with what the program offering it says it is.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    describe: Value,
}

async fn register(State(node): State<Node>, Path(name): Path<String>, Body(body): Body<Register>) -> Reply {
    let new = !node.0.sockets.has(&name);
    node.0.sockets.register(&name, body.describe);
    node.0.heard.notify_waiters();
    if new {
        node.learn_apps(vec![name]);
    }
    done()
}

/// The next ask carried to a part, waiting a while: `{ id, ask }`, or nothing.
async fn next_ask(State(node): State<Node>, Path(name): Path<String>) -> Result<axum::response::Response, ::api::Error> {
    Ok(match node.0.sockets.next(&name).await? {
        Some(ask) => Json(json!(ask)).into_response(),
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    })
}

/// A part's answer to an ask carried to it: `{ answer }`, sealed, or `{ error }`.
async fn answered(State(node): State<Node>, Path((name, id)): Path<(String, u64)>, Body(body): Body<Value>) -> Reply {
    node.0.sockets.reply(&name, id, body)?;
    done()
}

async fn asked(State(node): State<Node>, Body(signed): Body<Value>) -> Reply {
    Ok(Json(node.part_asked(&signed).await?))
}

/// `?from=<n>`: `global`'s steps after position n, for a part.
async fn steps(State(node): State<Node>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let from = q.get("from").map(|f| f.parse()).transpose()?.unwrap_or(0);
    Ok(Json(node.steps_for_parts(from).await?))
}

/// A service, with the endpoint it asked to be listed with.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listed {
    name: String,
    key: String,
    seal: String,
}

async fn list(State(node): State<Node>, Body(body): Body<Listed>) -> Reply {
    let endpoint = key::Endpoint { key: key::unhex(&body.key)?, seal: key::unhex(&body.seal)? };
    node.list_part(&endpoint, &body.name).await?;
    done()
}
