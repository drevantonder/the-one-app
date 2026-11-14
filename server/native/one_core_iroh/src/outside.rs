//! A server's connection to its network's address for outsiders' calls. What an assistant somewhere else asks at
//! the address over the web is passed down this connection by the relay there (`address/relay/relay.js`), and the
//! server's answer goes back up it.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use client::Network;
use futures_util::{SinkExt, StreamExt};
use iroh::SecretKey;
use rustler::LocalPid;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// The internet between a server and its network's address comes and goes, and what sits between them forgets a
/// connection that says nothing. So the server says something this often, and connects again after this long.
const EVERY: Duration = Duration::from_secs(30);

/// Keeps the connection open for as long as the server runs, handing each call to `asked` as a client's is.
pub async fn hear(network: Network, key: SecretKey, asked: LocalPid) {
    loop {
        // Why it ended doesn't matter: the next one is the same either way.
        let _ = heard(&network, &key, &asked).await;
        sleep(EVERY).await;
    }
}

async fn heard(network: &Network, key: &SecretKey, asked: &LocalPid) -> Result<()> {
    let address = network.address.trim_end_matches('/');
    let socket = format!("ws{}/relay/outside", address.strip_prefix("http").context("an address is http or https")?);
    let mut request = socket.into_client_request()?;
    for (name, value) in network.signed(key, "/relay/outside", "")? {
        request.headers_mut().insert(name, value.parse()?);
    }

    let (socket, _) = connect_async(request).await?;
    let (mut up, mut down) = socket.split();
    let (answers, mut answered) = mpsc::unbounded_channel();
    let mut say = interval(EVERY);
    // The address answers each thing said, so one that has said nothing for this long is gone.
    let quiet = sleep(EVERY * 2);
    tokio::pin!(quiet);
    loop {
        tokio::select! {
            message = down.next() => match message {
                Some(Ok(Message::Text(call))) => {
                    let (asked, answers) = (*asked, answers.clone());
                    tokio::spawn(async move {
                        let Ok(call) = serde_json::from_str::<Value>(call.as_str()) else { return };
                        let Some(message) = call["message"].as_str() else { return };
                        if let Some(answer) = crate::ask(&asked, None, message.as_bytes()).await {
                            let answer = json!({"call": call["call"], "answer": String::from_utf8_lossy(&answer)});
                            let _ = answers.send(answer.to_string());
                        }
                    });
                }
                // Its answer to what this end said.
                Some(Ok(_)) => quiet.as_mut().reset(Instant::now() + EVERY * 2),
                Some(Err(error)) => bail!(error),
                None => bail!("the address closed it"),
            },
            Some(answer) = answered.recv() => up.send(Message::text(answer)).await?,
            _ = say.tick() => up.send(Message::Ping(Default::default())).await?,
            _ = &mut quiet => bail!("the address went quiet"),
        }
    }
}
