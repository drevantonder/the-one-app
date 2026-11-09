//! Who asked, decided for every route: the layers `guarded` puts on the routes, and the client devices and this
//! machine's own programs they tell apart. A sign-in is never one of them: it only lets its device ask to be let in, at
//! auth, and a device that isn't let in reaches no node.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::json;
use tokio_stream::StreamExt;

use crate::config::YOURS;
use crate::node::{Caller, Credential, Node};

use super::local::{from_this_machine, not_this_machine};
use super::reply::Error;
use super::{Routes, Who};

/// A request a client device sent over iroh (src/node/net.rs), with the key its connection proved it holds (or the
/// device that vouched for a session's endpoint), and until when what it hands counts (that session's certificate's
/// `until`): only the node puts this on a request, as an extension, which nothing a client sends can set.
#[derive(Clone, Copy)]
pub struct FromClient(pub crate::keys::Key, pub u64);

/// `routes` as `Who` says each may be asked, with this machine's own programs asking any.
pub fn guarded(routes: Routes, node: Node) -> Router {
    let who: std::sync::Arc<std::collections::HashMap<String, Who>> = std::sync::Arc::new(routes.who.into_iter().collect());
    routes
        .router
        .route_layer(axum::middleware::from_fn_with_state((node.clone(), who), only_this_machine))
        .layer(axum::middleware::map_response(sandboxed))
        .layer(axum::middleware::from_fn_with_state(node.clone(), traced))
        .with_state(node)
}

/// A request that asks for a trace (`x-one-trace`, a client in developer mode) gets what it did here in the same
/// header of its answer, as base64 of its JSON (`node::trace`).
async fn traced(State(node): State<Node>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    use crate::node::trace;
    if !req.headers().contains_key(crate::protocol::header::TRACE) {
        return next.run(req).await;
    }
    let t = trace::Trace::new();
    let mut resp = trace::within(Some(t.clone()), next.run(req)).await;
    let mut done = t.done(&node.device_name());
    done["names"] = node.compute_names();
    let done = data_encoding::BASE64.encode(done.to_string().as_bytes());
    if let Ok(value) = done.parse() {
        resp.headers_mut().insert(crate::protocol::header::TRACE, value);
    }
    resp
}

/// Only this machine's own programs (`from_this_machine`) drive its node: no web page, an app's screen this node served
/// included, and no program in a VM or emulator here. A client device's request is someone's instead, for what its
/// route's `Who` lets them ask.
async fn only_this_machine(State((node, routes)): State<(Node, std::sync::Arc<std::collections::HashMap<String, Who>>)>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.extensions().get::<axum::extract::MatchedPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let who = routes.get(path.as_str()).copied().unwrap_or(Who::ThisMachine);
    if let Some(FromClient(key, until)) = req.extensions().get::<FromClient>().copied() {
        return from_client(&node, key, until, who, req, next).await;
    }
    if !from_this_machine(&node, &req) {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": not_this_machine() }))).into_response()).await;
    }
    next.run(req).await
}

/// A client device's request over iroh is its user's, for what `Who::Clients` lets in: the device must still be one of the network's, and the keys it hands are for its key, kept no longer than
/// `until`.
async fn from_client(node: &Node, key: crate::keys::Key, until: u64, who: Who, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if who != Who::Clients {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response()).await;
    }
    let Some((user, name)) = node.client_now(&key).await else {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "this isn't one of your devices: approve it on one that is", "code": crate::protocol::code::NOT_LET_IN }))).into_response()).await;
    };
    let credential = Credential::Client(key);
    if let Some(handed) = req.headers().get(crate::protocol::header::KEYS).and_then(|h| h.to_str().ok())
        && let Err(err) = node.hand(&credential, &user, handed, until)
    {
        return refuse(req, (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response()).await;
    }
    // A client older than what this build reads of who's who is told to update before it does anything.
    if crate::database::reads_of(req.headers().get(crate::protocol::header::READS).and_then(|h| h.to_str().ok())) < crate::database::READS {
        return refuse(req, (StatusCode::UPGRADE_REQUIRED, Json(json!({ "error": crate::protocol::update_refusal(YOURS), "code": crate::protocol::code::UPDATE }))).into_response()).await;
    }
    req.extensions_mut().insert(Me { caller: Caller { key, user, agent: None, credential }, client: Some(name) });
    next.run(req).await
}

/// Refuses a request once what it sends is read, dropped as it comes: refused mid-body, the rest unread, the connection
/// closes with a reset that can reach the client before the refusal does, which it sees as a broken connection instead.
/// One that sends more than 1 MiB, or takes longer than a few seconds (anyone can reach this), is
/// refused with the connection closed, holding nothing.
async fn refuse(req: axum::extract::Request, mut refusal: Response) -> Response {
    let mut body = req.into_body().into_data_stream();
    let drain = async {
        let mut read = 0;
        while let Some(chunk) = body.next().await {
            read += chunk.map_err(|_| ())?.len();
            if read > 1 << 20 {
                return Err(());
            }
        }
        Ok(())
    };
    if !matches!(tokio::time::timeout(std::time::Duration::from_secs(5), drain).await, Ok(Ok(()))) {
        refusal.headers_mut().insert(axum::http::header::CONNECTION, axum::http::HeaderValue::from_static("close"));
    }
    refusal
}

/// Nothing this node answers runs as script at its address, where a page would ask as this machine's own programs: a
/// browser shows it sandboxed with no script, and never sniffs it into another type.
async fn sandboxed(mut resp: Response) -> Response {
    let headers = resp.headers_mut();
    headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("sandbox"));
    headers.insert(axum::http::header::X_CONTENT_TYPE_OPTIONS, axum::http::HeaderValue::from_static("nosniff"));
    resp
}

/// Who asked: this machine's own user, or a client device of the network, whom this device acts for.
#[derive(Clone)]
pub(crate) struct Me {
    pub(crate) caller: Caller,
    /// A client device's name; none for this machine's own programs.
    pub(crate) client: Option<String>,
}

impl axum::extract::FromRequestParts<Node> for Me {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, node: &Node) -> Result<Me, Error> {
        match parts.extensions.get::<Me>() {
            Some(me) => Ok(me.clone()),
            None => Ok(Me { caller: Caller::local(node)?, client: None }),
        }
    }
}
