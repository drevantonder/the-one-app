//! Apps' screens as pages, and the events that say what changed.

use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::Response;
use axum::routing::get;
use serde_json::{Value, json};

use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::{Error, Me, Routes, Who, tagged};
use crate::node::Node;
use crate::says::screens::ScreenQuery;

pub fn routes() -> Routes {
    Routes::default()
        .route("/api/events", Who::Clients, get(events))
        .route("/api/screens/{app}/{screen}", Who::Clients, get(screen))
}

/// An app screen as a page, for shells' web views and the web client's frames.
async fn screen(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap, Path((app, name)): Path<(String, String)>, Query(q): Query<ScreenQuery>) -> Result<Response, Error> {
    let params: Value = q.params.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(json!({}));
    let html = node.screen(me.caller, &app, &name, params, q.scope.as_deref(), q.linked == Some(true)).await?;
    Ok(page(&headers, html))
}

/// A screen's page, named by its content but for its nonce (its ETag).
fn page(headers: &axum::http::HeaderMap, html: String) -> Response {
    let tag = crate::screen::tag(&html);
    tagged(headers, tag, axum::response::Html(html))
}

async fn events(State(node): State<Node>, me: Me) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    // Someone other than this machine's user hears only of the apps they reach.
    let user = (node.user().ok().as_deref() != Some(me.caller.user.as_str())).then_some(me.caller.user);
    let events = node.clone();
    let stream = BroadcastStream::new(node.subscribe())
        .filter_map(|e| e.ok())
        .map(move |e| match &user {
            Some(user) => events.event_for(e, user),
            None => e,
        })
        .map(|e| Ok(SseEvent::default().json_data(e).unwrap_or_default()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
