//! Push: sends notifications to phones through Apple's push service (APNs), for every user its device serves, so an app
//! or an agent can reach someone who isn't looking. Phones register their tokens with it; a push goes to the caller's
//! own phones, says which app sent it, and opens that app's screen when tapped. A hosted sidecar (`sidecars::Hosted`) on
//! one device: the white label's APNs key is a Vault item on Everyone switched on for push, which the node fills in for
//! each call (`load`), and its folder keeps devices.json, the phones registered. It says whether APNs accepted a push,
//! never whether anyone saw it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use http_body_util::BodyExt;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::sidecars::{Call, Host, input};

pub const NAME: &str = "push";

/// The host the key is filled in for: the item's sites are Apple's push hosts, and the token made with it goes only
/// there.
const APNS: &str = "api.push.apple.com";

pub fn describe() -> Value {
    json!({
        "title": "Push", "description": "Sends notifications to your phone", "icon": "bell-ringing", "serves": "everyone",
        "instructions": "Push sends a notification to your own phones through Apple's push service: push_send with a title and a body. An app pushes with ctx.sidecars.push.send, and tapping its push opens its screen. It says whether APNs accepted it, never whether anyone saw it.",
        "ops": [
            { "name": "send", "description": "Push a notification to your phones: { title, body, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many APNs accepted.",
              "input": { "type": "object", "required": ["title"], "properties": {
                "title": { "type": "string" },
                "body": { "type": "string" },
                "app": { "type": "string", "description": "the app it opens, when no app sends it" },
                "screen": { "type": "string", "description": "the screen it opens" },
                "params": { "type": "object", "description": "that screen's params" },
                "badge": { "type": "integer" },
                "devices": { "type": "array", "items": { "type": "object" }, "description": "only these devices, [{ token, platform, env }], not your registered phones" },
                "link": { "type": "object", "description": "{ app, screen, params }, as app, screen and params" },
              }}},
            { "name": "register", "description": "A phone registers for push: its APNs device token (hex), platform ios, env sandbox or production, and a name.",
              "input": { "type": "object", "required": ["token"], "properties": {
                "token": { "type": "string" }, "platform": { "type": "string" }, "env": { "type": "string" }, "name": { "type": "string" },
              }}},
            { "name": "unregister", "description": "A phone stops getting pushes.",
              "input": { "type": "object", "required": ["token"], "properties": { "token": { "type": "string" } }}},
            { "name": "devices", "reads": true, "description": "Your phones registered for push here.", "input": { "type": "object", "properties": {} }},
            { "name": "status", "reads": true, "description": "Whether push is set up here: the APNs team, key id and app it sends for, never the key.", "input": { "type": "object", "properties": {} }},
        ],
    })
}

/// A phone registered for push.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Device {
    token: String,
    platform: String,
    env: String,
    /// Whose phone it is, and what it's called, once registered here.
    #[serde(skip_serializing_if = "String::is_empty")]
    user: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(skip_serializing_if = "is_zero")]
    time: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Send {
    devices: Vec<Device>,
    title: String,
    body: String,
    app: String,
    screen: String,
    params: Option<Map<String, Value>>,
    link: Option<Map<String, Value>>,
    badge: Option<i64>,
}

/// What the Vault's Apple push item says: the Apple developer team, the APNs key's id, and the app's bundle id that
/// pushes go to (its topic).
struct Config {
    team_id: String,
    key_id: String,
    topic: String,
}

pub fn call(host: &Host, call: &Call) -> Result<Value> {
    let me = &call.caller;
    let or = |s: String, default: &str| if s.is_empty() { default.to_string() } else { s };
    match call.op.as_str() {
        "register" => {
            let mut d: Device = input(&call.input)?;
            d.token = d.token.to_lowercase();
            if !(16..=200).contains(&d.token.len()) || !d.token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                bail!("a device token is hex");
            }
            (d.user, d.time) = (me.user.clone(), crate::node::now_ms());
            (d.platform, d.env, d.name) = (or(d.platform, "ios"), or(d.env, "production"), or(d.name, &me.user_name));
            change(host, |all| {
                all.insert(d.token.clone(), d);
            })?;
            Ok(json!("Registered for push."))
        }
        "unregister" => {
            let token = input::<Device>(&call.input)?.token.to_lowercase();
            change(host, |all| {
                if all.get(&token).is_some_and(|d| d.user == me.user) {
                    all.remove(&token);
                }
            })?;
            Ok(json!("Unregistered."))
        }
        "devices" => Ok(mine(host, &me.user)
            .iter()
            .map(|d| json!({ "token": format!("{}…", d.token.get(..8).unwrap_or(&d.token)), "platform": d.platform, "env": d.env, "name": d.name, "time": d.time }))
            .collect()),
        "status" => Ok(match load(host, call.id) {
            Ok((c, _)) => json!({ "ready": true, "team_id": c.team_id, "key_id": c.key_id, "topic": c.topic }),
            Err(err) => json!({ "ready": false, "why": format!("{err:#}") }),
        }),
        "send" => {
            let mut push: Send = input(&call.input)?;
            if push.title.is_empty() {
                bail!("give it a title");
            }
            // Given devices are the old way, from before phones registered here.
            let registered = push.devices.is_empty();
            if registered {
                push.devices = mine(host, &me.user);
            }
            if push.devices.is_empty() {
                return Ok(json!({ "accepted": 0, "devices": 0, "why": "no phone of yours is registered for push" }));
            }
            if !on(host) {
                return Ok(json!({ "accepted": 0, "devices": push.devices.len(), "why": "push is off: an admin turns it on in Settings, under General" }));
            }
            let out = send(host, call.id, &push, link(call.from.as_deref().unwrap_or(""), &push))?;
            let gone: Vec<String> = serde_json::from_value(out["gone"].clone())?;
            if registered && !gone.is_empty() {
                change(host, |all| all.retain(|token, _| !gone.contains(token)))?;
            }
            Ok(out)
        }
        op => bail!("push has no operation {op}"),
    }
}

/// Whether the install's push setting names this sidecar; an admin turns push off in Settings by emptying it.
fn on(host: &Host) -> bool {
    match host.get("/v1/state") {
        Ok(state) => state["install"]["settings"]["push"].as_str().is_none_or(|push| push == NAME),
        Err(_) => true,
    }
}

/// What a tap opens: the screen of the app that sent the push (as the node says), or else the one the push names.
fn link(from: &str, push: &Send) -> Value {
    let first = |a: &str, b: &str| if a.is_empty() { b.to_string() } else { a.to_string() };
    let (mut app, mut screen, mut params) = (first(from, &push.app), push.screen.clone(), push.params.clone());
    if let Some(link) = &push.link {
        let text = |name: &str| link.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        (app, screen) = (first(&app, &text("app")), first(&screen, &text("screen")));
        params = params.or_else(|| link.get("params").and_then(Value::as_object).cloned());
    }
    if app.is_empty() {
        return Value::Null;
    }
    json!({ "app": app, "screen": first(&screen, "main"), "params": params.unwrap_or_default() })
}

/// devices.json, which only one call changes at a time.
static DEVICES: Mutex<()> = Mutex::new(());

fn devices(host: &Host) -> BTreeMap<String, Device> {
    std::fs::read(host.dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// A user's registered phones.
fn mine(host: &Host, user: &str) -> Vec<Device> {
    let _held = DEVICES.lock().unwrap();
    devices(host).into_values().filter(|d| d.user == user).collect()
}

/// Changes the registered phones and keeps them in devices.json.
fn change(host: &Host, f: impl FnOnce(&mut BTreeMap<String, Device>)) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let _held = DEVICES.lock().unwrap();
    let mut all = devices(host);
    f(&mut all);
    let tmp = host.dir.join("devices.json.new");
    let _ = std::fs::remove_file(&tmp);
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?.write_all(&serde_json::to_vec_pretty(&all)?)?;
    std::fs::rename(&tmp, host.dir.join("devices.json"))?;
    Ok(())
}

/// The config and the APNs key from the Vault: the fields of an item switched on for push (on Everyone, or the caller's
/// own), which the node fills in for this call, the key only for Apple's push hosts.
fn load(host: &Host, call: u64) -> Result<(Config, EcdsaKeyPair)> {
    let field = |name: &str| -> Result<String> {
        let placeholder = format!("{{{{{name}}}}}");
        let text = host.fill(call, &placeholder, APNS)?;
        match text.is_empty() || text == placeholder {
            true => Err(anyhow!("push isn't set up: an admin adds an item to the Vault on Everyone with team_id, key_id and topic, and key, the APNs key's text, secret and sent to api.push.apple.com and api.sandbox.push.apple.com, and switches push on for it")),
            false => Ok(text),
        }
    };
    let config = Config { team_id: field("team_id")?, key_id: field("key_id")?, topic: field("topic")? };
    let der = pem(&field("key")?).ok_or_else(|| anyhow!("the Vault's APNs key isn't a PEM key"))?;
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &ring::rand::SystemRandom::new()).map_err(|err| match err.to_string().as_str() {
        "WrongAlgorithm" => anyhow!("the Vault's APNs key isn't an ES256 key"),
        why => anyhow!("the Vault's APNs key: {why}"),
    })?;
    Ok((config, key))
}

/// The bytes of a PEM block's text, however its lines were broken when it was pasted.
fn pem(text: &str) -> Option<Vec<u8>> {
    let begin = text.find("-----BEGIN ")?;
    let rest = &text[begin + 11..];
    let body = &rest[rest.find("-----")? + 5..];
    let body = &body[..body.find("-----END ")?];
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    data_encoding::BASE64.decode(b64.as_bytes()).ok()
}

/// The provider token APNs takes for a team's key, made again every 50 minutes (it refuses one older than an hour, and
/// one made more often than every 20).
static TOKEN: Mutex<Option<(String, String, Instant)>> = Mutex::new(None);

fn jwt(c: &Config, key: &EcdsaKeyPair) -> Result<String> {
    let mut token = TOKEN.lock().unwrap();
    let whose = format!("{}/{}", c.team_id, c.key_id);
    if let Some((of, made, at)) = token.as_ref()
        && *of == whose
        && at.elapsed() < Duration::from_secs(50 * 60)
    {
        return Ok(made.clone());
    }
    let enc = data_encoding::BASE64URL_NOPAD;
    let header = json!({ "alg": "ES256", "kid": c.key_id }).to_string();
    let claims = json!({ "iss": c.team_id, "iat": crate::node::now_ms() / 1000 }).to_string();
    let signing = format!("{}.{}", enc.encode(header.as_bytes()), enc.encode(claims.as_bytes()));
    let signature = key.sign(&ring::rand::SystemRandom::new(), signing.as_bytes()).map_err(|_| anyhow!("couldn't sign the APNs token"))?;
    let made = format!("{signing}.{}", enc.encode(signature.as_ref()));
    *token = Some((whose, made.clone(), Instant::now()));
    Ok(made)
}

/// Pushes to each iPhone or iPad, and says how many APNs accepted and which tokens it said no longer work (the app was
/// deleted, say).
fn send(host: &Host, call: u64, push: &Send, link: Value) -> Result<Value> {
    let (c, key) = load(host, call)?;
    let token = jwt(&c, &key)?;
    let mut aps = json!({ "alert": { "title": push.title, "body": push.body }, "sound": "default" });
    if let Some(badge) = push.badge {
        aps["badge"] = json!(badge);
    }
    let payload = json!({ "aps": aps, "link": link }).to_string();
    let (mut accepted, mut gone, mut why) = (0, Vec::new(), None);
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let mut connections: BTreeMap<&str, hyper::client::conn::http2::SendRequest<String>> = BTreeMap::new();
        for d in push.devices.iter().filter(|d| d.platform.is_empty() || d.platform == "ios") {
            let apns = match d.env.as_str() {
                "sandbox" | "development" => "api.sandbox.push.apple.com",
                _ => "api.push.apple.com",
            };
            let asked = tokio::time::timeout(Duration::from_secs(15), async {
                let mut connection = match connections.remove(apns) {
                    Some(connection) => connection,
                    None => connect(apns).await?,
                };
                let req = hyper::Request::post(format!("https://{apns}/3/device/{}", d.token))
                    .header("authorization", format!("bearer {token}"))
                    .header("apns-topic", &c.topic)
                    .header("apns-push-type", "alert")
                    .header("apns-priority", "10")
                    .body(payload.clone())?;
                let resp = connection.send_request(req).await?;
                connections.insert(apns, connection);
                let status = resp.status().as_u16();
                let body = resp.into_body().collect().await?.to_bytes();
                anyhow::Ok((status, serde_json::from_slice::<Value>(&body).ok().and_then(|b| b["reason"].as_str().map(str::to_string)).unwrap_or_default()))
            });
            match asked.await.unwrap_or_else(|_| Err(anyhow!("APNs didn't answer"))) {
                Ok((200, _)) => accepted += 1,
                Ok((status, reason)) if status == 410 || reason == "BadDeviceToken" || reason == "Unregistered" => gone.push(d.token.clone()),
                Ok((status, reason)) => why = Some(format!("APNs said {status} {reason}")),
                Err(err) => why = Some(format!("{err:#}")),
            }
        }
    });
    let mut out = json!({ "accepted": accepted, "devices": push.devices.len(), "gone": gone });
    if let Some(why) = why {
        out["why"] = json!(why);
    }
    Ok(out)
}

/// An HTTP/2 connection to `host`, which is all APNs speaks.
async fn connect(host: &str) -> Result<hyper::client::conn::http2::SendRequest<String>> {
    use tokio_rustls::rustls;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions()?.with_root_certificates(roots).with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec()];
    let tcp = tokio::net::TcpStream::connect((host, 443)).await?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(tls)).connect(rustls::pki_types::ServerName::try_from(host.to_string())?, tcp).await?;
    let (send, connection) = hyper::client::conn::http2::handshake(hyper_util::rt::TokioExecutor::new(), hyper_util::rt::TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    Ok(send)
}
