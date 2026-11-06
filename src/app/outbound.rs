//! Workerd's global outbound: every request an app makes comes here, on a port of the node's own, with the token of
//! the call it was made in (`x-one-call`). The node makes the request itself, filling in that call's secrets for
//! their hosts (`src/vault.rs`). Requests to `one.internal` are the shim's asks of the node for the call: other
//! apps' tools and services' functions, blobs, and the network's apps.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::node::Node;

const LIMIT: usize = 64 << 20;

pub fn router(node: Node) -> axum::Router {
    axum::Router::new().fallback(handle).with_state(node)
}

async fn handle(State(node): State<Node>, req: Request) -> Response {
    match out(node, req).await {
        Ok(resp) => resp,
        Err(err) => (StatusCode::BAD_GATEWAY, format!("{err:#}")).into_response(),
    }
}

/// Makes an app's request, with its call's Vault fields filled in.
async fn out(node: Node, req: Request) -> Result<Response> {
    let (parts, body) = req.into_parts();
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let grant = header("x-one-call").and_then(|t| node.grant_of(&t));
    let authority = header("host").context("the request names no host")?;
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or("").to_string(),
        None => authority.split(':').next().unwrap_or("").to_lowercase(),
    };
    if host == "one.internal" {
        let body = axum::body::to_bytes(body, LIMIT).await?;
        return node.internal(&header("x-one-call").unwrap_or_default(), parts.method.as_str(), parts.uri.path(), body.to_vec()).await;
    }
    let scheme = header("x-forwarded-proto").filter(|p| p == "https").unwrap_or_else(|| "http".into());
    let port = authority.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap_or(if scheme == "https" { 443 } else { 80 });
    if node.names_app(&host) {
        return Ok((StatusCode::FORBIDDEN, format!("{host} is an app's name, not a site's: an app reaches its own service with ctx.services, and other apps with ctx.apps")).into_response());
    }
    if own_port(&node, &host, port) {
        return Ok((StatusCode::FORBIDDEN, "apps can't reach the node's own ports").into_response());
    }
    let url = format!("{scheme}://{authority}{}", parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"));
    let body = axum::body::to_bytes(body, LIMIT).await?.to_vec();
    // The call's items, opened only if the request has a placeholder, from the scopes it named, with their keys.
    let placed = url.contains("{{") || parts.headers.values().any(|v| v.as_bytes().windows(2).any(|w| w == b"{{")) || body.windows(2).any(|w| w == b"{{");
    let lookup = match &grant {
        Some(g) if placed => Some(node.lookup(&g.caller, &g.app, &g.scopes).await?),
        _ => None,
    };
    let fill = |text: &str| -> Result<String> {
        match &lookup {
            Some(l) => {
                l.refuse_shut(text)?;
                crate::vault::fill(text, &host, |name| l.get(name))
            }
            None => Ok(text.to_string()),
        }
    };
    let mut headers = Vec::new();
    for (name, value) in &parts.headers {
        if matches!(name.as_str(), "host" | "x-one-call" | "x-forwarded-proto" | "content-length" | "connection" | "transfer-encoding") {
            continue;
        }
        let value = match value.to_str() {
            Ok(text) => fill(text)?.into_bytes(),
            Err(_) => value.as_bytes().to_vec(),
        };
        headers.push((name.as_str().to_string(), value));
    }
    let body = match String::from_utf8(body) {
        Ok(text) if text.contains("{{") => fill(&text)?.into_bytes(),
        Ok(text) => text.into_bytes(),
        Err(e) => e.into_bytes(),
    };
    let method = parts.method.as_str().to_string();
    let (status, headers, bytes) = tokio::task::spawn_blocking(move || send(&method, &url, headers, body)).await??;
    let mut out = Response::builder().status(status);
    for (name, value) in headers {
        out = out.header(name, value);
    }
    Ok(out.body(Body::from(bytes))?)
}

/// Whether `host:port` is one of the node's own ports on this machine, which would let a request act as someone else
/// (the local API, workerd) or loop back here.
pub fn own_port(node: &Node, host: &str, port: u16) -> bool {
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified());
    loopback && node.own_ports().contains(&port)
}

/// Makes a request, filled in already: → its status, headers and body.
pub fn send(method: &str, url: &str, headers: Vec<(String, Vec<u8>)>, body: Vec<u8>) -> Result<(u16, Vec<(String, Vec<u8>)>, Vec<u8>)> {
    // Redirects go back to the app, whose next request comes here again: a secret never follows one to another host.
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(30))).build().into();
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let mut resp = match body.is_empty() {
        true => agent.run(builder.body(())?),
        false => agent.run(builder.body(body)?),
    }
    .or_else(|err| match err {
        ureq::Error::Timeout(_) => bail!("{url} took longer than 30s"),
        err => Err(err.into()),
    })?;
    let headers = resp.headers().iter().filter(|(name, _)| !matches!(name.as_str(), "content-length" | "transfer-encoding" | "connection")).map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec())).collect();
    let bytes = resp.body_mut().with_config().limit(LIMIT as u64).read_to_vec()?;
    Ok((resp.status().as_u16(), headers, bytes))
}
