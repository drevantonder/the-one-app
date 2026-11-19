//! MCP over stdio. Servers speak MCP themselves, so each message read is passed to the server and its answer
//! written back. With `--code`, each says it's for code mode (`OneCore.Mcp`), whose `execute` and `call` a server
//! runs, as it does for an agent over the web. Each says which client asked, as the client said in `initialize`, where
//! MCP's 2026-07-28 has every request say it.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use client::Client;

/// Serves MCP until stdin closes, in code mode with `code`. Closing it is how a client goes, as MCP's stdio has it,
/// so what it asked that's still running is dropped then, which tells the server its caller is gone and stops a
/// script that isn't a task. What the server notifies
/// about a request while it runs is passed on ahead of its answer, as MCP's progress is, where the request asked for
/// it with a `progressToken`.
pub async fn serve(client: Arc<Client>, code: bool) -> Result<()> {
    let mut messages = BufReader::new(tokio::io::stdin()).lines();
    let (lines, mut written) = mpsc::unbounded_channel::<Vec<u8>>();
    let writing = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(mut line) = written.recv().await {
            line.push(b'\n');
            out.write_all(&line).await?;
            out.flush().await?;
        }
        anyhow::Ok(())
    });
    let mut asks = JoinSet::new();
    let mut said = Value::Null;
    while let Some(message) = messages.next_line().await? {
        let (client, lines) = (client.clone(), lines.clone());
        let mut request: Value = serde_json::from_str(&message).unwrap_or_default();
        if request["method"] == "initialize" {
            said = request["params"]["clientInfo"].clone();
            // An agent's MCP client goes by its own name, never one of the One App's: those say its person is on
            // their own screen (the app's window, or a browser's), which a server believes of the One App alone.
            if said["name"].as_str().is_some_and(|name| name.starts_with("one-")) {
                said["name"] = json!(format!("mcp {}", said["name"].as_str().unwrap_or_default()));
            }
        }
        // What an agent says of itself in a request is only what it said as it began, renamed as above.
        if request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"].is_object() {
            request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = Value::Null;
        }
        if request["method"].is_string() && said.is_object() && request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"].is_null() {
            request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = said.clone();
        }
        asks.spawn(async move {
            if code && request["method"].is_string() {
                request["params"]["_meta"]["mode"] = json!("code");
            }
            let message = if request.is_object() { request.to_string() } else { message };
            let progress = !request["params"]["_meta"]["progressToken"].is_null();
            let notified = |line: &[u8]| drop(progress.then(|| lines.send(line.to_vec())));
            let response = match client.ask_notified(message.as_bytes(), notified).await {
                Ok(response) => response,
                Err(error) => refusal(&message, &error),
            };
            if !response.is_empty() {
                let _ = lines.send(response);
            }
        });
    }
    asks.shutdown().await;
    drop(lines);
    writing.await?
}

/// What a request that reached no server is answered with. A notification is answered with nothing.
fn refusal(message: &str, error: &anyhow::Error) -> Vec<u8> {
    match serde_json::from_str::<Value>(message).ok().and_then(|message| message.get("id").cloned()) {
        Some(id) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("{error:#}")}})
            .to_string()
            .into_bytes(),
        None => Vec::new(),
    }
}
