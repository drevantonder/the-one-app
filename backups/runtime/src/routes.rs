//! The route Settings' Backups screen adds a place with, which takes the key its user types and so is not a tool
//! (`backups::tools` are the others): who may ask it.

use axum::Json;
use axum::extract::State;
use axum::routing::post;

use api::runtime::Rt;
use api::{Body, Me, Reply, Routes, Who};
use backups::says::Place;

use crate::run;

pub(crate) fn routes() -> Routes<Rt> {
    Routes::default().route("/api/backups/place", Who::Clients, post(place))
}

async fn place(State(rt): State<Rt>, me: Me, Body(body): Body<Place>) -> Reply {
    run::only_admins(&rt, &me.caller)?;
    if me.caller.agent.is_some() && body.secret.is_some() {
        return Err(anyhow::anyhow!("agents never handle a place's key: ask the user with backups_add_place").into());
    }
    Ok(Json(run::add_place(&rt, &me.caller, &serde_json::to_value(body)?).await?))
}
