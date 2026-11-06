//! Services' routes: switching one on or off, and a service's program asking for its calls and answering them.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::api::{Error, Me, Reply, Routes, Who, done, only, text};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/service", Who::SignedIn, post(service))
        .route("/v1/services/{app}", Who::ThisMachine, post(register))
        .route("/v1/services/{app}/calls", Who::ThisMachine, get(next_call))
        .route("/v1/services/{app}/calls/{id}", Who::ThisMachine, post(reply))
        .route("/v1/services/{app}/calls/{id}/vault", Who::ThisMachine, post(fill))
}

/// `{ name, device?, installed? }`: removes a service from a device (this one by default, for this machine's own
/// programs), or with `installed` lets it register there again.
async fn service(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["name", "device", "on"])?;
    node.set_service(&me.caller, body["device"].as_str(), text(&body, "name")?, body["on"] == true).await?;
    done()
}

async fn register(State(node): State<Node>, Path(app): Path<String>, Json(describe): Json<Value>) -> Reply {
    node.register_service(&app, describe).await?;
    done()
}

async fn next_call(State(node): State<Node>, Path(app): Path<String>) -> Result<Response, Error> {
    node.stand_down(&app)?;
    Ok(match node.services().next(&app).await? {
        Some(call) => Json(json!(call)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn reply(State(node): State<Node>, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Reply {
    node.services().reply(&app, id, body)?;
    done()
}

/// A service's text filled from the Vault for a call of its that waits, for the host it's going to.
async fn fill(State(node): State<Node>, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Reply {
    let text = node.service_fill(&app, id, text(&body, "text")?, text(&body, "host")?).await?;
    Ok(Json(json!({ "text": text })))
}
