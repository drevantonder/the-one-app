//! Services' routes that are features of the runtime: switching one on or off on a device, and what a computer's oned
//! asks of its device to keep its services running (`oned::keep_parts`), through its onet, which passes it to the
//! device's own app runtime.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use api::runtime::Rt;
use api::{Body, Me, Reply, Routes, Who, done, given};

pub fn routes() -> Routes<Rt> {
    Routes::default()
        .route("/api/service", Who::ThisMachine, post(service))
        .route("/api/services", Who::ThisMachine, get(wanted))
        .route("/api/services/declared", Who::ThisMachine, post(declared))
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

async fn service(State(rt): State<Rt>, me: Me, Body(body): Body<Service>) -> Reply {
    crate::set_service(&rt, &me.caller, body.device.as_deref(), given(&body.name, "name")?, body.on).await?;
    done()
}

async fn wanted(State(rt): State<Rt>) -> Reply {
    Ok(Json(json!(crate::parts_wanted(&rt)?)))
}

/// The service a request is about.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

async fn declared(State(rt): State<Rt>, Body(body): Body<Named>) -> Reply {
    Ok(Json(json!(crate::service_declared(&rt, &body.name).await?)))
}

/// A service, with the endpoint it asked to be listed with.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listed {
    name: String,
    key: String,
    seal: String,
}

async fn list(State(rt): State<Rt>, Body(body): Body<Listed>) -> Reply {
    let endpoint = key::Endpoint { key: key::unhex(&body.key)?, seal: key::unhex(&body.seal)? };
    crate::list_part(&rt, &endpoint, &body.name).await?;
    done()
}
