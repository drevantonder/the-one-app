//! Who asked, decided for every route: the layers `guarded` puts on the routes, and the sign-in tokens, client devices and
//! this machine's own programs they tell apart.

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

/// The keys a token caller hands this device for its calls: `keys::seal_handoff`'s.
pub const KEYS_HEADER: &str = "x-one-keys";

/// A session's endpoint's certificate (`keys::certify`), on each request it sends over iroh: who it asks as.
pub const ENDPOINT_HEADER: &str = "x-one-endpoint";

/// A request a client device sent over iroh (src/node/net.rs), with the key its connection proved it holds (or the
/// device that vouched for a session's endpoint), and until when what it hands counts (that session's certificate's
/// `until`): only the node puts this on a request, as an extension, which nothing a client sends can set.
#[derive(Clone, Copy)]
pub struct FromClient(pub crate::keys::Key, pub u64);

/// `routes` as `Who` says each may be asked, with this machine's own programs asking any.
pub fn guarded(routes: Routes, node: Node) -> Router {
    let who: std::sync::Arc<std::collections::HashMap<&'static str, Who>> = std::sync::Arc::new(routes.who.into_iter().collect());
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
    if !req.headers().contains_key(trace::HEADER) {
        return next.run(req).await;
    }
    let t = trace::Trace::new();
    let mut resp = trace::within(Some(t.clone()), next.run(req)).await;
    let mut done = t.done(&node.device_name());
    done["names"] = node.compute_names();
    let done = data_encoding::BASE64.encode(done.to_string().as_bytes());
    if let Ok(value) = done.parse() {
        resp.headers_mut().insert(trace::HEADER, value);
    }
    resp
}

/// Only this machine's own programs (`from_this_machine`) drive its node: no web page, an app's screen this node served
/// included, and no program in a VM or emulator here. A request with an access token from auth, or from a client
/// device, is someone's instead, for what its route's `Who` lets them ask.
async fn only_this_machine(State((node, routes)): State<(Node, std::sync::Arc<std::collections::HashMap<&'static str, Who>>)>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.extensions().get::<axum::extract::MatchedPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    let who = routes.get(path.as_str()).copied().unwrap_or(Who::ThisMachine);
    if who == Who::Anyone {
        return next.run(req).await;
    }
    if let Some(FromClient(key, until)) = req.extensions().get::<FromClient>().copied() {
        return from_client(&node, key, until, who, req, next).await;
    }
    let token = req.headers().get(axum::http::header::AUTHORIZATION).and_then(|a| a.to_str().ok()).and_then(|a| a.strip_prefix("Bearer ")).map(|t| t.trim().to_string());
    if let Some(token) = token {
        // This machine's own programs (its CLI and MCP, signed in) may ask anything; from elsewhere, only `Who::SignedIn`.
        if who != Who::SignedIn && !from_this_machine(&node, &req) {
            return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response()).await;
        }
        return match signed_in(&node, token.clone()).await {
            // This machine's own programs, signed in as its node's own person, are this device's own client: they
            // use its keys, as when not signed in.
            Ok(me) if from_this_machine(&node, &req) && node.user().is_ok_and(|u| u == me.caller.user) => next.run(req).await,
            Ok(me) => {
                // The keys its caller hands this device for the token's calls, sealed to it.
                if let Some(handed) = req.headers().get(KEYS_HEADER).and_then(|h| h.to_str().ok())
                    && let Err(err) = node.hand(&me.caller.credential, &me.caller.user, handed, u64::MAX)
                {
                    return refuse(req, (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response()).await;
                }
                req.extensions_mut().insert(me);
                next.run(req).await
            }
            Err(err) if err.is::<NotJoined>() => refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": NotJoined.to_string() }))).into_response()).await,
            Err(err) => refuse(req, (StatusCode::UNAUTHORIZED, Json(json!({ "error": format!("{err:#}") }))).into_response()).await,
        };
    }
    if !from_this_machine(&node, &req) {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": not_this_machine() }))).into_response()).await;
    }
    next.run(req).await
}

/// A client device's request over iroh is its person's, as a sign-in's is, for what `Who::SignedIn` and `Who::Clients`
/// let in: the device must still be one of the network's, and the keys it hands are for its key, kept no longer than
/// `until`.
async fn from_client(node: &Node, key: crate::keys::Key, until: u64, who: Who, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !matches!(who, Who::SignedIn | Who::Clients) {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response()).await;
    }
    let Some((user, name)) = node.client_now(&key).await else {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "this isn't one of your devices: approve it on one that is", "code": crate::protocol::code::NOT_LET_IN }))).into_response()).await;
    };
    let credential = Credential::Client(key);
    if let Some(handed) = req.headers().get(KEYS_HEADER).and_then(|h| h.to_str().ok())
        && let Err(err) = node.hand(&credential, &user, handed, until)
    {
        return refuse(req, (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response()).await;
    }
    // A client older than what this build reads of who's who is told to update before it does anything.
    if crate::database::reads_of(req.headers().get(crate::database::HEADER).and_then(|h| h.to_str().ok())) < crate::database::READS {
        return refuse(req, (StatusCode::UPGRADE_REQUIRED, Json(json!({ "error": crate::protocol::update_refusal(YOURS), "code": crate::protocol::code::UPDATE }))).into_response()).await;
    }
    req.extensions_mut().insert(Me { caller: Caller { key, user, agent: None, credential }, sign_in: Some(name) });
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

/// Who asked: this machine's own user, or someone signed in at the network's address, whom this device acts for.
#[derive(Clone)]
pub(crate) struct Me {
    pub(crate) caller: Caller,
    /// A sign-in's client's name; none for this machine's own programs.
    pub(crate) sign_in: Option<String>,
}

impl axum::extract::FromRequestParts<Node> for Me {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, node: &Node) -> Result<Me, Error> {
        match parts.extensions.get::<Me>() {
            Some(me) => Ok(me.clone()),
            None => Ok(Me { caller: Caller::local(node)?, sign_in: None }),
        }
    }
}

/// Someone signed in who isn't a person of the network yet: they are once one of their devices, or the admin who
/// invited them, approves one of theirs.
#[derive(Debug)]
pub(super) struct NotJoined;

impl std::fmt::Display for NotJoined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "you haven't joined yet: the person who invited you approves this device with the code it shows")
    }
}

impl std::error::Error for NotJoined {}

/// A node's sign-ins checked lately, by their token's hash, until they expire.
pub(crate) type SignedIns = std::sync::Mutex<std::collections::HashMap<[u8; 32], (Me, u64)>>;

/// Whom an access token from auth is for: one auth at the network's address signed for it, for a person of
/// the network.
async fn signed_in(node: &Node, token: String) -> anyhow::Result<Me> {
    use sha2::Digest;
    let now = crate::node::now_ms();
    let hash: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    let kept = node.signed_ins().lock().unwrap().get(&hash).filter(|(_, until)| now < *until).map(|(me, _)| me.clone());
    if let Some(me) = kept {
        // Someone removed since is signed in no more, whatever their token says.
        if node.is_user(&me.caller.user) {
            return Ok(me);
        }
        node.signed_ins().lock().unwrap().remove(&hash);
        anyhow::bail!(NotJoined);
    }
    let address = node.address()?;
    let issuer = node.issuer()?;
    let asked = token.clone();
    let claims = tokio::task::spawn_blocking(move || {
        let claims = issuer.verify(&asked, now)?;
        // Only the network's own apps sign in for its address; a remote MCP client's token is for `<address>/mcp`, the
        // remote MCP server, whose device it is, and never a sign-in here.
        let address = address.trim_end_matches('/');
        anyhow::ensure!(claims.for_resource(address), "that sign-in isn't for {address}");
        Ok::<_, anyhow::Error>(claims)
    })
    .await??;
    let (user, until) = (claims.sub.clone(), claims.exp * 1000);
    anyhow::ensure!(node.is_user(&user), NotJoined);
    let me = Me { caller: Caller { key: node.id(), user, agent: None, credential: Credential::SignIn(token) }, sign_in: Some(String::new()) };
    let mut kept = node.signed_ins().lock().unwrap();
    kept.retain(|_, (_, until)| now < *until);
    kept.insert(hash, (me.clone(), until));
    Ok(me)
}
