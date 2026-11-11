//! The routes that run a call: `/api/call`, one the Cloudflare node passes on, and an app's own requests.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};
use serde_json::{Value, json};

use crate::node::{Caller, Node};
use ::api::call::CallRequest;

use super::{Routes, Who};
use super::guard::Me;
use super::reply::{Body, Error, Reply, failed};

/// What a client that picks the node it asks itself says with each request (onet/src/lib.rs): a call this node can't
/// run is answered 421 with the nodes that can (`node::Elsewhere`), not passed on.

/// An app's own requests (`app_http`): how stock git reaches Repos.
pub(super) const APP_ROUTE: &str = "/{app}/{*path}";

/// Whether `path` is one of an app's own requests (`APP_ROUTE`), not the API's.
pub fn app_route(path: &str) -> bool {
    ::api::path::app_request(path).is_some()
}

pub(super) fn routes() -> Routes {
    Routes::default()
        .route("/api/call", Who::Clients, post(call).layer(axum::extract::DefaultBodyLimit::max(64 << 20)))
        // An app's own requests, run as its tool `_http` for its user, whose checks are the app's: git's, to Repos.
        .route(APP_ROUTE, Who::Clients, any(app_http))
}

async fn call(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap, Body(req): Body<CallRequest>) -> Reply {
    Ok(Json(call_for(&node, me, req, headers.contains_key(custom_build::protocol::header::ELSEWHERE)).await?))
}

/// A call as `/api/call` takes it, for `me`: → `{ ok }`. A client that `picks` its node has it run here, or told where.
async fn call_for(node: &Node, me: Me, mut req: CallRequest, picks: bool) -> anyhow::Result<Value> {
    // Only the node says an app made a call, and how deep it is.
    (req.from, req.depth) = (None, 0);
    if picks {
        return Ok(json!({ "ok": node.call_picked(req, me.caller).await? }));
    }
    Ok(json!({ "ok": node.call_as(req, me.caller).await? }))
}

/// A call the Cloudflare node passes on over iroh (src/node/net.rs): its user's, as `me.user` says, run with the keys
/// of its databases they handed the Cloudflare node, which it hands this device (`keys`, sealed to it for the
/// Cloudflare node's key, `Credential::Passed`) for the scopes the call names alone, and never this device's own: no
/// access token comes with it, so none of theirs reaches a node on a machine, whose owner could use it as them.
/// → `{ ok }`, or what failed as the API says it.
pub(crate) async fn call_from_cloudflare_node(node: &Node, remote: key::Key, mut call: Value) -> Value {
    let (me, keys) = match call.as_object_mut() {
        Some(c) => (c.remove("me").unwrap_or_default(), c.remove("keys")),
        None => (Value::Null, None),
    };
    let ran = async {
        let user = me["user"].as_str().filter(|u| !u.is_empty()).ok_or_else(|| anyhow::anyhow!("whose call is it?"))?.to_string();
        anyhow::ensure!(node.is_user(&user), "{user} isn't in this network");
        let handed = keys.as_ref().and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("a call passed on brings the keys it was handed"))?;
        let credential = node.take_passed(&remote, &user, &call, handed, false)?;
        let client = me["client"].as_str().unwrap_or_default().to_string();
        // As the Cloudflare node's, never this device's own: no service counts it as its user's own node.
        call_for(node, Me { caller: Caller { key: remote, user, agent: None, credential }, client: Some(client) }, serde_json::from_value(call)?, false).await
    };
    ran.await.unwrap_or_else(|err| failed(&err))
}

/// The header carrying a line an app's `_http` wants git to show.
pub const APP_SAYS: &str = "x-app-says";

/// A request to `/<app>/<path>`, answered by the app's `_http` tool, run as whoever asked (this machine's user,
/// or a client device's), whose checks are the app's own: the body reaches it as a blob, read as it comes
/// and deleted once it's answered unless something keeps it, and it answers `{ status, type, text }`, or
/// `{ status, type, blob }` for bytes it keeps, with `say` for a line git shows whoever asked (`APP_SAYS`).
async fn app_http(State(node): State<Node>, me: Me, Path((app, path)): Path<(String, String)>, req: axum::extract::Request) -> Result<Response, Error> {
    let method = req.method().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let empty = matches!(method.as_str(), "GET" | "HEAD") || req.headers().get(axum::http::header::CONTENT_LENGTH).is_some_and(|l| l == "0");
    let body = match empty {
        true => None,
        false => Some(node.put_blob_from(&me.caller, req.into_body()).await?),
    };
    let body = body.and_then(|(hash, size)| (size > 0).then_some(hash));
    let input = json!({ "method": method, "path": path, "query": query, "body": body });
    let call = CallRequest { app, tool: "_http".into(), input, puts: body.iter().cloned().collect(), ..Default::default() };
    let out = node.call_as(call, me.caller.clone()).await;
    if let Some(hash) = &body {
        node.drop_put(hash);
    }
    let out = out?;
    let status = StatusCode::from_u16(out["status"].as_u64().unwrap_or(200) as u16)?;
    let kind = out["type"].as_str().unwrap_or("text/plain").to_string();
    let bytes = match out["blob"].as_str() {
        Some(hash) => node.blob_for(&me.caller, hash).await?,
        None => out["text"].as_str().unwrap_or("").as_bytes().to_vec(),
    };
    let mut reply = (status, [(axum::http::header::CONTENT_TYPE, kind)], bytes).into_response();
    if let Some(said) = out["say"].as_str() {
        reply.headers_mut().insert(APP_SAYS, axum::http::HeaderValue::from_str(said)?);
    }
    Ok(reply)
}
