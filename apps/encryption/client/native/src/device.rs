//! What only a device does, which the crate's doc says. A server's NIF is built without it.

use anyhow::{Context, Result, anyhow, bail};
use client::{Client, Input, InputResolver};
use data_encoding::BASE64;
use futures_util::future::BoxFuture;
use iroh_base::PublicKey;
use serde_json::{Map, Value, json};

use crate::cipher::{self, Key};

/// The encryption app's tool that says what a device holds of its owner's key.
const HOLDS: &str = "encryption_key";

/// What this fills in of a tool's inputs, which a client hands every call it makes (`client::InputResolver`).
pub struct Encrypts;

impl InputResolver for Encrypts {
    fn formats(&self) -> &'static [&'static str] {
        &["secret", "owner-key", "artifact-keys", "passkey-keys"]
    }

    fn resolve<'a>(&'a self, client: &'a Client, input: Input<'a>) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let Input { app, format, says, typed, inputs, subject } = input;
            let filled = match (format, typed) {
                ("secret", Some(text)) => secret(client, app, says, text, subject).await?,
                ("owner-key", typed) => owner_key_for(client, inputs, typed, subject).await?,
                ("artifact-keys", None) => artifact_keys(client, app, says, inputs, subject).await?,
                ("passkey-keys", None) => passkey_keys(client, app, says, subject).await?,
                _ => return Ok(None),
            };
            Ok(Some(filled))
        })
    }
}

/// Calls a tool for `subject`, if there is one, and gives what it answered.
async fn tool_for(client: &Client, name: &str, inputs: Value, subject: Option<&str>) -> Result<Value> {
    let mut params = json!({"name": name, "arguments": inputs});
    if let Some(subject) = subject {
        params["_meta"]["subject"] = subject.into();
    }
    let response = Box::pin(client.request("tools/call", params)).await?;
    let text = response["content"][0]["text"].as_str().unwrap_or_default();
    if response["isError"] == true {
        bail!("{text}")
    }
    Ok(serde_json::from_str(text).unwrap_or_else(|_| text.into()))
}

/// What the app's tool that an input names says of the artifact it's kept in: its name, its epoch, and the
/// servers that open it.
async fn unlocked_on(client: &Client, app: &str, says: &Value, subject: Option<&str>) -> Result<Value> {
    let tool = says["unlocked_on"].as_str().context("the input doesn't say which tool lists the servers")?;
    tool_for(client, &format!("{app}_{tool}"), json!({}), subject).await
}

/// `text` encrypted to each server that opens what it's kept in, by the server's key.
async fn secret(client: &Client, app: &str, says: &Value, text: &str, subject: Option<&str>) -> Result<String> {
    let unlocked_on = unlocked_on(client, app, says, subject).await?;
    let servers = unlocked_on["servers"].as_array().map(Vec::as_slice).unwrap_or_default();
    if servers.is_empty() {
        bail!("No server opens what this is kept in yet. Choose one first, from a trusted device.")
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

/// This device's copy of the key of `subject`, or of whoever it asks for, with who that is: a person, a group or the
/// network. It's kept encrypted to this device, and opened here each time it's needed.
async fn owner_key_of(client: &Client, subject: Option<&str>) -> Result<(Key, String)> {
    let holds = tool_for(client, HOLDS, json!({}), subject).await?;
    let (owner, me) =
        (holds["owner"].as_str().unwrap_or_default().to_owned(), client.device_key().public().to_string());
    let Some(slot) = holds["slot"].as_str() else {
        bail!("This device isn't trusted yet. Approve it from a trusted device, or with your recovery key.")
    };
    let slot = BASE64.decode(slot.as_bytes()).context("that isn't a key for this device")?;
    let key = cipher::open(client.device_key(), &cipher::for_device(&owner, &me), &slot)?;
    Ok((cipher::key(&key)?, owner))
}

/// The key of `subject`, or of whoever this client asks for, encrypted to the device the input `device` names: the
/// one typed as a recovery key, this device's copy, or for someone who has none yet, a new one, for this device
/// itself.
async fn owner_key_for(
    client: &Client,
    inputs: &Map<String, Value>,
    typed: Option<&str>,
    subject: Option<&str>,
) -> Result<String> {
    let holds = tool_for(client, HOLDS, json!({}), subject).await?;
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
        (None, Some(_)) => owner_key_of(client, subject).await?.0,
        (None, None) if device == me => cipher::owner(),
        (None, None) => bail!("You have no key yet. It's made on the first device you set up, for itself."),
    };
    let to: PublicKey = device.parse().context("that isn't a device's key")?;
    let slot = cipher::seal(&to, &cipher::for_device(owner, device), key.as_slice())?;
    Ok(json!({"check": cipher::check(&key), "slot": BASE64.encode(&slot)}).to_string())
}

/// The keys of the artifact an app's tool names, encrypted to each server in the input `servers`. Taking a
/// server off starts a new epoch, whose key it was never given.
async fn artifact_keys(
    client: &Client,
    app: &str,
    says: &Value,
    inputs: &Map<String, Value>,
    subject: Option<&str>,
) -> Result<String> {
    let unlocked_on = unlocked_on(client, app, says, subject).await?;
    let artifact = unlocked_on["artifact"].as_str().context("the tool didn't name the artifact")?;
    let was = unlocked_on["epoch"].as_u64().unwrap_or_default() as u32;
    let keys = |value: &Value| -> Vec<String> {
        value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
    };
    let (had, servers) = (keys(&unlocked_on["servers"]), keys(inputs.get("servers").unwrap_or(&Value::Null)));
    let epoch = if was == 0 || had.iter().any(|server| !servers.contains(server)) { was + 1 } else { was };
    let (owner, _) = owner_key_of(client, subject).await?;
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

/// The owner's key, encrypted to each of their passkeys that the app's tool an input names (`passkeys`) lists with a
/// `prf_key` and doesn't say `unlocks` yet, by its credential ID, with the key it was encrypted to.
async fn passkey_keys(client: &Client, app: &str, says: &Value, subject: Option<&str>) -> Result<String> {
    let tool = says["passkeys"].as_str().context("the input doesn't say which tool lists the passkeys")?;
    let listed = tool_for(client, &format!("{app}_{tool}"), json!({}), subject).await?;
    let needed: Vec<(&str, &str, Vec<u8>)> = listed["passkeys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|passkey| passkey["unlocks"] != true)
        .filter_map(|passkey| {
            let (credential, prf_key) = (passkey["credential"].as_str()?, passkey["prf_key"].as_str()?);
            let to = data_encoding::HEXLOWER.decode(prf_key.as_bytes()).ok().filter(|to| to.len() == 32)?;
            Some((credential, prf_key, to))
        })
        .collect();
    let mut keys = Map::new();
    if needed.is_empty() {
        return Ok(Value::Object(keys).to_string());
    }
    let (key, owner) = owner_key_of(client, subject).await?;
    for (credential, prf_key, to) in needed {
        let sealed = to_passkey(&to, &format!("one/encryption/1/passkey\n{owner}\n{credential}"), key.as_slice())?;
        keys.insert(credential.to_owned(), json!({"prf_key": prf_key, "key": BASE64.encode(&sealed)}));
    }
    Ok(Value::Object(keys).to_string())
}

/// Encrypts `data` to a passkey's key, for what `about` says, as `cipher::seal` does to a machine's: HPKE (RFC 9180),
/// to the X25519 key that the sign-in page derives from the passkey's WebAuthn PRF output with HPKE's `DeriveKeyPair`,
/// and opens it with (`apps/sign-in/edge/page/passkey.ts`). A passkey is no machine, so it's here, not in `cipher`.
fn to_passkey(to: &[u8], about: &str, data: &[u8]) -> Result<Vec<u8>> {
    use hpke::{
        Deserializable, Kem, OpModeS, Serializable, aead::ChaCha20Poly1305, kdf::HkdfSha256, kem::X25519HkdfSha256,
    };
    let to = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(to).map_err(|_| anyhow!("that isn't a passkey's key"))?;
    let (key, sealed) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeS::Base,
        &to,
        about.as_bytes(),
        data,
        b"",
    )
    .map_err(|_| anyhow!("couldn't encrypt to that passkey's key"))?;
    Ok([key.to_bytes().as_slice(), &sealed].concat())
}

/// The recovery key of `subject`, or of whoever this client asks for: a person, or a group or the network they own.
/// It's their key as a person writes it down. Only a device that holds the key gives it, and only its window
/// asks: no command or tool answers with one.
pub async fn recovery_key(client: &Client, subject: Option<&str>) -> Result<String> {
    Ok(cipher::recovery(&owner_key_of(client, subject).await?.0))
}

/// Lets this device hold the key of `subject`, or of whoever this client asks for, with the recovery key its person
/// typed in its window.
pub async fn recover(client: &Client, recovery_key: &str, subject: Option<&str>) -> Result<()> {
    tool_for(client, "encryption_approve", json!({"key": recovery_key}), subject).await.map(drop)
}
