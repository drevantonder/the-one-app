//! A client's report of what went wrong on it.

use axum::extract::State;
use axum::routing::post;

use crate::api::{Body, Me, Reply, Routes, Who, done};
use crate::node::Node;
use crate::says::insights::Report;

pub fn routes() -> Routes {
    Routes::default().route("/api/insights/report", Who::Clients, post(report))
}

async fn report(State(node): State<Node>, me: Me, Body(body): Body<Report>) -> Reply {
    node.report_insights(&me.caller, &body.platform, &body.events).await;
    done()
}
