//! Devices' routes: letting one in, naming and taking one out, inviting someone, and where an app's schedules run.

use axum::Json;
use axum::extract::State;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;

use crate::api::{Body, Me, Reply, Routes, Who, done, given};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    user: Option<String>,
}

async fn pair(State(node): State<Node>, me: Me, Body(body): Body<Pair>) -> Reply {
    let (code, user) = node.pair_as(&me.caller, body.user.as_deref())?;
    Ok(Json(json!({ "code": code, "user": user })))
}

/// Renames one of its person's devices (by its `key`), or with no key, this one, for this machine's own programs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Name {
    name: String,
    key: Option<String>,
}

async fn name(State(node): State<Node>, me: Me, Body(body): Body<Name>) -> Reply {
    node.rename(&me.caller, body.key.as_deref(), given(&body.name, "name")?).await?;
    done()
}

/// A device, by name or key (`Node::remove_device`), or a person (`user`: `Node::remove_person`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    key: Option<String>,
    user: Option<String>,
}

async fn remove(State(node): State<Node>, me: Me, Body(body): Body<Remove>) -> Reply {
    match body.user {
        Some(user) => Ok(Json(node.remove_person(&me.caller, &user).await?)),
        None => Ok(Json(node.remove_device(&me.caller, given(body.key.as_deref().unwrap_or_default(), "key")?).await?)),
    }
}

/// A device, by its key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Device {
    key: String,
}

/// Erases what a device keeps, asked of it directly (`Node::erase`), before it's removed.
async fn erase(State(node): State<Node>, me: Me, Body(body): Body<Device>) -> Reply {
    Ok(Json(node.erase(&me.caller, given(&body.key, "key")?).await?))
}

/// A link that lets someone new join, for this device's person, an admin (`Node::invite_someone`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    name: String,
    #[serde(default)]
    admin: bool,
}

/// → `{ link, until }`.
async fn invite_link(State(node): State<Node>, me: Me, Body(body): Body<Invite>) -> Reply {
    let role = if body.admin { Role::Admin } else { Role::Member };
    let (link, until) = node.invite_someone(&me.caller, given(&body.name, "name")?, role).await?;
    Ok(Json(json!({ "link": link, "until": until })))
}

/// The requests to join this device's person may approve (`Node::device_requests`).
async fn device_requests(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(json!(node.device_requests(&me.caller).await?)))
}

/// Lets in the device showing `code`, as whoever asked, or with `recovery`, the network's recovery key (hex), read from
/// a file here by the CLI and never kept.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approve {
    code: String,
    #[serde(default)]
    confirm: bool,
    recovery: Option<String>,
}

async fn approve(State(node): State<Node>, me: Me, Body(body): Body<Approve>) -> Reply {
    let code = given(&body.code, "code")?;
    if let Some(recovery) = &body.recovery {
        let secret: [u8; 32] = crate::keys::unhex(recovery.trim()).map_err(|_| anyhow::anyhow!("a recovery key is 64 hex characters"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        return Ok(Json(node.approve_with_recovery(&me.caller, code, &key, body.confirm).await?));
    }
    Ok(Json(node.approve(&me.caller, code, body.confirm).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Code {
    code: String,
}

/// Turns down the device showing `code` (`Node::refuse_device`).
async fn refuse_device(State(node): State<Node>, me: Me, Body(body): Body<Code>) -> Reply {
    Ok(Json(node.refuse_device(&me.caller, given(&body.code, "code")?).await?))
}

/// Forgets what one of the caller's devices signed in with.
async fn forget_sign_in(State(node): State<Node>, me: Me, Body(body): Body<Device>) -> Reply {
    node.forget_sign_in(&me.caller, given(&body.key, "key")?).await?;
    Ok(Json(json!({})))
}

/// Runs an app's schedules in a scope (`user` by default) on a device of the network, for `days` (30 by default; 0
/// stops them), as this machine's person (`Node::set_app_schedules`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppSchedules {
    device: String,
    app: String,
    scope: Option<String>,
    days: Option<u64>,
}

async fn app_schedules(State(node): State<Node>, Body(body): Body<AppSchedules>) -> Reply {
    let me = Caller::local(&node)?;
    let scope = node.scope_for(&me, body.scope.as_deref().unwrap_or("user"))?;
    let days = body.days.unwrap_or(crate::node::APP_SCHEDULES_DAYS);
    Ok(Json(node.set_app_schedules(&me, given(&body.device, "device")?, given(&body.app, "app")?, &scope, days).await?))
}

/// The network's machines (names or keys) that run the caller's own things besides their own, as their own device
/// says.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Runs {
    on: Vec<String>,
}

async fn runs(State(node): State<Node>, me: Me, Body(body): Body<Runs>) -> Reply {
    let on = body.on.iter().map(|d| node.find_machine(d)).collect::<anyhow::Result<Vec<_>>>()?;
    node.member_content(&me.caller, crate::database::content::runs(&on)).await?;
    done()
}
