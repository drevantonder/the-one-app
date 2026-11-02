//! The relay, how devices reach each other with no address of their own: each keeps one WebSocket to the install's
//! address (`/relay`, kernel/gateway/worker.js), proving its key there, and what it sends another device's key is passed
//! to that device's. The relay is blind: each message is sealed to the key of the device it's for and signed by the one
//! that sent it, bound to both, its id and when, so the gateway passing it on can read, change or pass off none of it.
//! Asks (a call, a hello, joining) each get an answer, sealed the same way; notices (something changed) get none.

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use super::*;

/// How long a socket may go without a word from the relay before it's taken as gone: it's pinged twice as often.
const SILENT: Duration = Duration::from_secs(60);
/// How far a message's time may be from this device's clock; one seen within it isn't taken twice.
const FRESH: u64 = 5 * 60_000;
/// The most a call or its answer may carry: an email of a claim's receipts, base64, fits.
const MESSAGE_LIMIT: usize = 64 << 20;
/// A message bigger than this goes to the relay in pieces of it, each passed on as it comes.
const PIECE: usize = 512 << 10;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// This device's socket to the relay, and the asks it's waiting on.
pub(super) struct Relay {
    /// When this device started: a message sent before then may have been taken already, so it isn't.
    started: u64,
    /// The task keeping the socket, with the address it's to.
    task: Mutex<Option<(String, tokio::task::JoinHandle<()>)>>,
    /// What's sent on the socket open now.
    out: Mutex<Option<mpsc::UnboundedSender<String>>>,
    /// Each ask waiting on its answer: the device asked, and where its answer goes.
    asks: Mutex<HashMap<u64, (Key, oneshot::Sender<Result<Value>>)>>,
    /// The messages taken lately, by who sent them and their id, with when they were sent.
    seen: Mutex<HashMap<(Key, u64), u64>>,
    /// The pieces of each big message on its way, by who sent it and its id: when the first came, and those here.
    pieces: Mutex<HashMap<(String, u64), (Instant, Vec<Option<String>>)>>,
}

impl Relay {
    pub(super) fn new() -> Relay {
        Relay { started: now_ms(), task: Default::default(), out: Default::default(), asks: Default::default(), seen: Default::default(), pieces: Default::default() }
    }
}

/// Why an ask never reached the device: it isn't connected to the relay, or this one isn't.
#[derive(Debug)]
pub(super) struct NotThere(String);

impl std::fmt::Display for NotThere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotThere {}

impl Node {
    /// Keeps this device's socket to the relay at the install's address, once it has one and is a device of it.
    pub(super) fn follow_relay(&self) {
        let wanted = self.with(|i| Ok(i.r.members.device(&self.0.id).and(i.r.members.settings.get("address").cloned()))).ok().flatten();
        let mut task = self.0.relay.task.lock().unwrap();
        if task.as_ref().map(|(a, _)| a) == wanted.as_ref() {
            return;
        }
        if let Some((_, old)) = task.take() {
            old.abort();
        }
        if let Some(address) = wanted {
            *task = Some((address.clone(), tokio::spawn(self.clone().keep_relay(address))));
        }
    }

    /// Keeps a socket to the relay open, opening it again whenever it closes, waiting longer each time it fails.
    async fn keep_relay(self, address: String) {
        let mut wait = Duration::from_secs(1);
        loop {
            let opened = Instant::now();
            if let Err(err) = self.use_relay(&address).await {
                tracing::debug!("the relay at {address}: {err:#}");
            }
            *self.0.relay.out.lock().unwrap() = None;
            for (_, (_, ask)) in self.0.relay.asks.lock().unwrap().drain() {
                let _ = ask.send(Err(anyhow!("the connection to the relay closed")));
            }
            if opened.elapsed() > SILENT {
                wait = Duration::from_secs(1);
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(60));
        }
    }

    /// Opens a socket to the relay as this device, and proves its key there.
    async fn open_relay(&self, address: &str) -> Result<Socket> {
        let base = address.trim_end_matches('/').replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(format!("{base}/relay?key={}", hex(&self.0.id)))).await.context("timed out")??;
        let challenge = next_said(&mut ws).await?;
        let challenge = challenge["challenge"].as_str().context("the relay sent no challenge")?;
        let signature = self.0.key.sign(format!("relay:{challenge}").as_bytes()).to_hex();
        ws.send(Message::text(json!({ "signature": signature }).to_string())).await?;
        ensure!(next_said(&mut ws).await?["ready"] == true, "the relay didn't take this device's key");
        Ok(ws)
    }

    /// Runs one socket to the relay until it closes: passing on what this device sends, and taking what it's sent.
    async fn use_relay(&self, address: &str) -> Result<()> {
        let mut ws = self.open_relay(address).await?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        *self.0.relay.out.lock().unwrap() = Some(tx);
        tracing::info!("this device is reached through the relay at {address}");
        let mut ping = tokio::time::interval(SILENT / 2);
        let mut heard = Instant::now();
        loop {
            tokio::select! {
                msg = ws.next() => {
                    heard = Instant::now();
                    match msg.context("the relay closed")?? {
                        Message::Text(text) => match serde_json::from_str(&text) {
                            Ok(msg) => self.from_relay(msg),
                            Err(err) => tracing::debug!("the relay sent what isn't JSON: {err}"),
                        },
                        Message::Close(_) => bail!("the relay closed"),
                        _ => {}
                    }
                }
                out = rx.recv() => ws.send(Message::text(out.context("the socket was replaced")?)).await?,
                _ = ping.tick() => {
                    ensure!(heard.elapsed() < SILENT, "the relay said nothing for {}s", SILENT.as_secs());
                    ws.send(Message::Ping(Vec::new().into())).await?;
                }
            }
        }
    }

    /// What the relay passed on: another device's ask (answered in the background), an answer to this device's, word
    /// that a device asked isn't there, or a notice.
    fn from_relay(&self, msg: Value) {
        let from = msg["from"].as_str().unwrap_or("").to_string();
        let msg = match msg.get("piece") {
            Some(_) => match self.piece(&from, &msg) {
                Some(whole) => whole,
                None => return,
            },
            None => msg,
        };
        if msg["gone"] == true {
            let asked = msg["id"].as_u64().and_then(|id| self.0.relay.asks.lock().unwrap().remove(&id));
            if let Some((_, ask)) = asked {
                let _ = ask.send(Err(NotThere(format!("{} isn't connected", short(&unhex(&from).unwrap_or_default()))).into()));
            }
            return;
        }
        // The serverless node's word that it wrote, which is only a hint, and the relay's own to pass on.
        if from == "serverless" {
            if let Some(changed) = msg["notice"].get("changed") {
                self.changed_from(changed);
            }
            return;
        }
        let (sender, kind, id, body) = match self.open_from(&msg) {
            Ok(opened) => opened,
            Err(err) => return tracing::debug!("the relay passed on a message that didn't open: {err:#}"),
        };
        match kind.as_str() {
            "answer" => {
                let mut asks = self.0.relay.asks.lock().unwrap();
                // Only the device asked answers.
                if asks.get(&id).is_some_and(|(to, _)| *to == sender) {
                    let (_, ask) = asks.remove(&id).unwrap();
                    let _ = ask.send(Ok(body));
                }
            }
            "notice" => {
                if self.with(|i| Ok(i.r.members.device(&sender).is_some())).unwrap_or(false) {
                    self.changed_from(&body["changed"]);
                }
            }
            "ask" => {
                let node = self.clone();
                tokio::spawn(async move {
                    let answer = node.answer(sender, &body).await;
                    if let Err(err) = node.send_sealed(&sender, "answer", id, &answer) {
                        tracing::debug!("answering {}: {err:#}", short(&sender));
                    }
                });
            }
            _ => {}
        }
    }

    /// Keeps a piece of a big message: → the message, once every piece of it is here.
    fn piece(&self, from: &str, msg: &Value) -> Option<Value> {
        let (id, n, of) = (msg["id"].as_u64()?, msg["piece"]["n"].as_u64()? as usize, msg["piece"]["of"].as_u64()? as usize);
        let text = msg["piece"]["text"].as_str()?;
        if of > MESSAGE_LIMIT * 2 / PIECE || n >= of || text.len() > PIECE {
            return None;
        }
        let mut pieces = self.0.relay.pieces.lock().unwrap();
        pieces.retain(|_, (first, _)| first.elapsed() < Duration::from_secs(120));
        let (_, got) = pieces.entry((from.to_string(), id)).or_insert_with(|| (Instant::now(), vec![None; of]));
        if got.len() != of {
            return None;
        }
        got[n] = Some(text.to_string());
        if got.iter().any(Option::is_none) {
            return None;
        }
        let (_, got) = pieces.remove(&(from.to_string(), id))?;
        let mut whole: Value = serde_json::from_str(&got.into_iter().flatten().collect::<String>()).ok()?;
        whole["from"] = json!(from);
        Some(whole)
    }

    /// A notice that something changed: `{ scope, app?, database?, at? }`.
    fn changed_from(&self, changed: &Value) {
        let Ok(scope) = Scope::parse(changed["scope"].as_str().unwrap_or("")) else { return };
        let wrote = changed["app"].as_str().zip(changed["database"].as_str()).map(|(app, db)| (app, db, changed["at"].as_u64().unwrap_or(0)));
        self.changed_there(&scope, wrote);
    }

    /// Answers another device's ask: a call (a hello among them), or joining the install.
    async fn answer(&self, remote: Key, ask: &Value) -> Value {
        if let Some(join) = ask.get("join") {
            let joined = match serde_json::from_value(join.clone()) {
                Ok(request) => self.admit(remote, request).await.and_then(|r| Ok(serde_json::to_value(r)?)),
                Err(err) => Err(err.into()),
            };
            return joined.unwrap_or_else(|err| json!({ "error": format!("{err:#}") }));
        }
        self.accept_call(remote, &ask["call"]).await.json()
    }

    /// A message for `to`, sealed to its key and signed by this device: of `kind` (`ask`, `answer` or `notice`), with
    /// `id`.
    fn sealed_for(&self, to: &Key, kind: &str, id: u64, body: &Value) -> Result<Value> {
        let body = body.to_string();
        ensure!(body.len() <= MESSAGE_LIMIT, "it's {} MB, more than another device is sent at once ({} MB)", body.len() >> 20, MESSAGE_LIMIT >> 20);
        let time = now_ms();
        let bound = bound(kind, &self.0.id, to, id, time);
        let key: [u8; 32] = rand::random();
        let wrapped = crate::backups::seal(to, &key)?;
        let sealed = crate::db::seal(&key, &bound, body.as_bytes());
        let signature = self.0.key.sign(&signed(&bound, &wrapped, &sealed)).to_hex();
        let b64 = |b: &[u8]| data_encoding::BASE64.encode(b);
        Ok(json!({ "to": hex(to), "kind": kind, "id": id, "time": time, "key": b64(&wrapped), "sealed": b64(&sealed), "signature": signature }))
    }

    /// Opens a message sealed for this device, checking who signed it and that it's new: → (who sent it, its kind, its
    /// id, what it says).
    fn open_from(&self, msg: &Value) -> Result<(Key, String, u64, Value)> {
        let sender = unhex(msg["from"].as_str().context("from whom?")?)?;
        let (kind, id, time) = (msg["kind"].as_str().context("of what kind?")?.to_string(), msg["id"].as_u64().context("no id")?, msg["time"].as_u64().context("no time")?);
        let now = now_ms();
        ensure!(now.abs_diff(time) < FRESH, "it was sent too long ago, or this device's clock is wrong");
        // One sent before this device started may have been taken before then, which it no longer knows.
        ensure!(time >= self.0.relay.started, "it was sent before this device started");
        let b64 = |f: &str| data_encoding::BASE64.decode(msg[f].as_str().unwrap_or("").as_bytes()).context("not base64");
        let (wrapped, sealed) = (b64("key")?, b64("sealed")?);
        let bound = bound(&kind, &sender, &self.0.id, id, time);
        let signature: [u8; 64] = unhex_bytes(msg["signature"].as_str().unwrap_or(""))?.try_into().map_err(|_| anyhow!("not a signature"))?;
        ed25519_dalek::VerifyingKey::from_bytes(&sender)?.verify_strict(&signed(&bound, &wrapped, &sealed), &ed25519_dalek::Signature::from_bytes(&signature)).context("its signature isn't its sender's")?;
        {
            let mut seen = self.0.relay.seen.lock().unwrap();
            seen.retain(|_, t| now.abs_diff(*t) < FRESH);
            ensure!(seen.insert((sender, id), time).is_none(), "it was taken already");
        }
        let key = crate::backups::unseal(self.0.key.as_bytes(), &wrapped)?;
        Ok((sender, kind, id, serde_json::from_slice(&crate::db::open(&key, &bound, &sealed)?)?))
    }

    /// Sends a message sealed for `to`: a big one in pieces, the first of its kind, so the relay says at once when an
    /// ask's device isn't there.
    fn send_sealed(&self, to: &Key, kind: &str, id: u64, body: &Value) -> Result<()> {
        let msg = self.sealed_for(to, kind, id, body)?.to_string();
        let out = self.0.relay.out.lock().unwrap().clone().ok_or_else(|| NotThere("this device isn't connected to your One App's relay".into()))?;
        let not_there = |_| NotThere("this device isn't connected to your One App's relay".into());
        if msg.len() <= PIECE {
            return Ok(out.send(msg).map_err(not_there)?);
        }
        // Sealed, signed JSON is ASCII, so it splits anywhere.
        let pieces: Vec<&[u8]> = msg.as_bytes().chunks(PIECE).collect();
        for (n, text) in pieces.iter().enumerate() {
            let kind = if n == 0 { kind } else { "piece" };
            let piece = json!({ "to": hex(to), "kind": kind, "id": id, "piece": { "n": n, "of": pieces.len(), "text": std::str::from_utf8(text)? } });
            out.send(piece.to_string()).map_err(not_there)?;
        }
        Ok(())
    }

    /// Asks another device through the relay, waiting `limit` for its answer. An error that's a `NotThere` means it
    /// never reached that device.
    pub(super) async fn ask_relay(&self, to: Key, ask: Value, limit: Duration) -> Result<Value> {
        let id = new_message_id();
        let (tx, rx) = oneshot::channel();
        self.0.relay.asks.lock().unwrap().insert(id, (to, tx));
        if let Err(err) = self.send_sealed(&to, "ask", id, &ask) {
            self.0.relay.asks.lock().unwrap().remove(&id);
            return Err(err);
        }
        match tokio::time::timeout(limit, rx).await {
            Ok(answer) => answer?,
            Err(_) => {
                self.0.relay.asks.lock().unwrap().remove(&id);
                bail!("timed out")
            }
        }
    }

    /// Tells each of `to` that something changed, expecting no answer.
    pub(super) fn notify(&self, to: &[Key], changed: &Value) {
        for device in to {
            let id = new_message_id();
            if let Err(err) = self.send_sealed(device, "notice", id, &json!({ "changed": changed })) {
                tracing::debug!("telling {}: {err:#}", short(device));
            }
        }
    }

    /// Asks a device at `address` once, on a socket of its own, as a device not yet in the install does to join.
    pub(super) async fn ask_relay_once(&self, address: &str, to: Key, ask: Value) -> Result<Value> {
        let mut ws = self.open_relay(address).await?;
        let id = new_message_id();
        ws.send(Message::text(self.sealed_for(&to, "ask", id, &ask)?.to_string())).await?;
        let answer = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let msg = next_said(&mut ws).await?;
                if msg["gone"] == true {
                    bail!("the device that made the pairing code isn't on");
                }
                if let Ok((sender, kind, got, body)) = self.open_from(&msg) {
                    if sender == to && kind == "answer" && got == id {
                        return Ok(body);
                    }
                }
            }
        })
        .await
        .context("the device that made the pairing code didn't answer")?;
        let _ = ws.close(None).await;
        answer
    }
}

/// What a message is bound to: its kind, who sends it to whom, its id and when.
fn bound(kind: &str, from: &Key, to: &Key, id: u64, time: u64) -> String {
    format!("relay {kind} {} {} {id} {time}", hex(from), hex(to))
}

/// What a message's signature is over: what it's bound to, its key sealed to its receiver, and its sealed body.
fn signed(bound: &str, wrapped: &[u8], sealed: &[u8]) -> Vec<u8> {
    let mut out = bound.as_bytes().to_vec();
    out.extend((wrapped.len() as u32).to_be_bytes());
    out.extend(wrapped);
    out.extend(sealed);
    out
}

/// A message's id: at random, and small enough that JavaScript, which the relay is, reads it as it is.
fn new_message_id() -> u64 {
    rand::random::<u64>() >> 12
}

fn unhex_bytes(s: &str) -> Result<Vec<u8>> {
    Ok(data_encoding::HEXLOWER.decode(s.as_bytes())?)
}

/// The next thing the relay says, as JSON.
async fn next_said(ws: &mut Socket) -> Result<Value> {
    loop {
        match tokio::time::timeout(SILENT, ws.next()).await.context("the relay said nothing")?.context("the relay closed")?? {
            Message::Text(text) => return Ok(serde_json::from_str(&text)?),
            Message::Close(frame) => bail!("the relay closed{}", frame.map(|f| format!(": {}", f.reason)).unwrap_or_default()),
            _ => {}
        }
    }
}
