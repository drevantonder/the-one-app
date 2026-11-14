//! The client's Rust half, with no window: it holds the client's key, reaches a server by its key, and keeps what
//! servers answered. The CLI (`cli`) and MCP (`mcp`) are this, run from a terminal or by an agent.
//!
//! A client asks a server in MCP's JSON-RPC: one message on a stream of its own, and the answer back on that stream.

pub mod cli;
pub mod mcp;

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, NetReportConfig, SecretKey};
use serde_json::{Value, json};
use tokio::sync::Mutex;

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

/// A client, kept in its home folder: its `key`, the address of the `server` it asks, and what servers answered
/// (`kept`).
pub struct Client {
    home: PathBuf,
    endpoint: Endpoint,
    connection: Mutex<Option<Connection>>,
}

impl Client {
    pub async fn open(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        let endpoint = endpoint(&home.join("key")).await?;
        Ok(Self { home, endpoint, connection: Mutex::new(None) })
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

    /// The network's tools as the server lists them now, kept for when no server answers.
    pub async fn tools(&self) -> Result<Value> {
        let tools = self.request("tools/list", json!({})).await?;
        fs::create_dir_all(self.home.join("kept"))?;
        fs::write(self.kept_tools_file(), tools.to_string())?;
        Ok(tools)
    }

    /// The network's tools as a server last listed them.
    pub fn kept_tools(&self) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.kept_tools_file()).ok()?).ok()
    }

    fn kept_tools_file(&self) -> PathBuf {
        self.home.join("kept").join("tools.json")
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
        Ok(connection.insert(open).clone())
    }
}
