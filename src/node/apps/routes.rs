//! Apps' routes: listing them, deploying, adding, arranging, deleting and forking them, and their code.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{Body, Error, Me, Reply, Routes, Who, done, given, tagged};
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

/// An app's `bundle`, or its `files` (each file's path → its text, built here as `one deploy` builds a folder), to
/// deploy, making the app first if there's none by that name, owned by `owner` (as `owner` reads it) or the caller.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deploy {
    app: String,
    bundle: Option<String>,
    files: Option<serde_json::Map<String, Value>>,
    owner: Option<String>,
    #[serde(default)]
    wait: bool,
}

/// → what `Node::deploy_making` says.
async fn deploy(State(node): State<Node>, me: Me, Body(body): Body<Deploy>) -> Reply {
    let bundle = match &body.files {
        Some(files) => {
            let files = crate::app::deploy::given(files)?;
            // Built only where the deploy was asked, by its runtime: never on a node for someone elsewhere.
            if let Some((screen, _)) = files.iter().find(|(p, _)| crate::app::deploy::has_screens([p.as_str()])) {
                return Err(anyhow::anyhow!("{screen} is a React screen, built on the machine that deploys it with Bun or Node: deploy it from there, with {cli} deploy <folder> or {cli} mcp's deploy tool", cli = crate::config::CLI).into());
            }
            let built = tokio::task::spawn_blocking(move || crate::app::deploy::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>()));
            node.put_blob_for(&me.caller, &built.await.map_err(anyhow::Error::from)??)?
        }
        None => given(body.bundle.as_deref().unwrap_or_default(), "bundle")?.to_string(),
    };
    let owner = body.owner.as_deref().map(|o| owner(&node, o)).transpose()?;
    Ok(Json(node.deploy_making(&me.caller, given(&body.app, "app")?, &bundle, owner.as_deref(), body.wait).await?))
}

#[derive(Deserialize)]
struct SourceQuery {
    bundle: Option<String>,
    path: Option<String>,
}

/// Adds an app (or with `installed: false` takes it out) in `scope`: `user`, `group:<id>` (or a group's name) or
/// `global`; `node`, the device (by name, or `cloudflare`) that runs its services. Adding an app also adds the apps
/// it uses.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    app: String,
    scope: String,
    #[serde(default = "yes")]
    installed: bool,
    node: Option<String>,
}

fn yes() -> bool {
    true
}

async fn install(State(node): State<Node>, me: Me, Body(body): Body<Install>) -> Reply {
    let scope = node.scope_for(&me.caller, given(&body.scope, "scope")?)?;
    node.install(&me.caller, given(&body.app, "app")?, &scope, body.installed, body.node.as_deref()).await?;
    done()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Schedule {
    app: String,
    schedule: String,
    scope: Option<String>,
    #[serde(default = "yes")]
    on: bool,
}

async fn schedule(State(node): State<Node>, me: Me, Body(body): Body<Schedule>) -> Reply {
    let scope = node.scope_for(&me.caller, body.scope.as_deref().unwrap_or("user"))?;
    node.switch_schedule(&me.caller, given(&body.app, "app")?, given(&body.schedule, "schedule")?, &scope, body.on).await?;
    done()
}

/// How the local user arranges one of their pages (`pages` with their own: the order of their groups' pages,
/// `{ order, hidden }`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrange {
    page: String,
    #[serde(default)]
    items: Value,
    #[serde(default)]
    hidden: Value,
    #[serde(default)]
    pages: Value,
}

async fn arrange(State(node): State<Node>, me: Me, Body(body): Body<Arrange>) -> Reply {
    node.arrange(&me.caller, given(&body.page, "page")?, body.items, body.hidden, body.pages).await?;
    done()
}

/// Deletes an app (`how: deleted`), restores it (`restored`) or erases it for good (`erased`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retire {
    app: String,
    how: String,
}

async fn retire(State(node): State<Node>, me: Me, Body(body): Body<Retire>) -> Reply {
    node.retire(&me.caller, given(&body.app, "app")?, given(&body.how, "how")?).await?;
    done()
}

/// A new app called `name`, from `app`, with data of its own. `owner` as `owner` reads it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fork {
    app: String,
    name: String,
    owner: Option<String>,
}

async fn fork(State(node): State<Node>, Body(body): Body<Fork>) -> Reply {
    let owner = body.owner.as_deref().map(|o| owner(&node, o)).transpose()?;
    node.fork(&Caller::local(&node)?, given(&body.app, "app")?, given(&body.name, "name")?, owner.as_deref()).await?;
    done()
}

/// An app's deployed code: its files, or with `path` one of them; with `bundle`, an earlier deploy's.
async fn source(State(node): State<Node>, Path(app): Path<String>, Query(q): Query<SourceQuery>) -> Reply {
    Ok(Json(node.source(&app, q.bundle.as_deref(), q.path.as_deref()).await?))
}
