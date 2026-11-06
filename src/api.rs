//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; how a client device
//! reaches it over iroh (src/node/net.rs), as its person; and how someone signed in at the network's address
//! (this machine's CLI and MCP, after `sign-in`) reaches it, as themselves, for what their sign-in opens.
//!
//! A request with an access token from sign-in for the network's address (never a remote MCP client's, which is for
//! `<address>/mcp`) is its person's, and so is its call's; someone it's for who isn't a person of the network yet is
//! let in as a member by a device whose own person is an admin, since sign-in lets in only those an admin invited
//! or people at the network's domain. A request without one is this machine's own programs', over its loopback with the
//! token in the node's home (`local-token`, which only its user reads), or refused, but for a picture a window's app
//! list names by its content (`/v1/images/<name>`), anyone's who has that name. A file put from elsewhere is read back
//! only by whoever put it or may use a scope that names it, up to 512 MiB each and 2 GiB per person that nothing keeps
//! yet, deleted an hour on (or once its request is answered) unless something does.

use crate::config::YOURS;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::database::scopes::Role;
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

/// Serves MCP at `/mcp`, for this machine's own programs and the network's client devices over iroh: the remote MCP server,
/// remote MCP clients' device, among them (cloudflare/mcp/).
pub fn serve_mcp(handler: axum::routing::MethodRouter<Node>) {
    let _ = MCP.set(handler);
}

/// The file in a node's home holding the token its machine's own programs send, in `LOCAL_TOKEN_HEADER`, to say they
/// are: only the machine's user can read it. The CLI, its MCP, services and the Mac app read it there.
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
pub struct FromClient(pub crate::keys::Key, pub u64);

/// The client devices whose MCP requests this node's own MCP is answering (`mcp_http`), each by a ticket made for that
/// request alone: a random secret only this process sees, so nothing a request says names a device, and the keys
/// handed for one are reached only as its connection proved it.
static ASKING_AS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (crate::keys::Key, u64)>>> = std::sync::LazyLock::new(Default::default);

/// What this node's own MCP asks its API with while it answers a client device's request: its ticket, until dropped.
pub struct AskingAs(String);

impl AskingAs {
    /// For the client device `key`, whose handed keys count until `until` (`FromClient`'s).
    pub fn new(key: crate::keys::Key, until: u64) -> AskingAs {
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
fn asking_as(ticket: &str) -> Option<(crate::keys::Key, u64)> {
    ASKING_AS.lock().unwrap().get(ticket).copied()
}

/// Serves the local API on `listener`, knowing each request's peer, which `from_this_machine` checks.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    serve_until(listener, router, std::future::pending()).await
}

/// `serve`, until `stop`: then it takes no more connections, and returns once those it has are answered, or after
/// `handoff::drain_within`. As a node hands over, it stops taking connections at once, leaving them to the next, and
/// its answers say to close theirs for five seconds before it closes those left, so clients that keep one don't send on
/// a closed one.
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

/// Who may ask a route. This machine's own programs may ask any; the rest only what its route says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Anyone who has its name: a picture named by its content.
    Anyone,
    /// Also someone signed in at the network's address, and the network's client devices over iroh, each as their
    /// person, with what that person may do.
    SignedIn,
    /// Also the network's client devices over iroh, as their person, but no sign-in from elsewhere: MCP.
    Clients,
    /// Only this machine's own programs.
    ThisMachine,
}

/// The local API's routes, each with who may ask it (`Who`): what `guarded` lets in.
#[derive(Default)]
pub struct Routes {
    router: Router<Node>,
    who: Vec<(&'static str, Who)>,
}

impl Routes {
    pub fn route(mut self, path: &'static str, who: Who, handler: axum::routing::MethodRouter<Node>) -> Routes {
        self.router = self.router.route(path, handler);
        self.who.push((path, who));
        self
    }

    pub fn merge(mut self, other: Routes) -> Routes {
        self.router = self.router.merge(other.router);
        self.who.extend(other.who);
        self
    }

    /// Every route's path, with who may ask it.
    pub fn who(&self) -> &[(&'static str, Who)] {
        &self.who
    }
}

/// Every route of the local API, unguarded: `guarded` lets in who each says.
pub fn routes() -> Routes {
    use Who::*;
    let mcp = MCP.get().cloned().unwrap_or_else(|| any(|| async { StatusCode::NOT_FOUND }));
    Routes::default()
        .route("/mcp", Clients, mcp)
        .route("/llms.txt", ThisMachine, get(llms))
        .route("/skills/{name}/{*path}", ThisMachine, get(skill_file))
        .route("/v1/state", SignedIn, get(state))
        .route("/v1/keys", SignedIn, get(keys_get).post(keys_post))
        .route("/v1/call", SignedIn, post(call).layer(axum::extract::DefaultBodyLimit::max(64 << 20)))
        .route("/v1/create", ThisMachine, post(create))
        .route("/v1/join", ThisMachine, post(join))
        .route("/v1/role", SignedIn, post(role))
        .route("/v1/settings", SignedIn, post(settings))
        .route("/v1/steps", SignedIn, post(steps))
        .route("/v1/steps/global", SignedIn, get(global_steps))
        .route("/hooks/prompt", ThisMachine, post(prompt))
        // An app's own requests, run as its tool `_http` for its person, whose checks are the app's: git's, to Repos.
        .route(APP_ROUTE, SignedIn, any(app_http))
        .merge(crate::node::vault::routes::routes())
        .merge(crate::node::backups::routes::routes())
        .merge(crate::node::storage::routes::routes())
        .merge(crate::node::devices::routes::routes())
        .merge(crate::node::groups::routes::routes())
        .merge(crate::node::widgets::routes::routes())
        .merge(crate::node::apps::routes::routes())
        .merge(crate::node::services::routes::routes())
        .merge(crate::node::releases::routes::routes())
        .merge(crate::node::blobs::routes::routes())
        .merge(crate::node::screens::routes::routes())
}

/// An app's own requests (`app_http`): how stock git reaches Repos.
pub const APP_ROUTE: &str = "/{app}/{*path}";

/// Whether `path` is one of an app's own requests (`APP_ROUTE`), not the API's.
pub fn app_route(path: &str) -> bool {
    let (app, rest) = path.strip_prefix('/').and_then(|p| p.split_once('/')).unwrap_or_default();
    !app.is_empty() && !rest.is_empty() && !matches!(app, "v1" | "mcp" | "hooks" | "skills")
}

/// The most this machine's own programs put in one file.
pub(crate) const LOCAL_PUT: u64 = 1 << 30;

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

/// Whether a client device may ask `path` (`Who::SignedIn` or `Who::Clients`), as the node's routes match it.
pub fn clients_may(path: &str) -> bool {
    let asked: Vec<&str> = path.split('/').collect();
    app_route(path)
        || routes().who().iter().filter(|(r, w)| *r != APP_ROUTE && matches!(w, Who::SignedIn | Who::Clients)).any(|(route, _)| {
            let route: Vec<&str> = route.split('/').collect();
            route.len() == asked.len() && route.iter().zip(&asked).all(|(r, a)| r == a || (r.starts_with('{') && !a.is_empty()))
        })
}

/// A request that asks for a trace (`x-one-trace`, a client in developer mode) gets what it did here in the same
/// header of its answer, as base64 of its JSON (`node::trace`): an older node answers without it.
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

/// Headers that say a request was passed on from elsewhere, as proxies add them.
pub const FORWARDED: &[&str] = &["forwarded", "x-forwarded-for", "x-forwarded-host", "x-real-ip", "cf-connecting-ip", "cf-worker"];


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
/// included, and no program in a VM or emulator here. A request with an access token from sign-in, or from a client
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
    // This machine's own MCP, answering a client device's request (`mcp_http`), asks as that device, by the ticket it
    // made for that request.
    if let Some((key, until)) = token.as_deref().and_then(asking_as) {
        if !from_this_machine(&node, &req) {
            return refuse(req, (StatusCode::UNAUTHORIZED, Json(json!({ "error": "that isn't a sign-in" }))).into_response()).await;
        }
        return from_client(&node, key, until, who, req, next).await;
    }
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
    let token = crate::node::client_token(&key);
    if let Some(handed) = req.headers().get(KEYS_HEADER).and_then(|h| h.to_str().ok())
        && let Err(err) = node.hand(&token, &user, handed, until)
    {
        return refuse(req, (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{err:#}") }))).into_response()).await;
    }
    // A client older than what this build reads of who's who is told to update before it does anything.
    if crate::database::reads_of(req.headers().get(crate::database::HEADER).and_then(|h| h.to_str().ok())) < crate::database::READS {
        return refuse(req, (StatusCode::UPGRADE_REQUIRED, Json(json!({ "error": crate::protocol::update_refusal(YOURS), "code": crate::protocol::code::UPDATE }))).into_response()).await;
    }
    req.extensions_mut().insert(Me { caller: Caller { key, user, agent: None, token: Some(token) }, sign_in: Some(name) });
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
    // A proxy on this machine still says it passed on someone else's request.
    let forwarded = FORWARDED.iter().any(|h| req.headers().contains_key(*h));
    let page = header("origin").is_some() || header("sec-fetch-site").is_some_and(|s| s != "none");
    peer && token && !forwarded && !page
}

/// Nothing this node answers runs as script at its address, where a page would ask as this machine's own programs: a
/// browser shows it sandboxed with no script, and never sniffs it into another type.
async fn sandboxed(mut resp: Response) -> Response {
    let headers = resp.headers_mut();
    headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, axum::http::HeaderValue::from_static("sandbox"));
    headers.insert(axum::http::header::X_CONTENT_TYPE_OPTIONS, axum::http::HeaderValue::from_static("nosniff"));
    resp
}

pub(crate) struct Error(pub(crate) anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if let Some(e) = self.0.downcast_ref::<crate::node::Elsewhere>() {
            return (StatusCode::MISDIRECTED_REQUEST, Json(json!({ "error": e.why, "elsewhere": e.nodes }))).into_response();
        }
        let status = match self.0.downcast_ref::<crate::node::Needs>() {
            Some(_) => StatusCode::PRECONDITION_REQUIRED,
            None => StatusCode::BAD_REQUEST,
        };
        (status, Json(failed(&self.0))).into_response()
    }
}

/// Why something failed, as the API says it: `{ error }`, with `{ code }` for the kinds callers act on (`protocol::code`),
/// and `{ need, to, seal }` for keys a call wasn't handed.
fn failed(err: &anyhow::Error) -> Value {
    match err.downcast_ref::<crate::node::Needs>() {
        Some(n) => json!({ "error": n.to_string(), "need": n.need, "to": n.to, "seal": n.seal }),
        None => match crate::protocol::Coded::of(err) {
            Some(code) => json!({ "error": format!("{err:#}"), "code": code }),
            None => json!({ "error": format!("{err:#}") }),
        },
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.into())
    }
}

pub(crate) type Reply = Result<Json<Value>, Error>;

pub(crate) fn done() -> Reply {
    Ok(Json(json!({})))
}

pub(crate) fn local(node: &Node) -> Result<Caller, Error> {
    Ok(Caller { key: node.id(), user: node.user()?, agent: None, token: None })
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
            None => Ok(Me { caller: local(node)?, sign_in: None }),
        }
    }
}

/// Someone signed in who isn't a person of the network yet: they are once one of their devices, or the admin who
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

/// Whom an access token from sign-in is for: one sign-in at the network's address signed for it, for a person of
/// the network.
async fn signed_in(node: &Node, token: String) -> anyhow::Result<Me> {
    use sha2::Digest;
    let now = crate::node::now_ms();
    let hash: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    let kept = SIGNED_INS.lock().unwrap().get(&hash).filter(|(_, until)| now < *until).map(|(me, _)| me.clone());
    if let Some(me) = kept {
        // Someone removed since is signed in no more, whatever their token says.
        if node.is_user(&me.caller.user) {
            return Ok(me);
        }
        SIGNED_INS.lock().unwrap().remove(&hash);
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
    let me = Me { caller: Caller { key: node.id(), user, agent: None, token: Some(token) }, sign_in: Some(String::new()) };
    let mut kept = SIGNED_INS.lock().unwrap();
    kept.retain(|_, (_, until)| now < *until);
    kept.insert(hash, (me.clone(), until));
    Ok(me)
}

/// Refuses a body with anything but `keys`, so a word a route doesn't take (one renamed, say) fails rather than being
/// passed over: a call or service meant for one device running on another.
pub(crate) fn only(body: &Value, keys: &[&str]) -> Result<(), Error> {
    match body.as_object().and_then(|o| o.keys().find(|k| !keys.contains(&k.as_str()))) {
        Some(other) => Err(anyhow::anyhow!("{other} isn't something this takes: {}", keys.join(", ")).into()),
        None => Ok(()),
    }
}

pub(crate) fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str, Error> {
    body[key].as_str().filter(|s| !s.trim().is_empty()).map(str::trim).ok_or_else(|| anyhow::anyhow!("give {key}").into())
}



/// `?scope=&to=<endpoint key>` or `?scope=&app=`: a scope's keys, sealed (`Node::sealed_keys`).
async fn keys_get(State(node): State<Node>, me: Me, Query(q): Query<std::collections::HashMap<String, String>>) -> Reply {
    let scope = crate::database::scopes::Scope::parse(q.get("scope").map(String::as_str).unwrap_or(""))?;
    Ok(Json(node.sealed_keys(&me.caller, &scope, q.get("to").map(String::as_str), q.get("app").map(String::as_str)).await?))
}

/// `{ scope, n, to, sealed }`, `{ scope, n, app, sealed }` or `{ scope, n, app, to, sealed }`: keeps a key sealed by the
/// caller's device (`Node::seal_key`).
async fn keys_post(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    only(&body, &["scope", "n", "to", "app", "sealed"])?;
    let scope = crate::database::scopes::Scope::parse(text(&body, "scope")?)?;
    let n = body["n"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| anyhow::anyhow!("give n, the epoch"))?;
    Ok(Json(node.seal_key(&me.caller, &scope, n, body["to"].as_str(), body["app"].as_str(), text(&body, "sealed")?).await?))
}

async fn state(State(node): State<Node>, signed_in: Option<axum::Extension<Me>>) -> Json<Value> {
    Json(match signed_in {
        Some(me) => node.state_for_sign_in(&me.caller, me.sign_in.as_deref().unwrap_or_default()),
        None => node.state(),
    })
}










/// What a client that picks the node it asks itself says with each request (client/src/lib.rs): a call this node can't
/// run is answered 421 with the nodes that can (`node::Elsewhere`), not passed on.
const ELSEWHERE_HEADER: &str = "x-one-elsewhere";

async fn call(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap, Json(body): Json<Value>) -> Reply {
    Ok(Json(call_for(&node, me, body, headers.contains_key(ELSEWHERE_HEADER)).await?))
}

/// A call as `/v1/call` takes it, for `me`: → `{ ok }`. A client that `picks` its node has it run here, or told where.
async fn call_for(node: &Node, me: Me, body: Value, picks: bool) -> anyhow::Result<Value> {
    only(&body, &["id", "app", "service", "op", "input", "scopes", "device", "agent", "depth", "from", "client"]).map_err(|e| e.0)?;
    let mut req: CallRequest = serde_json::from_value(body)?;
    // Only the node says an app made a call, and how deep it is.
    (req.from, req.depth) = (None, 0);
    if me.sign_in.is_some() {
        req = node.for_sign_in(req)?;
    }
    if picks {
        return Ok(json!({ "ok": node.call_picked(req, me.caller).await? }));
    }
    Ok(json!({ "ok": node.call_as(req, me.caller).await? }))
}

/// A call the Cloudflare node passes on over iroh (src/node/net.rs): its person's, as `me.user` says, run with the keys
/// of its databases they handed the Cloudflare node, which it hands this device (`keys`, sealed to it for the
/// Cloudflare node's key, `node::passed_token`) for the scopes the call names alone, and never this device's own: no
/// access token comes with it, so none of theirs reaches a node on a machine, whose owner could use it as them.
/// → `{ ok }`, or what failed as the API says it.
pub(crate) async fn call_from_cloudflare_node(node: &Node, remote: crate::keys::Key, mut call: Value) -> Value {
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
        // As the Cloudflare node's, never this device's own: no service counts it as its person's own node.
        call_for(node, Me { caller: Caller { key: remote, user, agent: None, token: Some(token) }, sign_in: Some(client) }, call, false).await
    };
    ran.await.unwrap_or_else(|err| failed(&err))
}




async fn create(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let recovery = node.create_with(text(&body, "name")?, text(&body, "user_name")?, body["user"].as_str(), crate::config::STARTERS).await?;
    Ok(Json(json!({ "recovery": recovery })))
}


async fn join(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.join(text(&body, "code")?).await?;
    done()
}




async fn role(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body["role"].clone())?;
    node.member_as(&me.caller, "role", json!({ "user": node.find_user(text(&body, "user")?)?, "role": role })).await?;
    done()
}

/// `{ name, value }`: one of the network's settings, for admins; `value: null` puts back the custom build's default.
async fn settings(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.member_as(&me.caller, "setting", json!({ "name": text(&body, "name")?, "value": body["value"] })).await?;
    done()
}


/// `[{ header, signature, body }]` (each base64url): steps of `global`'s scope database the caller's own device
/// signed, each for the next position, which this device checks count and writes (`database::steps`).
async fn steps(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let list = body.as_array().ok_or_else(|| anyhow::anyhow!("give a list of steps"))?;
    let b = |v: &Value, k: &str| data_encoding::BASE64URL_NOPAD.decode(v[k].as_str().unwrap_or("").as_bytes()).map_err(|_| anyhow::anyhow!("{k} isn't base64url"));
    let list = list.iter().map(|v| crate::database::steps::Step::decode(&b(v, "header")?, &b(v, "signature")?, &b(v, "body")?)).collect::<anyhow::Result<Vec<_>>>()?;
    node.put_signed_steps(&me.caller, list).await?;
    done()
}

/// `?from=<n>`: `global`'s scope database's steps after position n (`Node::global_steps`).
async fn global_steps(State(node): State<Node>, me: Me, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Reply {
    let from = q.get("from").map(|f| f.parse()).transpose()?.unwrap_or(0);
    let welcome = q.get("welcome").and_then(|w| crate::database::steps::Welcome::parse(w));
    Ok(Json(node.global_steps(&me.caller, from, welcome).await?))
}


















/// For agent harnesses' prompt hooks: the body is the hook's JSON, the reply is plain text to show the agent.
async fn prompt(State(node): State<Node>, body: String) -> Response {
    let event: Value = serde_json::from_str(&body).unwrap_or_default();
    match node.prompt(event).await {
        Ok(text) => text.into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}




/// A request to `/<app>/<path>`, answered by the app's `_http` tool, run as whoever asked (this machine's person,
/// a client device's, or a sign-in's), whose checks are the app's own: the body reaches it as a blob, read as it comes
/// and deleted once it's answered unless something keeps it, and it answers `{ status, type, text }`, or
/// `{ status, type, blob }` for bytes it keeps.
async fn app_http(State(node): State<Node>, me: Me, Path((app, path)): Path<(String, String)>, req: axum::extract::Request) -> Result<Response, Error> {
    let method = req.method().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let remote = me.sign_in.is_some();
    let empty = matches!(method.as_str(), "GET" | "HEAD") || req.headers().get(axum::http::header::CONTENT_LENGTH).is_some_and(|l| l == "0");
    let body = match (empty, remote) {
        (true, _) => None,
        (false, true) => Some(node.put_blob_from(&me.caller, req.into_body()).await?),
        (false, false) => Some(node.put_blob_streamed(req.into_body(), LOCAL_PUT).await?),
    };
    let body = body.and_then(|(hash, size)| (size > 0).then_some(hash));
    let input = json!({ "method": method, "path": path, "query": query, "body": body });
    let mut call = CallRequest { app, tool: "_http".into(), input, puts: body.iter().cloned().collect(), ..Default::default() };
    if remote {
        call = node.for_sign_in(call)?;
    }
    let out = node.call_as(call, me.caller.clone()).await;
    if let Some(hash) = &body {
        node.drop_put(hash);
    }
    let out = out?;
    let status = StatusCode::from_u16(out["status"].as_u64().unwrap_or(200) as u16)?;
    let kind = out["type"].as_str().unwrap_or("text/plain").to_string();
    let bytes = match out["blob"].as_str() {
        Some(hash) if remote => node.blob_for(&me.caller, hash).await?,
        Some(hash) => node.blob(hash).await?,
        None => out["text"].as_str().unwrap_or("").as_bytes().to_vec(),
    };
    Ok((status, [(axum::http::header::CONTENT_TYPE, kind)], bytes).into_response())
}










/// An answer named by its content (`tag`, its ETag): to a client that has it already, only that.
pub(crate) fn tagged(headers: &axum::http::HeaderMap, tag: String, answer: impl IntoResponse) -> Response {
    if headers.get(axum::http::header::IF_NONE_MATCH).is_some_and(|had| had.as_bytes() == tag.as_bytes()) {
        return (StatusCode::NOT_MODIFIED, [(axum::http::header::ETAG, tag)]).into_response();
    }
    ([(axum::http::header::ETAG, tag)], answer).into_response()
}



#[cfg(test)]
mod tests {
    /// Who may ask each route, all of them: a route that lets in more than it should gives a sign-in, or a client
    /// device, this machine's rights, so any change to this list is one to look at in review.
    #[test]
    fn who_may_ask_each_route() {
        use super::Who::*;
        let want = [
            ("/mcp", Clients),
            ("/llms.txt", ThisMachine),
            ("/skills/{name}/{*path}", ThisMachine),
            ("/v1/state", SignedIn),
            ("/v1/keys", SignedIn),
            ("/v1/apps", SignedIn),
            ("/v1/labs", SignedIn),
            ("/v1/images/{name}", Anyone),
            ("/v1/call", SignedIn),
            ("/v1/widgets", SignedIn),
            ("/v1/badges", SignedIn),
            ("/v1/create", ThisMachine),
            ("/v1/pair", SignedIn),
            ("/v1/join", ThisMachine),
            ("/v1/name", SignedIn),
            ("/v1/remove", SignedIn),
            ("/v1/erase", SignedIn),
            ("/v1/role", SignedIn),
            ("/v1/settings", SignedIn),
            ("/v1/steps", SignedIn),
            ("/v1/steps/global", SignedIn),
            ("/v1/runs", SignedIn),
            ("/v1/group", SignedIn),
            ("/v1/invite", SignedIn),
            ("/v1/invite-link", SignedIn),
            ("/v1/devices/requests", SignedIn),
            ("/v1/devices/approve", SignedIn),
            ("/v1/devices/refuse", SignedIn),
            ("/v1/devices/forget-sign-in", SignedIn),
            ("/v1/app-schedules", ThisMachine),
            ("/v1/accept", SignedIn),
            ("/v1/handle", SignedIn),
            ("/v1/member", SignedIn),
            ("/v1/deploy", SignedIn),
            ("/v1/install", SignedIn),
            ("/v1/schedule", ThisMachine),
            ("/v1/arrange", SignedIn),
            ("/v1/retire", SignedIn),
            ("/v1/service", SignedIn),
            ("/v1/fork", ThisMachine),
            ("/v1/source/{app}", ThisMachine),
            ("/v1/storage", ThisMachine),
            ("/v1/backups", SignedIn),
            ("/v1/backups/now", SignedIn),
            ("/v1/backups/place", SignedIn),
            ("/v1/backups/request", SignedIn),
            ("/v1/backups/check", SignedIn),
            ("/v1/vault", SignedIn),
            ("/v1/vault/requests", SignedIn),
            ("/v1/vault/request", SignedIn),
            ("/v1/vault/answer", SignedIn),
            ("/v1/vault/fetch", SignedIn),
            ("/v1/vault/grant", ThisMachine),
            ("/v1/node-release", ThisMachine),
            ("/v1/service-release", ThisMachine),
            ("/v1/blobs", SignedIn),
            ("/v1/blobs/{hash}", SignedIn),
            ("/v1/events", SignedIn),
            ("/v1/screens/{app}/{screen}", SignedIn),
            ("/v1/client-screens/{name}", SignedIn),
            ("/hooks/prompt", ThisMachine),
            ("/v1/services/{app}", ThisMachine),
            ("/v1/services/{app}/calls", ThisMachine),
            ("/v1/services/{app}/calls/{id}", ThisMachine),
            ("/v1/services/{app}/calls/{id}/vault", ThisMachine),
            (super::APP_ROUTE, SignedIn),
        ];
        let mut got = super::routes().who().to_vec();
        let mut want = want.to_vec();
        got.sort_by_key(|(p, _)| *p);
        want.sort_by_key(|(p, _)| *p);
        assert_eq!(got, want);
    }
}
