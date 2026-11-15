//! Stands in for the app's window in a test (`plugins/vault/test/vault.sh`): the window is the only thing that shows
//! a person their recovery key, and a test has none. It writes the recovery key of whoever the client in `ONE_HOME`
//! asks for (`ONE_FOR`, or `ONE_AS`) to the file it's given, which the test then hands to another device as that person would type it.
//!
//!     ONE_HOME=<home> ONE_AS=<user> cargo run --release -p encryption_client --example recovery_key <file>

use std::sync::Arc;

use anyhow::Context;
use client::Client;
use encryption_client::Encrypts;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file = std::env::args().nth(1).context("recovery_key <file>")?;
    let client = Client::open(client::home()?, vec![Arc::new(Encrypts)]).await?;
    std::fs::write(file, encryption_client::recovery_key(&client, None).await?)?;
    client.close().await;
    Ok(())
}
