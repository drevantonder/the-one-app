//! Apps' routes: listing them, deploying, adding, arranging, deleting and forking them, and their code.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Error, Me, Reply, Routes, Who, done, tagged, text};
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/apps", Who::SignedIn, get(apps))
        .route("/v1/labs", Who::SignedIn, get(labs))
        .route("/v1/images/{name}", Who::Anyone, get(image))
        .route("/v1/deploy", Who::SignedIn, post(deploy))
        .route("/v1/install", Who::SignedIn, post(install))
        .route("/v1/schedule", Who::ThisMachine, post(schedule))
        .route("/v1/arrange", Who::SignedIn, post(arrange))
        .route("/v1/retire", Who::SignedIn, post(retire))
        .route("/v1/fork", Who::ThisMachine, post(fork))
        .route("/v1/source/{app}", Who::ThisMachine, get(source))
}

/// Who owns an app, as a request names them: `user:<id>`, `group:<id>`, `admins`, or a group's name.
fn owner(node: &Node, named: &str) -> anyhow::Result<String> {
    crate::database::scopes::Principal::parse(named).map(|p| p.to_string()).or_else(|_| node.find_group(named).map(|g| format!("group:{g}")))
}

#[derive(Deserialize)]
struct AppsQuery {
    /// As a window draws them (`Node::apps_drawn`).
    light: Option<bool>,
}

async fn apps(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap, Query(q): Query<AppsQuery>) -> Result<Response, Error> {
    let apps = if q.light == Some(true) { node.apps_drawn(&me.caller).await? } else { node.apps(&me.caller).await? };
    let body = serde_json::to_string(&apps)?;
    let tag = crate::screen::tag(&body);
    Ok(tagged(&headers, tag, ([(axum::http::header::CONTENT_TYPE, "application/json")], body)))
}

/// Every lab, for an admin's Settings › Labs (`Node::labs`).
async fn labs(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(json!(node.labs(&me.caller).await?)))
}

/// A picture a window's app list names (`Node::apps_drawn`), for anyone who has its name, which is its content's:
/// kept for good, since it can't change.
async fn image(State(node): State<Node>, Path(name): Path<String>) -> Response {
    let found = if name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()) { node.named_image(&name).await } else { None };
    match found {
        Some((kind, bytes)) => ([(axum::http::header::CONTENT_TYPE, kind), (axum::http::header::CACHE_CONTROL, "public, max-age=31536000, immutable".into())], bytes).into_response(),
        None => (StatusCode::NOT_FOUND, [(axum::http::header::CACHE_CONTROL, "no-store")]).into_response(),
    }
}

/// `{ app, bundle, owner?, wait? }`, or `{ app, files, … }` (each file's path → its text, built here as `one deploy`
/// builds a folder): deploys it, making the app first if there's none by that name, owned by `owner` (as `owner` reads it) or the
/// caller. → what `Node::deploy_making` says.
async fn deploy(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let bundle = match body["files"].as_object() {
        Some(files) => {
            let files = crate::app::deploy::given(files)?;
            // Built only where the deploy was asked, by its runtime: never on a node for someone elsewhere.
            if let Some((screen, _)) = files.iter().find(|(p, _)| crate::app::deploy::has_screens([p.as_str()])) {
                return Err(anyhow::anyhow!("{screen} is a React screen, built on the machine that deploys it with Bun or Node: deploy it from there, with {cli} deploy <folder> or {cli} mcp's deploy tool", cli = crate::config::CLI).into());
            }
            let built = tokio::task::spawn_blocking(move || crate::app::deploy::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>()));
            node.put_blob_for(&me.caller, &built.await.map_err(anyhow::Error::from)??)?
        }
        None => text(&body, "bundle")?.to_string(),
    };
    let owner = body["owner"].as_str().map(|o| owner(&node, o)).transpose()?;
    Ok(Json(node.deploy_making(&me.caller, text(&body, "app")?, &bundle, owner.as_deref(), body["wait"] == true).await?))
}

#[derive(Deserialize)]
struct SourceQuery {
    bundle: Option<String>,
    path: Option<String>,
}

/// `{ app, scope, installed, node }`: `scope` is `user`, `group:<id>` (or a group's name) or `global`; `node`, the
/// device (by name, or `cloudflare`) that runs its services. Installing an app also installs the apps it uses.
async fn install(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let scope = node.scope_for(&me.caller, text(&body, "scope")?)?;
    node.install(&me.caller, text(&body, "app")?, &scope, body["installed"] != false, body["node"].as_str()).await?;
    done()
}

async fn schedule(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let scope = node.scope_for(&me.caller, body["scope"].as_str().unwrap_or("user"))?;
    node.switch_schedule(&me.caller, text(&body, "app")?, text(&body, "schedule")?, &scope, body["on"] != false).await?;
    done()
}

/// `{ page, items, hidden, pages }`: how the local user arranges one of their pages (`pages` with their own: the order
/// of their groups' pages, `{ order, hidden }`).
async fn arrange(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.arrange(&me.caller, text(&body, "page")?, body["items"].clone(), body["hidden"].clone(), body["pages"].clone()).await?;
    done()
}

/// `{ app, how }`: deletes an app (`deleted`), restores it (`restored`) or erases it for good (`erased`).
async fn retire(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.retire(&me.caller, text(&body, "app")?, text(&body, "how")?).await?;
    done()
}

/// `{ app, name, owner? }`: a new app called `name`, from `app`, with data of its own. `owner` as `owner` reads it.
async fn fork(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let owner = body["owner"].as_str().map(|o| owner(&node, o)).transpose()?;
    node.fork(&Caller::local(&node)?, text(&body, "app")?, text(&body, "name")?, owner.as_deref()).await?;
    done()
}

/// An app's deployed code: its files, or with `path` one of them; with `bundle`, an earlier deploy's.
async fn source(State(node): State<Node>, Path(app): Path<String>, Query(q): Query<SourceQuery>) -> Reply {
    Ok(Json(node.source(&app, q.bundle.as_deref(), q.path.as_deref()).await?))
}
