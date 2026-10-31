//! A client device: one that keeps no log (a phone) and asks the space's other devices to do the work.
//! It joins like any device (SPEC.md, "Join"), then sends requests over `one/client/1`.
//! The C functions at the bottom are what the iPhone and Android shells call.

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

const JOIN_ALPN: &[u8] = b"one/join/1";
const CLIENT_ALPN: &[u8] = b"one/client/1";

#[derive(Serialize, Deserialize)]
struct PairingCode {
    space: [u8; 32],
    from: EndpointAddr,
    secret: [u8; 16],
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    secret: String,
    name: String,
    /// Devices to ask, most recently useful first.
    peers: Vec<EndpointAddr>,
}

pub struct Client {
    endpoint: Endpoint,
    path: PathBuf,
    saved: Mutex<Saved>,
    events: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
}

impl Client {
    pub async fn start(dir: PathBuf, name: &str) -> Result<Client> {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("client.json");
        let mut saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if saved.secret.is_empty() {
            saved.secret = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 32]>());
        }
        if saved.name.is_empty() {
            saved.name = name.to_string();
        }
        let secret: [u8; 32] = data_encoding::HEXLOWER.decode(saved.secret.as_bytes())?.try_into().map_err(|_| anyhow::anyhow!("bad key"))?;
        let endpoint = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&secret)).bind().await?;
        let client = Client { endpoint, path, saved: Mutex::new(saved), events: tokio::sync::Mutex::new(None) };
        client.save()?;
        Ok(client)
    }

    fn save(&self) -> Result<()> {
        std::fs::write(&self.path, serde_json::to_vec_pretty(&*self.saved.lock().unwrap())?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn id(&self) -> String {
        data_encoding::HEXLOWER.encode(self.endpoint.id().as_bytes())
    }

    pub fn joined(&self) -> bool {
        !self.saved.lock().unwrap().peers.is_empty()
    }

    /// Sets the name this device gives the space it joins next.
    pub fn set_name(&self, name: &str) -> Result<()> {
        self.saved.lock().unwrap().name = name.to_string();
        self.save()
    }

    pub async fn join(&self, code: &str) -> Result<Value> {
        let bytes = data_encoding::BASE32_NOPAD.decode(code.trim().to_uppercase().as_bytes()).context("that isn't a pairing code")?;
        let code: PairingCode = ciborium::from_reader(bytes.as_slice()).context("that isn't a pairing code")?;
        let conn = self.endpoint.connect(code.from.clone(), JOIN_ALPN).await.context("couldn't reach the device that made the pairing code")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let name = self.saved.lock().unwrap().name.clone();
        send.write_all(&serde_json::to_vec(&json!({ "secret": code.secret, "name": name, "client": true }))?).await?;
        send.finish()?;
        let reply: Value = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the pairing code was refused"));
        }
        {
            let mut saved = self.saved.lock().unwrap();
            saved.peers.retain(|p| p.id != code.from.id);
            saved.peers.insert(0, code.from);
        }
        self.save()?;
        // Learn the space's other devices, so any of them can answer later.
        if let Ok(state) = self.request("GET", "/v1/state", Value::Null).await {
            self.learn(&state);
        }
        Ok(reply)
    }

    fn learn(&self, state: &Value) {
        let me = self.endpoint.id();
        let mut saved = self.saved.lock().unwrap();
        for space in state["spaces"].as_array().into_iter().flatten() {
            for d in space["devices"].as_array().into_iter().flatten().filter(|d| d["client"] != true) {
                let Some(id) = d["device"].as_str().and_then(|h| data_encoding::HEXLOWER.decode(h.as_bytes()).ok()) else { continue };
                let Ok(id) = <[u8; 32]>::try_from(id.as_slice()) else { continue };
                let Ok(id) = EndpointId::from_bytes(&id) else { continue };
                if id != me && !saved.peers.iter().any(|p| p.id == id) {
                    saved.peers.push(EndpointAddr::new(id));
                }
            }
        }
    }

    /// Sends one request to the first device that answers, and remembers which one did.
    pub async fn request(&self, method: &str, path: &str, body: Value) -> Result<Value> {
        let peers = self.saved.lock().unwrap().peers.clone();
        if peers.is_empty() {
            bail!("join a space first");
        }
        let request = serde_json::to_vec(&json!({ "method": method, "path": path, "body": body }))?;
        let mut last = anyhow::anyhow!("no device answered");
        for peer in peers {
            match tokio::time::timeout(Duration::from_secs(12), self.ask(peer.clone(), &request)).await {
                Ok(Ok(reply)) => {
                    let mut saved = self.saved.lock().unwrap();
                    if saved.peers.first().map(|p| p.id) != Some(peer.id) {
                        saved.peers.retain(|p| p.id != peer.id);
                        saved.peers.insert(0, peer);
                    }
                    return Ok(reply);
                }
                Ok(Err(err)) => last = err,
                Err(_) => last = anyhow::anyhow!("timed out"),
            }
        }
        Err(last.context("none of the space's devices answered"))
    }

    async fn ask(&self, peer: EndpointAddr, request: &[u8]) -> Result<Value> {
        // Dial by id: saved addresses go stale when a device restarts, discovery finds the current one.
        let conn = self.endpoint.connect(EndpointAddr::new(peer.id), CLIENT_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(request).await?;
        send.finish()?;
        Ok(serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?)
    }

    /// Streams the node's events (`changed`, `peers`, `spaces`) until the connection drops.
    pub async fn watch(&self, tx: mpsc::UnboundedSender<Value>) -> Result<()> {
        let peer = self.saved.lock().unwrap().peers.first().cloned().context("join a space first")?;
        let conn = self.endpoint.connect(peer, CLIENT_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(&json!({ "method": "GET", "path": "/v1/events" }))?).await?;
        send.finish()?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let Some(n) = recv.read(&mut chunk).await? else { return Ok(()) };
            buf.extend_from_slice(&chunk[..n]);
            while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=i).collect();
                if let Ok(event) = serde_json::from_slice(&line) {
                    if tx.send(event).is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

// The C interface. Every function returns a JSON string the caller frees with `one_free`.

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static CLIENT: OnceLock<Client> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

fn out(result: Result<Value>) -> *mut c_char {
    let value = match result {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": format!("{e:#}") }),
    };
    CString::new(value.to_string()).unwrap_or_default().into_raw()
}

unsafe fn arg(s: *const c_char) -> String {
    if s.is_null() { String::new() } else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() }
}

fn client() -> Result<&'static Client> {
    CLIENT.get().context("call one_start first")
}

/// Starts the client with its files in `dir`, calling this device `name`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_start(dir: *const c_char, name: *const c_char) -> *mut c_char {
    let (dir, name) = unsafe { (arg(dir), arg(name)) };
    out(runtime().block_on(async {
        if CLIENT.get().is_none() {
            let c = Client::start(PathBuf::from(dir), &name).await?;
            let _ = CLIENT.set(c);
        }
        let c = client()?;
        Ok(json!({ "device": c.id(), "joined": c.joined() }))
    }))
}

/// Joins the space a pairing code is for, as `name` (or the name it started with, if empty).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_join(code: *const c_char, name: *const c_char) -> *mut c_char {
    let (code, name) = unsafe { (arg(code), arg(name)) };
    out(runtime().block_on(async {
        let c = client()?;
        if !name.trim().is_empty() {
            c.set_name(name.trim())?;
        }
        c.join(&code).await
    }))
}

/// Sends `method path` with a JSON `body` (or empty) to the space's devices.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_request(method: *const c_char, path: *const c_char, body: *const c_char) -> *mut c_char {
    let (method, path, body) = unsafe { (arg(method), arg(path), arg(body)) };
    out(runtime().block_on(async {
        let body: Value = if body.is_empty() { Value::Null } else { serde_json::from_str(&body)? };
        let reply = client()?.request(&method, &path, body).await?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("failed"));
        }
        Ok(reply)
    }))
}

/// Waits up to `timeout_ms` for the next event; returns `{"ok":null}` if none came.
#[unsafe(no_mangle)]
pub extern "C" fn one_next_event(timeout_ms: u64) -> *mut c_char {
    out(runtime().block_on(async {
        let c = client()?;
        let mut guard = c.events.lock().await;
        if guard.is_none() {
            let (tx, rx) = mpsc::unbounded_channel();
            *guard = Some(rx);
            runtime().spawn(async move {
                loop {
                    let _ = c.watch(tx.clone()).await;
                    if tx.is_closed() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            });
        }
        let rx = guard.as_mut().unwrap();
        Ok(tokio::time::timeout(Duration::from_millis(timeout_ms), rx.recv()).await.ok().flatten().unwrap_or(Value::Null))
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn one_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
