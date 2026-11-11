//! Switching an app's schedule, as Settings and the CLI ask it: who may ask it.

use axum::extract::State;
use axum::routing::post;
use serde::Deserialize;

use api::runtime::prelude::*;
use api::{Body, Me, Reply, Routes, Who, done, given};

use crate::view;

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default().route("/api/schedule", Who::ThisMachine, post(schedule))
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

async fn schedule(State(rt): State<Rt>, me: Me, Body(body): Body<Schedule>) -> Reply {
    let scope = rt.read(|r| r.scope_for(&me.caller.user, body.scope.as_deref().unwrap_or("user")))?;
    view::switch(&rt, &me.caller, given(&body.app, "app")?, given(&body.schedule, "schedule")?, &scope, body.on).await?;
    done()
}
