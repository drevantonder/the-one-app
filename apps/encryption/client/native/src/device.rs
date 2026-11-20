//! What only a device does, which the crate's doc says. A server's NIF is built without it.

use anyhow::{Context, Result, anyhow, bail};
use client::{Client, Input, InputResolver};
use data_encoding::BASE64;
use futures_util::future::BoxFuture;
use iroh_base::{PublicKey, Signature};
use serde_json::{Map, Value, json};

use crate::cipher::{self, Key};

/// The encryption app's tool that says what a device holds of its owner's key.
const HOLDS: &str = "encryption_key";

/// What this fills in of a tool's inputs, which a client hands every call it makes (`client::InputResolver`).
pub struct Encrypts;

impl InputResolver for Encrypts {
    fn formats(&self) -> &'static [&'static str] {
        &["secret", "passkey-keys"]
    }

    fn resolve<'a>(&'a self, client: &'a Client, input: Input<'a>) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            let Input { app, format, says, typed, inputs, subject } = input;
            let filled = match (format, typed) {
                ("secret", Some(text)) => secret(client, app, says, text, subject).await?,
                ("passkey-keys", None) => passkey_keys(client, app, says, inputs, subject).await?,
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

/// The key an owner signs which servers open each of their artifacts with: Ed25519, from their key (HKDF, RFC 5869),
/// as the sign-in page derives it to sign as their passkey hands the keys over (`apps/sign-in/edge/page/passkey.ts`),
/// so a device that holds the key checks with it, and a server, which never holds it, can't say its owner chose it.
fn signer(owner: &Key) -> iroh_base::SecretKey {
    let mut seed = zeroize::Zeroizing::new([0; 32]);
    hkdf::Hkdf::<sha2::Sha256>::new(None, owner.as_slice())
        .expand(b"one/encryption/1/signer", seed.as_mut_slice())
        .expect("32 bytes fit");
    iroh_base::SecretKey::from_bytes(&seed)
}

/// What an owner signs of an artifact as their passkey hands its keys to servers: those servers at that epoch.
fn signed_list(artifact: &str, epoch: u32, servers: &[String]) -> Vec<u8> {
    let mut servers = servers.to_vec();
    servers.sort();
    format!("one/encryption/1/servers\n{artifact}\n{epoch}\n{}", servers.join("\n")).into_bytes()
}

/// The keys a JSON list names.
fn keys(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// What this device has seen of who signs which servers open what (`chosen`), kept in its home: each owner's signer,
/// by its public key, and each artifact it has seen a signed list of. So a server can't later say an owner signs with
/// another key, or that an artifact's list was never signed: trust on first use, as SSH's known hosts and Signal's
/// safety numbers are.
fn seen_file() -> Result<std::path::PathBuf> {
    Ok(client::home()?.join("encryption-seen.json"))
}

fn seen() -> Value {
    seen_file().ok().and_then(|file| std::fs::read(file).ok()).and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_else(|| json!({}))
}

fn keep_seen(seen: &Value) -> Result<()> {
    let file = seen_file()?;
    let writing = file.with_extension("json.writing");
    std::fs::write(&writing, seen.to_string())?;
    std::fs::rename(writing, file).context("couldn't keep what this device saw of who signs what")
}

/// Who an artifact belongs to, by its name: `users/<name>`, `groups/<name>` or `network`.
fn owner_of(artifact: &str) -> String {
    let parts: Vec<&str> = artifact.split('/').collect();
    match parts.as_slice() {
        [kind @ ("users" | "groups"), name, ..] => format!("{kind}/{name}"),
        _ => "network".to_owned(),
    }
}

/// Whether the servers an app's tool says open an artifact (`unlocked_on`) are the ones its owner chose: signed with
/// the owner's signer, which this device derives where it holds their key, and otherwise is the one it saw first.
/// `Ok(true)` for a list its owner signed, `Ok(false)` for an unsigned one of an artifact this device never saw signed
/// (one chosen before lists were signed), and an error for any other, which nothing is sent to.
async fn chosen(client: &Client, unlocked_on: &Value, subject: Option<&str>) -> Result<bool> {
    let artifact = unlocked_on["artifact"].as_str().context("the tool didn't name the artifact")?;
    let epoch = unlocked_on["epoch"].as_u64().unwrap_or_default() as u32;
    let mut seen = seen();
    let signed_before = seen["signed"].as_array().is_some_and(|signed| signed.iter().any(|a| a == artifact));
    let Some(signature) = unlocked_on["signature"].as_str() else {
        if signed_before {
            bail!("Which servers open {artifact} was signed by its owner before, and isn't now, so nothing was sent. Choose them again from a trusted device.")
        }
        return Ok(false);
    };
    let owner = owner_of(artifact);
    let signer = match owner_key_of(client, subject).await {
        Ok((key, _)) => signer(&key).public(),
        Err(_) => {
            let said: PublicKey = unlocked_on["signer"].as_str().unwrap_or_default().parse().context("the tool didn't say who signed it")?;
            match seen["signers"][&owner].as_str() {
                Some(first) if first != said.to_string() => bail!("{owner}'s key that signs which servers open what isn't the one this device saw before, so nothing was sent. Check with them, from a trusted device."),
                _ => seen["signers"][&owner] = said.to_string().into(),
            }
            said
        }
    };
    let signature = data_encoding::HEXLOWER.decode(signature.as_bytes()).ok().and_then(|bytes| Signature::try_from(bytes.as_slice()).ok());
    let servers = keys(&unlocked_on["servers"]);
    if signature.is_none_or(|signature| signer.verify(&signed_list(artifact, epoch, &servers), &signature).is_err()) {
        bail!("The servers said to open {artifact} aren't the ones its owner chose, so nothing was sent.")
    }
    if !signed_before {
        let mut signed = seen["signed"].as_array().cloned().unwrap_or_default();
        signed.push(artifact.into());
        seen["signed"] = signed.into();
    }
    keep_seen(&seen)?;
    Ok(true)
}

/// `text` encrypted to each server that opens what it's kept in, by the server's key, as its owner chose them.
async fn secret(client: &Client, app: &str, says: &Value, text: &str, subject: Option<&str>) -> Result<String> {
    let unlocked_on = unlocked_on(client, app, says, subject).await?;
    chosen(client, &unlocked_on, subject).await?;
    let servers = keys(&unlocked_on["servers"]);
    if servers.is_empty() {
        bail!("No server opens what this is kept in yet. Choose one first, from a trusted device.")
    }
    let mut sealed = Map::new();
    for server in &servers {
        let to: PublicKey = server.parse().context("that isn't a server's key")?;
        sealed.insert(server.clone(), BASE64.encode(&cipher::seal(&to, &cipher::for_secret(server), text.as_bytes())?).into());
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

/// The owner's key, encrypted to the one passkey the input `passkey` names, which its person allowed, as the app's
/// tool an input names (`passkeys`) lists it with a `prf_key`: by its credential ID, with the key it was encrypted to.
/// Whoever uses a passkey it's encrypted to has the key, so it's never encrypted to one nobody named.
async fn passkey_keys(
    client: &Client,
    app: &str,
    says: &Value,
    inputs: &Map<String, Value>,
    subject: Option<&str>,
) -> Result<String> {
    let named = inputs.get("passkey").and_then(Value::as_str).context("say which passkey your person allowed")?;
    let tool = says["passkeys"].as_str().context("the input doesn't say which tool lists the passkeys")?;
    let listed = tool_for(client, &format!("{app}_{tool}"), json!({}), subject).await?;
    let passkey = listed["passkeys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|passkey| passkey["id"] == named)
        .context("that isn't one of your passkeys")?;
    let (Some(credential), Some(prf_key)) = (passkey["credential"].as_str(), passkey["prf_key"].as_str()) else {
        bail!("That passkey hasn't said yet whether it can unlock your key. Use it once to sign in.")
    };
    // Its person checked the code the sign-in page showed them is this key's, so nobody else's is put in its place.
    if inputs.get("code").and_then(Value::as_str) != Some(code(prf_key).as_str()) {
        bail!("That isn't the code of the passkey your person checked, so your key wasn't encrypted to it. Set it up again, and check its code.")
    }
    let to = data_encoding::HEXLOWER
        .decode(prf_key.as_bytes())
        .ok()
        .filter(|to| to.len() == 32)
        .context("That passkey can't unlock your key on a new device.")?;
    let (key, owner) = owner_key_of(client, subject).await?;
    let sealed = to_passkey(&to, &format!("one/encryption/1/passkey\n{owner}\n{credential}"), key.as_slice())?;
    Ok(json!({credential: {"prf_key": prf_key, "key": BASE64.encode(&sealed)}}).to_string())
}

/// A passkey's key, as its person checks it's the one they used, as Signal's safety numbers are: 20 digits in fours
/// of five from its SHA-256, the same the sign-in page shows (`apps/sign-in/edge/page/passkey.ts`).
fn code(key: &str) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(format!("one/passkey/1/code\n{key}").as_bytes());
    let number = hash[..9].iter().fold(0u128, |number, &byte| (number << 8) | u128::from(byte)) % 10u128.pow(20);
    let digits = format!("{number:020}");
    digits.as_bytes().chunks(5).map(|chunk| std::str::from_utf8(chunk).expect("digits")).collect::<Vec<_>>().join(" ")
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
/// typed in its window: it's encrypted to this device here, as a passkey's is on the sign-in page, and kept with
/// `store`, which keeps it only if it's that person's key.
pub async fn recover(client: &Client, recovery_key: &str, subject: Option<&str>) -> Result<()> {
    let holds = tool_for(client, HOLDS, json!({}), subject).await?;
    let (owner, me) = (holds["owner"].as_str().unwrap_or_default(), client.device_key().public());
    let Some(check) = holds["check"].as_str() else { bail!("There's no key to recover: nothing of yours is encrypted yet.") };
    let key = cipher::recovered(recovery_key)?;
    if cipher::check(&key) != check {
        bail!("That isn't your recovery key. Check it against what you wrote down.")
    }
    let slot = cipher::seal(&me, &cipher::for_device(owner, &me.to_string()), key.as_slice())?;
    let device = json!({"key": me.to_string(), "name": client::name(), "check": check, "slot": BASE64.encode(&slot)});
    tool_for(client, "encryption_store", json!({"device": device}), subject).await.map(drop)
}
