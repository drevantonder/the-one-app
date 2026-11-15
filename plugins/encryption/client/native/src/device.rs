//! What only a device does, which the crate's doc says. A server's NIF is built without it.

use anyhow::{Context, Result, bail};
use client::{Client, InputResolver, Input};
use data_encoding::BASE64;
use futures_util::future::BoxFuture;
use iroh_base::PublicKey;
use serde_json::{Map, Value, json};

use crate::cipher::{self, Key};

/// The encryption plugin's tool that says what a device holds of its owner's key.
const HOLDS: &str = "encryption_key";

/// What this fills in of a tool's inputs, which a client hands every call it makes (`client::InputResolver`).
pub struct Encrypts;

impl InputResolver for Encrypts {
    fn formats(&self) -> &'static [&'static str] {
        &["secret", "owner-key", "artifact-keys", "backup-recipient", "backup-key"]
    }

    fn resolve<'a>(&'a self, client: &'a Client, input: Input<'a>) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let Input { plugin, format, says, typed, inputs, who } = input;
            let filled = match (format, typed) {
                ("secret", Some(text)) => secret(client, plugin, says, text, who).await?,
                ("owner-key", typed) => owner_key_for(client, inputs, typed, who).await?,
                ("artifact-keys", None) => artifact_keys(client, plugin, says, inputs, who).await?,
                ("backup-recipient", None) => backups_key(client, plugin, says, who).await?.0.public().to_string(),
                ("backup-key", None) => backup_key(client, plugin, says, who).await?,
                _ => return Ok(None),
            };
            Ok(Some(filled))
        })
    }
}

/// Calls a tool for someone, if it says who, and gives what it answered.
async fn tool_for(client: &Client, name: &str, inputs: Value, who: Option<&str>) -> Result<Value> {
    let mut params = json!({"name": name, "arguments": inputs});
    if let Some(who) = who {
        params["_meta"]["for"] = who.into();
    }
    let answer = Box::pin(client.request("tools/call", params)).await?;
    let said = answer["content"][0]["text"].as_str().unwrap_or_default();
    if answer["isError"] == true {
        bail!("{said}")
    }
    Ok(serde_json::from_str(said).unwrap_or_else(|_| said.into()))
}

/// What the plugin's tool that an input names says of the artifact it's kept in: its name, its epoch, and the
/// servers that open it.
async fn opened_by(client: &Client, plugin: &str, says: &Value, who: Option<&str>) -> Result<Value> {
    let tool = says["opened_by"].as_str().context("the input doesn't say which tool lists the servers")?;
    tool_for(client, &format!("{plugin}_{tool}"), json!({}), who).await
}

/// `text` encrypted to each server that opens what it's kept in, by the server's key.
async fn secret(client: &Client, plugin: &str, says: &Value, text: &str, who: Option<&str>) -> Result<String> {
    let opened_by = opened_by(client, plugin, says, who).await?;
    let servers = opened_by["servers"].as_array().map(Vec::as_slice).unwrap_or_default();
    if servers.is_empty() {
        bail!("No server opens what this is kept in yet. Choose one first, from a device that holds your key.")
    }
    let mut sealed = Map::new();
    for server in servers.iter().filter_map(Value::as_str) {
        let to: PublicKey = server.parse().context("that isn't a server's key")?;
        sealed.insert(
            server.to_owned(),
            BASE64.encode(&cipher::seal(&to, &cipher::for_secret(server), text.as_bytes())?).into(),
        );
    }
    Ok(Value::Object(sealed).to_string())
}

/// This device's copy of the key of `who`, or of whoever it asks for, with who that is: a person, a group or the
/// network. It's kept encrypted to this device, and opened here each time it's needed.
async fn owner_key_of(client: &Client, who: Option<&str>) -> Result<(Key, String)> {
    let holds = tool_for(client, HOLDS, json!({}), who).await?;
    let (owner, me) =
        (holds["owner"].as_str().unwrap_or_default().to_owned(), client.device_key().public().to_string());
    let Some(slot) = holds["slot"].as_str() else {
        bail!("This device doesn't hold your key. Let it in from a device that does, or with your recovery key.")
    };
    let slot = BASE64.decode(slot.as_bytes()).context("that isn't a key for this device")?;
    let key = cipher::open(client.device_key(), &cipher::for_device(&owner, &me), &slot)?;
    Ok((cipher::key(&key)?, owner))
}

/// The key of `who`, or of whoever this client asks for, encrypted to the device the input `device` names: the
/// one typed as a recovery key, this device's copy, or for someone who has none yet, a new one, for this device
/// itself.
async fn owner_key_for(
    client: &Client,
    inputs: &Map<String, Value>,
    typed: Option<&str>,
    who: Option<&str>,
) -> Result<String> {
    let holds = tool_for(client, HOLDS, json!({}), who).await?;
    let (owner, me) = (holds["owner"].as_str().unwrap_or_default(), client.device_key().public().to_string());
    let device = inputs.get("device").and_then(Value::as_str).unwrap_or(&me);
    let key = match (typed, holds["check"].as_str()) {
        (Some(typed), Some(check)) => {
            let key = cipher::recovered(typed)?;
            if cipher::check(&key) != check {
                bail!("That isn't your recovery key. Check it against what you wrote down.")
            }
            key
        }
        (Some(_), None) => bail!("There's no key to recover: nothing of yours is encrypted yet."),
        (None, Some(_)) => owner_key_of(client, who).await?.0,
        (None, None) if device == me => cipher::owner(),
        (None, None) => bail!("You have no key yet. It's made on the first device you set up, for itself."),
    };
    let to: PublicKey = device.parse().context("that isn't a device's key")?;
    let slot = cipher::seal(&to, &cipher::for_device(owner, device), key.as_slice())?;
    Ok(json!({"check": cipher::check(&key), "slot": BASE64.encode(&slot)}).to_string())
}

/// The keys of the artifact a plugin's tool names, encrypted to each server in the input `servers`. Taking a
/// server off starts a new epoch, whose key it was never given.
async fn artifact_keys(
    client: &Client,
    plugin: &str,
    says: &Value,
    inputs: &Map<String, Value>,
    who: Option<&str>,
) -> Result<String> {
    let opened_by = opened_by(client, plugin, says, who).await?;
    let artifact = opened_by["artifact"].as_str().context("the tool didn't name the artifact")?;
    let was = opened_by["epoch"].as_u64().unwrap_or_default() as u32;
    let keys = |value: &Value| -> Vec<String> {
        value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
    };
    let (had, servers) = (keys(&opened_by["servers"]), keys(inputs.get("servers").unwrap_or(&Value::Null)));
    let epoch = if was == 0 || had.iter().any(|server| !servers.contains(server)) { was + 1 } else { was };
    let (owner, _) = owner_key_of(client, who).await?;
    let joined = cipher::joined(&cipher::artifact(&owner, artifact, epoch));
    let mut slots = Map::new();
    for server in &servers {
        let to: PublicKey = server.parse().context("that isn't a server's key")?;
        slots.insert(
            server.clone(),
            BASE64.encode(&cipher::seal(&to, &cipher::for_server(artifact, server), &joined)?).into(),
        );
    }
    Ok(json!({"artifact": artifact, "epoch": epoch, "was": was, "slots": slots}).to_string())
}

/// The backup key of whoever the plugin's tool that the input names says owns the backups, with what the tool
/// said.
async fn backups_key(
    client: &Client,
    plugin: &str,
    says: &Value,
    who: Option<&str>,
) -> Result<(iroh_base::SecretKey, Value)> {
    let opened_by = opened_by(client, plugin, says, who).await?;
    let owner = opened_by["owner"].as_str().context("the tool didn't say whose backup key it is")?;
    Ok((cipher::backups(&owner_key_of(client, Some(owner)).await?.0), opened_by))
}

/// The backup key, encrypted to each server the plugin's tool lists, for one of them to restore with.
async fn backup_key(client: &Client, plugin: &str, says: &Value, who: Option<&str>) -> Result<String> {
    let (key, opened_by) = backups_key(client, plugin, says, who).await?;
    let mut sealed = Map::new();
    for server in opened_by["servers"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let to: PublicKey = server.parse().context("that isn't a server's key")?;
        let key = zeroize::Zeroizing::new(key.to_bytes());
        sealed.insert(
            server.to_owned(),
            BASE64.encode(&cipher::seal(&to, &cipher::for_restoring(server), key.as_slice())?).into(),
        );
    }
    Ok(Value::Object(sealed).to_string())
}

/// The recovery key of `who`, or of whoever this client asks for: a person, or a group or the network they own.
/// It's their key as a person writes it down. Only a device that holds the key gives it, and only its window
/// asks: no command or tool answers with one.
pub async fn recovery_key(client: &Client, who: Option<&str>) -> Result<String> {
    Ok(cipher::recovery(&owner_key_of(client, who).await?.0))
}

/// Lets this device hold the key of `who`, or of whoever this client asks for, with the recovery key its person
/// typed in its window.
pub async fn recover(client: &Client, recovery_key: &str, who: Option<&str>) -> Result<()> {
    tool_for(client, "encryption_let_in", json!({"key": recovery_key}), who).await.map(drop)
}
