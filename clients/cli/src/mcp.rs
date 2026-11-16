//! MCP over stdio. Servers speak MCP themselves, so each message read is passed to the server and its answer
//! written back. With `--code`, each says it's for code mode (`OneCore.Mcp`), and a script `execute` runs is run here,
//! on this device (`codemode`).

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use client::Client;

/// Serves MCP until stdin closes and every answer is written, in code mode with `code`.
pub async fn serve(client: Arc<Client>, code: bool) -> Result<()> {
    let mut messages = BufReader::new(tokio::io::stdin()).lines();
    let out = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut asks = JoinSet::new();
    while let Some(message) = messages.next_line().await? {
        let (client, out) = (client.clone(), out.clone());
        asks.spawn(async move {
            let mut asked: Value = serde_json::from_str(&message).unwrap_or_default();
            if code && asked["method"].is_string() {
                asked["params"]["_meta"]["mode"] = json!("code");
            }
            let message = if asked.is_object() { asked.to_string() } else { message };
            let mut answer = if asked["method"] == "tools/call" && asked["params"]["name"] == "execute" {
                execute(&client, &asked).await
            } else {
                match client.ask(message.as_bytes()).await {
                    Ok(answer) => answer,
                    Err(error) => refusal(&message, &error),
                }
            };
            if !answer.is_empty() {
                answer.push(b'\n');
                let mut out = out.lock().await;
                out.write_all(&answer).await?;
                out.flush().await?;
            }
            anyhow::Ok(())
        });
    }
    while let Some(asked) = asks.join_next().await {
        asked??;
    }
    Ok(())
}

/// The answer to a call of `execute`: its script, run here. It says it's a complete result, as MCP has every result say
/// from 2026-07-28, which a client of an earlier version ignores.
async fn execute(client: &Arc<Client>, asked: &Value) -> Vec<u8> {
    let code = asked["params"]["arguments"]["code"].as_str().unwrap_or_default();
    let (ok, text) = crate::codemode::run(client, code).await.unwrap_or_else(|error| (false, format!("{error:#}")));
    let result = json!({"resultType": "complete", "content": [{"type": "text", "text": text}], "isError": !ok});
    json!({"jsonrpc": "2.0", "id": asked["id"], "result": result}).to_string().into_bytes()
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
