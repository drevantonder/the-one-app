//! Apps' routes: listing them, deploying, adding, arranging, deleting and forking them, and their code.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::openapi::Op;
use crate::api::{Body, Error, Me, Reply, Routes, Who, done, given, tagged};
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/apps", Who::SignedIn, get(apps))
        .says(Op::get("Every app the caller can see, with its tools where it's added for them").answers_as(json!({
            "type": "array",
            "items": {
                "type": "object",
                "required": ["name", "owner", "deploys", "added"],
                "properties": {
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "owner": { "type": "string", "description": "user:<id>, group:<id> or admins" },
                    "from": { "type": ["string", "null"], "description": "the app it was forked from" },
                    "forks": { "type": "array", "items": { "type": "string" } },
                    "deploys": { "type": "integer" },
                    "bundle": { "type": ["string", "null"], "description": "its latest deploy" },
                    "added": { "type": "array", "items": { "type": "string" }, "description": "the scopes the caller sees it added in" },
                    "deleted": { "type": "object", "description": "{ time, by, yours }, once deleted", "additionalProperties": true },
                    "tools": { "type": "array", "items": crate::api::openapi::tool_spec(), "description": "where it's added for the caller" },
                    "widgets": { "type": "array", "items": { "type": "object" } },
                    "screens": { "type": "array", "items": { "type": "object" } },
                    "schedules": { "type": "array", "items": { "type": "object" } }
                },
                "additionalProperties": true
            }
        })))
        .route("/v1/labs", Who::SignedIn, get(labs))
        .says(Op::get("Every lab, for an admin").answers_as(json!({
            "type": "array",
            "items": { "type": "object", "required": ["title", "description", "on", "setting"], "properties": { "title": { "type": "string" }, "description": { "type": "string" }, "on": { "type": "boolean" }, "setting": { "type": "string" } }, "additionalProperties": true }
        })))
        .route("/v1/deploy", Who::SignedIn, post(deploy))
        .says(Op::post("Deploy an app, making it first if there's none by that name").body::<Deploy>().answers_as(json!({
            "type": "object",
            "required": ["bundle", "made", "here"],
            "properties": {
                "bundle": { "type": "string", "description": "the deploy's hash" },
                "made": { "type": "boolean", "description": "whether the app is new" },
                "before": { "type": ["string", "null"], "description": "the deploy it replaced" },
                "here": { "type": "boolean", "description": "whether this device runs it yet" },
                "tools": { "type": "array", "items": { "type": "string" }, "description": "its tools, once this device runs it" },
                "lacking": { "type": "array", "items": { "type": "string" }, "description": "those that give no title or no output" },
                "failed": { "type": "string", "description": "why it doesn't start" }
            }
        })))
        .route("/v1/schedule", Who::ThisMachine, post(schedule))
        .route("/v1/arrange", Who::SignedIn, post(arrange))
        .says(Op::post("Arrange one of the caller's pages").body::<Arrange>().done())
        .route("/v1/fork", Who::ThisMachine, post(fork))
        .route("/v1/source/{app}", Who::ThisMachine, get(source))
}

/// Who owns an app, as a request names them: `user:<id>`, `group:<id>`, `admins`, or a group's name.
fn owner(node: &Node, named: &str) -> anyhow::Result<String> {
    crate::database::scopes::Principal::parse(named).map(|p| p.to_string()).or_else(|_| node.find_group(named).map(|g| format!("group:{g}")))
}

async fn apps(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap) -> Result<Response, Error> {
    let apps = node.apps(&me.caller).await?;
    let body = serde_json::to_string(&apps)?;
    let tag = crate::screen::tag(&body);
    Ok(tagged(&headers, tag, ([(axum::http::header::CONTENT_TYPE, "application/json")], body)))
}

/// Every lab, for an admin's Settings › Labs (`Node::labs`).
async fn labs(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(json!(node.labs(&me.caller).await?)))
}

/// An app's `bundle`, or its `files` (each file's path → its text, built here as `one deploy` builds a folder), to
/// deploy, making the app first if there's none by that name, owned by `owner` (as `owner` reads it) or the caller.
#[derive(Deserialize, schemars::JsonSchema)]
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

fn yes() -> bool {
    true
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
#[derive(Deserialize, schemars::JsonSchema)]
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
