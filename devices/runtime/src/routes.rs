//! Devices' routes that aren't tools: erasing what a device keeps, and who holds the recovery key.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};

use api::runtime::Rt;
use api::{Body, Me, Reply, Routes, Who, given};
use devices::says::Device;

pub fn routes() -> Routes<Rt> {
    Routes::default().route("/api/erase", Who::Clients, post(erase)).route("/api/devices/recovery-holders", Who::Clients, get(recovery_holders))
}

/// Erases what a device keeps, asked of it directly (`erase`), before it's removed.
async fn erase(State(rt): State<Rt>, me: Me, Body(body): Body<Device>) -> Reply {
    Ok(Json(crate::erase(&rt, &me.caller, given(&body.key, "key")?).await?))
}

async fn recovery_holders(State(rt): State<Rt>, me: Me) -> Reply {
    Ok(Json(crate::recovery_holders(&rt, &me.caller).await?))
}
