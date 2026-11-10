//! The computer's loopback API, as its onet answers it: this computer's own programs only (its token, from its
//! loopback, passed on by nothing, from no page), each request answered here or passed on as this device.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use one::api::Who;
use one::config::{CLI, YOURS};
use one::node::NetworkFile;
use one::protocol::{Coded, code};
use serde_json::{Value, json};

use super::Onet;
use crate::keyring::Address;
use crate::{Body, Sent};

/// The most of a request's body kept in memory: a bigger one (a git push's pack, a file) goes to disk, and streams to
/// where it's passed on from there.
const IN_MEMORY: usize = 4 << 20;

/// The headers a request is passed on without: how it was framed and who it's from are said again where it goes.
const OWN_HEADERS: &[&str] = &["host", "content-length", "transfer-encoding", "connection", one::protocol::header::LOCAL_TOKEN, "authorization"];

/// Each route of the API with who may ask it, to tell what only this computer's own runtime answers.
struct Routes(Vec<(Vec<String>, Who)>);

impl Routes {
    fn new() -> Routes {
        Routes(one::api::routes().who().iter().map(|(path, who)| (path.split('/').map(str::to_string).collect(), *who)).collect())
    }

    /// Who may ask `path`, by the route that takes it, as the API's router picks it, the one with most of its parts
    /// written out: a route this API doesn't have is for whoever answers it to refuse.
    fn who(&self, path: &str) -> Who {
        let parts: Vec<&str> = path.split('/').collect();
        let fits = |route: &[String]| match route.last().is_some_and(|l| l.starts_with("{*")) {
            true => parts.len() >= route.len() && route.iter().zip(&parts).all(|(r, p)| r.starts_with('{') || r == p),
            false => parts.len() == route.len() && route.iter().zip(&parts).all(|(r, p)| r.starts_with('{') || r == p),
        };
        let written = |route: &[String]| route.iter().filter(|r| !r.starts_with('{')).count();
        self.0.iter().filter(|(route, _)| fits(route)).max_by_key(|(route, _)| written(route)).map_or(Who::Clients, |(_, who)| *who)
    }
}

pub(super) async fn serve(onet: Arc<Onet>) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(&onet.listen).await.with_context(|| format!("listen on {}", onet.listen))?;
    let mut router = axum::Router::new().fallback(answer).with_state((onet, Arc::new(Routes::new())));
    if let one::mode::Mode::Throwaway(network) = one::mode::here() {
        router = network.serve_beside(router);
    }
    Ok(one::api::serve(listener, router).await?)
}

fn said(status: StatusCode, err: &anyhow::Error) -> Response {
    let mut out = json!({ "error": format!("{err:#}") });
    if let Some(c) = Coded::of(err) {
        out["code"] = json!(c);
    }
    (status, axum::Json(out)).into_response()
}

fn refused(status: StatusCode, error: String, code: Option<&str>) -> Response {
    let mut out = json!({ "error": error });
    if let Some(c) = code {
        out["code"] = json!(c);
    }
    (status, axum::Json(out)).into_response()
}

async fn answer(State((onet, routes)): State<(Arc<Onet>, Arc<Routes>)>, req: Request) -> Response {
    if !one::api::from_this_machine(&onet.token, &req) {
        return refused(StatusCode::FORBIDDEN, one::api::not_this_machine(), Some(code::NOT_THIS_MACHINE));
    }
    let (method, path) = (req.method().to_string(), req.uri().path().to_string());
    let done = match (method.as_str(), path.as_str()) {
        ("POST", "/api/join") => join(&onet, req).await,
        ("POST", "/api/create") => create(&onet, req).await,
        ("POST", "/api/devices/approve-with-recovery" | "/api/devices/recovery-key/replace" | "/api/devices/recovery-holders/seal") => recovery(&onet, &path, req).await,
        ("GET", "/api/holds") => Ok(held(&onet)),
        ("POST", "/api/holds/shared") => shared(&onet, req).await,
        ("POST", "/api/holds/write") => write(&onet, req).await,
        ("POST", "/api/holds/sign") => sign(&onet, req).await,
        _ => return pass_on(&onet, &routes, req).await,
    };
    match done {
        Ok(out) => axum::Json(out).into_response(),
        Err(err) => said(StatusCode::BAD_REQUEST, &err),
    }
}

/// What a request's JSON body is, as `T`.
async fn body<T: serde::de::DeserializeOwned>(req: Request) -> Result<T> {
    let bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("the body isn't what this takes: {e}"))
}


/// Joins the network a pairing code is from, as this computer (`one::node::join_with`), and reaches it.
async fn join(onet: &Arc<Onet>, req: Request) -> Result<Value> {
    let asked: one::says::api::Join = body(req).await?;
    anyhow::ensure!(NetworkFile::read(&onet.home)?.is_none(), "this computer is in a network already");
    let joined = one::node::join_with(&onet.key, &onet.name, &asked.code).await?;
    joined.network.write(&onet.home)?;
    // Its keys are sealed to it by the device that let it in, in a moment: it reaches the network once they are.
    if let Err(err) = onet.start().await {
        tracing::info!("joined; {err:#}");
    }
    Ok(json!({}))
}

/// A new network, made by this device's own app runtime, which deploys its first apps: then this computer is its first
/// device, and the runtime its.
async fn create(onet: &Arc<Onet>, req: Request) -> Result<Value> {
    anyhow::ensure!(NetworkFile::read(&onet.home)?.is_none(), "this computer is in a network already");
    let runtime = onet.runtime.as_ref().context("a network is made by this computer's own app runtime, which isn't running here")?;
    let asked = axum::body::to_bytes(req.into_body(), 1 << 20).await?.to_vec();
    let (status, _, made) = runtime.ask("POST".into(), "/api/create".into(), vec![("content-type".into(), "application/json".into())], asked).await?;
    let made: Value = serde_json::from_slice(&made)?;
    anyhow::ensure!(status == 200, "{}", made["error"].as_str().unwrap_or("this computer's own app runtime didn't make the network"));
    onet.start().await?;
    Ok(made)
}

/// What this device's own app runtime, an endpoint of its own key, asks of the device's key (`one::node::Holder`): the
/// device's key and seal key.
fn held(onet: &Onet) -> Value {
    let seal = one::keys::public_of(&one::keys::seal_secret(&onet.key));
    json!({ "key": one::keys::hex(&onet.key.verifying_key().to_bytes()), "seal": one::keys::hex(&seal) })
}

/// The secret the device's seal key shares with the key given: what opens something sealed to the device.
async fn shared(onet: &Onet, req: Request) -> Result<Value> {
    use data_encoding::BASE64URL_NOPAD as B64U;
    let asked: one::node::HeldShare = body(req).await?;
    let share = <[u8; 32]>::try_from(B64U.decode(asked.share.as_bytes())?).map_err(|_| anyhow::anyhow!("a share is 32 bytes"))?;
    Ok(json!({ "shared": B64U.encode(&one::keys::agree(&one::keys::seal_secret(&onet.key), &share)) }))
}

/// A step of `global` written as the device, for its user alone, to the bucket itself.
async fn write(onet: &Onet, req: Request) -> Result<Value> {
    let asked: one::node::HeldWrite = body(req).await?;
    let device = onet.device().await?;
    let keyring = device.keyring();
    anyhow::ensure!(keyring.user().as_deref() == Some(asked.user.as_str()), "the device writes in global only as its own user");
    keyring.write_to_bucket(&*device, &*device, &asked.user, asked.content).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(json!({}))
}

/// A request to the relay or auth signed as the device (`one::node::sign_request`).
async fn sign(onet: &Onet, req: Request) -> Result<Value> {
    use data_encoding::BASE64URL_NOPAD as B64U;
    let asked: one::node::HeldSign = body(req).await?;
    let signature = one::node::sign_request(&onet.key, &B64U.decode(asked.bytes.as_bytes())?)?;
    Ok(json!({ "signature": B64U.encode(&signature) }))
}

/// What the network's recovery key does on this computer, signed here as this device (`one::settings::recovery`).
async fn recovery(onet: &Onet, path: &str, req: Request) -> Result<Value> {
    use one::says::devices::{ApproveWithRecovery, ReplaceRecovery, SealRecovery, recovery_key};
    use one::settings::recovery;
    let device = onet.device().await?;
    let (keyring, client) = device.reached().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let me = keyring.mine(&client, &*device);
    match path {
        "/api/devices/approve-with-recovery" => {
            let asked: ApproveWithRecovery = body(req).await?;
            recovery::approve(&me, &asked.code, &recovery_key(&asked.recovery, "a recovery key")?, asked.confirm).await
        }
        "/api/devices/recovery-key/replace" => {
            let asked: ReplaceRecovery = body(req).await?;
            recovery::replace(&me, &recovery_key(&asked.recovery, "a recovery key")?, &recovery_key(&asked.next, "the next recovery key")?, asked.confirm).await
        }
        _ => {
            let asked: SealRecovery = body(req).await?;
            recovery::seal(&me, &asked.to, &asked.copy).await.map(|()| json!({}))
        }
    }
}

/// A request passed on as this device (`Device::fetch`): answered by its keyring if it signs it, else asked of the best
/// machine for it, its own app runtime first, as this computer's own program. A route only this computer's own programs
/// may ask goes nowhere else.
async fn pass_on(onet: &Onet, routes: &Routes, req: Request) -> Response {
    let device = match (onet.device().await, &onet.runtime) {
        (Ok(device), _) if !device.address().is_empty() => device,
        (_, Some(runtime)) => return to_runtime(runtime, req).await,
        (Ok(_), None) => return refused(StatusCode::CONFLICT, format!("{YOURS} has no address yet, which this computer reaches it at: an admin sets one with {CLI} cloudflare use"), None),
        (Err(err), None) => return said(StatusCode::CONFLICT, &err),
    };
    let path = req.uri().path().to_string();
    // What its own app runtime serves itself for this computer's programs, as a node that's its own device did: a
    // route a client answers alone, a node answers with more than that (`one::settings::on_a_node`).
    let method = req.method().to_string();
    if let Some(runtime) = &onet.runtime
        && device.local().is_some()
        && one::settings::answered(&path)
        && !one::settings::on_a_node().any(|(m, p)| m == method && p == path)
    {
        return to_runtime(runtime, req).await;
    }
    if routes.who(&path) == Who::ThisMachine && !device.local().is_some() {
        return refused(StatusCode::FORBIDDEN, format!("only this computer's own app runtime answers {path}, and it isn't running here"), None);
    }
    let asked = req.uri().path_and_query().map_or(path, |p| p.as_str().to_string());
    let headers: Vec<(String, String)> = req.headers().iter().filter(|(n, _)| !OWN_HEADERS.contains(&n.as_str())).filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string()))).collect();
    let spooled = match Spooled::read(req.into_body()).await {
        Ok(spooled) => spooled,
        Err(err) => return said(StatusCode::BAD_REQUEST, &err.into()),
    };
    let fetched = device.fetch(&method, &asked, headers, spooled.sent()).await;
    drop(spooled);
    let response = match fetched {
        Ok(response) => response,
        Err(err) if err.not_let_in => return refused(StatusCode::FORBIDDEN, format!("{err}"), Some(code::NOT_LET_IN)),
        Err(err) => return refused(StatusCode::BAD_GATEWAY, format!("couldn't reach {YOURS}'s devices: {err}"), err.code.as_deref()),
    };
    let mut answer = Response::builder().status(response.status);
    for (name, value) in &response.headers {
        if !["connection", "transfer-encoding", "content-length", "keep-alive"].contains(&name.as_str()) {
            answer = answer.header(name, value);
        }
    }
    answer.body(axum::body::Body::from_stream(streamed(response.body))).unwrap_or_else(|err| said(StatusCode::BAD_GATEWAY, &err.into()))
}

/// A request passed to this device's own app runtime as this computer's own program, while this computer isn't in a
/// network, or its network has no address: it answers what it can with none, makes the network, and sets its address.
async fn to_runtime(runtime: &super::Runtime, req: Request) -> Response {
    let (method, path) = (req.method().to_string(), req.uri().path_and_query().map_or_else(|| req.uri().path().to_string(), |p| p.as_str().to_string()));
    let headers: Vec<(String, String)> = req.headers().iter().filter(|(n, _)| !OWN_HEADERS.contains(&n.as_str())).filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string()))).collect();
    let body = match axum::body::to_bytes(req.into_body(), IN_MEMORY).await {
        Ok(body) => body.to_vec(),
        Err(err) => return said(StatusCode::BAD_REQUEST, &err.into()),
    };
    match runtime.ask(method, path, headers, body).await {
        Ok((status, headers, body)) => {
            let mut answer = Response::builder().status(status);
            for (name, value) in headers.iter().filter(|(n, _)| !["connection", "transfer-encoding", "content-length", "keep-alive"].contains(&n.as_str())) {
                answer = answer.header(name, value);
            }
            answer.body(axum::body::Body::from(body)).unwrap_or_else(|err| said(StatusCode::BAD_GATEWAY, &err.into()))
        }
        Err(err) => said(StatusCode::BAD_GATEWAY, &err.context("this computer's own app runtime didn't answer")),
    }
}

/// A response's body, as it comes.
fn streamed(body: Body) -> impl n0_future::Stream<Item = std::io::Result<Bytes>> {
    n0_future::stream::unfold(Some(body), |body| async move {
        let mut body = body?;
        match body.chunk().await {
            Ok(Some(part)) => Some((Ok(Bytes::from(part)), Some(body))),
            Ok(None) => None,
            Err(err) => Some((Err(std::io::Error::other(err.to_string())), None)),
        }
    })
}

/// A request's body, read: in memory, or past `IN_MEMORY`, in a file only this user reads, deleted once it's sent, as a
/// machine it's sent to may ask for it again with the keys it needs.
enum Spooled {
    Bytes(Vec<u8>),
    File(std::path::PathBuf),
}

impl Spooled {
    async fn read(body: axum::body::Body) -> std::io::Result<Spooled> {
        use n0_future::StreamExt;
        use tokio::io::AsyncWriteExt;
        let mut stream = body.into_data_stream();
        let (mut sent, mut file) = (Vec::new(), None::<(tokio::fs::File, std::path::PathBuf)>);
        while let Some(part) = stream.next().await {
            let part = part.map_err(std::io::Error::other)?;
            match &mut file {
                Some((f, _)) => f.write_all(&part).await?,
                None if sent.len() + part.len() <= IN_MEMORY => sent.extend_from_slice(&part),
                None => {
                    let path = std::env::temp_dir().join(format!("{CLI}-sending-{}", one::keys::hex(&rand::random::<[u8; 32]>())));
                    let mut options = tokio::fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    options.mode(0o600);
                    let mut f = options.open(&path).await?;
                    f.write_all(&sent).await?;
                    f.write_all(&part).await?;
                    sent.clear();
                    file = Some((f, path));
                }
            }
        }
        match file {
            Some((mut f, path)) => {
                f.flush().await?;
                Ok(Spooled::File(path))
            }
            None => Ok(Spooled::Bytes(sent)),
        }
    }

    fn sent(&self) -> Sent {
        match self {
            Spooled::Bytes(bytes) => Sent::Bytes(bytes.clone()),
            Spooled::File(path) => Sent::File(path.clone()),
        }
    }
}

impl Drop for Spooled {
    fn drop(&mut self) {
        if let Spooled::File(path) = self {
            let _ = std::fs::remove_file(path);
        }
    }
}
