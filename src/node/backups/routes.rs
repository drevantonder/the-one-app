//! The route Settings' Backups screen adds a place with, which takes the key its user types and so is not a tool
//! (`crate::backups::tools` are the others): who may ask it.

use axum::Json;
use axum::extract::State;
use axum::routing::post;

use crate::api::{Body, Me, Reply, Routes, Who};
use crate::node::Node;
use crate::says::backups::Place;

pub fn routes() -> Routes {
    Routes::default().route("/api/backups/place", Who::Clients, post(place))
}

async fn place(State(node): State<Node>, me: Me, Body(body): Body<Place>) -> Reply {
    node.only_admins_back_up(&me.caller)?;
    if me.caller.agent.is_some() && body.secret.is_some() {
        return Err(anyhow::anyhow!("agents never handle a place's key: ask the user with backups_add_place").into());
    }
    Ok(Json(node.add_place(&me.caller, &serde_json::to_value(body)?).await?))
}
