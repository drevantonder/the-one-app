//! Keys as devices hold them: each endpoint's own key, each scope's numbered epochs sealed to the endpoints that may
//! open it, what's sealed under an epoch, signed requests, and the keys a caller hands a node for one call.
//! `kernel/shared/crypto.js` writes the same formats, and `tests/contracts.rs` checks they agree.
//!
//! ```text
//! <folder>/_epochs/<n>/<endpoint key hex>   epoch n's key, sealed to that endpoint's seal key (`kernel::seal_to`)
//! <folder>/_epochs/<n>/recovery             and to the install's recovery key
//! <folder>/<app>/_key/<n>                    the app's key in epoch n, sealed under epoch n's key (`kernel::seal_app_key`)
//! ```
//!
//! An endpoint is known by its Ed25519 key, which signs, and things are sealed to its X25519 key: for one in Rust, its
//! Ed25519 key's twin; a browser's is a WebCrypto key of its own. So each is listed with both, and nothing assumes twins.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, ensure};
use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD as B64U, HEXLOWER};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// How old a signed request may be when it's checked, in milliseconds, either way.
pub const FRESH: u64 = 60_000;

fn hkdf(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<Sha256>::new(None, ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// An endpoint as the install lists it: the key it signs with (its id) and the key things are sealed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(with = "hex32")]
    pub key: [u8; 32],
    #[serde(with = "hex32")]
    pub seal: [u8; 32],
}

impl Endpoint {
    /// An endpoint in Rust: its signing key's public half, and its twin to seal to.
    pub fn of(key: &SigningKey) -> Endpoint {
        let (_, seal) = crate::kernel::twin(&key.to_bytes());
        Endpoint { key: key.verifying_key().to_bytes(), seal }
    }
}

/// An endpoint's X25519 secret, for opening what's sealed to it, as `Endpoint::of` lists it.
pub fn seal_secret(key: &SigningKey) -> [u8; 32] {
    crate::kernel::twin(&key.to_bytes()).0
}

// Canonical JSON and signatures.

/// JSON's bytes with every object's keys in order and no spaces: what's signed and hashed. Numbers are integers.
pub fn canonical(v: &Value) -> Vec<u8> {
    // serde_json keeps an object's keys sorted (no preserve_order), as JavaScript's `canonical` sorts them.
    serde_json::to_vec(v).expect("json in memory")
}

/// What's signed for `domain` (`one/urls`, `one/endpoints`, ...): the domain, a zero byte, and canonical JSON.
fn signed_bytes(domain: &str, v: &Value) -> Vec<u8> {
    [domain.as_bytes(), &[0], &canonical(v)].concat()
}

pub fn sign(key: &SigningKey, domain: &str, v: &Value) -> [u8; 64] {
    key.sign(&signed_bytes(domain, v)).to_bytes()
}

pub fn verify(key: &[u8; 32], domain: &str, v: &Value, signature: &[u8]) -> Result<()> {
    let key = VerifyingKey::from_bytes(key).map_err(|_| anyhow!("not an Ed25519 key"))?;
    let signature = ed25519_dalek::Signature::from_slice(signature).map_err(|_| anyhow!("not a signature"))?;
    key.verify_strict(&signed_bytes(domain, v), &signature).map_err(|_| anyhow!("its signature is wrong"))
}

/// A request signed by an endpoint, as the URL signer and the allowed endpoints take it: the fields asked, with `at`
/// (milliseconds), `by` (the endpoint's key, hex) and `sig` (base64url) over the rest, under `domain`.
pub fn signed_request(key: &SigningKey, domain: &str, mut fields: Value, at: u64) -> Value {
    fields["at"] = json!(at);
    fields["by"] = json!(HEXLOWER.encode(key.verifying_key().as_bytes()));
    let sig = sign(key, domain, &fields);
    fields["sig"] = json!(B64U.encode(&sig));
    fields
}

/// Checks a signed request at `now`: → who signed it, and the fields without `sig`.
pub fn check_request(domain: &str, request: &Value, now: u64) -> Result<([u8; 32], Value)> {
    let (by, fields, at) = check_signed(domain, request)?;
    ensure!(at.abs_diff(now) <= FRESH, "it was signed too long ago, or the clock that signed it is wrong");
    Ok((by, fields))
}

/// A signed request's signature, whenever it was signed: → who signed it, the fields without `sig`, and `at`.
fn check_signed(domain: &str, request: &Value) -> Result<([u8; 32], Value, u64)> {
    let mut fields = request.clone();
    let sig = fields.as_object_mut().context("not a request")?.remove("sig").context("it isn't signed")?;
    let sig = B64U.decode(sig.as_str().unwrap_or("").as_bytes()).map_err(|_| anyhow!("its signature isn't base64url"))?;
    let by = unhex(fields["by"].as_str().unwrap_or(""))?;
    let at = fields["at"].as_u64().context("it doesn't say when it was signed")?;
    verify(&by, domain, &fields, &sig)?;
    Ok((by, fields, at))
}

// Session endpoints.

/// What a device signs to vouch for an endpoint of a session of its own.
pub const ENDPOINT: &str = "one/endpoint";
/// The longest a session's endpoint is vouched for, in milliseconds.
pub const SESSION: u64 = 24 * 3_600_000;

/// A device's word that `endpoint` is its own until `until` (ms, at most `SESSION` after `at`): a signed request,
/// as base64url JSON, the way the relay (iroh's auth token) and nodes (`x-one-endpoint`) take it. A browser's device
/// key can't be read out of WebCrypto, and iroh signs with raw bytes, so its endpoint is a key of the session's, and
/// this certificate is how the relay and nodes take that endpoint as the device.
pub fn certify(device: &SigningKey, endpoint: &[u8; 32], until: u64, at: u64) -> String {
    let request = signed_request(device, ENDPOINT, json!({ "endpoint": HEXLOWER.encode(endpoint), "until": until }), at);
    B64U.encode(&canonical(&request))
}

/// A session's endpoint, as its device vouched for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    /// The device that signed it.
    pub by: [u8; 32],
    pub endpoint: [u8; 32],
    pub until: u64,
}

/// Checks a certificate (`certify`'s) at `now`: signed, not yet expired, for no longer than `SESSION`, and not signed in
/// the future. Who signed it is for the caller to check: it counts only if they're a device of the install.
pub fn check_certificate(certificate: &str, now: u64) -> Result<Session> {
    let request: Value = B64U.decode(certificate.trim().as_bytes()).ok().and_then(|b| serde_json::from_slice(&b).ok()).context("a certificate is base64url JSON")?;
    let (by, fields, at) = check_signed(ENDPOINT, &request)?;
    let endpoint = unhex(fields["endpoint"].as_str().unwrap_or(""))?;
    let until = fields["until"].as_u64().context("a certificate says until when")?;
    ensure!(at <= now + FRESH, "it was signed in the future, or the clock that signed it is wrong");
    ensure!(until > now, "that session has ended");
    ensure!(until <= at + SESSION, "a session lasts a day at most");
    Ok(Session { by, endpoint, until })
}

// Epochs.

/// Where epoch `n` of a scope is kept sealed to `to`: an endpoint key, or `recovery`.
pub fn epoch_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/_epochs/{n}/{to}", crate::bucket::folder(scope))
}

/// What an `epoch` entry says of its key, so a device takes a key from the bucket as epoch n only if it's the one the
/// signed entry made: HKDF-SHA256(key, "one/epoch-hash"), hex.
pub fn epoch_hash(key: &[u8; 32]) -> String {
    HEXLOWER.encode(&hkdf(key, b"one/epoch-hash"))
}

/// Where an app's key in epoch `n` of a scope is kept, sealed under that epoch's key.
pub fn app_key_name(scope: &str, app: &str, n: u32) -> String {
    format!("{}/{app}/_key/{n}", crate::bucket::folder(scope))
}

/// Seals an object under an epoch's key (or a key from it) and its name: epoch (u32, big-endian) ‖ nonce ‖
/// AES-256-GCM(key, nonce, bytes, aad = name), so it can't be read, changed or passed off as another object, and a
/// reader knows which epoch's key opens it before trying.
pub fn seal(epoch: u32, key: &[u8; 32], name: &str, bytes: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = epoch.to_be_bytes().to_vec();
    out.extend(nonce);
    out.extend(Aes256Gcm::new(key.into()).encrypt(&nonce.into(), Payload { msg: bytes, aad: name.as_bytes() }).expect("encrypts"));
    out
}

/// The epoch a sealed object names.
pub fn epoch_of(sealed: &[u8]) -> Result<u32> {
    ensure!(sealed.len() >= 4 + 12 + 16, "too short to be sealed");
    Ok(u32::from_be_bytes(sealed[..4].try_into().expect("4 bytes")))
}

/// Opens what `seal` sealed, with its epoch's key.
pub fn open(key: &[u8; 32], name: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    epoch_of(sealed).with_context(|| format!("{name} isn't sealed"))?;
    let nonce: [u8; 12] = sealed[4..16].try_into().expect("12 bytes");
    Aes256Gcm::new(key.into()).decrypt(&nonce.into(), Payload { msg: &sealed[16..], aad: name.as_bytes() }).map_err(|_| anyhow!("{name} didn't open with its key"))
}

// A call's keys.

/// A database key a caller hands a node for one call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handed {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
    #[serde(with = "b64key")]
    pub key: [u8; 32],
}

/// What a caller hands a node, sealed to it: the database keys a call needs, for calls with one access token (`for`,
/// its sha256 in hex) or from one client device over iroh (`for`, its key in hex), until `until` (milliseconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    #[serde(rename = "for")]
    pub for_: String,
    pub until: u64,
    pub keys: Vec<Handed>,
}

/// The `for` of a handoff with calls that bring `token`.
pub fn token_id(token: &str) -> String {
    HEXLOWER.encode(&sha256(token.as_bytes()))
}

/// A handoff sealed to the node's seal key, as base64url: the `x-one-keys` header's value.
pub fn seal_handoff(to: &[u8; 32], handoff: &Handoff) -> Result<String> {
    Ok(B64U.encode(&crate::kernel::seal_to(to, &serde_json::to_vec(handoff)?)?))
}

/// Opens a handoff sealed to this node, with its seal secret.
pub fn open_handoff(secret: &[u8; 32], sealed: &str) -> Result<Handoff> {
    let bytes = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("the keys handed aren't base64url"))?;
    let plain = crate::kernel::open_sealed(secret, &bytes).context("the keys handed weren't sealed to this device")?;
    serde_json::from_slice(&plain).context("the keys handed aren't a handoff")
}

/// A key a call lacked, which its caller hands it when it sends the call again: what a node answers with 428.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Need {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
}

// Devices joining.

/// The code a new device shows, which a device of its person types or scans to approve it: the first 8 base32
/// characters of the sha256 of its key and seal key, so whoever passes on its request can swap neither.
pub fn join_code(device: &Endpoint) -> String {
    BASE32_NOPAD.encode(&sha256(&[device.key, device.seal].concat()))[..8].to_string()
}

/// A newcomer's proof that their invite's secret was theirs: HKDF-SHA256(secret, "one/invite\0" ‖ their device's
/// key ‖ its seal key), hex.
pub fn invite_proof(secret: &[u8], device: &Endpoint) -> String {
    HEXLOWER.encode(&hkdf(secret, &[&b"one/invite\0"[..], &device.key, &device.seal].concat()))
}

/// A passkey's device keys, from its PRF output: (the Ed25519 seed it signs with, the X25519 secret sealed to).
pub fn passkey_keys(prf: &[u8]) -> (SigningKey, [u8; 32]) {
    let seed = hkdf(prf, b"one/passkey-device/sign");
    (SigningKey::from_bytes(&seed), hkdf(prf, b"one/passkey-device/seal"))
}

// Encodings.

pub fn unhex(s: &str) -> Result<[u8; 32]> {
    HEXLOWER.decode(s.as_bytes()).ok().and_then(|b| b.try_into().ok()).context("not a key: 64 hex characters")
}

mod hex32 {
    pub fn serialize<S: serde::Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&data_encoding::HEXLOWER.encode(v))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        super::unhex(&s).map_err(serde::de::Error::custom)
    }
}

mod b64key {
    pub fn serialize<S: serde::Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&data_encoding::BASE64URL_NOPAD.encode(v))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        data_encoding::BASE64URL_NOPAD.decode(s.as_bytes()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| serde::de::Error::custom("a key is 32 bytes, base64url"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_request_is_checked_whole_and_fresh() {
        let key = SigningKey::from_bytes(&rand::random());
        let req = signed_request(&key, "one/urls", json!({ "read": ["a/b"] }), 1_000_000);
        let (by, fields) = check_request("one/urls", &req, 1_000_000 + FRESH).unwrap();
        assert_eq!(by, key.verifying_key().to_bytes());
        assert_eq!(fields["read"], json!(["a/b"]));
        assert!(check_request("one/urls", &req, 1_000_000 + FRESH + 1).is_err(), "too old");
        assert!(check_request("one/endpoints", &req, 1_000_000).is_err(), "only for its domain");
        let mut changed = req.clone();
        changed["read"] = json!(["a/c"]);
        assert!(check_request("one/urls", &changed, 1_000_000).is_err(), "nothing in it changes");
    }

    #[test]
    fn a_certificate_vouches_for_one_endpoint_for_a_day_at_most() {
        let device = SigningKey::from_bytes(&rand::random());
        let endpoint: [u8; 32] = rand::random();
        let at = 1_000_000_000;
        let cert = certify(&device, &endpoint, at + SESSION, at);
        let session = check_certificate(&cert, at + SESSION - 1).unwrap();
        assert_eq!(session, Session { by: device.verifying_key().to_bytes(), endpoint, until: at + SESSION });
        assert!(check_certificate(&cert, at + SESSION).is_err(), "ended");
        assert!(check_certificate(&cert, at - FRESH - 1).is_err(), "signed in the future");
        assert!(check_certificate(&certify(&device, &endpoint, at + SESSION + 1, at), at).is_err(), "longer than a day");
        let request = signed_request(&device, "one/relay", json!({ "endpoint": HEXLOWER.encode(&endpoint), "until": at + 1 }), at);
        assert!(check_certificate(&B64U.encode(&canonical(&request)), at).is_err(), "only for its domain");
    }

    #[test]
    fn a_sealed_object_opens_only_as_itself_and_names_its_epoch() {
        let key: [u8; 32] = rand::random();
        let sealed = seal(7, &key, "user-1/notes/main/1", b"step");
        assert_eq!(epoch_of(&sealed).unwrap(), 7);
        assert_eq!(open(&key, "user-1/notes/main/1", &sealed).unwrap(), b"step");
        assert!(open(&key, "user-1/notes/main/2", &sealed).is_err(), "not as another object");
        assert!(open(&rand::random(), "user-1/notes/main/1", &sealed).is_err(), "not with another key");
    }

    #[test]
    fn a_handoff_opens_only_for_the_node_it_was_sealed_to() {
        let node = SigningKey::from_bytes(&rand::random());
        let other = SigningKey::from_bytes(&rand::random());
        let handoff = Handoff { for_: token_id("t"), until: 5, keys: vec![Handed { scope: "global".into(), epoch: 1, app: "board".into(), database: "main".into(), key: rand::random() }] };
        let sealed = seal_handoff(&Endpoint::of(&node).seal, &handoff).unwrap();
        assert_eq!(open_handoff(&seal_secret(&node), &sealed).unwrap(), handoff);
        assert!(open_handoff(&seal_secret(&other), &sealed).is_err());
    }
}
