//! Services' routes: switching one on or off on a device, and what a computer's oned asks of its device to keep its
//! services running (`oned::keep_parts`), through its onet, which passes it to the device's own app runtime.

use axum::Json;
use axum::response::IntoResponse;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Body, Me, Reply, Routes, Who, done, given};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/service", Who::ThisMachine, post(service))
        .route("/api/services", Who::ThisMachine, get(wanted).post(said))
        .route("/api/services/builds", Who::ThisMachine, get(builds))
        .route("/api/services/builds/{hash}", Who::ThisMachine, get(build))
        .route("/api/services/declared", Who::ThisMachine, post(declared))
        .route("/api/services/hello", Who::ThisMachine, post(hello))
        .route("/api/services/list", Who::ThisMachine, post(list))
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

async fn build(State(node): State<Node>, Path(hash): Path<String>) -> Result<axum::response::Response, crate::api::Error> {
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

async fn hello(State(node): State<Node>, Body(body): Body<Named>) -> Reply {
    Ok(Json(json!({ "up": node.hello_part(&body.name).await })))
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
    let endpoint = crate::keys::Endpoint { key: crate::keys::unhex(&body.key)?, seal: crate::keys::unhex(&body.seal)? };
    node.list_part(&endpoint, &body.name).await?;
    done()
}
