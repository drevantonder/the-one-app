//! The MCP server (`api_features::mcp`) over HTTP at `/mcp`, asking this app runtime in-process as whoever sent the request.

use serde_json::{Value, json};

use ::api::Client;
use api_features::mcp::{Session, answer};
use api_features::skills::Mode;

/// MCP over Streamable HTTP, at `/mcp`: for a client device over iroh, as its user (the remote MCP server,
/// onet/worker/, is remote MCP clients' device), or for a program on this machine, as its user. Each POST is
/// answered with JSON; the server sends nothing unasked, so there's no stream to GET. A tool that needs keys it wasn't
/// handed is answered 428 with what it needs, as `/api/call` is, for its caller to hand them and ask again. Nothing is
/// kept between requests, since the next may reach another node: the session id carries only the client's name,
/// credited on its calls.
pub async fn http(
    axum::extract::State(node): axum::extract::State<crate::Node>,
    client: Option<axum::Extension<crate::api::FromClient>>,
    peer: Option<axum::Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let elsewhere = client.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION);
    let asks = Client::new(crate::api::InProcess::new(node, client.map(|c| c.0), peer.map(|p| p.0), &headers), elsewhere, ::api::Reader::Mcp.client());
    let agent = headers.get("mcp-session-id").and_then(|s| s.to_str().ok()).and_then(|s| s.split_once('.')).and_then(|(_, name)| data_encoding::BASE64URL_NOPAD.decode(name.as_bytes()).ok()).and_then(|n| String::from_utf8(n).ok());
    let Ok(msg) = serde_json::from_str::<Value>(&body) else {
        let error = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "the body isn't JSON" } });
        return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    };
    let initialize = msg["method"] == "initialize";
    let answered = tokio::task::spawn_blocking(move || {
        let mut session = Session { agent, mode: Mode::All, stdio: false };
        let reply = match &msg {
            Value::Array(batch) => Some(json!(batch.iter().filter_map(|m| Some(answer(&asks, m, &mut session)?.0)).collect::<Vec<_>>())).filter(|r| r.as_array().is_some_and(|r| !r.is_empty())),
            msg => answer(&asks, msg, &mut session).map(|(reply, _)| reply),
        };
        (reply, session.agent, asks.needed())
    })
    .await;
    match answered {
        Ok((_, _, Some(needed))) => (StatusCode::PRECONDITION_REQUIRED, axum::Json(needed)).into_response(),
        Ok((None, _, None)) => StatusCode::ACCEPTED.into_response(),
        Ok((Some(reply), agent, None)) => {
            let mut resp = axum::Json(reply).into_response();
            if initialize {
                let session = format!("{:016x}.{}", rand::random::<u64>(), data_encoding::BASE64URL_NOPAD.encode(agent.unwrap_or_default().as_bytes()));
                resp.headers_mut().insert("mcp-session-id", session.parse().expect("base64url and hex are header-safe"));
            }
            resp
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": err.to_string() }))).into_response(),
    }
}
