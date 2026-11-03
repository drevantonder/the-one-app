//! iroh, how devices reach each other: each is an endpoint whose id is its device key, reached directly where a path
//! opens (a LAN, a tailnet, a port its router maps) and otherwise through the relay at the install's address
//! (kernel/relay/), its only relay, which lets in only the install's endpoints. iroh's QUIC is end to end and proves
//! both ends' keys, so what devices say needs no sealing of its own: an ask is one stream, its answer the same stream
//! back. Devices are found by the install's list of them, never n0's discovery.

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher as _};

use super::*;

/// How long reaching a device may take.
const CONNECT: Duration = Duration::from_secs(10);
/// The most an ask or its answer may carry: an email of a claim's receipts, base64, fits.
const MESSAGE_LIMIT: usize = 64 << 20;

/// This device's endpoint, and its connections to other devices.
#[derive(Default)]
pub(super) struct Net {
    /// The endpoint, with the install's address its relay is at.
    endpoint: Mutex<Option<(String, Endpoint)>>,
    /// The task binding the endpoint and taking what others open, with the address it's for.
    task: Mutex<Option<(String, tokio::task::JoinHandle<()>)>>,
    /// A connection to each device, opened by either end; each held while it's being opened.
    conns: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<Option<Connection>>>>>,
}

/// Why an ask never reached the device.
#[derive(Debug)]
pub(super) struct NotThere(pub(super) String);

impl std::fmt::Display for NotThere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotThere {}

/// What devices of this white label speak: another's never meet them.
fn alpn() -> Vec<u8> {
    format!("{}/1", crate::config::CLI).into_bytes()
}

/// The relay at the install's address, which serves no QUIC address discovery, a Worker having no UDP; with `ticket`,
/// a pairing code's, which lets a computer joining in.
fn relay(address: &str, ticket: Option<&str>) -> Result<(RelayUrl, RelayMap)> {
    let url: RelayUrl = address.trim_end_matches('/').parse().context("the install's address isn't a URL")?;
    let mut config = RelayConfig::from(url.clone());
    config.quic = None;
    if let Some(ticket) = ticket {
        config = config.with_auth_token(ticket);
    }
    Ok((url, config.into()))
}

impl Node {
    /// Keeps this device's endpoint, once it's a device of an install with an address.
    pub(super) fn follow_net(&self) {
        let wanted = self.with(|i| Ok(i.r.members.device(&self.0.id).and(i.r.members.settings.get("address").cloned()))).ok().flatten();
        let mut task = self.0.net.task.lock().unwrap();
        if task.as_ref().map(|(a, _)| a) == wanted.as_ref() {
            return;
        }
        if let Some((_, old)) = task.take() {
            old.abort();
        }
        if let Some((_, endpoint)) = self.0.net.endpoint.lock().unwrap().take() {
            tokio::spawn(async move { endpoint.close().await });
        }
        self.0.net.conns.lock().unwrap().clear();
        if let Some(address) = wanted {
            *task = Some((address.clone(), tokio::spawn(self.clone().keep_net(address))));
        }
    }

    /// Binds this device's endpoint, then takes the connections other devices open, until it's replaced.
    async fn keep_net(self, address: String) {
        let mut wait = Duration::from_secs(1);
        let endpoint = loop {
            match self.bind(&address, None).await {
                Ok(endpoint) => break endpoint,
                Err(err) => tracing::warn!("this device's endpoint: {err:#}"),
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
        };
        *self.0.net.endpoint.lock().unwrap() = Some((address.clone(), endpoint.clone()));
        tracing::info!("this device is reached through the relay at {address}");
        while let Some(incoming) = endpoint.accept().await {
            let node = self.clone();
            tokio::spawn(async move {
                let conn = match incoming.accept() {
                    Ok(accepting) => accepting.await,
                    Err(err) => return tracing::debug!("a connection that didn't open: {err:#}"),
                };
                match conn {
                    Ok(conn) => node.take_conn(conn).await,
                    Err(err) => tracing::debug!("a connection that didn't open: {err:#}"),
                }
            });
        }
    }

    async fn bind(&self, address: &str, ticket: Option<&str>) -> Result<Endpoint> {
        let (_, map) = relay(address, ticket)?;
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(self.0.key.as_bytes()))
            .alpns(vec![alpn()])
            .relay_mode(RelayMode::Custom(map))
            .bind()
            .await?;
        Ok(endpoint)
    }

    /// Answers what another device asks on a connection it opened; one that isn't the install's yet may only ask to
    /// join it.
    async fn take_conn(&self, conn: Connection) {
        let remote = *conn.remote_id().as_bytes();
        let device = self.is_device(&remote);
        if device {
            // Asks go back on it too, unless one's open already.
            let slot = self.conn_slot(&remote);
            if let Ok(mut held) = slot.try_lock() {
                if held.as_ref().is_none_or(|c| c.close_reason().is_some()) {
                    *held = Some(conn.clone());
                }
            }
        }
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let node = self.clone();
            tokio::spawn(async move {
                let msg: Value = match recv.read_to_end(MESSAGE_LIMIT).await.map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
                    Ok(msg) => msg,
                    Err(err) => return tracing::debug!("what {} sent: {err:#}", short(&remote)),
                };
                let answer = match (msg["kind"].as_str(), device) {
                    (Some("ask"), true) => node.accept_call(remote, &msg["body"]["call"]).await.json(),
                    (Some("ask"), false) if msg["body"].get("join").is_some() => node.joining(remote, &msg["body"]["join"]).await,
                    (Some("notice"), true) => {
                        node.changed_from(&msg["body"]["changed"]);
                        Value::Null
                    }
                    _ => json!({ "error": "only your One App's devices are answered" }),
                };
                if let Err(err) = async { send.write_all(answer.to_string().as_bytes()).await?; send.finish()?; anyhow::Ok(()) }.await {
                    tracing::debug!("answering {}: {err:#}", short(&remote));
                }
            });
        }
    }

    /// Admits a computer joining with a pairing code: → its reply, or why not.
    async fn joining(&self, remote: Key, join: &Value) -> Value {
        let joined = match serde_json::from_value(join.clone()) {
            Ok(request) => self.admit(remote, request).await.and_then(|r| Ok(serde_json::to_value(r)?)),
            Err(err) => Err(err.into()),
        };
        joined.unwrap_or_else(|err| json!({ "error": format!("{err:#}") }))
    }

    /// A notice that something changed: `{ scope, app?, database?, at? }`.
    fn changed_from(&self, changed: &Value) {
        let Ok(scope) = Scope::parse(changed["scope"].as_str().unwrap_or("")) else { return };
        let wrote = changed["app"].as_str().zip(changed["database"].as_str()).map(|(app, db)| (app, db, changed["at"].as_u64().unwrap_or(0)));
        self.changed_there(&scope, wrote);
    }

    fn is_device(&self, key: &Key) -> bool {
        self.with(|i| Ok(i.r.members.device(key).is_some())).unwrap_or(false)
    }

    fn conn_slot(&self, to: &Key) -> Arc<tokio::sync::Mutex<Option<Connection>>> {
        self.0.net.conns.lock().unwrap().entry(*to).or_default().clone()
    }

    /// A connection to `to`: the one open, or a new one (`fresh`, or if that one closed). A `NotThere` if it can't
    /// be reached.
    async fn conn(&self, to: &Key, fresh: bool) -> Result<Connection> {
        let (address, endpoint) = self.0.net.endpoint.lock().unwrap().clone().ok_or_else(|| NotThere("this device isn't connected to your One App's relay".into()))?;
        let slot = self.conn_slot(to);
        let mut held = slot.lock().await;
        if let Some(conn) = held.as_ref().filter(|c| !fresh && c.close_reason().is_none()) {
            return Ok(conn.clone());
        }
        // Off the relay, a device is reached only on a connection it opened.
        if !endpoint.home_relay_status().get().iter().any(|r| r.is_connected()) {
            bail!(NotThere("this device isn't connected to your One App's relay".into()));
        }
        let (url, _) = relay(&address, None)?;
        let addr = EndpointAddr::new(PublicKey::from_bytes(to)?).with_relay_url(url);
        let conn = match tokio::time::timeout(CONNECT, endpoint.connect(addr, &alpn())).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => bail!(NotThere(format!("{} couldn't be reached: {err:#}", short(to)))),
            Err(_) => bail!(NotThere(format!("{} isn't connected", short(to)))),
        };
        *held = Some(conn.clone());
        Ok(conn)
    }

    /// Sends `to` a message of `kind` on a stream of its own, on the connection open or, if that one's gone, a new
    /// one: → its answer, waiting `limit` for it, if `answered`.
    async fn send(&self, to: &Key, kind: &str, body: &Value, answered: bool, limit: Duration) -> Result<Value> {
        let msg = json!({ "kind": kind, "body": body }).to_string();
        ensure!(msg.len() <= MESSAGE_LIMIT, "it's {} MB, more than another device is sent at once ({} MB)", msg.len() >> 20, MESSAGE_LIMIT >> 20);
        let mut fresh = false;
        let (mut send, mut recv) = loop {
            let conn = self.conn(to, fresh).await?;
            match conn.open_bi().await {
                Ok(stream) => break stream,
                Err(_) if !fresh => fresh = true,
                Err(err) => bail!(NotThere(format!("{} couldn't be reached: {err:#}", short(to)))),
            }
        };
        send.write_all(msg.as_bytes()).await?;
        send.finish()?;
        if !answered {
            return Ok(Value::Null);
        }
        let answer = tokio::time::timeout(limit, recv.read_to_end(MESSAGE_LIMIT)).await.context("timed out")??;
        Ok(serde_json::from_slice(&answer)?)
    }

    /// Asks another device to run a call, waiting `limit` for its answer. An error that's a `NotThere` means it never
    /// reached that device.
    pub(super) async fn ask(&self, to: Key, call: Value, limit: Duration) -> Result<Value> {
        self.send(&to, "ask", &json!({ "call": call }), true, limit).await
    }

    /// Tells each of `to` that something changed, expecting no answer.
    pub(super) fn notify(&self, to: &[Key], changed: &Value) {
        for device in to.iter().copied() {
            let (node, body) = (self.clone(), json!({ "changed": changed }));
            tokio::spawn(async move {
                if let Err(err) = node.send(&device, "notice", &body, false, CONNECT).await {
                    tracing::debug!("telling {}: {err:#}", short(&device));
                }
            });
        }
    }

    /// Asks the device that made a pairing code to admit this one, through the relay at `address`, which lets this
    /// one in for the code's `ticket`. The device that made it may not have told the relay of it yet: it's tried for a
    /// few seconds.
    pub(super) async fn ask_to_join(&self, address: &str, ticket: &str, to: Key, join: Value) -> Result<Value> {
        let endpoint = self.bind(address, Some(ticket)).await?;
        let (url, _) = relay(address, None)?;
        let addr = EndpointAddr::new(PublicKey::from_bytes(&to)?).with_relay_url(url);
        let mut tried = 0;
        let conn = loop {
            match tokio::time::timeout(CONNECT, endpoint.connect(addr.clone(), &alpn())).await {
                Ok(Ok(conn)) => break conn,
                _ if tried < 3 => tried += 1,
                Ok(Err(err)) => bail!("the device that made the pairing code couldn't be reached: {err:#}"),
                Err(_) => bail!("the device that made the pairing code isn't on"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        let answer = async {
            let (mut send, mut recv) = conn.open_bi().await?;
            send.write_all(json!({ "kind": "ask", "body": { "join": join } }).to_string().as_bytes()).await?;
            send.finish()?;
            let answer = tokio::time::timeout(Duration::from_secs(30), recv.read_to_end(MESSAGE_LIMIT)).await.context("the device that made the pairing code didn't answer")??;
            anyhow::Ok(serde_json::from_slice(&answer)?)
        }
        .await;
        conn.close(0u32.into(), b"joined");
        endpoint.close().await;
        answer
    }

    /// Tells the relay at the install's address something, signed by this device: a device it admitted (`add`), or a
    /// pairing code it made (`pairing`, the hash of its ticket, and `until`). Tried a few times, in the background.
    pub(super) fn tell_relay(&self, fields: Value) {
        let Some(address) = self.with(|i| Ok(i.r.members.settings.get("address").cloned())).ok().flatten() else {
            return tracing::debug!("telling the relay: your One App has no address yet");
        };
        let key = ed25519_dalek::SigningKey::from_bytes(self.0.key.as_bytes());
        tokio::task::spawn_blocking(move || {
            let url = format!("{}/relay/tell", address.trim_end_matches('/'));
            let mut wait = Duration::from_secs(1);
            for _ in 0..5 {
                let body = crate::keys::signed_request(&key, "one/relay", fields.clone(), now_ms());
                match ureq::post(&url).send_json(&body) {
                    Ok(_) => return,
                    Err(ureq::Error::StatusCode(code)) if code < 500 => return tracing::warn!("the relay refused what this device told it ({code})"),
                    Err(err) => tracing::debug!("telling the relay: {err}"),
                }
                std::thread::sleep(wait);
                wait *= 2;
            }
        });
    }
}

/// What a computer joining shows the relay for a pairing code's `secret`: not the secret, which admits it.
pub(super) fn pairing_ticket(secret: &[u8; 16]) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::new().chain_update(b"relay pairing:").chain_update(secret).finalize())
}

/// What a device that made a pairing code tells the relay of it: the hash of its ticket.
pub(super) fn ticket_hash(ticket: &str) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(ticket.as_bytes()))
}
