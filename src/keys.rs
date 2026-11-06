//! Keys as devices hold them: each endpoint's own key, each scope's numbered epochs sealed to the endpoints that may
//! open it, what's sealed under an epoch, signed requests, and the keys a caller hands a node for one call.
//! `cloudflare/shared/crypto.js` writes the same formats, and `tests/contracts.rs` checks they agree.
//!
//! Every client is a device with its own key pair, never shared by a person's devices: a native app's in the Keychain,
//! keystore or Credential Locker, a browser's a non-extractable WebCrypto key in IndexedDB, a local agent's its
//! machine's. A new device gets in only when one of its person's devices approves it, from a code or QR that shows
//! which device it is, which seals it the keys of the scopes it may open. A person who lost a device is let back in by
//! another, by a copy sealed under a passkey's PRF (optional for most, kept because synced passkeys make it
//! self-service), or by the install's recovery key, kept offline by an admin, each use of it an entry in the log.
//! A scope gets a new epoch when a person, or a device that didn't sign itself out (lost or stolen), loses the right
//! to open it, never on sign-out, where the device deletes its own key: the keys are sealed to everyone who still
//! belongs. Epochs never go down along a database's positions (`src/db.rs`).
//!
//! ```text
//! <folder>/_epochs/<n>/<endpoint key hex>   epoch n's key, sealed to that endpoint's seal key (`cloudflare::seal_to`)
//! <folder>/_epochs/<n>/recovery             and to the install's recovery key
//! <folder>/<app>/_key/<n>                    the app's key in epoch n, sealed under epoch n's key (`cloudflare::seal_app_key`)
//! <folder>/<app>/_grants/<endpoint>/<n>      and sealed to an endpoint a `grant` keeps it running on (`cloudflare::seal_to`)
//! <folder>/_vault/<n>/<endpoint key hex>     the scope's Vault secret key n, sealed to a machine its `vault` entry grants
//! <folder>/_vault/<n>/recovery               and to the install's recovery key
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
        let (_, seal) = crate::cloudflare::twin(&key.to_bytes());
        Endpoint { key: key.verifying_key().to_bytes(), seal }
    }
}

/// What a native app's device seals the last answers of its screens' calls under (client/src/device.rs): its own, from
/// its key, so no other device (another person's sign-in has its own) opens them.
pub fn answers_key(key: &SigningKey) -> [u8; 32] {
    hkdf(&key.to_bytes(), b"one/answers")
}

/// An endpoint's X25519 secret, for opening what's sealed to it, as `Endpoint::of` lists it.
pub fn seal_secret(key: &SigningKey) -> [u8; 32] {
    crate::cloudflare::twin(&key.to_bytes()).0
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

/// When to sign a request that something takes once by its `at` (the relay): `now` (milliseconds), or one more than
/// the last this process signed, so two of its requests in one millisecond are never the same.
pub fn next_at(now: u64) -> u64 {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let last = LAST.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
    if last < now {
        return now;
    }
    LAST.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

/// Whether the relay refused a request because another with the same `at` from the same key came first, which signing
/// it again a moment later cures: two processes of one device signing in one millisecond.
pub fn taken_already(said: &str) -> bool {
    said.contains("taken already")
}

/// How long to wait before signing again a request (signed `at`) that was taken already: a few milliseconds that
/// differ between processes, so the two don't land on the same one again.
pub fn again_in(at: u64) -> std::time::Duration {
    std::time::Duration::from_millis(2 + (at ^ u64::from(std::process::id())) % 23)
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

/// A device's word that `endpoint` is its own until `until` (ms, at most `SESSION` after `at`, and after whenever it's
/// checked, so a device whose clock is fast asks for a little less): a signed request,
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

/// Checks a certificate (`certify`'s) at `now`: signed, not yet expired, for no longer than `SESSION` from when it was
/// signed or from `now`, whichever is sooner, and not signed in the future. Who signed it is for the caller to check: it counts only if they're a device of the install.
pub fn check_certificate(certificate: &str, now: u64) -> Result<Session> {
    let request: Value = B64U.decode(certificate.trim().as_bytes()).ok().and_then(|b| serde_json::from_slice(&b).ok()).context("a certificate is base64url JSON")?;
    let (by, fields, at) = check_signed(ENDPOINT, &request)?;
    let endpoint = unhex(fields["endpoint"].as_str().unwrap_or(""))?;
    let until = fields["until"].as_u64().context("a certificate says until when")?;
    ensure!(at <= now + FRESH, "it was signed in the future, or the clock that signed it is wrong");
    ensure!(until > now, "that session has ended");
    ensure!(until <= at.min(now) + SESSION, "a session lasts a day at most");
    Ok(Session { by, endpoint, until })
}

// Epochs.

/// Where epoch `n` of a scope is kept sealed to `to`: an endpoint key, or `recovery`.
pub fn epoch_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/_epochs/{n}/{to}", crate::bucket::folder(scope))
}

/// What an `epoch` entry says of its key, so a device takes a key from the bucket as epoch n only if it's the one the
/// signed entry made, and a `grant` of each app key it seals: HKDF-SHA256(key, "one/epoch-hash"), hex.
pub fn epoch_hash(key: &[u8; 32]) -> String {
    HEXLOWER.encode(&hkdf(key, b"one/epoch-hash"))
}

/// Where an app's key in epoch `n` of a scope is kept, sealed under that epoch's key.
pub fn app_key_name(scope: &str, app: &str, n: u32) -> String {
    format!("{}/{app}/_key/{n}", crate::bucket::folder(scope))
}

/// Where an app's key in epoch `n` of a scope is kept sealed to an endpoint that keeps the app running there (a
/// `grant`, for standing work): its key in hex.
pub fn grant_name(scope: &str, app: &str, endpoint: &str, n: u32) -> String {
    format!("{}/{app}/_grants/{endpoint}/{n}", crate::bucket::folder(scope))
}

/// Where a scope's Vault secret key `n` is kept sealed to `to`: a machine's key granted it, or `recovery`. Its public
/// half is in the scope's `vault` entry, and what's sealed to that opens only on those machines.
pub fn vault_key_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/_vault/{n}/{to}", crate::bucket::folder(scope))
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
    Ok(B64U.encode(&crate::cloudflare::seal_to(to, &serde_json::to_vec(handoff)?)?))
}

/// Opens a handoff sealed to this node, with its seal secret.
pub fn open_handoff(secret: &[u8; 32], sealed: &str) -> Result<Handoff> {
    let bytes = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("the keys handed aren't base64url"))?;
    let plain = crate::cloudflare::open_sealed(secret, &bytes).context("the keys handed weren't sealed to this device")?;
    serde_json::from_slice(&plain).context("the keys handed aren't a handoff")
}

/// What a node with no sign-in to pass on (the Cloudflare node, or a machine's passing on a call it was handed keys
/// for) asks another node to run a call with (`src/node/net.rs`): the call, naming `scopes`, the scopes it reaches,
/// the person it's for, and the keys of those scopes it was handed, sealed to that node's `seal` key for calls from
/// its own endpoint (`from`), until `until`. Never a sign-in's token, which whoever runs the node could use as them;
/// nor keys of a scope the call doesn't name. The other node runs it on those keys alone, and only if `from` may run
/// every one of `scopes`.
pub fn forwarded(call: &Value, user: &str, client: &str, from: &[u8; 32], seal: &[u8; 32], until: u64, keys: Vec<Handed>, scopes: &[String]) -> Result<Value> {
    let mut call = call.as_object().cloned().unwrap_or_default();
    for k in ["me", "keys", "token"] {
        call.remove(k);
    }
    call.insert("me".into(), json!({ "user": user, "client": client }));
    call.insert("scopes".into(), json!(scopes));
    let keys: Vec<Handed> = keys.into_iter().filter(|k| scopes.contains(&k.scope)).collect();
    call.insert("keys".into(), seal_handoff(seal, &Handoff { for_: HEXLOWER.encode(from), until, keys })?.into());
    Ok(json!({ "kind": "ask", "body": { "call": call } }))
}

/// A key a call lacked, which its caller hands it when it sends the call again: what a node answers with 428.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Need {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
}

/// The node's databases of a scope: its Vault's items (`vault::DATABASE`) and its log (`log::NAME`), which hold who's
/// who there.
const VAULT: &str = "vault";
const LOG: &str = "log";
/// The node's database of the blobs a scope's entries name (`bucket::BLOBS`).
const BLOBS: &str = "blobs";
/// An app's own databases: its data, and its services' records.
const APP_DATABASE: &str = "main";
// Named before services were called services: it holds sealed records (AGENTS.md's history among them), which only
// clients could copy under a new name.
const RECORDS_DATABASE: &str = "sidecar";

/// The app a kind of request runs, as clients name kinds to remember what each needed: an app's call (`call <app>`),
/// or its screens (`/v1/screens/<app>`). None for any other.
pub fn app_of(route: &str) -> Option<&str> {
    route.strip_prefix("call ").or_else(|| route.strip_prefix("/v1/screens/").filter(|a| !a.contains('/'))).filter(|a| !a.is_empty())
}

/// Whether a client hands a node the key of `need`'s database with a kind of request (`route`, as `app_of` reads it),
/// when the node asks (428): only what that kind of request can use. One that runs an app, that app's databases; an
/// MCP request (`/mcp`), which may run any, or widgets and badges, which run every app's, apps' own databases; and each
/// of those the node's Vault and log of a scope too (which fill in its items, and say who's who there), and `global`'s
/// blobs, where apps' deployed code is (a node that keeps nothing, the Cloudflare node, reads it there). The list of
/// apps, which says what each does from its code, the log and that code. The Vault's routes, its Vault and log; any
/// other, the log alone. `cloudflare/shared/crypto.js`'s `mayHand` is the same.
///
/// `global`'s blobs are those its entries name (deploys' bundles, releases, apps' icons), which everyone in the install
/// may read already: any of them may add any app, and read its code. Another scope's are its own people's files, and
/// never go with a request.
pub fn may_hand(route: &str, need: &Need) -> bool {
    let node = |of: &[&str]| need.app == crate::members::APP && of.contains(&need.database.as_str());
    let apps = need.app != crate::members::APP && [APP_DATABASE, RECORDS_DATABASE].contains(&need.database.as_str());
    let code = need.scope == "global" && node(&[BLOBS]);
    match (app_of(route), route) {
        (Some(app), _) => need.app == app || node(&[VAULT, LOG]) || code,
        (None, "/mcp" | "/v1/widgets" | "/v1/badges") => apps || node(&[VAULT, LOG]) || code,
        (None, "/v1/apps") => node(&[LOG]) || code,
        (None, r) if r == "/v1/vault" || r.starts_with("/v1/vault/") => node(&[VAULT, LOG]),
        (None, _) => node(&[LOG]),
    }
}

/// The oldest epoch of `database`'s key a reader needs, of a scope whose newest epoch is `newest`: the one its newest
/// copy is under, as its newest hint (`hint`, if it has one) says, which holds all before it; every epoch from the first
/// while it has no copy; the newest if it has no hint, being new. Blobs have neither: each is kept under the epoch it
/// was put in, so every epoch from the first. Nodes ask for no older, and clients hand no older.
/// `cloudflare/shared/crypto.js`'s `oldestNeeded` is the same.
pub fn oldest_needed(database: &str, hint: Option<&[u8]>, newest: u32) -> u32 {
    if database == BLOBS {
        return 1;
    }
    let newest = newest.max(1);
    let Some(hint) = hint else { return newest };
    let v: Value = serde_json::from_slice(hint).unwrap_or_default();
    match (v["copy"].as_u64().unwrap_or(0), v["epoch"].as_u64().unwrap_or(0)) {
        (0, _) => 1,
        (_, epoch) => u32::try_from(epoch).unwrap_or(u32::MAX).clamp(1, newest),
    }
}

/// The most keys a node's 428 may ask one request to hand, once each: a request that reads every app on a page
/// (`/v1/badges`, `/v1/widgets`, `/mcp`) needs each one's, in each epoch from the oldest it needs. More is a node asking
/// for everything.
pub const MOST_NEEDED: usize = 64;

/// What a node's 428 (`answer`) asks a request of `route` to hand: taken only from the node that answered (`node`, hex,
/// as its connection proved), naming itself; one or more keys, MOST_NEEDED at most, each of a database that kind of
/// request uses (`may_hand`). None if it asks for anything else. The seal key it names isn't read: what's handed is
/// sealed to the one the install's entries list for that node. `cloudflare/shared/crypto.js`'s `askedFor` is the same.
pub fn asked_for(route: &str, node: &str, answer: &Value) -> Option<Vec<Need>> {
    if answer["to"].as_str() != Some(node) {
        return None;
    }
    let mut need: Vec<Need> = serde_json::from_value(answer["need"].clone()).ok()?;
    need.sort();
    need.dedup();
    (!need.is_empty() && need.len() <= MOST_NEEDED && need.iter().all(|n| may_hand(route, n))).then_some(need)
}

/// How long a client waits, each time, to ask again a node that doesn't list it yet (403 `not_let_in`) while the relay
/// still lets it in: a node may not have read its approval yet. After the last, it says so.
pub const BEHIND: [u64; 2] = [2_000, 4_000];

/// What sign-in said of a device's request to be let in (`GET /api/auth/devices/requests/<key>`: its status, 0 if it
/// didn't answer, and its JSON), which every client acts on the same way (`cloudflare/shared/crypto.js`'s `waited`).
#[derive(Debug, PartialEq)]
pub enum Waited {
    /// Its sign-in is no more: it signs in again.
    SignedOut,
    /// Approved, in the install whose first entry (hex) the device that approved it named, if it named one: the only
    /// one it then takes; and once who's who is written as steps, where that device read them to, which this one
    /// starts from (`database::steps::Welcome`), if it said. It's in once the relay lets it.
    Approved { genesis: Option<String>, welcome: Option<String> },
    /// Gone from sign-in (approved and cleared, turned down, or waited too long): it's in if the relay lets it, else it
    /// asks again.
    Gone,
    /// Still waiting, or sign-in didn't answer.
    Waiting,
}

pub fn waited(status: u16, said: &Value) -> Waited {
    match status {
        401 => Waited::SignedOut,
        404 => Waited::Gone,
        200..=299 if said["approved"] == true => Waited::Approved {
            genesis: said["genesis"].as_str().filter(|g| g.len() == 64 && g.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).map(str::to_string),
            welcome: said["welcome"].as_str().filter(|w| crate::database::steps::Welcome::parse(w).is_some()).map(str::to_string),
        },
        _ => Waited::Waiting,
    }
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

/// Whether two secrets are the same, in time that doesn't depend on where they differ.
pub fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
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
        assert!(check_certificate(&certify(&device, &endpoint, at + SESSION, at), at - 1).is_err(), "longer than a day from now");
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
        let handoff = Handoff { for_: token_id("t"), until: 5, keys: vec![Handed { scope: "global".into(), epoch: 1, app: "tally".into(), database: "main".into(), key: rand::random() }] };
        let sealed = seal_handoff(&Endpoint::of(&node).seal, &handoff).unwrap();
        assert_eq!(open_handoff(&seal_secret(&node), &sealed).unwrap(), handoff);
        assert!(open_handoff(&seal_secret(&other), &sealed).is_err());
    }

}
