//! The local HTTP API: how shells, the CLI and MCP reach this device's node.

use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::entry::{hex, unhex};
use crate::node::Node;

pub fn router(node: Node) -> Router {
    Router::new()
        .route("/v1/state", get(state))
        .route("/v1/apps", get(apps))
        .route("/v1/call", post(call))
        .route("/v1/widgets", get(widgets))
        .route("/v1/spaces", post(create))
        .route("/v1/invite", post(invite))
        .route("/v1/join", post(join))
        .route("/v1/device", post(rename))
        .route("/v1/events", get(events))
        .with_state(node)
}

struct Error(anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{:#}", self.0) }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.into())
    }
}

type Reply = Result<Json<Value>, Error>;

async fn state(State(node): State<Node>) -> Json<Value> {
    Json(node.state())
}

async fn apps(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.apps().await?)))
}

#[derive(Deserialize)]
struct CallRequest {
    space: Option<String>,
    device: Option<String>,
    app: String,
    op: String,
    #[serde(default)]
    input: Value,
}

async fn call(State(node): State<Node>, Json(req): Json<CallRequest>) -> Reply {
    let space = node.find_space(req.space.as_deref())?;
    let input = if req.input.is_null() { json!({}) } else { req.input };
    let result = match req.device {
        Some(device) => node.call_on(unhex(&device)?, space, &req.app, &req.op, input).await?,
        None => node.call(space, &req.app, &req.op, input).await?,
    };
    Ok(Json(json!({ "ok": result })))
}

#[derive(Deserialize)]
struct SpaceQuery {
    space: Option<String>,
}

async fn widgets(State(node): State<Node>, Query(q): Query<SpaceQuery>) -> Reply {
    let space = node.find_space(q.space.as_deref())?;
    Ok(Json(json!(node.widgets(space).await?)))
}

#[derive(Deserialize)]
struct CreateRequest {
    name: String,
}

async fn create(State(node): State<Node>, Json(req): Json<CreateRequest>) -> Reply {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(anyhow::anyhow!("give the space a name").into());
    }
    let space = node.create_space(name).await?;
    Ok(Json(json!({ "id": hex(&space) })))
}

#[derive(Deserialize)]
struct InviteRequest {
    space: Option<String>,
    #[serde(default)]
    mine: bool,
}

async fn invite(State(node): State<Node>, Json(req): Json<InviteRequest>) -> Reply {
    let space = node.find_space(req.space.as_deref())?;
    Ok(Json(json!({ "ticket": node.invite(space, req.mine)? })))
}

#[derive(Deserialize)]
struct JoinRequest {
    ticket: String,
}

async fn join(State(node): State<Node>, Json(req): Json<JoinRequest>) -> Reply {
    let space = node.join(&req.ticket).await?;
    Ok(Json(json!({ "id": hex(&space) })))
}

#[derive(Deserialize)]
struct RenameRequest {
    name: String,
}

async fn rename(State(node): State<Node>, Json(req): Json<RenameRequest>) -> Reply {
    node.rename_device(req.name.trim()).await?;
    Ok(Json(json!({})))
}

async fn events(State(node): State<Node>) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(node.subscribe())
        .filter_map(|e| e.ok())
        .map(|e| Ok(SseEvent::default().json_data(e).unwrap_or_default()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
