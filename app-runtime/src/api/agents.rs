//! What agents read from the app runtime: its skills at `/llms.txt`, on its Tailscale address too for agents on the user's
//! other devices, and what an agent harness's prompt hook asks.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::Host;

use super::Routes;
use ::api::Who;
use ::api::local::port;

/// The skills for this machine's user (api/mcp/src/skills.rs).
async fn skills(host: &Host) -> Vec<::mcp::skills::Skill> {
    let apps = match host.caller() {
        Ok(me) => host.apps(&me).await.map(|a| json!(a)).unwrap_or_else(|err| {
            tracing::warn!("the skills leave out apps' own: {err:#}");
            json!([])
        }),
        // An app runtime not in a network yet has no user whose apps they'd be.
        Err(_) => json!([]),
    };
    let sets: Vec<Value> = onet::features().iter().map(::api::Feature::listed).chain(apps.as_array().into_iter().flatten().cloned()).collect();
    mcp::skills::all(&sets)
}

fn markdown(text: String) -> Response {
    ([(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")], text).into_response()
}

/// The root skill's body, each skill under it linked at this device, as whoever asked reached it.
pub(super) fn routes() -> Routes {
    Routes::default().route("/llms.txt", Who::ThisMachine, get(llms)).route("/skills/{name}/{*path}", Who::ThisMachine, get(skill_file)).route(::api::path::HOOKS_PROMPT, Who::ThisMachine, post(prompt))
}

async fn llms(State(host): State<Host>, headers: axum::http::HeaderMap) -> Response {
    let authority = headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("127.0.0.1").to_string();
    let deeper = skills(&host).await.split_off(1);
    markdown(::mcp::skills::root(&deeper, |s| format!("[{}](http://{authority}/skills/{}/SKILL.md)", s.name, s.name)).body)
}

async fn skill_file(State(host): State<Host>, Path((name, path)): Path<(String, String)>) -> Response {
    match skills(&host).await.into_iter().find(|s| s.name == name).and_then(|s| s.file(&path)) {
        Some(text) => markdown(text),
        None => (StatusCode::NOT_FOUND, "no such skill or file").into_response(),
    }
}

/// Serves the skills, and nothing else, on this machine's Tailscale address and the local API's port, so an agent on
/// any of the user's devices can read them, starting at /llms.txt. Looks again every minute: Tailscale may come up
/// later, or move.
pub async fn serve_guide(host: Host) {
    let port = port();
    let mut serving: Option<(std::net::Ipv4Addr, tokio::task::JoinHandle<()>)> = None;
    loop {
        let ip = tailnet_address();
        if ip != serving.as_ref().map(|s| s.0) {
            if let Some((_, task)) = serving.take() {
                task.abort();
            }
            host.set_guide(None);
            if let Some(ip) = ip {
                match tokio::net::TcpListener::bind((ip, port)).await {
                    Ok(listener) => {
                        host.set_guide(Some(format!("http://{ip}:{port}/llms.txt")));
                        let app = Router::new().route("/llms.txt", get(llms)).route("/skills/{name}/{*path}", get(skill_file)).with_state(host.clone());
                        serving = Some((ip, tokio::spawn(async move {
                            if let Err(err) = axum::serve(listener, app).await {
                                tracing::warn!("serving the skills: {err}");
                            }
                        })));
                    }
                    Err(e) => tracing::debug!("can't serve the skills on {ip}:{port}: {e}"),
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

/// This machine's Tailscale address, in 100.64.0.0/10, if it has one.
#[cfg(windows)]
fn tailnet_address() -> Option<std::net::Ipv4Addr> {
    None
}

/// This machine's Tailscale address, in 100.64.0.0/10, if it has one.
#[cfg(unix)]
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

/// For agent harnesses' prompt hooks: the body is the hook's JSON, the reply is plain text to show the agent.
async fn prompt(State(host): State<Host>, body: String) -> Response {
    let event: Value = serde_json::from_str(&body).unwrap_or_default();
    match host.prompt(event).await {
        Ok(text) => text.into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}
