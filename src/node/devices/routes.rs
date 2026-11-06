//! Devices' routes: letting one in, naming and taking one out, inviting someone, and where an app's schedules run.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::api::{Me, Reply, Routes, Who, done, only, text};
use crate::database::scopes::Role;
use crate::node::{Caller, Node};

pub fn routes() -> Routes {
    Routes::default()
        .route("/v1/pair", Who::SignedIn, post(pair))
        .route("/v1/name", Who::SignedIn, post(name))
        .route("/v1/remove", Who::SignedIn, post(remove))
        .route("/v1/erase", Who::SignedIn, post(erase))
        .route("/v1/invite-link", Who::SignedIn, post(invite_link))
        .route("/v1/devices/requests", Who::SignedIn, get(device_requests))
        .route("/v1/devices/approve", Who::SignedIn, post(approve))
        .route("/v1/devices/refuse", Who::SignedIn, post(refuse_device))
        .route("/v1/devices/forget-sign-in", Who::SignedIn, post(forget_sign_in))
        .route("/v1/app-schedules", Who::ThisMachine, post(app_schedules))
        .route("/v1/runs", Who::SignedIn, post(runs))
}

async fn pair(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let (code, user) = node.pair_as(&me.caller, body["user"].as_str())?;
    Ok(Json(json!({ "code": code, "user": user })))
}

/// `{ name, key? }`: renames one of its person's devices (by its key), or with no key, this one, for this machine's
/// own programs.
async fn name(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.rename(&me.caller, body["key"].as_str(), text(&body, "name")?).await?;
    done()
}

/// `{ key }` (a device, by name or key: `Node::remove_device`) or `{ user }` (a person: `Node::remove_person`).
async fn remove(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    match body["user"].as_str() {
        Some(user) => Ok(Json(node.remove_person(&me.caller, user).await?)),
        None => Ok(Json(node.remove_device(&me.caller, text(&body, "key")?).await?)),
    }
}

/// `{ key }`: erases what a device keeps, asked of it directly (`Node::erase`), before it's removed.
async fn erase(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["key"])?;
    Ok(Json(node.erase(&me.caller, text(&body, "key")?).await?))
}

/// `{ name, admin? }`: a link that lets someone new join, for this device's person, an admin (`Node::invite_someone`):
/// → `{ link, until }`.
async fn invite_link(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["name", "admin"])?;
    let role = if body["admin"] == true { Role::Admin } else { Role::Member };
    let (link, until) = node.invite_someone(&me.caller, text(&body, "name")?, role).await?;
    Ok(Json(json!({ "link": link, "until": until })))
}

/// The requests to join this device's person may approve (`Node::device_requests`).
async fn device_requests(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(json!(node.device_requests(&me.caller).await?)))
}

/// `{ code, confirm?, recovery? }`: lets in the device showing `code`, as whoever asked, or with `recovery`,
/// the network's recovery key (hex), read from a file here by the CLI and never kept.
async fn approve(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["code", "confirm", "recovery"])?;
    let (code, confirm) = (text(&body, "code")?, body["confirm"] == true);
    if let Some(recovery) = body["recovery"].as_str() {
        let secret: [u8; 32] = crate::keys::unhex(recovery.trim()).map_err(|_| anyhow::anyhow!("a recovery key is 64 hex characters"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        return Ok(Json(node.approve_with_recovery(&me.caller, code, &key, confirm).await?));
    }
    Ok(Json(node.approve(&me.caller, code, confirm).await?))
}

/// `{ code }`: turns down the device showing `code` (`Node::refuse_device`).
async fn refuse_device(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["code"])?;
    Ok(Json(node.refuse_device(&me.caller, text(&body, "code")?).await?))
}

/// `{ key }`: forgets what one of the caller's devices signed in with.
async fn forget_sign_in(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["key"])?;
    node.forget_sign_in(&me.caller, text(&body, "key")?).await?;
    Ok(Json(json!({})))
}

/// `{ device, app, scope?, days? }`: runs an app's schedules in a scope (`user` by default) on a device of the network,
/// for `days` (30 by default; 0 stops them), as this machine's person (`Node::set_app_schedules`).
async fn app_schedules(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    only(&body, &["device", "app", "scope", "days"])?;
    let me = Caller::local(&node)?;
    let scope = node.scope_for(&me, body["scope"].as_str().unwrap_or("user"))?;
    let days = body["days"].as_u64().unwrap_or(crate::node::APP_SCHEDULES_DAYS);
    Ok(Json(node.set_app_schedules(&me, text(&body, "device")?, text(&body, "app")?, &scope, days).await?))
}

/// `{ on: [device] }` (names or keys): the network's machines that run the caller's own things besides their own, as
/// their own device says.
async fn runs(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["on"])?;
    let on = body["on"].as_array().ok_or_else(|| anyhow::anyhow!("on is a list of machines"))?.iter().map(|d| node.find_machine(d.as_str().unwrap_or(""))).collect::<anyhow::Result<Vec<_>>>()?;
    node.member_content(&me.caller, crate::database::content::runs(&on)).await?;
    done()
}
