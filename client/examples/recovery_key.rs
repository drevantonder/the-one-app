//! Stands in for the app's window in a proof (`core/test/vault.sh`): the window is the only thing that shows a
//! person their recovery key, and a proof has none. It writes the recovery key of whoever the client in `ONE_HOME`
//! asks for (`ONE_FOR`, or `ONE_AS`) to the file it's given, which the proof then hands to another device as that person would type it.
//!
//!     ONE_HOME=<home> ONE_AS=<user> cargo run --release -p client --example recovery_key <file>

use anyhow::Context;
use client::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file = std::env::args().nth(1).context("recovery_key <file>")?;
    let client = Client::open(client::home()?).await?;
    std::fs::write(file, client.recovery_key(None).await?)?;
    client.close().await;
    Ok(())
}
