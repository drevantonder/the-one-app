//! Joining a network with a pairing code, as a computer does (its onet, or an app runtime joining by itself): asking
//! the device that made the code, over the relay at the network's address the code names, to let it in.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use api::home::{NetworkFile, bucket_root};
use api::net::{CONNECT, Endpoints, MESSAGE_LIMIT, alpn, relay};
use bucket::Place;
use custom_build::protocol::message as M;
use custom_build::protocol::{Coded, code};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, SecretKey};
use key::{Key, SigningKey};
use serde_json::{Value, json};

use crate::{JoinReply, JoinRequest, PairingCode};

/// A computer that joined with a pairing code: the network's address, and the network as its home keeps it.
pub struct Joined {
    pub address: String,
    pub network: NetworkFile,
}

/// Asks the device that made a pairing code, `code`, to let in the computer whose key is `key`, called `name`, over the
/// relay at the network's address the code names, showing it the code's ticket: → the network, once it's let in.
pub async fn join_with(key: &SigningKey, name: &str, code: &str) -> Result<Joined> {
    let code = PairingCode::parse(code)?;
    let request = JoinRequest { secret: code.secret, name: name.to_string(), as_: "device".into() };
    let reply = ask_to_join(key, &code.address, &pairing_ticket(&code.secret), code.from, serde_json::to_value(&request)?).await.map_err(|err| Coded::error(code::UNREACHABLE, format!("couldn't reach the device that made the pairing code: {err:#}")))?;
    if let Some(err) = reply.get("error") {
        return Err(Coded::or_plain(reply["code"].as_str().map(str::to_string), err.as_str().unwrap_or("the pairing code was refused")));
    }
    let reply: JoinReply = serde_json::from_value(reply)?;
    // A folder on another machine isn't here: it's this network's bucket only if it says so. S3 is reached through
    // the URL signer, which this device can ask once it knows the network's address.
    let here = |f: &BTreeMap<String, String>| Place::from_fields(f).ok().and_then(|p| bucket_root(&p).ok().flatten()).is_some_and(|r| r == reply.root.as_bytes());
    let mut bucket = reply.bucket.filter(|f| f.contains_key("endpoint") || here(f));
    // Its address is in the log, read from the bucket through it: until then, it's the one the code names.
    if let Some(f) = bucket.as_mut().filter(|f| f.contains_key("endpoint")) {
        f.insert("address".into(), code.address.clone());
    }
    if bucket.is_none() {
        tracing::warn!("this device can't reach the network's bucket: it keeps no app's data until the network moves to S3-compatible storage");
    }
    Ok(Joined { address: code.address, network: NetworkFile { root: reply.root, user: reply.user, bucket } })
}

/// Asks the device `to`, which made a pairing code, to let in the computer whose key is `key` (`join`), through the relay
/// at `address`, which lets it in for the code's `ticket`: → its answer. The device that made it may not have told the
/// relay of it yet: it's tried for a few seconds.
pub async fn ask_to_join(key: &SigningKey, address: &str, ticket: &str, to: Key, join: Value) -> Result<Value> {
    let (url, map) = relay(address, Some(ticket))?;
    let endpoint = Endpoint::builder(Endpoints).secret_key(SecretKey::from_bytes(key.as_bytes())).relay_mode(RelayMode::Custom(map)).bind().await?;
    let addr = EndpointAddr::new(PublicKey::from_bytes(&to)?).with_relay_url(url);
    let mut tried = 0;
    let conn = loop {
        match tokio::time::timeout(CONNECT, endpoint.connect(addr.clone(), &alpn())).await {
            Ok(Ok(conn)) => break conn,
            _ if tried < 3 => tried += 1,
            Ok(Err(err)) => bail!("the device that made the pairing code couldn't be reached: {err:#}"),
            Err(_) => bail!("the device that made the pairing code isn't on"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let answer = async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(json!({ "kind": M::ASK, "body": { "join": join } }).to_string().as_bytes()).await?;
        send.finish()?;
        let answer = tokio::time::timeout(Duration::from_secs(30), recv.read_to_end(MESSAGE_LIMIT)).await.context("the device that made the pairing code didn't answer")??;
        anyhow::Ok(serde_json::from_slice(&answer)?)
    }
    .await;
    conn.close(0u32.into(), b"joined");
    endpoint.close().await;
    answer
}

/// What a computer joining shows the relay for a pairing code's `secret`: not the secret, which admits it.
pub fn pairing_ticket(secret: &[u8; 16]) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::new().chain_update(b"relay pairing:").chain_update(secret).finalize())
}

/// What a device that made a pairing code tells the relay of it: the hash of its ticket.
pub fn ticket_hash(ticket: &str) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(ticket.as_bytes()))
}
