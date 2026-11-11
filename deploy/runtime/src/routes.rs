//! The deploy's routes: deploying an app, forking one, and its code.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use serde::Deserialize;

use api::runtime::prelude::*;
use api::{Body, Me, Reply, Routes, Who, done, given};
use deploy::says::Deploy;

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default()
        .route("/api/deploy", Who::Clients, post(deploy))
        .route("/api/fork", Who::ThisMachine, post(fork))
        .route("/api/source/{app}", Who::ThisMachine, get(source))
}

/// Who owns an app, as a request names them: `user:<id>`, `group:<id>`, `admins`, or a group's name.
fn owner(rt: &Rt, named: &str) -> anyhow::Result<String> {
    scope::scopes::Principal::parse(named).map(|p| p.to_string()).or_else(|_| rt.read(|r| r.db().find_group(named)).map(|g| format!("group:{g}")))
}

/// → what `deploy_making` says.
async fn deploy(State(rt): State<Rt>, me: Me, Body(body): Body<Deploy>) -> Reply {
    let bundle = match &body.files {
        Some(files) => {
            let files = deploy::build::given(files)?;
            // Built only where the deploy was asked, by its runtime: never on an app runtime for someone elsewhere.
            if let Some((screen, _)) = files.iter().find(|(p, _)| deploy::build::has_screens([p.as_str()])) {
                return Err(anyhow::anyhow!("{screen} is a React screen, built on the machine that deploys it with Bun or Node: deploy it from there, with {cli} deploy <folder> or {cli} mcp's deploy tool", cli = custom_build::config::CLI).into());
            }
            let built = tokio::task::spawn_blocking(move || deploy::build::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>()));
            database_runtime::put_for(&rt, &me.caller, &built.await.map_err(anyhow::Error::from)??)?
        }
        None => given(body.bundle.as_deref().unwrap_or_default(), "bundle")?.to_string(),
    };
    let owner = body.owner.as_deref().map(|o| owner(&rt, o)).transpose()?;
    Ok(Json(crate::deploy_making(&rt, &me.caller, given(&body.app, "app")?, &bundle, owner.as_deref(), body.wait).await?))
}

/// A new app called `name`, from `app`, with data of its own. `owner` as `owner` reads it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fork {
    app: String,
    name: String,
    owner: Option<String>,
}

async fn fork(State(rt): State<Rt>, Body(body): Body<Fork>) -> Reply {
    let owner = body.owner.as_deref().map(|o| owner(&rt, o)).transpose()?;
    crate::fork(&rt, &rt.caller()?, given(&body.app, "app")?, given(&body.name, "name")?, owner.as_deref()).await?;
    done()
}

#[derive(Deserialize)]
struct SourceQuery {
    bundle: Option<String>,
    path: Option<String>,
}

/// An app's deployed code: its files, or with `path` one of them; with `bundle`, an earlier deploy's.
async fn source(State(rt): State<Rt>, Path(app): Path<String>, Query(q): Query<SourceQuery>) -> Reply {
    Ok(Json(crate::source(&rt, &app, q.bundle.as_deref(), q.path.as_deref()).await?))
}
