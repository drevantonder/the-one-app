//! Apps' routes: listing them and their labs, arranging pages, and switching their schedules.

use axum::Json;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use crate::api::Routes;
use ::api::Body;
use ::api::Error;
use ::api::Me;
use ::api::Reply;
use ::api::Who;
use ::api::done;
use ::api::given;
use ::api::tagged;
use crate::node::Node;
use crate::says::apps::Arrange;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/apps", Who::Clients, get(apps))
        .route("/api/labs", Who::Clients, get(labs))
        .route("/api/schedule", Who::ThisMachine, post(schedule))
        .route("/api/arrange", Who::Clients, post(arrange))
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

async fn arrange(State(node): State<Node>, me: Me, Body(body): Body<Arrange>) -> Reply {
    node.arrange(&me.caller, given(&body.page, "page")?, body.items, body.hidden, body.pages).await?;
    done()
}
