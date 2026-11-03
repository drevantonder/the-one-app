//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; how a client device
//! reaches it over iroh (src/node/net.rs), as its person; and how someone signed in with the install's kernel (a
//! browser, through the gateway, or the Mac app's window here) reaches it, as themselves, for what their sign-in opens.

use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::members::Role;
use crate::node::{CallRequest, Caller, Node};

/// Where this machine's node serves its local API: the config's port, or `<CLI>_ADDR`.
pub fn addr() -> String {
    std::env::var(format!("{}_ADDR", crate::config::CLI.to_uppercase())).unwrap_or_else(|_| format!("127.0.0.1:{}", crate::config::PORT))
}

pub fn router(node: Node) -> Router {
    guarded(routes(), node)
}

/// MCP over HTTP, at `/mcp` (the CLI's `mcp_http`), which this crate can't name: set once, before the node serves.
static MCP: std::sync::OnceLock<axum::routing::MethodRouter<Node>> = std::sync::OnceLock::new();

/// Serves MCP at `/mcp`, for this machine's own programs and the install's client devices over iroh: the MCP Worker,
/// remote MCP clients' device, among them (kernel/mcp/).
pub fn serve_mcp(handler: axum::routing::MethodRouter<Node>) {
    let _ = MCP.set(handler);
}

/// The file in a node's home holding the token its machine's own programs send, in `LOCAL_TOKEN_HEADER`, to say they
/// are: only the machine's user can read it. The CLI, its MCP, sidecars and the Mac app read it there.
pub const LOCAL_TOKEN: &str = "local-token";
pub const LOCAL_TOKEN_HEADER: &str = "x-one-local-token";
/// The keys a token caller hands this device for its calls: `keys::seal_handoff`'s.
pub const KEYS_HEADER: &str = "x-one-keys";

/// A session's endpoint's certificate (`keys::certify`), on each request it sends over iroh: who it asks as.
pub const ENDPOINT_HEADER: &str = "x-one-endpoint";

/// A request a client device sent over iroh (src/node/net.rs), with the key its connection proved it holds (or the
/// device that vouched for a session's endpoint), and until when what it hands counts (that session's certificate's
/// `until`): only the node puts this on a request, as an extension, which nothing a client sends can set.
#[derive(Clone, Copy)]
pub struct FromClient(pub crate::entry::Key, pub u64);

/// The client devices whose MCP requests this node's own MCP is answering (`mcp_http`), each by a ticket made for that
/// request alone: a random secret only this process sees, so nothing a request says names a device, and the keys
/// handed for one are reached only as its connection proved it.
static ASKING_AS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (crate::entry::Key, u64)>>> = std::sync::LazyLock::new(Default::default);

/// What this node's own MCP asks its API with while it answers a client device's request: its ticket, until dropped.
pub struct AskingAs(String);

impl AskingAs {
    /// For the client device `key`, whose handed keys count until `until` (`FromClient`'s).
    pub fn new(key: crate::entry::Key, until: u64) -> AskingAs {
        let ticket = format!("ask.{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>()));
        ASKING_AS.lock().unwrap().insert(ticket.clone(), (key, until));
        AskingAs(ticket)
    }

    pub fn ticket(&self) -> &str {
        &self.0
    }
}

impl Drop for AskingAs {
    fn drop(&mut self) {
        ASKING_AS.lock().unwrap().remove(&self.0);
    }
}

/// The client device a ticket is for, and until when what it hands counts, while its request is answered.
fn asking_as(ticket: &str) -> Option<(crate::entry::Key, u64)> {
    ASKING_AS.lock().unwrap().get(ticket).copied()
}

/// Serves the local API on `listener`, knowing each request's peer, which `from_this_machine` checks.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    serve_until(listener, router, std::future::pending()).await
}

/// `serve`, until `stop`: then it takes no more connections, and returns once those it has are answered, or after
/// `handoff::drain_within`. As a node hands over, it stops taking connections at once, leaving them to the next, and
/// its answers say to close theirs for five seconds before it closes those left, so clients that keep one (the
/// gateway's, through the tunnel, whose closing reaches it late) don't send on a closed one.
pub async fn serve_until(listener: tokio::net::TcpListener, router: Router, stop: impl std::future::Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    let (stopped, after) = tokio::sync::oneshot::channel::<()>();
    let router = router.layer(axum::middleware::map_response(|mut resp: Response| async move {
        if crate::handoff::stopped() {
            resp.headers_mut().insert(axum::http::header::CONNECTION, axum::http::HeaderValue::from_static("close"));
        }
        resp
    }));
    // Tapped only so its connections still say their peer.
    let listener = axum::serve::ListenerExt::tap_io(UntilHandover(listener), |_| {});
    let serving = axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(async move {
        stop.await;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = stopped.send(());
    });
    let serving = std::future::IntoFuture::into_future(serving);
    let deadline = async {
        let _ = after.await;
        tokio::time::sleep(crate::handoff::drain_within()).await;
    };
    tokio::select! {
        served = serving => served,
        _ = deadline => Ok(()),
    }
}

/// A listener that stops taking connections once the node hands over, for the next one to take.
struct UntilHandover(tokio::net::TcpListener);

impl axum::serve::Listener for UntilHandover {
    type Io = tokio::net::TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        tokio::select! {
            accepted = axum::serve::Listener::accept(&mut self.0) => accepted,
            _ = crate::handoff::stopping() => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.0.local_addr()
    }
}

/// An HTTP client for this machine's programs, asking the node as one (`as_this_machine`).
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).middleware(as_this_machine).build().into()
}

/// What a request asks the node at `addr()` carries the token in the node's home (`home()`), read when it's sent, so a
/// node started after the client is asked as itself.
pub fn as_this_machine(mut req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    if req.uri().authority().is_some_and(|a| a.as_str() == addr())
        && let Some(token) = local_token(&crate::home())
        && let Ok(value) = token.parse()
    {
        req.headers_mut().insert(LOCAL_TOKEN_HEADER, value);
    }
    next.handle(req)
}

/// The token of the node with home `home`, once it has started there.
pub fn local_token(home: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(home.join(LOCAL_TOKEN)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Every route of the local API, unguarded: `guarded` lets only this machine's programs and sign-ins in.
pub fn routes() -> Router<Node> {
    let routes = match MCP.get() {
        Some(mcp) => Router::new().route("/mcp", mcp.clone()),
        None => Router::new(),
    };
    routes
        .route("/llms.txt", get(llms))
        .route("/skills/{name}/{*path}", get(skill_file))
        .route("/v1/state", get(state))
        .route("/v1/endpoint", get(endpoint))
        .route("/v1/hand", post(hand))
        .route("/v1/keys", get(keys_get).post(keys_post))
        .route("/v1/apps", get(apps))
        .route("/v1/call", post(call).layer(axum::extract::DefaultBodyLimit::max(64 << 20)))
        .route("/v1/widgets", get(widgets))
        .route("/v1/badges", get(badges))
        .route("/v1/create", post(create))
        .route("/v1/pair", post(pair))
        .route("/v1/join", post(join))
        .route("/v1/name", post(name))
        .route("/v1/remove", post(remove))
        .route("/v1/role", post(role))
        .route("/v1/settings", post(settings))
        .route("/v1/entries", get(heads).post(entries))
        .route("/v1/use", post(use_))
        .route("/v1/runs", post(runs))
        .route("/v1/group", post(group))
        .route("/v1/invite", post(invite))
        .route("/v1/invite-link", post(invite_link))
        .route("/v1/devices/requests", get(device_requests))
        .route("/v1/services/install-link", post(install_link))
        .route("/v1/devices/approve", post(approve))
        .route("/v1/accept", post(accept))
        .route("/v1/member", post(member))
        .route("/v1/app", post(app))
        .route("/v1/deploy", post(deploy))
        .route("/v1/install", post(install))
        .route("/v1/schedule", post(schedule))
        .route("/v1/arrange", post(arrange))
        .route("/v1/retire", post(retire))
        .route("/v1/sidecar", post(sidecar))
        .route("/v1/fork", post(fork))
        .route("/v1/source/{app}", get(source))
        .route("/v1/kernel/run", post(kernel_run))
        .route("/v1/storage", get(storage_get).post(storage_post))
        .route("/v1/storage/{op}", post(storage_post))
        .route("/v1/tunnel", get(tunnel_get).post(tunnel_post))
        .route("/v1/backups", get(backups_get))
        .route("/v1/backups/{op}", post(backups_post))
        .route("/v1/vault", get(vault_get).post(vault_post))
        .route("/v1/vault/requests", get(vault_get))
        .route("/v1/vault/request", post(vault_post))
        .route("/v1/vault/answer", post(vault_post))
        .route("/v1/vault/fetch", post(vault_post))
        .route("/v1/node-release", post(release))
        .route("/v1/blobs", post(put_blob).layer(axum::extract::DefaultBodyLimit::max(1 << 30)))
        .route("/v1/blobs/{hash}", get(blob))
        .route("/v1/events", get(events))
        .route("/v1/screens/{app}/{screen}", get(screen))
        .route("/v1/client-screens/{name}", get(client_screen))
        .route("/hooks/{app}/{name}", post(hook))
        .route("/v1/sidecars/{app}", post(register))
        .route("/v1/sidecars/{app}/calls", get(next_call))
        .route("/v1/sidecars/{app}/calls/{id}", post(reply))
        .route("/v1/sidecars/{app}/calls/{id}/vault", post(fill))
        .route("/{app}/{*path}", any(app_http).layer(axum::extract::DefaultBodyLimit::max(1 << 30)))
}

/// Routes as only this machine's own programs, and people signed in with the kernel for what `SIGNED_IN` lists, may
/// use them.
pub fn guarded(routes: Router<Node>, node: Node) -> Router {
    routes.route_layer(axum::middleware::from_fn_with_state(node.clone(), only_this_machine)).layer(axum::middleware::map_response(sandboxed)).with_state(node)
}

/// Headers that say a request was passed on from elsewhere: the gateway sends `forwarded`, tunnels and proxies the rest.
pub const FORWARDED: &[&str] = &["forwarded", "x-forwarded-for", "x-forwarded-host", "x-real-ip", "cf-connecting-ip", "cf-worker"];

/// What someone signed in with the kernel may ask; the rest is for this machine's own programs.
pub const SIGNED_IN: &[&str] = &[
    "/v1/state",
    "/v1/endpoint",
    "/v1/keys",
    "/v1/apps",
    "/v1/call",
    "/v1/widgets",
    "/v1/badges",
    "/v1/screens/{app}/{screen}",
    "/v1/client-screens/{name}",
    "/v1/blobs/{hash}",
    "/v1/events",
    "/v1/arrange",
    "/v1/install",
    "/v1/retire",
    "/v1/vault",
    "/v1/vault/requests",
    "/v1/vault/request",
    "/v1/vault/answer",
    "/v1/vault/fetch",
    // What Settings' screens ask: each checked as its person.
    "/v1/pair",
    "/v1/remove",
    "/v1/role",
    "/v1/settings",
    "/v1/entries",
    "/v1/use",
    "/v1/runs",
    "/v1/group",
    "/v1/invite",
    "/v1/invite-link",
    "/v1/accept",
    "/v1/member",
    "/v1/backups",
    "/v1/backups/{op}",
    // A run the schedules service hands in, with its own token.
    "/v1/kernel/run",
];

/// The skills for this machine's person (src/skills.rs).
async fn skills(node: &Node) -> Vec<crate::skills::Skill> {
    let apps = match local(node) {
        Ok(me) => node.apps(&me).await.map(|a| json!(a)).unwrap_or_default(),
        Err(_) => json!([]),
    };
    crate::skills::all(apps.as_array().map(Vec::as_slice).unwrap_or_default())
}

fn markdown(text: String) -> Response {
    ([(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")], text).into_response()
}

/// The root skill's body, each skill under it linked at this device, as whoever asked reached it.
async fn llms(State(node): State<Node>, headers: axum::http::HeaderMap) -> Response {
    let host = headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("127.0.0.1").to_string();
    let deeper = skills(&node).await.split_off(1);
    markdown(crate::skills::root(&deeper, |s| format!("[{}](http://{host}/skills/{}/SKILL.md)", s.name, s.name)).body)
}

/// One of a skill's files.
async fn skill_file(State(node): State<Node>, Path((name, path)): Path<(String, String)>) -> Response {
    match skills(&node).await.into_iter().find(|s| s.name == name).and_then(|s| s.file(&path)) {
        Some(text) => markdown(text),
        None => (StatusCode::NOT_FOUND, "no such skill or file").into_response(),
    }
}

/// The local API's port.
pub fn port() -> u16 {
    addr().rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(crate::config::PORT)
}

/// Serves the skills, and nothing else, on this machine's Tailscale address and the local API's port, so an agent on
/// any of the person's devices can read them, starting at /llms.txt. Looks again every minute: Tailscale may come up
/// later, or move.
pub async fn serve_guide(node: Node) {
    let port = port();
    let mut serving: Option<(std::net::Ipv4Addr, tokio::task::JoinHandle<()>)> = None;
    loop {
        let ip = tailnet_address();
        if ip != serving.as_ref().map(|s| s.0) {
            if let Some((_, task)) = serving.take() {
                task.abort();
            }
            crate::skills::set_address(None);
            if let Some(ip) = ip {
                match tokio::net::TcpListener::bind((ip, port)).await {
                    Ok(listener) => {
                        crate::skills::set_address(Some(format!("http://{ip}:{port}/llms.txt")));
                        let app = Router::new().route("/llms.txt", get(llms)).route("/skills/{name}/{*path}", get(skill_file)).with_state(node.clone());
                        serving = Some((ip, tokio::spawn(async move { axum::serve(listener, app).await.ok(); })));
                    }
                    Err(e) => tracing::debug!("can't serve the skills on {ip}:{port}: {e}"),
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

/// This machine's Tailscale address, in 100.64.0.0/10, if it has one.
fn tailnet_address() -> Option<std::net::Ipv4Addr> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut at = list;
    while let Some(ifa) = unsafe { at.as_ref() } {
        if let Some(sa) = unsafe { ifa.ifa_addr.as_ref() }
            && sa.sa_family as i32 == libc::AF_INET
        {
            let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            if ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]) {
                found = Some(ip);
                break;
            }
        }
        at = ifa.ifa_next;
    }
    unsafe { libc::freeifaddrs(list) };
    found
}

/// Only this machine's own programs (`from_this_machine`) drive its node: no web page, an app's screen this node served
/// included, and no program in a VM or emulator here. A request with a kernel sign-in's access token is someone's
/// instead, for what `SIGNED_IN` lists.
async fn only_this_machine(State(node): State<Node>, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.extensions().get::<axum::extract::MatchedPath>().map(|p| p.as_str().to_string()).unwrap_or_default();
    if let Some(FromClient(key, until)) = req.extensions().get::<FromClient>().copied() {
        return from_client(&node, key, until, &path, req, next).await;
    }
    let token = req.headers().get(axum::http::header::AUTHORIZATION).and_then(|a| a.to_str().ok()).and_then(|a| a.strip_prefix("Bearer ")).map(|t| t.trim().to_string());
    // This machine's own MCP, answering a client device's request (`mcp_http`), asks as that device, by the ticket it
    // made for that request.
    if let Some((key, until)) = token.as_deref().and_then(asking_as) {
        if !from_this_machine(&node, &req) {
            return refuse(req, (StatusCode::UNAUTHORIZED, Json(json!({ "error": "that isn't a sign-in" }))).into_response()).await;
        }
        return from_client(&node, key, until, &path, req, next).await;
    }
    if let Some(token) = token {
        // This machine's own programs (its CLI and MCP, signed in) may ask anything; from elsewhere, only `SIGNED_IN`.
        if !SIGNED_IN.contains(&path.as_str()) && !from_this_machine(&node, &req) {
            return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response()).await;
        }
        // A run the schedules service hands in brings a token for that alone.
        if token.starts_with("run.") != (path == "/v1/kernel/run") {
            return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only a scheduled run asks that, and it asks nothing else" }))).into_response()).await;
        }
        return match signed_in(&node, token.clone()).await {
            // This machine's own programs, signed in as its node's own person, are this device's own client: they
            // use its keys, as when not signed in.
            Ok(me) if from_this_machine(&node, &req) && node.user().is_ok_and(|u| u == me.caller.user) && !token.starts_with("run.") => next.run(req).await,
            Ok(me) => {
                // The keys its caller hands this device for the token's calls, sealed to it.
                if let Some(handed) = req.headers().get(KEYS_HEADER).and_then(|h| h.to_str().ok())
                    && let Err(err) = node.hand(&token, &me.caller.user, handed, u64::MAX)
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

/// A client device's request over iroh is its person's, as a sign-in's is, for what `SIGNED_IN` lists and MCP: the
/// device must still be one of the install's, and the keys it hands are for its key, kept no longer than `until`.
async fn from_client(node: &Node, key: crate::entry::Key, until: u64, path: &str, mut req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !(SIGNED_IN.contains(&path) || path == "/mcp") || path == "/v1/kernel/run" {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response()).await;
    }
    let Some((user, name)) = node.client(&key) else {
        return refuse(req, (StatusCode::FORBIDDEN, Json(json!({ "error": "this isn't one of your devices: approve it on one that is" }))).into_response()).await;
    };
    let token = crate::node::client_token(&key);
    if let Some(handed) = req.headers().get(KEYS_HEADER).and_then(|h| h.to_str().ok())
        && let Err(err) = node.hand(&token, &user, handed, until)
    {
        return refuse(req, (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response()).await;
    }
    req.extensions_mut().insert(Me { caller: Caller { key, user, agent: None, token: Some(token) }, sign_in: Some(name) });
    next.run(req).await
}

/// Refuses a request once what it sends is read, dropped as it comes: refused mid-body, the rest unread, the connection
/// closes with a reset that can reach the client before the refusal does, which it sees as a broken connection instead.
/// One that sends more than 1 MiB, or takes longer than a few seconds (anyone can reach this through the gateway), is
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

/// What a program that didn't show the token is told, as JSON: an older client shows it as it shows any error. One
/// started before the node updated (an agent's MCP server, most often) is this machine's, and starting it again mends it.
fn not_this_machine() -> String {
    let name = crate::config::NAME;
    format!("{name} on this machine was updated after this program started, or this isn't one of this machine's own programs: start it again. An agent reconnects its {name} MCP server (/mcp in Claude Code) or starts a new session.")
}

/// Whether a request is from one of this machine's own programs: over a connection from this machine (its loopback),
/// with the token in the node's home (`LOCAL_TOKEN`), passed on by nothing, and from no page. Its address alone says
/// nothing: a program in a VM or emulator here reaches 127.0.0.1 through QEMU (10.0.2.2) or a forwarded port, and
/// names whatever host it likes, but can't read the token, which only this machine's user can. A browser says when a
/// page asked, its own address's too (`Origin`, `Sec-Fetch-Site`), and a screen this node served is an app's code,
/// which only a shell or a sandboxed frame answers.
fn from_this_machine(node: &Node, req: &axum::extract::Request) -> bool {
    let header = |name| req.headers().get(name).and_then(|h| h.to_str().ok());
    let peer = req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().is_some_and(|c| c.0.ip().is_loopback());
    // Compared as hashes, so how long it takes says nothing of the token.
    let digest = |t: &str| <sha2::Sha256 as sha2::Digest>::digest(t.as_bytes());
    let token = header(LOCAL_TOKEN_HEADER).is_some_and(|t| digest(t.trim()) == digest(node.local_token()));
    // The gateway, a tunnel or a proxy on this machine still says it passed on someone else's request.
    let forwarded = FORWARDED.iter().any(|h| req.headers().contains_key(*h));
    let page = header("origin").is_some() || header("sec-fetch-site").is_some_and(|s| s != "none");
    peer && token && !forwarded && !page
}

/// Nothing this node answers runs as script at its address, where a page would ask as this machine's own programs: a
/// browser shows it as the gateway does, sandboxed with no script, and never sniffs it into another type.
async fn sandboxed(mut resp: Response) -> Response {
    let headers = resp.headers_mut();
    headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("sandbox"));
    headers.insert(axum::http::header::X_CONTENT_TYPE_OPTIONS, axum::http::HeaderValue::from_static("nosniff"));
    resp
}

struct Error(anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0.downcast_ref::<crate::node::Needs>() {
            Some(_) => StatusCode::PRECONDITION_REQUIRED,
            None => StatusCode::BAD_REQUEST,
        };
        (status, Json(failed(&self.0))).into_response()
    }
}

/// Why something failed, as the API says it: `{ error }`, with `{ need, to, seal }` for keys a call wasn't handed.
fn failed(err: &anyhow::Error) -> Value {
    match err.downcast_ref::<crate::node::Needs>() {
        Some(n) => json!({ "error": n.to_string(), "need": n.need, "to": n.to, "seal": n.seal }),
        None => json!({ "error": format!("{err:#}") }),
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.into())
    }
}

type Reply = Result<Json<Value>, Error>;

fn done() -> Reply {
    Ok(Json(json!({})))
}

fn local(node: &Node) -> Result<Caller, Error> {
    Ok(Caller { key: node.id(), user: node.user()?, agent: None, token: None })
}

/// Who asked: this machine's own user, or someone signed in with the kernel, whom this device acts for.
#[derive(Clone)]
struct Me {
    caller: Caller,
    /// A sign-in's client's name; none for this machine's own programs.
    sign_in: Option<String>,
}

impl axum::extract::FromRequestParts<Node> for Me {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, node: &Node) -> Result<Me, Error> {
        match parts.extensions.get::<Me>() {
            Some(me) => Ok(me.clone()),
            None => Ok(Me { caller: local(node)?, sign_in: None }),
        }
    }
}

/// Someone signed in who isn't a person of the install yet: they are once one of their devices, or the admin who
/// invited them, approves one of theirs.
#[derive(Debug)]
struct NotJoined;

impl std::fmt::Display for NotJoined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "you haven't joined yet: the person who invited you approves this device with the code it shows")
    }
}

impl std::error::Error for NotJoined {}

/// Sign-ins checked lately, by their token's hash, until they expire.
static SIGNED_INS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<[u8; 32], (Me, u64)>>> = std::sync::LazyLock::new(Default::default);

/// Whom a kernel access token is for: one the auth service at the install's address signed for it, for a person of
/// the install.
async fn signed_in(node: &Node, token: String) -> anyhow::Result<Me> {
    use sha2::Digest;
    let now = crate::node::now_ms();
    let hash: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    if let Some((me, _)) = SIGNED_INS.lock().unwrap().get(&hash).filter(|(_, until)| now < *until) {
        return Ok(me.clone());
    }
    let address = node.kernel()?;
    let issuer = node.issuer()?;
    let asked = token.clone();
    let claims = tokio::task::spawn_blocking(move || {
        let claims = issuer.verify(&asked, now)?;
        // Only the install's own apps sign in for its address; a remote MCP client's token is for `<address>/mcp`, the
        // MCP Worker, whose device it is, and never a sign-in here.
        let address = address.trim_end_matches('/');
        anyhow::ensure!(claims.for_resource(address), "that sign-in isn't for {address}");
        Ok::<_, anyhow::Error>(claims)
    })
    .await??;
    let (user, until) = (claims.sub.clone(), claims.exp * 1000);
    anyhow::ensure!(node.is_user(&user), NotJoined);
    let me = Me { caller: Caller { key: node.id(), user, agent: None, token: Some(token) }, sign_in: Some(String::new()) };
    let mut kept = SIGNED_INS.lock().unwrap();
    kept.retain(|_, (_, until)| now < *until);
    kept.insert(hash, (me.clone(), until));
    Ok(me)
}

/// Refuses a body with anything but `keys`, so a word a route doesn't take (one renamed, say) fails rather than being
/// passed over: a call or sidecar meant for one device running on another.
fn only(body: &Value, keys: &[&str]) -> Result<(), Error> {
    match body.as_object().and_then(|o| o.keys().find(|k| !keys.contains(&k.as_str()))) {
        Some(other) => Err(anyhow::anyhow!("{other} isn't something this takes: {}", keys.join(", ")).into()),
        None => Ok(()),
    }
}

fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str, Error> {
    body[key].as_str().filter(|s| !s.trim().is_empty()).map(str::trim).ok_or_else(|| anyhow::anyhow!("give {key}").into())
}

/// A slot of a schedule the schedules service runs: `kernel::Run`, as the person who let it run.
async fn kernel_run(State(node): State<Node>, me: Me, Json(run): Json<crate::kernel::Run>) -> Reply {
    if !me.caller.token.as_deref().is_some_and(|t| t.starts_with("run.")) {
        return Err(anyhow::anyhow!("only the schedules service hands in runs").into());
    }
    Ok(Json(node.kernel_run(&me.caller, &run).await?))
}

/// This device as an endpoint, `{ key, seal }` (hex): what a caller seals the keys it hands a call to (`x-one-keys`).
async fn endpoint(State(node): State<Node>) -> Json<Value> {
    Json(json!(node.endpoint()))
}

/// `{ seal, token, need }`: what this machine's own programs hand another node for their person's calls there with
/// `token` (`x-one-keys`): the keys a 428's `need` names, each in its epoch, sealed to its `seal` key, the serverless
/// node's or a node's that runs those scopes. → `{ keys }`.
async fn hand(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["seal", "token", "need"])?;
    let seal = crate::keys::unhex(text(&body, "seal")?)?;
    let mut need: Vec<crate::keys::Need> = serde_json::from_value(body["need"].clone())?;
    need.sort();
    need.dedup();
    Ok(Json(json!({ "keys": node.hand_keys(&me.caller, &seal, text(&body, "token")?, &need).await? })))
}

/// `?scope=&to=<endpoint key>` or `?scope=&app=`: a scope's keys, sealed (`Node::sealed_keys`).
async fn keys_get(State(node): State<Node>, me: Me, Query(q): Query<std::collections::HashMap<String, String>>) -> Reply {
    let scope = crate::entry::Scope::parse(q.get("scope").map(String::as_str).unwrap_or(""))?;
    Ok(Json(node.sealed_keys(&me.caller, &scope, q.get("to").map(String::as_str), q.get("app").map(String::as_str)).await?))
}

/// `{ scope, n, to, sealed }` or `{ scope, n, app, sealed }`: keeps a key sealed by the caller's device (`Node::seal_key`).
async fn keys_post(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["scope", "n", "to", "app", "sealed"])?;
    let scope = crate::entry::Scope::parse(text(&body, "scope")?)?;
    let n = body["n"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| anyhow::anyhow!("give n, the epoch"))?;
    Ok(Json(node.seal_key(&me.caller, &scope, n, body["to"].as_str(), body["app"].as_str(), text(&body, "sealed")?).await?))
}

async fn state(State(node): State<Node>, signed_in: Option<axum::Extension<Me>>) -> Json<Value> {
    Json(match signed_in {
        Some(me) => node.state_for_sign_in(&me.caller, me.sign_in.as_deref().unwrap_or_default()),
        None => node.state(),
    })
}

async fn apps(State(node): State<Node>, me: Me) -> Reply {
    Ok(Json(json!(node.apps(&me.caller).await?)))
}

/// `{ name, device?, installed? }`: removes a sidecar from a device (this one by default), or with `installed` lets it
/// register there again.
async fn sidecar(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    only(&body, &["name", "device", "installed"])?;
    node.set_sidecar(&local(&node)?, body["device"].as_str(), text(&body, "name")?, body["installed"] == true).await?;
    done()
}

/// `{ app, how }`: deletes an app (`deleted`), restores it (`restored`) or erases it for good (`erased`).
async fn retire(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.retire(&me.caller, text(&body, "app")?, text(&body, "how")?).await?;
    done()
}

/// `{ page, items, hidden, pages }`: how the local user arranges one of their pages (`pages` with their own: the order
/// of their groups' pages, `{ order, hidden }`).
async fn arrange(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.arrange(&me.caller, text(&body, "page")?, body["items"].clone(), body["hidden"].clone(), body["pages"].clone()).await?;
    done()
}

async fn schedule(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let scope = node.scope_for(&me.caller, body["scope"].as_str().unwrap_or("user"))?;
    node.switch_schedule(&me.caller, text(&body, "app")?, text(&body, "schedule")?, &scope, body["on"] != false).await?;
    done()
}

async fn call(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    Ok(Json(call_for(&node, me, body).await?))
}

/// A call as `/v1/call` takes it, for `me`: → `{ ok }`.
async fn call_for(node: &Node, me: Me, body: Value) -> anyhow::Result<Value> {
    only(&body, &["id", "app", "sidecar", "op", "input", "scopes", "device", "agent", "depth", "from", "client"]).map_err(|e| e.0)?;
    let mut req: CallRequest = serde_json::from_value(body)?;
    // Only the node says an app made a call, and how deep it is.
    (req.from, req.depth) = (None, 0);
    if me.sign_in.is_some() {
        req = node.for_sign_in(req)?;
    }
    Ok(json!({ "ok": node.call_as(req, me.caller).await? }))
}

/// A call the serverless node passes on over iroh (src/node/net.rs): its person's, as `me.user` says, run with the keys
/// of its databases they handed the serverless node, which it hands this device (`keys`, sealed to it for the
/// serverless node's key, `node::passed_token`) for the scopes the call names alone, and never this device's own: no
/// access token comes with it, so none of theirs reaches a node on a machine, whose owner could use it as them.
/// → `{ ok }`, or what failed as the API says it.
pub(crate) async fn call_from_serverless(node: &Node, remote: crate::entry::Key, mut call: Value) -> Value {
    let (me, keys) = match call.as_object_mut() {
        Some(c) => (c.remove("me").unwrap_or_default(), c.remove("keys")),
        None => (Value::Null, None),
    };
    let ran = async {
        let user = me["user"].as_str().filter(|u| !u.is_empty()).ok_or_else(|| anyhow::anyhow!("whose call is it?"))?.to_string();
        anyhow::ensure!(node.is_user(&user), NotJoined);
        let handed = keys.as_ref().and_then(Value::as_str).ok_or_else(|| anyhow::anyhow!("a call passed on brings the keys it was handed"))?;
        let token = node.take_passed(&remote, &user, &call, handed, false)?;
        let client = me["client"].as_str().unwrap_or_default().to_string();
        call_for(node, Me { caller: Caller { key: node.id(), user, agent: None, token: Some(token) }, sign_in: Some(client) }, call).await
    };
    ran.await.unwrap_or_else(|err| failed(&err))
}

#[derive(Deserialize)]
struct WidgetsQuery {
    scope: Option<String>,
}

async fn badges(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
    Ok(Json(node.badges(me.caller, q.scope.as_deref()).await?))
}

async fn widgets(State(node): State<Node>, me: Me, Query(q): Query<WidgetsQuery>) -> Reply {
    Ok(Json(json!(node.widgets(me.caller, q.scope.as_deref()).await?)))
}

async fn create(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let recovery = node.create(text(&body, "name")?, text(&body, "user_name")?).await?;
    Ok(Json(json!({ "recovery": recovery })))
}

async fn pair(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let (code, user) = node.pair_as(&me.caller, body["user"].as_str())?;
    Ok(Json(json!({ "code": code, "user": user })))
}

async fn join(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.join(text(&body, "code")?).await?;
    done()
}

async fn name(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.rename_device(text(&body, "name")?).await?;
    done()
}

/// `{ key }` (a device, by name or key: `Node::remove_device`) or `{ user }` (a person: `Node::remove_person`).
async fn remove(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    match body["user"].as_str() {
        Some(user) => Ok(Json(node.remove_person(&me.caller, user).await?)),
        None => Ok(Json(node.remove_device(&me.caller, text(&body, "key")?).await?)),
    }
}

async fn role(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body["role"].clone())?;
    node.member_as(&me.caller, "role", json!({ "user": node.find_user(text(&body, "user")?)?, "role": role })).await?;
    done()
}

/// `{ name, value }`: one of the install's settings, for admins; `value: null` puts back the white label's default.
async fn settings(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.member_as(&me.caller, "setting", json!({ "name": text(&body, "name")?, "value": body["value"] })).await?;
    done()
}

/// `?scope=`: what a device signing an entry there needs: `{ epoch, previous, logs }` (`Node::entry_heads`).
async fn heads(State(node): State<Node>, me: Me, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let scope = match q.get("scope").map(String::as_str) {
        None | Some("user") => crate::entry::Scope::User(me.caller.user.clone()),
        Some(s) => crate::entry::Scope::parse(s)?,
    };
    Ok(Json(node.entry_heads(&me.caller, &scope).await?))
}

/// `[{ header, signature, body }]` (each base64url): entries the caller's own device signed, which this device checks
/// count and writes (`src/entry.rs`).
async fn entries(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let list = body.as_array().ok_or_else(|| anyhow::anyhow!("give a list of entries"))?;
    let entries = list.iter().map(crate::entry::Entry::from_json).collect::<anyhow::Result<Vec<_>>>()?;
    node.put_signed(&me.caller, entries).await?;
    done()
}

/// `{ on: [device] }` (names or keys): the install's machines that run the caller's own things besides their own, as
/// their own device says.
async fn runs(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["on"])?;
    let on = body["on"].as_array().ok_or_else(|| anyhow::anyhow!("on is a list of machines"))?.iter().map(|d| node.find_machine(d.as_str().unwrap_or(""))).collect::<anyhow::Result<Vec<_>>>()?;
    node.member_content(&me.caller, crate::members::runs(&on)).await?;
    done()
}

/// `{ device, use }`: puts a device (its name or key, or `cloudflare`) first, needed or last, for an admin.
async fn use_(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.set_use(&me.caller, text(&body, "device")?, text(&body, "use")?).await?;
    done()
}

async fn group(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let group = node.make_group(&me.caller, text(&body, "name")?).await?;
    Ok(Json(json!({ "group": group })))
}

async fn invite(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body.get("role").cloned().unwrap_or(json!("member")))?;
    let (group, user) = (node.find_group(text(&body, "group")?)?, node.find_user(text(&body, "user")?)?);
    node.member_as(&me.caller, "invite", json!({ "group": group, "user": user, "role": role })).await?;
    done()
}

/// `{ name, admin? }`: a link that lets someone new join, for this device's person, an admin (`Node::invite_someone`):
/// → `{ link, until }`.
async fn invite_link(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["name", "admin"])?;
    let role = if body["admin"] == true { Role::Admin } else { Role::Member };
    let (link, until) = node.invite_someone(&me.caller, text(&body, "name")?, role).await?;
    Ok(Json(json!({ "link": link, "until": until })))
}

/// A service's install link, for this machine's person, from this machine alone (`Node::install_link`): → `{ link }`.
async fn install_link(State(node): State<Node>) -> Reply {
    Ok(Json(json!({ "link": node.install_link(&local(&node)?).await? })))
}

/// The requests to join this device's person may approve (`Node::device_requests`).
async fn device_requests(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.device_requests(&local(&node)?).await?)))
}

/// `{ code, confirm? }`: approves the device showing `code` (`Node::approve`).
/// `{ code, confirm?, recovery? }`: lets in the device showing `code`, as this machine's person, or with `recovery`,
/// the install's recovery key (hex), read from a file here by the CLI and never kept.
async fn approve(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    only(&body, &["code", "confirm", "recovery"])?;
    let (code, confirm) = (text(&body, "code")?, body["confirm"] == true);
    if let Some(recovery) = body["recovery"].as_str() {
        let secret: [u8; 32] = crate::keys::unhex(recovery.trim()).map_err(|_| anyhow::anyhow!("a recovery key is 64 hex characters"))?;
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        return Ok(Json(node.approve_with_recovery(&local(&node)?, code, &key, confirm).await?));
    }
    Ok(Json(node.approve(&local(&node)?, code, confirm).await?))
}

async fn accept(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.member_as(&me.caller, "accept", json!({ "group": node.find_group(text(&body, "group")?)? })).await?;
    done()
}

/// `{ group, user, role }`, with `role: null` to remove them.
async fn member(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Option<Role> = serde_json::from_value(body["role"].clone())?;
    let (group, user) = (node.find_group(text(&body, "group")?)?, node.find_user(text(&body, "user")?)?);
    node.member_as(&me.caller, "member", json!({ "group": group, "user": user, "role": role })).await?;
    done()
}

/// `{ name, owner, from? }`: `owner` is `user:<id>`, `group:<id>`, `admins`, or a group's name.
async fn app(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let owner = text(&body, "owner")?;
    let owner = match crate::members::Principal::parse(owner) {
        Ok(p) => p.to_string(),
        Err(_) => format!("group:{}", node.find_group(owner)?),
    };
    let mut data = json!({ "name": text(&body, "name")?, "owner": owner });
    if let Some(from) = body["from"].as_str() {
        data["from"] = json!(from);
    }
    node.member("app", data).await?;
    done()
}

async fn deploy(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.deploy(&local(&node)?, text(&body, "app")?, text(&body, "bundle")?).await?;
    done()
}

#[derive(Deserialize)]
struct SourceQuery {
    bundle: Option<String>,
    path: Option<String>,
}

/// An app's deployed code: its files, or with `path` one of them; with `bundle`, an earlier deploy's.
async fn source(State(node): State<Node>, Path(app): Path<String>, Query(q): Query<SourceQuery>) -> Reply {
    Ok(Json(node.source(&app, q.bundle.as_deref(), q.path.as_deref()).await?))
}

/// `{ app, scope, installed }`: `scope` is `user`, `group:<id>` (or a group's name) or `global`.
/// Installing an app also installs the apps it uses.
async fn install(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let scope = node.scope_for(&me.caller, text(&body, "scope")?)?;
    node.install(&me.caller, text(&body, "app")?, &scope, body["installed"] != false).await?;
    done()
}

/// `{ app, name, owner? }`: a new app called `name`, from `app`, with data of its own. `owner` as for `/v1/app`.
async fn fork(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let owner = match body["owner"].as_str() {
        Some(o) => Some(crate::members::Principal::parse(o).map(|p| p.to_string()).or_else(|_| node.find_group(o).map(|g| format!("group:{g}")))?),
        None => None,
    };
    node.fork(&local(&node)?, text(&body, "app")?, text(&body, "name")?, owner.as_deref()).await?;
    done()
}

/// Where the install keeps its data, `/v1/storage…`.
async fn storage_get(State(node): State<Node>, uri: axum::http::Uri) -> Reply {
    Ok(Json(node.storage_route(&local(&node)?, "GET", uri.path(), &json!({})).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

async fn storage_post(State(node): State<Node>, uri: axum::http::Uri, Json(body): Json<Value>) -> Reply {
    Ok(Json(node.storage_route(&local(&node)?, "POST", uri.path(), &body).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

/// The install's tunnel, `/v1/tunnel`.
async fn tunnel_get(State(node): State<Node>) -> Reply {
    Ok(Json(node.tunnel_route(&local(&node)?, "GET", &json!({})).await?))
}

async fn tunnel_post(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    Ok(Json(node.tunnel_route(&local(&node)?, "POST", &body).await?))
}

/// Backups' routes, `/v1/backups…`.
async fn backups_get(State(node): State<Node>, me: Me, uri: axum::http::Uri) -> Reply {
    Ok(Json(node.backups_route(&me.caller, "GET", uri.path(), &json!({})).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

async fn backups_post(State(node): State<Node>, me: Me, uri: axum::http::Uri, Json(body): Json<Value>) -> Reply {
    Ok(Json(node.backups_route(&me.caller, "POST", uri.path(), &body).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

/// The Vault's routes, `/v1/vault…`: a GET's query goes to the node as a
/// POST's body does.
async fn vault_get(State(node): State<Node>, me: Me, uri: axum::http::Uri, Query(q): Query<std::collections::HashMap<String, String>>) -> Reply {
    let input: serde_json::Map<String, Value> = q.into_iter().map(|(k, v)| (k, json!(v))).collect();
    Ok(Json(node.vault_route(&me.caller, "GET", uri.path(), &json!(input)).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

async fn vault_post(State(node): State<Node>, me: Me, uri: axum::http::Uri, Json(body): Json<Value>) -> Reply {
    Ok(Json(node.vault_route(&me.caller, "POST", uri.path(), &body).await?.ok_or_else(|| anyhow::anyhow!("no such route"))?))
}

/// For agent harnesses' hooks: the body is the hook's JSON, the reply is plain text to show the agent.
async fn hook(State(node): State<Node>, Path((app, name)): Path<(String, String)>, body: String) -> Response {
    let event: Value = serde_json::from_str(&body).unwrap_or_default();
    match node.hook(&app, &name, event).await {
        Ok(Value::String(text)) => text.into_response(),
        Ok(Value::Null) => String::new().into_response(),
        Ok(other) => other.to_string().into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct ReleaseRequest {
    path: String,
    commit: String,
    target: String,
    #[serde(default)]
    committed: u64,
    signature: String,
}

async fn release(State(node): State<Node>, Json(req): Json<ReleaseRequest>) -> Reply {
    let r = node.publish(std::path::Path::new(&req.path), &req.commit, &req.target, req.committed, &req.signature).await?;
    Ok(Json(json!(r)))
}

/// A request to `/<app>/<path>`, answered by the app's `_http` operation, run as this machine's person: the body
/// reaches it as a blob, and it answers `{ status, type, text }`, or `{ status, type, blob }` for bytes it keeps.
async fn app_http(State(node): State<Node>, Path((app, path)): Path<(String, String)>, req: axum::extract::Request) -> Result<Response, Error> {
    let method = req.method().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let body = axum::body::to_bytes(req.into_body(), 1 << 30).await?;
    let body = if body.is_empty() { None } else { Some(node.put_blob(&body)?) };
    let input = json!({ "method": method, "path": path, "query": query, "body": body });
    let out = node.call(CallRequest { app, op: "_http".into(), input, ..Default::default() }).await?;
    let status = StatusCode::from_u16(out["status"].as_u64().unwrap_or(200) as u16)?;
    let kind = out["type"].as_str().unwrap_or("text/plain").to_string();
    let bytes = match out["blob"].as_str() {
        Some(hash) => node.blob(hash).await?,
        None => out["text"].as_str().unwrap_or("").as_bytes().to_vec(),
    };
    Ok((status, [(axum::http::header::CONTENT_TYPE, kind)], bytes).into_response())
}

async fn put_blob(State(node): State<Node>, me: Me, body: axum::body::Bytes) -> Reply {
    let hash = match me.sign_in {
        Some(_) => node.put_blob_for(&me.caller, &body)?,
        None => node.put_blob(&body)?,
    };
    Ok(Json(json!({ "blob": hash, "size": body.len() })))
}

async fn blob(State(node): State<Node>, me: Me, Path(hash): Path<String>) -> Result<Response, Error> {
    let bytes = match me.sign_in {
        Some(_) => node.blob_for(&me.caller, &hash).await?,
        None => node.blob(&hash).await?,
    };
    Ok(([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

async fn register(State(node): State<Node>, Path(app): Path<String>, Json(describe): Json<Value>) -> Reply {
    node.register_sidecar(&app, describe).await?;
    done()
}

async fn next_call(State(node): State<Node>, Path(app): Path<String>) -> Result<Response, Error> {
    Ok(match node.sidecars().next(&app).await? {
        Some(call) => Json(json!(call)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn reply(State(node): State<Node>, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Reply {
    node.sidecars().reply(&app, id, body)?;
    done()
}

/// A sidecar's text filled from the Vault for a call of its that waits, for the host it's going to.
async fn fill(State(node): State<Node>, Path((app, id)): Path<(String, u64)>, Json(body): Json<Value>) -> Reply {
    let text = node.sidecar_fill(&app, id, text(&body, "text")?, text(&body, "host")?).await?;
    Ok(Json(json!({ "text": text })))
}

#[derive(Deserialize)]
struct ScreenQuery {
    params: Option<String>,
    scope: Option<String>,
}

/// An app screen as a page, for shells' web views and the web client's frames.
async fn screen(State(node): State<Node>, me: Me, Path((app, name)): Path<(String, String)>, Query(q): Query<ScreenQuery>) -> Result<Response, Error> {
    let params: Value = q.params.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(json!({}));
    let html = node.screen(me.caller, &app, &name, params, q.scope.as_deref()).await?;
    Ok(axum::response::Html(html).into_response())
}

/// A client's own screen as a page (see `Node::client_screen`).
async fn client_screen(State(node): State<Node>, me: Me, Path(name): Path<String>, Query(q): Query<ScreenQuery>) -> Result<Response, Error> {
    let params: Value = q.params.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(json!({}));
    Ok(axum::response::Html(node.client_screen(me.caller, &name, params)?).into_response())
}

async fn events(State(node): State<Node>, me: Me) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    // Someone other than this machine's person hears only of the apps they reach.
    let user = (node.user().ok().as_deref() != Some(me.caller.user.as_str())).then_some(me.caller.user);
    let events = node.clone();
    let stream = BroadcastStream::new(node.subscribe())
        .filter_map(|e| e.ok())
        .map(move |e| match &user {
            Some(user) => events.event_for(e, user),
            None => e,
        })
        .map(|e| Ok(SseEvent::default().json_data(e).unwrap_or_default()));
    // Ended as the node hands over, so its client asks the new one.
    let stream = futures_util::StreamExt::take_until(stream, Box::pin(crate::handoff::stopping()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
