//! The local HTTP API: how this machine's shells, CLI and MCP reach its node, as its local user; and how someone
//! signed in with the install's kernel (a browser, through the gateway, or the Mac app's window here) reaches it, as
//! themselves, for what their sign-in opens.

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

use crate::entry::hex;
use crate::members::Role;
use crate::node::{CallRequest, Caller, Node};

/// Where this machine's node serves its local API: the config's port, or `<CLI>_ADDR`.
pub fn addr() -> String {
    std::env::var(format!("{}_ADDR", crate::config::CLI.to_uppercase())).unwrap_or_else(|_| format!("127.0.0.1:{}", crate::config::PORT))
}

pub fn router(node: Node) -> Router {
    guarded(routes(), node)
}

/// The file in a node's home holding the token its machine's own programs send, in `LOCAL_TOKEN_HEADER`, to say they
/// are: only the machine's user can read it. The CLI, its MCP, sidecars and the Mac app read it there.
pub const LOCAL_TOKEN: &str = "local-token";
pub const LOCAL_TOKEN_HEADER: &str = "x-one-local-token";

/// Serves the local API on `listener`, knowing each request's peer, which `from_this_machine` checks.
pub async fn serve(listener: tokio::net::TcpListener, router: Router) -> std::io::Result<()> {
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await
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
    Router::new()
        .route("/llms.txt", get(llms))
        .route("/skills/{name}/{*path}", get(skill_file))
        .route("/v1/state", get(state))
        .route("/v1/apps", get(apps))
        .route("/v1/call", post(call).layer(axum::extract::DefaultBodyLimit::max(64 << 20)))
        .route("/v1/widgets", get(widgets))
        .route("/v1/badges", get(badges))
        .route("/v1/icons", get(icons))
        .route("/v1/create", post(create))
        .route("/v1/pair", post(pair))
        .route("/v1/join", post(join))
        .route("/v1/name", post(name))
        .route("/v1/remove", post(remove))
        .route("/v1/role", post(role))
        .route("/v1/relays", post(relays))
        .route("/v1/settings", post(settings))
        .route("/v1/acl", post(acl))
        .route("/v1/trust", post(trust))
        .route("/v1/use", post(use_))
        .route("/v1/group", post(group))
        .route("/v1/invite", post(invite))
        .route("/v1/invite-link", post(invite_link))
        .route("/v1/accept", post(accept))
        .route("/v1/member", post(member))
        .route("/v1/app", post(app))
        .route("/v1/deploy", post(deploy))
        .route("/v1/install", post(install))
        .route("/v1/schedule", post(schedule))
        .route("/v1/arrange", post(arrange))
        .route("/v1/retire", post(retire))
        .route("/v1/sidecar", post(sidecar))
        // What /v1/sidecar was called first.
        .route("/v1/driver", post(sidecar))
        .route("/v1/fork", post(fork))
        .route("/v1/source/{app}", get(source))
        .route("/v1/kernel/keys", post(kernel_keys))
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
    "/v1/apps",
    "/v1/call",
    "/v1/widgets",
    "/v1/badges",
    "/v1/icons",
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
    "/mcp",
    // What Settings' screens ask: each checked as its person.
    "/v1/pair",
    "/v1/remove",
    "/v1/role",
    "/v1/settings",
    "/v1/trust",
    "/v1/use",
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

/// The root skill's body, each skill under it linked at this server, as whoever asked reached it.
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

/// Phosphor's icons, which apps name for their tiles, screens and widgets (vendor/phosphor): each as its path on a
/// 256-point grid, drawn filled. They change only with a release.
async fn icons() -> Response {
    let headers = [(axum::http::header::CONTENT_TYPE, "application/json"), (axum::http::header::CACHE_CONTROL, "max-age=86400")];
    (headers, include_str!("../vendor/phosphor/icons.json")).into_response()
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
    let token = req.headers().get(axum::http::header::AUTHORIZATION).and_then(|a| a.to_str().ok()).and_then(|a| a.strip_prefix("Bearer ")).map(|t| t.trim().to_string());
    if let Some(token) = token {
        // This machine's own programs (its CLI and MCP, signed in) may ask anything; from elsewhere, only `SIGNED_IN`.
        if !SIGNED_IN.contains(&path.as_str()) && !from_this_machine(&node, &req) {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": "only this machine's own programs may use this" }))).into_response();
        }
        // A run the schedules service hands in brings a token for that alone.
        if token.starts_with("run.") != (path == "/v1/kernel/run") {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": "only a scheduled run asks that, and it asks nothing else" }))).into_response();
        }
        return match signed_in(&node, token).await {
            Ok(me) => {
                req.extensions_mut().insert(me);
                next.run(req).await
            }
            Err(err) if path == "/mcp" => sign_in_first(&node, Some(format!("{err:#}"))),
            Err(err) => (StatusCode::UNAUTHORIZED, Json(json!({ "error": format!("{err:#}") }))).into_response(),
        };
    }
    if !from_this_machine(&node, &req) {
        if path == "/mcp" {
            return sign_in_first(&node, None);
        }
        return (StatusCode::FORBIDDEN, Json(json!({ "error": not_this_machine() }))).into_response();
    }
    next.run(req).await
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

/// An MCP client elsewhere is told where to sign in, as MCP's authorization spec says: the install's
/// protected-resource metadata, at its address, which names the kernel's auth service.
fn sign_in_first(node: &Node, error: Option<String>) -> Response {
    let Ok((address, _)) = node.kernel() else {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "this install has no address to sign in at yet, so only this machine's own programs may use this" }))).into_response();
    };
    let invalid = if error.is_some() { r#"error="invalid_token", "# } else { "" };
    let challenge = format!(r#"Bearer {invalid}resource_metadata="{}/.well-known/oauth-protected-resource/mcp", scope="openid profile offline_access""#, address.trim_end_matches('/'));
    let error = error.unwrap_or_else(|| format!("sign in at {address} first"));
    (StatusCode::UNAUTHORIZED, [(axum::http::header::WWW_AUTHENTICATE, challenge)], Json(json!({ "error": error }))).into_response()
}

struct Error(anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("{:#}", self.0) }))).into_response()
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

/// Who asked: this machine's own user, or someone signed in with the kernel, whom this server acts for.
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

/// Sign-ins checked lately, by their token's hash, until they expire.
static SIGNED_INS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<[u8; 32], (Me, u64)>>> = std::sync::LazyLock::new(Default::default);

/// Whom a kernel access token is for: one the auth service at the install's address signed for it, whose session at
/// the vault is open, for a person of the install. A scheduled run's token (`run.…`) is no access token: only the
/// vault says whose it is.
async fn signed_in(node: &Node, token: String) -> anyhow::Result<Me> {
    use sha2::Digest;
    let now = crate::node::now_ms();
    let hash: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    if let Some((me, _)) = SIGNED_INS.lock().unwrap().get(&hash).filter(|(_, until)| now < *until) {
        return Ok(me.clone());
    }
    let (address, vault) = node.kernel()?;
    let issuer = node.issuer()?;
    let asked = token.clone();
    let (user, until, session, name) = tokio::task::spawn_blocking(move || {
        let vault = crate::kernel::Vault { url: vault };
        if asked.starts_with("run.") {
            let session = vault.session(&asked)?;
            anyhow::ensure!(session.client.as_deref() == Some("schedules"), "that token isn't a scheduled run's");
            return Ok((session.user.clone(), session.until, session, String::new()));
        }
        let claims = issuer.verify(&asked, now)?;
        // MCP clients sign in for the address's MCP endpoint, as MCP names a server.
        let address = address.trim_end_matches('/');
        anyhow::ensure!(claims.for_resource(address) || claims.for_resource(&format!("{address}/mcp")), "that sign-in isn't for {address}");
        let session = vault.session(&asked)?;
        anyhow::ensure!(session.user == claims.sub, "that sign-in's session is someone else's");
        Ok((claims.sub, (claims.exp * 1000).min(session.until), session, claims.name))
    })
    .await??;
    if !node.is_user(&user) {
        node.let_in(&user, &name).await?;
    }
    let me = Me { caller: Caller { key: node.id(), user, agent: None, token: Some(token) }, sign_in: Some(session.name.clone().unwrap_or_default()) };
    let mut kept = SIGNED_INS.lock().unwrap();
    kept.retain(|_, (_, until)| now < *until);
    kept.insert(hash, (me.clone(), until));
    Ok(me)
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

/// The keys of the scopes this server holds, for this machine's admin to hand to the install's kernel.
async fn kernel_keys(State(node): State<Node>) -> Reply {
    Ok(Json(json!(node.keys_for_kernel(&local(&node)?).await?)))
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

/// `{ name, server?, installed? }`: removes a sidecar from a server (this one by default), or with `installed` lets it
/// register there again.
async fn sidecar(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.set_sidecar(&local(&node)?, body["server"].as_str(), text(&body, "name")?, body["installed"] == true).await?;
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

async fn schedule(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let me = local(&node)?;
    let scope = node.scope_for(&me, body["scope"].as_str().unwrap_or("user"))?;
    node.switch_schedule(&me, text(&body, "app")?, text(&body, "schedule")?, &scope, body["on"] != false).await?;
    done()
}

async fn call(State(node): State<Node>, me: Me, Json(mut req): Json<CallRequest>) -> Reply {
    // Only the node says an app made a call, and how deep it is.
    (req.from, req.depth) = (None, 0);
    if me.sign_in.is_some() {
        req = node.for_sign_in(req)?;
    }
    Ok(Json(json!({ "ok": node.call_as(req, me.caller).await? })))
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
    node.rename_server(text(&body, "name")?).await?;
    done()
}

/// `{ key }` (a server or client, by name or key) or `{ user }`.
async fn remove(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    match body["user"].as_str() {
        Some(user) => node.member_as(&me.caller, "remove", json!({ "user": node.find_user(user)? })).await?,
        None => node.member_as(&me.caller, "remove", json!({ "key": hex(&node.find_machine(text(&body, "key")?)?) })).await?,
    }
    done()
}

async fn role(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body["role"].clone())?;
    node.member_as(&me.caller, "role", json!({ "user": node.find_user(text(&body, "user")?)?, "role": role })).await?;
    done()
}

async fn relays(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    node.member("relays", json!({ "urls": body["urls"] })).await?;
    done()
}

/// `{ name, value }`: one of the install's settings, for admins; `value: null` puts back the white label's default.
async fn settings(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.member_as(&me.caller, "setting", json!({ "name": text(&body, "name")?, "value": body["value"] })).await?;
    done()
}

/// `{ server, owners, users }`: owners and users are `user:<id>`, `group:<id>`, `install`, or a user's or group's name.
async fn acl(State(node): State<Node>, Json(body): Json<Value>) -> Reply {
    let server = node.find_server(text(&body, "server")?)?;
    let principals = |key: &str| -> Result<Vec<String>, Error> {
        body[key]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("give {key}"))?
            .iter()
            .map(|p| {
                let p = p.as_str().unwrap_or("");
                Ok(match crate::members::Principal::parse(p) {
                    Ok(p) => p.to_string(),
                    Err(_) => match node.find_user(p) {
                        Ok(u) => format!("user:{u}"),
                        Err(_) => format!("group:{}", node.find_group(p)?),
                    },
                })
            })
            .collect()
    };
    node.member("acl", json!({ "server": hex(&server), "owners": principals("owners")?, "users": principals("users")? })).await?;
    done()
}

async fn trust(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let server = node.find_server(text(&body, "server")?)?;
    node.member_as(&me.caller, if body["trust"] == false { "untrust" } else { "trust" }, json!({ "server": hex(&server) })).await?;
    done()
}

/// `{ server, use }`: puts a server (its name or key, or `cloudflare`) first, needed or last, for an admin.
async fn use_(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    node.set_use(&me.caller, text(&body, "server")?, text(&body, "use")?).await?;
    done()
}

async fn group(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let group = crate::entry::new_id();
    node.member_as(&me.caller, "group", json!({ "group": group, "name": text(&body, "name")? })).await?;
    Ok(Json(json!({ "group": group })))
}

async fn invite(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let role: Role = serde_json::from_value(body.get("role").cloned().unwrap_or(json!("member")))?;
    let (group, user) = (node.find_group(text(&body, "group")?)?, node.find_user(text(&body, "user")?)?);
    node.member_as(&me.caller, "invite", json!({ "group": group, "user": user, "role": role })).await?;
    done()
}

/// `{ name }`: a link that lets someone new join, for an admin signed in: → `{ link, until }`.
async fn invite_link(State(node): State<Node>, me: Me, Json(body): Json<Value>) -> Reply {
    let (link, until) = node.invite_link(&me.caller, text(&body, "name")?).await?;
    Ok(Json(json!({ "link": link, "until": until })))
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
    let text = node.sidecar_fill(&app, id, text(&body, "text")?, text(&body, "host")?)?;
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

async fn events(State(node): State<Node>) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(node.subscribe())
        .filter_map(|e| e.ok())
        .map(|e| Ok(SseEvent::default().json_data(e).unwrap_or_default()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
