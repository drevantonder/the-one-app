//! A service reached through its device's app runtime, which carries what it's asked unread. A node asking it signs
//! the ask with its own key and seals it to the service's seal key; the runtime hands those bytes to the service's part
//! over the loopback it long-polls (`Parts`), and the part checks the signature, that it's fresh and taken once, and
//! that the signer may call it, exactly as it checked the QUIC peer when it was an endpoint of its own; its answer is
//! signed by it and sealed to the asker. So a hacked runtime can drop or delay asks, and can't forge, replay or read
//! them, nor make up an answer. What it learns is when, and how big.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use anyhow::{Context, Result, ensure};
use data_encoding::BASE64URL_NOPAD as B64;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use ::key::{Endpoint, FRESH, Key, hex};

/// What an ask to a service is signed for.
pub const ASK: &str = "one/service-ask";
/// What a service's answer is signed for.
pub const ANSWER: &str = "one/service-answer";

/// An ask a part is handed by its runtime: its id there, and the sealed bytes, base64url.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Carried {
    pub id: u64,
    pub ask: String,
}

impl super::Job for Carried {
    fn id(&self) -> u64 {
        self.id
    }
}

/// The parts of a device registered at its app runtime, by service name.
pub type Parts = super::Hub<Carried>;

/// `body` asked of the service `to` by `from`: signed, then sealed to the service. → the bytes, and the `at` it was
/// signed with, which its answer names.
pub fn ask(from: &SigningKey, to: &Endpoint, body: Value) -> Result<(Vec<u8>, u64)> {
    let at = ::key::next_at(now_ms());
    let signed = ::key::signed_request(from, ASK, json!({ "to": hex(&to.key), "body": body }), at);
    Ok((::key::seal_to(&to.seal, &serde_json::to_vec(&signed)?)?, at))
}

/// An ask the service `me` was handed, opened and checked: signed for it, fresh, and never taken before. → who
/// signed it, what it asks, and its `at`.
pub fn open_ask(me: &SigningKey, sealed: &[u8], once: &Once) -> Result<(Key, Value, u64)> {
    let opened = ::key::open_sealed(&::key::seal_secret(me), sealed).context("it isn't sealed to this service")?;
    let signed: Value = serde_json::from_slice(&opened)?;
    signed_by(&signed, &me.verifying_key().to_bytes(), once)
}

/// A request signed for `to` (a service, or a service's device as it asks its runtime), checked: fresh, and taken
/// once. → who signed it, its body, and its `at`.
pub fn signed_by(signed: &Value, to: &Key, once: &Once) -> Result<(Key, Value, u64)> {
    let (by, fields) = ::key::check_request(ASK, signed, now_ms())?;
    ensure!(fields["to"] == hex(to), "it was asked of someone else");
    let at = fields["at"].as_u64().unwrap_or(0);
    once.take(&by, at)?;
    Ok((by, fields["body"].clone(), at))
}

/// A service's own ask of its runtime, signed: not sealed, as the runtime runs it. `device` is the service's device.
pub fn own_ask(me: &SigningKey, device: &Key, body: Value) -> Value {
    ::key::signed_request(me, ASK, json!({ "to": hex(device), "body": body }), ::key::next_at(now_ms()))
}

/// The service `me`'s answer to an ask signed at `at` by `to`: signed, then sealed to the asker.
pub fn answer(me: &SigningKey, to: &Endpoint, at: u64, answer: Value) -> Result<Vec<u8>> {
    let signed = ::key::signed_request(me, ANSWER, json!({ "to": hex(&to.key), "ask": at, "answer": answer }), ::key::next_at(now_ms()));
    ::key::seal_to(&to.seal, &serde_json::to_vec(&signed)?)
}

/// The answer to the ask `me` signed at `at` of the service `service`, opened and checked.
pub fn open_answer(me: &SigningKey, service: &Key, at: u64, sealed: &[u8]) -> Result<Value> {
    let opened = ::key::open_sealed(&::key::seal_secret(me), sealed).context("the answer isn't sealed to this device")?;
    let signed: Value = serde_json::from_slice(&opened)?;
    let (by, fields) = ::key::check_request(ANSWER, &signed, now_ms())?;
    ensure!(by == *service && fields["to"] == hex(&me.verifying_key().to_bytes()) && fields["ask"] == at, "that answer isn't the service's to this ask");
    Ok(fields["answer"].clone())
}

/// The asks each signer was taken at, within `FRESH` either way: an `at` again is a replay.
#[derive(Default)]
pub struct Once(Mutex<HashMap<Key, BTreeSet<u64>>>);

impl Once {
    pub fn take(&self, by: &Key, at: u64) -> Result<()> {
        let now = now_ms();
        let mut taken = self.0.lock().unwrap();
        taken.retain(|_, ats| {
            ats.retain(|a| a.abs_diff(now) <= FRESH);
            !ats.is_empty()
        });
        ensure!(taken.entry(*by).or_default().insert(at), "that ask was taken already");
        Ok(())
    }
}

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn unb64(text: &str) -> Result<Vec<u8>> {
    B64.decode(text.as_bytes()).map_err(|_| anyhow::anyhow!("it isn't base64url"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
