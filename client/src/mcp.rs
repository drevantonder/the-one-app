//! MCP over stdio. Servers speak MCP themselves, so each message read is passed to the server and its answer
//! written back.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::Client;

/// Serves MCP until stdin closes and every answer is written.
pub async fn serve(client: Arc<Client>) -> Result<()> {
    let mut messages = BufReader::new(tokio::io::stdin()).lines();
    let out = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut asks = JoinSet::new();
    while let Some(message) = messages.next_line().await? {
        let (client, out) = (client.clone(), out.clone());
        asks.spawn(async move {
            let mut answer = match client.ask(message.as_bytes()).await {
                Ok(answer) => answer,
                Err(error) => refusal(&message, &error),
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

/// What a request that reached no server is answered with. A notification is answered with nothing.
fn refusal(message: &str, error: &anyhow::Error) -> Vec<u8> {
    match serde_json::from_str::<Value>(message).ok().and_then(|message| message.get("id").cloned()) {
        Some(id) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("{error:#}")}})
            .to_string()
            .into_bytes(),
        None => Vec::new(),
    }
}
