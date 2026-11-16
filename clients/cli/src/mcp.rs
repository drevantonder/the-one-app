//! MCP over stdio. Servers speak MCP themselves, so each message read is passed to the server and its answer
//! written back. With `--code`, each says it's for code mode (`OneCore.Mcp`), and a script `execute` runs, and the
//! calls `call` makes, are run here, on this device (`codemode`).

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use client::Client;

/// Serves MCP until stdin closes and every answer is written, in code mode with `code`. What the server notifies
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
    while let Some(message) = messages.next_line().await? {
        let (client, lines) = (client.clone(), lines.clone());
        asks.spawn(async move {
            let mut request: Value = serde_json::from_str(&message).unwrap_or_default();
            if code && request["method"].is_string() {
                request["params"]["_meta"]["mode"] = json!("code");
            }
            let message = if request.is_object() { request.to_string() } else { message };
            let progress = !request["params"]["_meta"]["progressToken"].is_null();
            let local = request["method"] == "tools/call"
                && ["execute", "call"].contains(&request["params"]["name"].as_str().unwrap_or_default());
            let response = if local {
                here(&client, &request).await
            } else {
                let notified = |line: &[u8]| drop(progress.then(|| lines.send(line.to_vec())));
                match client.ask_notified(message.as_bytes(), notified).await {
                    Ok(response) => response,
                    Err(error) => refusal(&message, &error),
                }
            };
            if !response.is_empty() {
                let _ = lines.send(response);
            }
        });
    }
    drop(lines);
    while asks.join_next().await.transpose()?.is_some() {}
    writing.await?
}

/// The answer to a call of `execute`, its script run here, or of `call`, its calls asked from here. It says it's a
/// complete result, as MCP has every result say from 2026-07-28, which a client of an earlier version ignores.
async fn here(client: &Arc<Client>, request: &Value) -> Vec<u8> {
    let arguments = &request["params"]["arguments"];
    let result = if request["params"]["name"] == "call" {
        crate::codemode::call(client, &arguments["calls"]).await
    } else {
        let code = arguments["code"].as_str().unwrap_or_default();
        crate::codemode::run(client, code)
            .await
            .map(|(ok, text)| {
                json!({"content": [{"type": "text", "text": text}], "structuredContent": {"text": text}, "isError": !ok})
            })
    };
    let mut result = result
        .unwrap_or_else(|error| json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true}));
    result["resultType"] = json!("complete");
    json!({"jsonrpc": "2.0", "id": request["id"], "result": result}).to_string().into_bytes()
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
