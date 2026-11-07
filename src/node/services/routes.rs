//! Services' route: switching one on or off on a device.

use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;

use crate::api::{Body, Me, Reply, Routes, Who, done, given};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/service", Who::ThisMachine, post(service))
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
