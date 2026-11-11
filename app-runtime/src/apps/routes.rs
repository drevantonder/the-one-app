//! Apps' routes: listing them and their labs, and arranging pages.

use axum::Json;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
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
use crate::Host;
use api_features::says::apps::Arrange;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/apps", Who::Clients, get(apps))
        .route("/api/labs", Who::Clients, get(labs))
        .route("/api/arrange", Who::Clients, post(arrange))
}

async fn apps(State(host): State<Host>, me: Me, headers: axum::http::HeaderMap) -> Result<Response, Error> {
    let apps = host.apps(&me.caller).await?;
    let body = serde_json::to_string(&apps)?;
    let tag = ::ui::screen::tag(&body);
    Ok(tagged(&headers, tag, ([(axum::http::header::CONTENT_TYPE, "application/json")], body)))
}

/// Every lab, for an admin's Settings › Labs (`Host::labs`).
async fn labs(State(host): State<Host>, me: Me) -> Reply {
    Ok(Json(json!(host.labs(&me.caller).await?)))
}

async fn arrange(State(host): State<Host>, me: Me, Body(body): Body<Arrange>) -> Reply {
    host.arrange(&me.caller, given(&body.page, "page")?, body.items, body.hidden, body.pages).await?;
    done()
}
