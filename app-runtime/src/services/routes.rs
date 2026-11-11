//! Services' routes this runtime carries: what a computer's oned tells its device of its parts and asks of their
//! builds (`oned::keep_parts`), through its onet, and what the device's parts ask that runtime directly: their sockets
//! (`carry`), their own asks, and who's who. Switching them is services/runtime's.

use axum::Json;
use axum::response::IntoResponse;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::Routes;
use ::api::Body;
use ::api::Reply;
use ::api::Who;
use ::api::done;
use crate::Host;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/services", Who::ThisMachine, post(said))
        .route("/api/services/builds", Who::ThisMachine, get(builds))
        .route("/api/services/builds/{hash}", Who::ThisMachine, get(build))
        .route("/api/services/up", Who::ThisMachine, post(up))
        .route("/api/services/carry/{name}", Who::ThisMachine, get(next_ask).post(register))
        .route("/api/services/carry/{name}/{id}", Who::ThisMachine, post(answered))
        .route("/api/services/ask", Who::ThisMachine, post(asked))
        .route("/api/services/steps", Who::ThisMachine, get(steps))
}

/// What this device's oned runs, as its onet passes it on: `{ parts }`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Said {
    parts: Value,
}

async fn said(State(host): State<Host>, Body(body): Body<Said>) -> Reply {
    host.parts_from(&host.0.holds.device(), body.parts);
    done()
}

async fn builds(State(host): State<Host>) -> Reply {
    Ok(Json(json!(host.service_builds().await)))
}

async fn build(State(host): State<Host>, Path(hash): Path<String>) -> Result<axum::response::Response, ::api::Error> {
    let bytes = host.service_build(&hash).await?;
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

/// The service a request is about.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

/// Whether a part of this device is registered at its runtime: started, and reached.
async fn up(State(host): State<Host>, Body(body): Body<Named>) -> Reply {
    Ok(Json(json!({ "up": host.0.sockets.has(&body.name) })))
}

/// A part registering at its runtime, with what the program offering it says it is.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Register {
    describe: Value,
}

async fn register(State(host): State<Host>, Path(name): Path<String>, Body(body): Body<Register>) -> Reply {
    let new = !host.0.sockets.has(&name);
    host.0.sockets.register(&name, body.describe);
    host.0.heard.notify_waiters();
    if new {
        host.learn_apps(vec![name]);
    }
    done()
}

/// The next ask carried to a part, waiting a while: `{ id, ask }`, or nothing.
async fn next_ask(State(host): State<Host>, Path(name): Path<String>) -> Result<axum::response::Response, ::api::Error> {
    Ok(match host.0.sockets.next(&name).await? {
        Some(ask) => Json(json!(ask)).into_response(),
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    })
}

/// A part's answer to an ask carried to it: `{ answer }`, sealed, or `{ error }`.
async fn answered(State(host): State<Host>, Path((name, id)): Path<(String, u64)>, Body(body): Body<Value>) -> Reply {
    host.0.sockets.reply(&name, id, body)?;
    done()
}

async fn asked(State(host): State<Host>, Body(signed): Body<Value>) -> Reply {
    Ok(Json(host.part_asked(&signed).await?))
}

/// `?from=<n>`: `global`'s steps after position n, for a part.
async fn steps(State(host): State<Host>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let from = q.get("from").map(|f| f.parse()).transpose()?.unwrap_or(0);
    Ok(Json(host.steps_for_parts(from).await?))
}
