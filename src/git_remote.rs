//! Git and your One App network's apps that speak it (Repos), at one address on every computer, `<address>/<app>/…`. Git is told
//! (`url.<CLI>::<address>/.insteadOf`) to reach it as `<CLI>::<address>/<app>/…`, so it runs this program as
//! `git-remote-<CLI>`, its remote helper. That runs git's own HTTP transport (`git remote-http`) for the address, with
//! this program as its proxy while it runs: what git asks goes where every command's request goes, to this machine's
//! node, or from a computer that's only a device, over iroh to its user's nodes (`client_device::instead`). Git's own
//! transport speaks git; the proxy only carries it, and answers only the git it started (a secret only that git has).

use std::io::Read;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::body::{Body, Bytes};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use one::config::{CLI, NAME, YOURS};
use one::git_wire::helper;

/// Run as git's remote helper, `git-remote-<CLI> <remote> <url>`: → the exit code of git's HTTP transport.
pub fn run(args: &[String]) -> Result<i32> {
    let [remote, url] = args else { bail!("{} is git's, for {CLI}:: addresses: git clone <your repo's address>", helper()) };
    let bad = || format!("{url} isn't an address in {NAME}: <address>/<app>/…");
    let (scheme, rest) = url.split_once("://").with_context(bad)?;
    let (host, path) = rest.split_once('/').with_context(bad)?;
    let (address, path) = (format!("{scheme}://{host}"), format!("/{path}"));
    anyhow::ensure!(one::api::app_route(&path), bad());
    let ours = here(&address)?;
    if ours != address {
        bail!("this computer is in {YOURS} at {ours}, not at {address}: {CLI} sign-in --address {address}, then try again");
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let secret = one::keys::hex(&rand::random::<[u8; 32]>());
    let auth = format!("Basic {}", data_encoding::BASE64.encode(format!("git:{secret}").as_bytes()));
    std::thread::spawn(move || proxy(listener, auth));
    // Git's proxy for this one address alone, in its environment (no other user reads it), not its command line.
    let config: usize = std::env::var("GIT_CONFIG_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let plain = format!("http://{host}{path}");
    let status = std::process::Command::new("git")
        .args(["remote-http", remote, &plain])
        .env("GIT_CONFIG_COUNT", (config + 2).to_string())
        .env(format!("GIT_CONFIG_KEY_{config}"), "http.proxy")
        .env(format!("GIT_CONFIG_VALUE_{config}"), format!("http://git:{secret}@127.0.0.1:{port}"))
        .env(format!("GIT_CONFIG_KEY_{}", config + 1), "http.proxyAuthMethod")
        .env(format!("GIT_CONFIG_VALUE_{}", config + 1), "basic")
        .env_remove("no_proxy")
        .env_remove("NO_PROXY")
        .status()
        .context("couldn't run git's HTTP transport")?;
    Ok(status.code().unwrap_or(128))
}

/// The address of the install this computer reaches: its node's, or the one it's a device of; `asked` is the one git
/// was given.
fn here(asked: &str) -> Result<String> {
    if crate::client_device::node_here() {
        let state = crate::node().state()?;
        let address = state.address().with_context(|| format!("this computer's {NAME} has no address yet: an admin sets it with {CLI} cloudflare use"))?;
        return Ok(address.trim_end_matches('/').to_string());
    }
    if !crate::client_device::active() {
        bail!("this computer isn't one of your devices yet: {CLI} sign-in --address {asked}, then try again");
    }
    Ok(crate::sign_in::kept().map(|k| k.address.trim_end_matches('/').to_string()).unwrap_or_default())
}

/// Carries what git asks of an app's address on this machine's node, or its user's nodes, until this program exits.
fn proxy(listener: std::net::TcpListener, auth: String) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
    runtime.block_on(async move {
        listener.set_nonblocking(true).expect("a listener");
        let listener = tokio::net::TcpListener::from_std(listener).expect("a listener");
        let auth = Arc::new(auth);
        let app = axum::Router::new().fallback(move |req: axum::extract::Request| forward(auth.clone(), req));
        let _ = axum::serve(listener, app).await;
    });
}

async fn forward(auth: Arc<String>, req: axum::extract::Request) -> Response {
    if req.headers().get(header::PROXY_AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some(auth.as_str()) {
        return (StatusCode::PROXY_AUTHENTICATION_REQUIRED, [(header::PROXY_AUTHENTICATE, "Basic realm=\"git\"")]).into_response();
    }
    let path = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    if !one::api::app_route(&path) || !matches!(*req.method(), Method::GET | Method::POST) {
        return (StatusCode::NOT_FOUND, "not an app's").into_response();
    }
    let post = req.method() == Method::POST;
    let headers: Vec<(String, String)> = ["content-type", "accept", "content-encoding", "git-protocol"]
        .iter()
        .filter_map(|n| Some((n.to_string(), req.headers().get(*n)?.to_str().ok()?.to_string())))
        .collect();
    // Its body as it comes, read by the request below as it sends it.
    let (sending, sent) = tokio::sync::mpsc::channel::<Bytes>(8);
    let mut body = req.into_body().into_data_stream();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        while let Some(Ok(part)) = body.next().await {
            if sending.send(part).await.is_err() {
                break;
            }
        }
    });
    let (head, answered) = tokio::sync::oneshot::channel::<(u16, String, Option<String>)>();
    let (giving, given) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(8);
    std::thread::spawn(move || {
        let url = format!("http://{}{path}", one::api::addr());
        let asked = if post {
            let mut r = crate::to_node(crate::agent().post(&url));
            for (n, v) in &headers {
                r = r.header(n, v);
            }
            r.send(ureq::SendBody::from_owned_reader(Fed { sent, part: Bytes::new() }))
        } else {
            let mut r = crate::to_node(crate::agent().get(&url));
            for (n, v) in &headers {
                r = r.header(n, v);
            }
            r.call()
        };
        let mut resp = match asked {
            Ok(resp) => resp,
            Err(err) => return drop(head.send((502, "text/plain".into(), Some(format!("{err}"))))),
        };
        let status = resp.status().as_u16();
        let kind = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
        // What the node says is wrong, as git shows it: each line of a plain answer, after "remote:".
        if status >= 400 && kind.starts_with("application/json") {
            let said: serde_json::Value = resp.body_mut().read_json().unwrap_or_default();
            let error = said["error"].as_str().unwrap_or("that didn't work").to_string();
            return drop(head.send((status, "text/plain".into(), Some(one::tools::in_cli_words(&crate::node(), &error)))));
        }
        if head.send((status, kind, None)).is_err() {
            return;
        }
        let mut reader = resp.into_body().into_with_config().limit(u64::MAX).reader();
        let mut part = vec![0; 64 << 10];
        loop {
            match reader.read(&mut part) {
                Ok(0) => break,
                Ok(n) => {
                    if giving.blocking_send(Ok(Bytes::copy_from_slice(&part[..n]))).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = giving.blocking_send(Err(err));
                    break;
                }
            }
        }
    });
    let Ok((status, kind, said)) = answered.await else {
        return (StatusCode::BAD_GATEWAY, "the request stopped").into_response();
    };
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = match said {
        Some(said) => Body::from(format!("{said}\n")),
        None => Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(given)),
    };
    (status, [(header::CONTENT_TYPE, kind)], body).into_response()
}

struct Fed {
    sent: tokio::sync::mpsc::Receiver<Bytes>,
    part: Bytes,
}

impl Read for Fed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.part.is_empty() {
            match self.sent.blocking_recv() {
                Some(part) => self.part = part,
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.part.len());
        buf[..n].copy_from_slice(&self.part.split_to(n));
        Ok(n)
    }
}
