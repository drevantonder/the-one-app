//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user.

use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::entry::hex;
use crate::members::Role;
use crate::node::{CallRequest, Caller, Node};

pub fn router(node: Node) -> Router {
    Router::new()
        .route("/v1/state", get(state))
        .route("/v1/apps", get(apps))
        .route("/v1/call", post(call))
        .route("/v1/widgets", get(widgets))
        .route("/v1/create", post(create))
        .route("/v1/pair", post(pair))
        .route("/v1/join", post(join))
        .route("/v1/name", post(name))
        .route("/v1/remove", post(remove))
        .route("/v1/role", post(role))
        .route("/v1/relays", post(relays))
        .route("/v1/acl", post(acl))
        .route("/v1/trust", post(trust))
        .route("/v1/open", post(open))
        .route("/v1/group", post(group))
        .route("/v1/invite", post(invite))
        .route("/v1/accept", post(accept))
        .route("/v1/member", post(member))
        .route("/v1/app", post(app))
        .route("/v1/deploy", post(deploy))
        .route("/v1/install", post(install))
        .route("/v1/node-release", post(release))
        .route("/v1/blobs", post(put_blob).layer(axum::extract::DefaultBodyLimit::max(1 << 30)))
        .route("/v1/blobs/{hash}", get(blob))
        .route("/v1/events", get(events))
        .route("/v1/screens/{app}/{screen}", get(screen))
        .route("/hooks/{app}/{name}", post(hook))
        .route("/v1/providers/{app}", post(register))
        .route("/v1/providers/{app}/calls", get(next_call))
        .route("/v1/providers/{app}/calls/{id}", post(reply))
        .layer(axum::middleware::from_fn(only_this_machine))
        .with_state(node)
}

/// Browsers send an Origin, and a page that rebinds its DNS to 127.0.0.1 still sends its own Host:
/// refuse both, so no web page can drive this machine's node.
async fn only_this_machine(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let host = req.headers().get(axum::http::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let local = host.split(':').next().is_some_and(|h| matches!(h, "127.0.0.1" | "localhost" | "[::1]"));
    // A screen this node served may call it: its origin is this node's own address.
    let origin = req.headers().get(axum::http::header::ORIGIN).and_then(|o| o.to_str().ok());
    let own = origin.is_some_and(|o| o.strip_prefix("http://").is_some_and(|h| h == host));
    if (origin.is_some() && !own) || !local {
        return (StatusCode::FORBIDDEN, "only this machine's own programs may use this").into_response();
    }
    next.run(req).await
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

fn done() -> Reply {
    Ok(Json(json!({})))
}

fn local(node: &Node) -> Result<Caller, Error> {
    Ok(Caller { key: node.id(), user: node.user()? })
}

fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str, Error> {
    body[key].as_str().filter(|s| !s.trim().is_empty()).map(str::trim).ok_or_else(|| anyhow::anyhow!("give {key}").into())
}

async fn state(State(node): State<Node>) -> Json<Value> {
    Json(node.state())
}

async fn apps(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.apps(&local(&node)?).await?)))
}

async fn call(State(node): State<Node>, Json(req): Json<CallRequest>) -> Reply {
    Ok(Json(json!({ "ok": node.call(req).await? })))
}

#[derive(Deserialize)]
struct WidgetsQuery {
    scope: Option<String>,
}

async fn widgets(State(node): State<Node>, Query(q): Query<WidgetsQuery>) -> Reply {
    Ok(Json(json!(node.widgets(local(&node)?, q.scope.as_deref()).await?)))
}

async fn create(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let recovery = node.create(text(&body, "name")?, text(&body, "user_name")?).await?;
    Ok(Json(json!({ "recovery": recovery })))
}

async fn pair(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    Ok(Json(json!({ "code": node.pair(crate::node::new_user(&body)?)? })))
}

async fn join(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.join(text(&body, "code")?).await?;
    done()
}

async fn name(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.rename_server(text(&body, "name")?).await?;
    done()
}

/// `{ key }` (a server or client, by name or key) or `{ user }`.
async fn remove(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    match body["user"].as_str() {
        Some(user) => node.member("remove", json!({ "user": node.find_user(user)? })).await?,
        None => {
            let key = text(&body, "key")?;
            let found = node.state()["install"]["clients"].as_array().into_iter().flatten().chain(node.state()["install"]["servers"].as_array().into_iter().flatten())
                .find(|m| m["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(key)) || m["key"].as_str().is_some_and(|k| k.starts_with(key)))
                .and_then(|m| m["key"].as_str().map(str::to_string))
                .ok_or_else(|| anyhow::anyhow!("no server or client called {key}"))?;
            node.member("remove", json!({ "key": found })).await?;
        }
    }
    done()
}

async fn role(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body["role"].clone())?;
    node.member("role", json!({ "user": node.find_user(text(&body, "user")?)?, "role": role })).await?;
    done()
}

async fn relays(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.member("relays", json!({ "urls": body["urls"] })).await?;
    done()
}

/// `{ server, owners, users }`: owners and users are `user:<id>`, `group:<id>`, `install`, or a user's or group's name.
async fn acl(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let server = node.find_server(text(&body, "server")?)?;
    let principals = |key: &str| -> Result<Vec<String>, Error> {
        body[key]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("give {key}"))?
            .iter()
            .map(|p| {
                let p = p.as_str().unwrap_or("");
                Ok(match crate::members::Principal::parse(p) {
                    Ok(p) => p.to_string(),
                    Err(_) => match node.find_user(p) {
                        Ok(u) => format!("user:{u}"),
                        Err(_) => format!("group:{}", node.find_group(p)?),
                    },
                })
            })
            .collect()
    };
    node.member("acl", json!({ "server": hex(&server), "owners": principals("owners")?, "users": principals("users")? })).await?;
    done()
}

async fn trust(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let server = node.find_server(text(&body, "server")?)?;
    node.member(if body["trust"] == false { "untrust" } else { "trust" }, json!({ "server": hex(&server) })).await?;
    done()
}

async fn open(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let user = node.find_user(text(&body, "user")?)?;
    Ok(Json(json!({ "ok": node.open(&user, text(&body, "server")?).await? })))
}

async fn group(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let group = crate::entry::new_id();
    node.member("group", json!({ "group": group, "name": text(&body, "name")? })).await?;
    Ok(Json(json!({ "group": group })))
}

async fn invite(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body.get("role").cloned().unwrap_or(json!("member")))?;
    let (group, user) = (node.find_group(text(&body, "group")?)?, node.find_user(text(&body, "user")?)?);
    node.member("invite", json!({ "group": group, "user": user, "role": role })).await?;
    done()
}

async fn accept(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.member("accept", json!({ "group": node.find_group(text(&body, "group")?)? })).await?;
    done()
}

/// `{ group, user, role }`, with `role: null` to remove them.
async fn member(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let role: Option<Role> = serde_json::from_value(body["role"].clone())?;
    let (group, user) = (node.find_group(text(&body, "group")?)?, node.find_user(text(&body, "user")?)?);
    node.member("member", json!({ "group": group, "user": user, "role": role })).await?;
    done()
}

async fn app(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let mut data = json!({ "name": text(&body, "name")?, "owner": text(&body, "owner")? });
    if let Some(from) = body["from"].as_str() {
        data["from"] = json!(from);
    }
    node.member("app", data).await?;
    done()
}

async fn deploy(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let (app, bundle) = (text(&body, "app")?, text(&body, "bundle")?);
    crate::bundle::module(&node.blob(bundle).await?)?;
    node.append(&crate::entry::Scope::Global, crate::node::deploy(app, bundle)).await?;
    done()
}

/// `{ app, scope, installed }`: `scope` is `user`, `group:<id>` (or a group's name) or `global`.
async fn install(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let scope = match text(&body, "scope")? {
        "user" => crate::entry::Scope::User(node.user()?),
        s => crate::entry::Scope::parse(s).or_else(|_| node.find_group(s).map(crate::entry::Scope::Group))?,
    };
    let kind = if body["installed"] == false { "uninstall" } else { "install" };
    node.append(&scope, crate::members::node(kind, json!({ "app": text(&body, "app")? }))).await?;
    done()
}

/// For agent harnesses' hooks: the body is the hook's JSON, the reply is plain text to show the agent.
async fn hook(State(node): State<Node>, Path((app, name)): Path<(String, String)>, body: String) -> Response {
    let event: Value = serde_json::from_str(&body).unwrap_or_default();
    match node.hook(&app, &name, event).await {
        Ok(Value::String(text)) => text.into_response(),
        Ok(Value::Null) => String::new().into_response(),
        Ok(other) => other.to_string().into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct ReleaseRequest {
    path: String,
    commit: String,
    target: String,
    #[serde(default)]
    committed: u64,
    signature: String,
}

async fn release(State(node): State<Node>, Json(req): Json<ReleaseRequest>) -> Reply {
    let r = node.publish(std::path::Path::new(&req.path), &req.commit, &req.target, req.committed, &req.signature).await?;
    Ok(Json(json!(r)))
}

async fn put_blob(State(node): State<Node>, body: axum::body::Bytes) -> Reply {
    Ok(Json(json!({ "blob": node.put_blob(&body)?, "size": body.len() })))
}

async fn blob(State(node): State<Node>, Path(hash): Path<String>) -> Result<Response, Error> {
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], node.blob(&hash).await?).into_response())
}

async fn register(State(node): State<Node>, Path(app): Path<String>, Json(describe): Json<Value>) -> Reply {
    node.provide(&app, describe).await?;
    done()
}

async fn next_call(State(node): State<Node>, Path(app): Path<String>) -> Result<Response, Error> {
    Ok(match node.providers().next(&app).await? {
        Some(call) => Json(json!(call)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn reply(State(node): State<Node>, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Reply {
    node.providers().reply(&app, id, body)?;
    done()
}

#[derive(Deserialize)]
struct ScreenQuery {
    params: Option<String>,
    scope: Option<String>,
}

/// An app screen as a page, for shells' web views and for a browser on this machine.
async fn screen(State(node): State<Node>, Path((app, name)): Path<(String, String)>, Query(q): Query<ScreenQuery>) -> Result<Response, Error> {
    let params: Value = q.params.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(json!({}));
    let html = node.screen(local(&node)?, &app, &name, params, q.scope.as_deref()).await?;
    Ok(axum::response::Html(html).into_response())
}

async fn events(State(node): State<Node>) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(node.subscribe())
        .filter_map(|e| e.ok())
        .map(|e| Ok(SseEvent::default().json_data(e).unwrap_or_default()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
