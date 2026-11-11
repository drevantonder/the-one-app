//! A client's report of what went wrong on it.

use axum::extract::State;
use axum::routing::post;

use api::runtime::Rt;
use api::{Body, Me, Reply, Routes, Who, done};
use insights::says::Report;

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default().route("/api/insights/report", Who::Clients, post(report))
}

async fn report(State(rt): State<Rt>, me: Me, Body(body): Body<Report>) -> Reply {
    crate::report_insights(&rt, &me.caller, &body.platform, &body.events).await;
    done()
}
