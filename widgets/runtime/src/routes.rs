//! Widgets' routes: the user's widgets and badges, drawn by their apps.

use axum::Json;
use axum::extract::{Query, State};
use axum::routing::get;
use serde_json::json;

use api::runtime::prelude::*;
use api::{Me, Reply, Routes, Who};
use widgets::says::{WidgetsAsked, WidgetsQuery};

use crate::draw;

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default()
        .route("/api/widgets", Who::Clients, get(widgets))
        .route("/api/badges", Who::Clients, get(badges))
}

async fn widgets(State(rt): State<Rt>, me: Me, Query(q): Query<WidgetsAsked>) -> Reply {
    let only = q.kind.as_deref().map(|kind| kind.split_once('.').ok_or_else(|| api::Invalid("a widget's kind is <app>.<widget>".into()))).transpose()?;
    Ok(Json(json!(draw::widgets(&rt, me.caller, q.scope.as_deref(), only).await?)))
}

async fn badges(State(rt): State<Rt>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
    Ok(Json(draw::badges(&rt, me.caller, q.scope.as_deref()).await?))
}
