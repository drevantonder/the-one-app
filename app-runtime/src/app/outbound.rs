//! Workerd's global outbound: every request an app makes comes here, on a port of the node's own, with the token of
//! the call it was made in (`x-one-call`). The node makes the request itself, filling in that call's secrets for
//! their hosts (`src/vault.rs`). Requests to `one.internal` are the shim's asks of the node for the call: other
//! apps' tools and services' functions, blobs, and the network's apps.


use anyhow::{Context as _, Result};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::Node;

use platform::reach::LIMIT;

pub fn router(node: Node) -> axum::Router {
    axum::Router::new().fallback(handle).with_state(node)
}

async fn handle(State(node): State<Node>, req: Request) -> Response {
    match out(node, req).await {
        Ok(resp) => resp,
        // A kind of error callers act on (`protocol::code`) goes on in a header, which an app's ctx.apps and ctx.services
        // calls throw with.
        Err(err) => {
            let mut reply = (StatusCode::BAD_GATEWAY, format!("{err:#}")).into_response();
            if let Some(code) = custom_build::protocol::Coded::of(&err).and_then(|c| c.parse().ok()) {
                reply.headers_mut().insert(custom_build::protocol::header::CODE, code);
            }
            reply
        }
    }
}

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
        return node.internal(&header("x-one-call").unwrap_or_default(), parts.uri.path(), body.to_vec()).await;
    }
    let scheme = header("x-forwarded-proto").filter(|p| p == "https").unwrap_or_else(|| "http".into());
    if vault_runtime::names_app(&node.rt(), &host) {
        return Ok((StatusCode::FORBIDDEN, format!("{host} is an app's name, not a site's: an app reaches its own service with ctx.services, and other apps with ctx.apps")).into_response());
    }
    let target = written(parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"));
    let body = axum::body::to_bytes(body, LIMIT).await?.to_vec();
    // The call's items, opened only if the request has a placeholder, from the scopes it named, with their keys.
    let placed = target.contains("{{") || parts.headers.values().any(|v| v.as_bytes().windows(2).any(|w| w == b"{{")) || body.windows(2).any(|w| w == b"{{");
    let lookup = match &grant {
        Some(g) if placed => Some(vault_runtime::lookup(&node.rt(), &g.caller, &g.app, &g.scopes).await?),
        _ => None,
    };
    let fill = |text: &str| -> Result<String> {
        match &lookup {
            Some(l) => {
                l.refuse_shut(text)?;
                ::vault::fill(text, &host, |name| l.get(name))
            }
            None => Ok(text.to_string()),
        }
    };
    // Only the path and query are filled: the host is the one the request named, which the secret's hosts are checked against.
    let url = format!("{scheme}://{authority}{}", fill(&target)?.replace('{', "%7B").replace('}', "%7D").replace(' ', "%20"));
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
    let ports = node.own_ports();
    let (status, headers, bytes) = tokio::task::spawn_blocking(move || platform::reach::send(&method, &url, headers, body, ports)).await??;
    let mut out = Response::builder().status(status);
    for (name, value) in headers {
        out = out.header(name, value);
    }
    Ok(out.body(Body::from(bytes))?)
}

/// A request's path and query as the app wrote them: workerd gives a path's `{{` and `}}` as %7B%7B and %7D%7D, and a space
/// as %20, which a placeholder (`{{rs256 key claims}}`) can hold.
fn written(target: &str) -> String {
    let target = target.replace("%7B%7B", "{{").replace("%7D%7D", "}}");
    let mut out = String::with_capacity(target.len());
    let mut rest = target.as_str();
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        out.push_str(&rest[start..start + end].replace("%20", " "));
        rest = &rest[start + end..];
    }
    out.push_str(rest);
    out
}

