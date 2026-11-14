//! The client's Rust half, with no window: it holds the client's key, reaches a server by its key, and keeps what
//! servers answered (`kept`). The CLI (`cli`) and MCP (`mcp`) are this, run from a terminal or by an agent, and the
//! app is a window on it (`shell`).
//!
//! A client asks a server in MCP's JSON-RPC: one message on a stream of its own, and the answer back on that stream.
//! A server tells a client something the same way, on a stream the server opens.

pub mod cli;
mod kept;
pub mod mcp;

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, NetReportConfig, SecretKey};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};

pub use kept::Shown;

/// What servers and clients speak.
pub const ALPN: &[u8] = b"one/mcp/1";
/// The most one message may carry: a plugin's files fit.
pub const LIMIT: usize = 64 << 20;
/// How long reaching a server may take.
const CONNECT: Duration = Duration::from_secs(10);

/// An endpoint holding the key kept in `key_file`, which is made on first use. It reaches others, and is reached, by
/// key at the addresses it's told: with no relay, no address lookup and no probes, nothing about a network goes to
/// anyone outside it. A server's add-on binds this same endpoint.
pub async fn endpoint(key_file: &Path) -> Result<Endpoint> {
    Endpoint::builder(presets::Minimal)
        .secret_key(key(key_file)?)
        .alpns(vec![ALPN.to_vec()])
        .net_report_config(NetReportConfig::minimal())
        .bind()
        .await
        .context("couldn't open a socket")
}

fn key(file: &Path) -> Result<SecretKey> {
    fs::create_dir_all(file.parent().context("a key is kept in a folder")?)?;
    let mut new = fs::OpenOptions::new();
    new.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut new, 0o600);
    match new.open(file) {
        Ok(mut file) => {
            let key = SecretKey::generate();
            file.write_all(&key.to_bytes())?;
            Ok(key)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let bytes = fs::read(file)?.try_into().map_err(|_| anyhow!("{} doesn't hold a key", file.display()))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(error) => Err(error).with_context(|| format!("couldn't make {}", file.display())),
    }
}

/// The folder a client on a computer is kept in: `ONE_HOME` if that's set, as a network made to build or test with
/// sets it, and otherwise where the system keeps an app's data, under a name of this client's own. A phone gives
/// its app a folder instead.
pub fn home() -> Result<PathBuf> {
    match env::var_os("ONE_HOME") {
        Some(home) => Ok(home.into()),
        None => Ok(dirs::data_dir().context("this system has no folder for an app's data: set ONE_HOME")?.join(NAME)),
    }
}

/// What the app is to the system it's on: the name of its folder, and of the app among a phone's. The app before
/// this one is `one` and `The One App` there, and this is neither.
pub const NAME: &str = "app.theone.client";

/// A client, kept in its home folder: its `key`, the address of the `server` it asks, and what servers answered
/// (`kept`).
pub struct Client {
    home: PathBuf,
    endpoint: Endpoint,
    connection: Mutex<Option<Connection>>,
    told: broadcast::Sender<Value>,
}

impl Client {
    pub async fn open(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        let endpoint = endpoint(&home.join("key")).await?;
        Ok(Self { home, endpoint, connection: Mutex::new(None), told: broadcast::channel(16).0 })
    }

    /// What the server tells this client while it's connected: each a JSON-RPC notification.
    pub fn told(&self) -> broadcast::Receiver<Value> {
        self.told.subscribe()
    }

    /// Asks the server one JSON-RPC message and gives its answer, which for a notification is nothing.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>> {
        let (mut send, mut receive) = self.connection().await?.open_bi().await?;
        send.write_all(message).await?;
        send.finish()?;
        Ok(receive.read_to_end(LIMIT).await?)
    }

    /// Asks the server for `method`, and gives its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let answer = self.ask(message.to_string().as_bytes()).await?;
        let mut answer: Value = serde_json::from_slice(&answer).context("the server's answer isn't JSON")?;
        match answer.get("error") {
            Some(error) => bail!("{}", error["message"].as_str().unwrap_or("the server refused")),
            None => Ok(answer["result"].take()),
        }
    }

    /// Ends the connection, if one was made, and waits until the server has been told.
    pub async fn close(&self) {
        if self.connection.lock().await.is_some() {
            self.endpoint.close().await
        }
    }

    /// The connection to the server, made when there's none open. Its address is read each time, since a server
    /// that starts again may be somewhere else.
    async fn connection(&self) -> Result<Connection> {
        let mut connection = self.connection.lock().await;
        if let Some(open) = connection.as_ref().filter(|open| open.close_reason().is_none()) {
            return Ok(open.clone());
        }
        let file = self.home.join("server");
        let address = fs::read(&file).with_context(|| format!("{} doesn't say which server to ask", file.display()))?;
        let address: EndpointAddr =
            serde_json::from_slice(&address).with_context(|| format!("{} isn't a server's address", file.display()))?;
        let open = tokio::time::timeout(CONNECT, self.endpoint.connect(address, ALPN))
            .await
            .context("no server answered")?
            .context("no server answered")?;
        tokio::spawn(hear(open.clone(), self.told.clone()));
        Ok(connection.insert(open).clone())
    }
}

/// Passes on what the server tells this client over `connection`, until the connection ends.
async fn hear(connection: Connection, told: broadcast::Sender<Value>) {
    while let Ok(mut receive) = connection.accept_uni().await {
        let told = told.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            if let Ok(message) = serde_json::from_slice(&message) {
                // With nobody listening, as in the CLI, it's dropped.
                let _ = told.send(message);
            }
        });
    }
}
