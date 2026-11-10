//! A client's report of what went wrong on it.

use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;
use serde_json::Value;

use crate::api::openapi::Op;
use crate::api::{Body, Me, Reply, Routes, Who, done};
use crate::node::Node;

pub fn routes() -> Routes {
    Routes::default().route("/api/insights/report", Who::Clients, post(report)).says(Op::post("What went wrong on a client, and what it did before that").body::<Report>().done())
}

/// `events`: what the client's timeline holds (`cloudflare/web/insights.js`), each `{ name, time, … }`.
#[derive(Deserialize, schemars::JsonSchema)]
struct Report {
    platform: String,
    events: Vec<Value>,
}

async fn report(State(node): State<Node>, me: Me, Body(body): Body<Report>) -> Reply {
    node.report_insights(&me.caller, &body.platform, &body.events).await;
    done()
}
