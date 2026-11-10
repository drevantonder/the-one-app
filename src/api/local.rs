//! How this machine's own programs prove they are: the token in the node's home, which they send and the node checks.

use super::addr;

/// The file in a node's home holding the token its machine's own programs send, in `protocol::header::LOCAL_TOKEN`, to say they
/// are: only the machine's user can read it. The CLI, its MCP, services and the Mac app read it there.
pub const LOCAL_TOKEN: &str = "local-token";

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
        req.headers_mut().insert(crate::protocol::header::LOCAL_TOKEN, value);
    }
    next.handle(req)
}

/// The token of the node with home `home`, once it has started there.
pub fn local_token(home: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(home.join(LOCAL_TOKEN)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Headers that say a request was passed on from elsewhere, as proxies add them.
const FORWARDED: &[&str] = &["forwarded", "x-forwarded-for", "x-forwarded-host", "x-real-ip", "cf-connecting-ip", "cf-worker"];

/// What a program that didn't show the token is told by the node, with `code::NOT_THIS_MACHINE`: the node can't know which
/// home the program read its token from, so the program says that (`token_refused`) where it can.
pub fn not_this_machine() -> String {
    let name = crate::config::NAME;
    format!("{name} on this machine didn't take this request as from one of its own programs: it needs the token in the node's home, which only this machine's user can read.")
}

/// `not_this_machine` as the program refused tells it, which knows the `home` it read its token from: whether that was
/// the node's, and what sets it. One started before the node updated (an agent's MCP server, most often) is this
/// machine's, and starting it again mends it.
pub fn token_refused(home: &std::path::Path) -> String {
    let (name, var) = (crate::config::NAME, crate::config::env("HOME"));
    let how = if std::env::var_os(&var).is_some() { format!("{var} sets it") } else { format!("{var} isn't set, so it is the default") };
    let found = if local_token(home).is_some() { "the token in" } else { "no token in" };
    let file = home.join(LOCAL_TOKEN);
    format!("{name} on this machine didn't take this program as one of its own: it read {found} {} ({how}). If that isn't the folder where {name} keeps its data on this machine, set {var} to it. If the node was updated after this program started, start it again: an agent reconnects its {name} MCP server (/mcp in Claude Code) or starts a new session.", file.display())
}

/// Whether a request is from one of this machine's own programs: over a connection from this machine (its loopback),
/// with the token in the node's home (`LOCAL_TOKEN`), passed on by nothing, and from no page. Its address alone says
/// nothing: a program in a VM or emulator here reaches 127.0.0.1 through QEMU (10.0.2.2) or a forwarded port, and
/// names whatever host it likes, but can't read the token, which only this machine's user can. A browser says when a
/// page asked, its own address's too (`Origin`, `Sec-Fetch-Site`), and a screen this node served is an app's code,
/// which only a shell or a sandboxed frame answers.
pub fn from_this_machine(local_token: &str, req: &axum::extract::Request) -> bool {
    let header = |name| req.headers().get(name).and_then(|h| h.to_str().ok());
    let peer = req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().is_some_and(|c| c.0.ip().is_loopback());
    // Compared as hashes, so how long it takes says nothing of the token.
    let digest = |t: &str| <sha2::Sha256 as sha2::Digest>::digest(t.as_bytes());
    let token = header(crate::protocol::header::LOCAL_TOKEN).is_some_and(|t| digest(t.trim()) == digest(local_token));
    // A proxy on this machine still says it passed on someone else's request.
    let forwarded = FORWARDED.iter().any(|h| req.headers().contains_key(*h));
    let page = header("origin").is_some() || header("sec-fetch-site").is_some_and(|s| s != "none");
    peer && token && !forwarded && !page
}
