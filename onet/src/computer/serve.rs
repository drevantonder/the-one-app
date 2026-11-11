//! The computer's loopback API, as its onet answers it: this computer's own programs only (its token, from its
//! loopback, passed on by nothing, from no page), each request answered here or passed on as this device.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ::custom_build::config::{CLI, YOURS};
use api::home::NetworkFile;
use ::custom_build::protocol::{Coded, code};
use serde_json::{Value, json};

use super::Onet;
use crate::keyring::Address;
use crate::{Body, Device, Sent};

/// The most of a request's body kept in memory: a bigger one (a git push's pack, a file) goes to disk, and streams to
/// where it's passed on from there.
const IN_MEMORY: usize = 4 << 20;

/// The headers a request is passed on without: how it was framed and who it's from are said again where it goes.
const OWN_HEADERS: &[&str] = &["host", "content-length", "transfer-encoding", "connection", ::custom_build::protocol::header::LOCAL_TOKEN, "authorization"];

pub(super) async fn serve(onet: Arc<Onet>) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(&onet.listen).await.with_context(|| format!("listen on {}", onet.listen))?;
    let mut router = axum::Router::new().fallback(answer).with_state(onet);
    if let platform::mode::Mode::Throwaway(network) = platform::mode::here() {
        router = super::throwaway::serve_beside(network, router);
    }
    Ok(api::serve(listener, router).await?)
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

/// The routes a computer's onet answers itself, which `answer` takes before passing anything on: all but those of the
/// window's joining, notifications, kept answers and the bucket's move are its app runtime's routes too, for an app runtime that's its own device.
pub const OWN: &[&str] = &[
    "/api/join",
    "/api/join/ask",
    "/api/join/wait",
    "/api/notifications/register",
    "/api/notifications/open",
    "/api/kept",
    "/api/create",
    "/api/devices/approve-with-recovery",
    "/api/devices/recovery-key/replace",
    "/api/devices/recovery-holders/seal",
    "/api/holds",
    "/api/holds/shared",
    "/api/holds/write",
    "/api/holds/sign",
    "/api/holds/moved",
    "/api/relay",
];

async fn answer(State(onet): State<Arc<Onet>>, req: Request) -> Response {
    if !api::local::from_this_machine(&onet.token, &req) {
        return refused(StatusCode::FORBIDDEN, api::local::not_this_machine(), Some(code::NOT_THIS_MACHINE));
    }
    let (method, path) = (req.method().to_string(), req.uri().path().to_string());
    let done = match (method.as_str(), path.as_str()) {
        ("GET", "/api/join") => stands(&onet).await,
        ("POST", "/api/join") => join(&onet, req).await,
        ("POST", "/api/join/ask") => ask(&onet, req).await,
        ("POST", "/api/join/wait") => wait(&onet, req).await,
        ("POST", "/api/notifications/register") => register_notifications(&onet, req).await,
        ("POST", "/api/notifications/open") => open_note(&onet, req).await,
        ("POST", "/api/kept") => kept(&onet, req).await,
        ("POST", "/api/create") => create(&onet, req).await,
        ("POST", "/api/devices/approve-with-recovery" | "/api/devices/recovery-key/replace" | "/api/devices/recovery-holders/seal") => recovery(&onet, &path, req).await,
        ("GET", "/api/holds") => Ok(held(&onet)),
        ("POST", "/api/holds/shared") => shared(&onet, req).await,
        ("POST", "/api/holds/write") => write(&onet, req).await,
        ("POST", "/api/holds/sign") => sign(&onet, req).await,
        ("POST", "/api/holds/moved") => moved(&onet, req).await,
        ("GET", "/api/relay") => Ok(onet.relay()),
        ("POST", "/api/services") => services(&onet, req).await,
        _ => return pass_on(&onet, req).await,
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

/// Joins the network a pairing code is from, as this computer (`app_runtime::join_with`), and reaches it.
async fn join(onet: &Arc<Onet>, req: Request) -> Result<Value> {
    let asked: ::api::says::network::Join = body(req).await?;
    anyhow::ensure!(NetworkFile::read(&onet.home)?.is_none(), "this computer is in a network already");
    let joined = ::onboarding::join::join_with(&onet.key, &onet.name, &asked.code).await?;
    joined.network.write(&onet.home)?;
    // Its app runtime, started before there was a network, opens it now; one started later opens it as it starts.
    if let Some(runtime) = &onet.runtime {
        match runtime.ask("POST".into(), "/api/joined".into(), Vec::new(), Vec::new()).await {
            Ok((200, ..)) => {}
            Ok((_, _, said)) => tracing::warn!("joined; this computer's own app runtime didn't open the network: {}", String::from_utf8_lossy(&said)),
            Err(err) => tracing::info!("joined; this computer's own app runtime opens the network as it starts: {err:#}"),
        }
    }
    // Its keys are sealed to it by the device that let it in, in a moment: it reaches the network once they are.
    if let Err(err) = onet.start().await {
        tracing::info!("joined; {err:#}");
    }
    Ok(json!({}))
}

/// Where this computer stands with a network, for its desktop window, which is a view of it: in one (`in`, with its
/// address and this device's user), or not; and the device a sign-in here is for (`join`, its key and seal key) and the
/// code another of its user's devices lets it in by.
async fn stands(onet: &Onet) -> Result<Value> {
    let me = key::Endpoint::of(&onet.key);
    let mut out = json!({ "in": false, "join": format!("{}{}", key::hex(&me.key), key::hex(&me.seal)), "code": key::spaced_code(&key::join_code(&me)) });
    if let Some(network) = NetworkFile::read(&onet.home)? {
        let address = onet.device().await.ok().map(|d| d.address().to_string()).filter(|a| !a.is_empty());
        out["in"] = json!(true);
        out["user"] = json!(network.user);
        out["address"] = json!(address);
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    address: String,
    /// An access token of the user signed in on this computer, whose sign-in named this device.
    token: String,
    /// Someone new's invite secret, its proof.
    invite: Option<String>,
}

/// Asks the network at `address` to let this computer in, as the user signed in on it (its desktop window signs them in,
/// naming this device): → the code another of their devices approves it by, and whether an invite said yes already.
async fn ask(onet: &Onet, req: Request) -> Result<Value> {
    let asked: Ask = body(req).await?;
    anyhow::ensure!(NetworkFile::read(&onet.home)?.is_none(), "this computer is in a network already");
    let device = Arc::new(Device::new(onet.key.to_bytes(), &asked.address, &onet.home.join("onet").to_string_lossy()));
    let invited = device.ask(&asked.token, &onet.name, asked.invite.as_deref()).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let code = device.code();
    *onet.asking.lock().await = Some(device);
    Ok(json!({ "code": code, "invited": invited }))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Wait {
    token: String,
}

/// Whether this computer, having asked (`ask`), has been let in: once it has, its home names the network, its bucket
/// where who's who says (the `storage` setting, reached through the URL signer), and it starts as a device of it.
async fn wait(onet: &Arc<Onet>, req: Request) -> Result<Value> {
    let asked: Wait = body(req).await?;
    if NetworkFile::read(&onet.home)?.is_some() {
        return Ok(json!({ "in": true }));
    }
    let device = onet.asking.lock().await.clone().context("this computer hasn't asked to be let in: ask first")?;
    if !device.wait(&asked.token).await.map_err(|e| anyhow::anyhow!("{e}"))? {
        return Ok(json!({ "in": false }));
    }
    let keyring = device.keyring();
    let user = keyring.user().context("let in, but who's who names no user for this device yet")?;
    let root = std::fs::read_to_string(onet.home.join("onet").join("root")).context("let in, but the network's root wasn't kept")?;
    let storage = keyring.who(|db| db.setting(::bucket::STORAGE)).and_then(Result::ok).flatten();
    let bucket = match storage {
        Some(fields) => {
            let mut fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&fields).context("the storage setting isn't an S3 address")?;
            fields.insert("address".into(), device.address().to_string());
            Some(fields)
        }
        None => {
            tracing::warn!("this device can't reach the network's bucket: it keeps no app's data until the network moves to S3-compatible storage");
            None
        }
    };
    NetworkFile { root: root.trim().to_string(), user, bucket }.write(&onet.home)?;
    device.disconnect().await;
    *onet.asking.lock().await = None;
    if let Err(err) = onet.start().await {
        tracing::info!("let in; {err:#}");
    }
    Ok(json!({ "in": true }))
}

/// Registers where a push service reaches this device (`notifications::Address`): Windows' window hands it its channel.
async fn register_notifications(onet: &Onet, req: Request) -> Result<Value> {
    let address: notifications::Address = body(req).await?;
    onet.device().await?.register_notifications(&address).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(json!({}))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Carried {
    /// The note as its push service carried it (base64url).
    n: String,
}

/// A note sent to this device, opened with its key: → the note, and the key of the device that sent it.
async fn open_note(onet: &Onet, req: Request) -> Result<Value> {
    let carried: Carried = body(req).await?;
    let (note, from) = crate::open_note(onet.key.to_bytes(), &carried.n).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(json!({ "note": note, "from": from }))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Kept {
    app: String,
    tool: String,
    input: Value,
}

/// What a call of an app's that only reads answered last as it passed (`Device::kept`), for the window's screens to show
/// at once while they ask again: `{ answer, at }`, or null.
async fn kept(onet: &Onet, req: Request) -> Result<Value> {
    let asked: Kept = body(req).await?;
    let device = onet.device().await?;
    Ok(match device.kept(&asked.app, &asked.tool, asked.input.to_string().as_bytes()).await {
        Some((answer, at)) => json!({ "answer": serde_json::from_slice::<Value>(&answer)?, "at": at }),
        None => Value::Null,
    })
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

/// What this device's oned says its services are doing (`oned::keep_parts`), given to its own runtime by its
/// loopback, which another device asks over what devices speak to each other when a screen shows it, so the relay never
/// reads the services' names or what went wrong.
async fn services(onet: &Arc<Onet>, req: Request) -> Result<Value> {
    let said = axum::body::to_bytes(req.into_body(), 1 << 20).await?.to_vec();
    let runtime = onet.runtime.as_ref().context("this computer's own app runtime isn't running here")?;
    let (status, _, answer) = runtime.ask("POST".into(), "/api/services".into(), vec![("content-type".into(), "application/json".into())], said).await?;
    let answer: Value = serde_json::from_slice(&answer).unwrap_or_default();
    anyhow::ensure!(status == 200, "{}", answer["error"].as_str().unwrap_or("this computer's own app runtime didn't take that"));
    Ok(json!({}))
}

/// What this device's own app runtime, an endpoint of its own key, asks of the device's key (`app_runtime::Holder`): the
/// device's key and seal key.
fn held(onet: &Onet) -> Value {
    let seal = key::public_of(&key::seal_secret(&onet.key));
    json!({ "key": key::hex(&onet.key.verifying_key().to_bytes()), "seal": key::hex(&seal) })
}

/// The secret the device's seal key shares with the key given: what opens something sealed to the device.
async fn shared(onet: &Onet, req: Request) -> Result<Value> {
    use data_encoding::BASE64URL_NOPAD as B64U;
    let asked: api::holds::HeldShare = body(req).await?;
    let share = <[u8; 32]>::try_from(B64U.decode(asked.share.as_bytes())?).map_err(|_| anyhow::anyhow!("a share is 32 bytes"))?;
    Ok(json!({ "shared": B64U.encode(&key::agree(&key::seal_secret(&onet.key), &share)) }))
}

/// A step of `global` written as the device, for its user alone, to the bucket itself.
async fn write(onet: &Onet, req: Request) -> Result<Value> {
    let asked: api::holds::HeldWrite = body(req).await?;
    let device = onet.device().await?;
    let keyring = device.keyring();
    anyhow::ensure!(keyring.user().as_deref() == Some(asked.user.as_str()), "the device writes in global only as its own user");
    keyring.write_to_bucket(&*device, &*device, &asked.user, asked.contents).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(json!({}))
}

/// Its app runtime keeps the network's data elsewhere from now on: the device reads and writes it there too.
async fn moved(onet: &Onet, req: Request) -> Result<Value> {
    let asked: api::holds::HeldMoved = body(req).await?;
    onet.moved(&asked.bucket).await?;
    Ok(json!({}))
}

/// A request to the relay or auth signed as the device (`api::holds::sign_request`).
async fn sign(onet: &Onet, req: Request) -> Result<Value> {
    use data_encoding::BASE64URL_NOPAD as B64U;
    let asked: api::holds::HeldSign = body(req).await?;
    let signature = api::holds::sign_request(&onet.key, &B64U.decode(asked.bytes.as_bytes())?)?;
    Ok(json!({ "signature": B64U.encode(&signature) }))
}

/// What the network's recovery key does on this computer, signed here as this device (`::scope::settings::recovery`).
async fn recovery(onet: &Onet, path: &str, req: Request) -> Result<Value> {
    use ::devices::says::{ApproveWithRecovery, ReplaceRecovery, SealRecovery, recovery_key};
    use ::scope::settings::recovery;
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
/// machine for it, its own app runtime first, as this computer's own program. A route only a computer's own programs may
/// ask is refused by any other machine it reaches.
async fn pass_on(onet: &Arc<Onet>, req: Request) -> Response {
    let device = match (onet.device().await, &onet.runtime) {
        (Ok(device), _) if !device.address().is_empty() => device,
        (_, Some(runtime)) => {
            let answer = to_runtime(runtime, req).await;
            // What it asked may have given the network its address (the setting), from which onet reaches it.
            if let Err(err) = onet.start().await {
                tracing::info!("{err:#}");
            }
            return answer;
        }
        (Ok(_), None) => return refused(StatusCode::CONFLICT, format!("{YOURS} has no address yet, which this computer reaches it at: an admin sets one with {CLI} cloudflare use"), None),
        (Err(err), None) => return said(StatusCode::CONFLICT, &err),
    };
    let path = req.uri().path().to_string();
    // What its own app runtime serves itself for this computer's programs, as an app runtime that's its own device did: a
    // route a client answers alone, an app runtime answers with more than that (`::scope::settings::on_a_node`).
    let method = req.method().to_string();
    if let Some(runtime) = &onet.runtime
        && device.local().is_some()
        && ::scope::settings::answered(&path)
        && !::scope::settings::on_a_node().any(|(m, p)| m == method && p == path)
    {
        return to_runtime(runtime, req).await;
    }
    let asked = req.uri().path_and_query().map_or_else(|| path.clone(), |p| p.as_str().to_string());
    let headers: Vec<(String, String)> = req.headers().iter().filter(|(n, _)| !OWN_HEADERS.contains(&n.as_str())).filter_map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_string()))).collect();
    let mut spooled = match Spooled::read(req.into_body()).await {
        Ok(spooled) => spooled,
        Err(err) => return said(StatusCode::BAD_REQUEST, &err.into()),
    };
    // A call is named here, so whichever device it's asked of (another, once this computer's own app runtime stalls) it
    // runs once: the first to take its lease runs it.
    let mut called = None;
    if let Spooled::Bytes(sent) = &mut spooled
        && path == api::path::CALL
        && let Ok(mut call) = serde_json::from_slice::<Value>(sent)
    {
        called = call["app"].as_str().map(str::to_string);
        if call.is_object() && call.get("id").is_none() {
            call["id"] = json!(::scope::scopes::new_id());
            *sent = serde_json::to_vec(&call).unwrap_or_else(|_| sent.clone());
        }
    }
    let fetched = device.fetch(&method, &asked, headers, spooled.sent()).await;
    drop(spooled);
    let response = match fetched {
        // An app called is its user using it: their app schedules of it are renewed, as a screen's call does elsewhere.
        Ok(response) if response.status == 200 && let Some(app) = called => {
            let device = device.clone();
            tokio::spawn(async move { device.renew_app_schedules(&app).await });
            response
        }
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
                    let path = std::env::temp_dir().join(format!("{CLI}-sending-{}", key::hex(&rand::random::<[u8; 32]>())));
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
