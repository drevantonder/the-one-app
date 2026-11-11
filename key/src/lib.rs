//! Keys as devices hold them: each endpoint's own key, each scope's numbered epochs sealed to the endpoints that may
//! open it, what's sealed under an epoch, signed requests, and the keys a caller hands a node for one call.
//! `key/crypto.js` writes the same formats, and `tests/contracts.rs` checks they agree.
//!
//! Every client is a device with its own key pair, never shared by a user's devices: a native app's in the Keychain,
//! keystore or Credential Locker, a browser's a non-extractable WebCrypto key in IndexedDB, a local agent's its
//! machine's. A new device gets in only when one of its user's devices approves it, from a code or QR that shows
//! which device it is, which seals it the keys of the scopes it may open. A user who lost a device is let back in by
//! another, or by the network's recovery key, kept offline by an admin, each use of it an entry in the log.
//! A scope gets a new epoch when a user, or a device that didn't sign itself out (lost or stolen), loses the right
//! to open it, never on sign-out, where the device deletes its own key: the keys are sealed to everyone who still
//! belongs. Epochs never go down along a database's positions (`database/src/stored.rs`).
//!
//! ```text
//! <folder>/_epochs/<n>/<endpoint key hex>   epoch n's key, sealed to that endpoint's seal key (`keys::seal_to`)
//! <folder>/_epochs/<n>/recovery             and to the network's recovery key
//! <folder>/<app>/_key/<n>                    the app's key in epoch n, sealed under epoch n's key (`keys::seal_app_key`)
//! <folder>/<app>/_app_schedules/<endpoint>/<n>  and sealed to an endpoint that runs its app schedules (`keys::seal_to`)
//! <folder>/<app>/_app_schedules/vault/<n>    the scope's Vault database's key in epoch n, under the app's (`keys::seal`)
//! <folder>/_vault/<n>/<endpoint key hex>     the scope's Vault secret key n, sealed to a machine its `vault` entry grants
//! <folder>/_vault/<n>/recovery               and to the network's recovery key
//! global/_recovery/<recovery key hex>/<endpoint key hex>   the recovery key, sealed to a device of someone who holds it
//! <folder>/<app>/<database>/…               its steps, sealed with its key in their epoch, from the app's (`database_keys`)
//! ```
//!
//! An endpoint is known by its Ed25519 key, which signs, and things are sealed to its X25519 key: for one in Rust, its
//! Ed25519 key's twin; a browser's is a WebCrypto key of its own. So each is listed with both, and nothing assumes twins.

use aes_gcm::aead::{Aead, Payload};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, ensure};
use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD as B64U, HEXLOWER};
pub use ed25519_dalek::SigningKey;
use ed25519_dalek::{Signer, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

/// How old a signed request may be when it's checked, in milliseconds, either way.
pub const FRESH: u64 = 60_000;

/// The first byte of everything sealed since formats were numbered, and authenticated with it, so taking it off, or
/// putting it on what was sealed before, opens nothing. What was sealed before has no version and opens forever
/// (`tests/sealed/`). A later format is one more, and reads one more in `database::READS`: an older build takes what it
/// can't read for junk and passes over it, and what it kept is read again (`steps::read_again`).
pub const FORMAT: u8 = 1;
/// What's authenticated with a format, before what each seal adds.
pub const NUMBERED: &[u8] = &[FORMAT];
const UNNUMBERED: &[u8] = &[];

/// The ways `sealed` may have been written, newest first: what's authenticated with that format, and what follows its
/// version. What was sealed before can start with `FORMAT` by chance, where it starts with random bytes, so each is
/// tried: AES-GCM says which it was.
pub fn forms(sealed: &[u8]) -> impl Iterator<Item = (&'static [u8], &[u8])> {
    sealed.strip_prefix(NUMBERED).map(|body| (NUMBERED, body)).into_iter().chain([(UNNUMBERED, sealed)])
}

fn hkdf(ikm: &[u8], info: &[u8]) -> [u8; 32] {
    hkdf_salted(ikm, &[], info)
}

/// HKDF-SHA256 to 32 bytes. An empty salt is HMAC's zero key, as no salt is.
fn hkdf_salted(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// An endpoint as the network lists it: the key it signs with (its id) and the key things are sealed to.
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
        let (_, seal) = twin(&key.to_bytes());
        Endpoint { key: key.verifying_key().to_bytes(), seal }
    }
}

/// What a native app's device seals the last answers of its screens' calls under (onet/src/device.rs): its own, from
/// its key, so no other device (another user's sign-in has its own) opens them.
pub fn answers_key(key: &SigningKey) -> [u8; 32] {
    hkdf(&key.to_bytes(), b"one/answers")
}

/// An endpoint's X25519 secret, for opening what's sealed to it, as `Endpoint::of` lists it.
pub fn seal_secret(key: &SigningKey) -> [u8; 32] {
    twin(&key.to_bytes()).0
}

/// An app's key in a scope's epoch is random, kept sealed under the epoch's key (`keys::app_key_name`): `FORMAT` ‖ nonce
/// ‖ AES-256-GCM(HKDF(epoch key, "one/app\0<app>"), nonce, app key). Erasing the app deletes it, which destroys the key
/// and so the app's data.
pub fn seal_app_key(epoch_key: &[u8; 32], app: &str, app_key: &[u8; 32]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = NUMBERED.to_vec();
    out.extend(nonce);
    out.extend(cipher(&hkdf(epoch_key, &app_key_info(app))).encrypt(&nonce.into(), Payload { msg: &app_key[..], aad: NUMBERED }).expect("encrypting in memory"));
    out
}

pub fn open_app_key(epoch_key: &[u8; 32], app: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    let key = cipher(&hkdf(epoch_key, &app_key_info(app)));
    let mut forms = forms(sealed).filter(|(_, body)| body.len() == 60).peekable();
    ensure!(forms.peek().is_some(), "not a sealed app key");
    let plain = forms.find_map(|(aad, body)| key.decrypt(body[..12].into(), Payload { msg: &body[12..], aad }).ok()).ok_or_else(|| anyhow!("that epoch's key doesn't open {app}'s"))?;
    plain.try_into().map_err(|_| anyhow!("an app's key is 32 bytes"))
}

/// A database's key in an app, from the app's key: HKDF(app key, "one/database\0" ‖ what it's made from, `permanent`).
pub fn database_key(app_key: &[u8; 32], app: &str, database: &str) -> [u8; 32] {
    hkdf(app_key, format!("one/database\0{}", permanent(app, database).key).as_bytes())
}

/// What an app's or a database's keys, seals and place in the bucket are made from, which nothing changes afterwards:
/// the name it was first sealed under. A scope needs none of this: it's named by an id that's never changed, and its
/// name and handle are only who's who's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permanent<'a> {
    /// Its app's folder in a scope's, which its app's key is made from too.
    pub app: &'a str,
    /// Its folder in its app's.
    pub database: &'a str,
    /// What its key is made from.
    pub key: &'a str,
}

/// Something now called `app` (`database` in it, or the whole app for none), first sealed as `first_app` and
/// `first_database`. `key_only` is a database that moved in the bucket and kept its old key.
struct Renamed {
    app: &'static str,
    database: Option<&'static str>,
    first_app: &'static str,
    first_database: Option<&'static str>,
    key_only: bool,
}

/// Every rename since something was first sealed: a rename adds a row here and changes nothing sealed, and a row is
/// never taken away, nor its old name given to anything else. The scope database's steps are sealed with the frozen
/// log's key, as every bucket's are.
const RENAMED: &[Renamed] = {
    use crate::{APP, CoreDb};
    &[Renamed { app: APP, database: Some(CoreDb::Scope.name()), first_app: APP, first_database: Some(CoreDb::Log.name()), key_only: true }]
};

/// What `app`'s `database` is made from. Pass an empty `database` for the app alone.
pub fn permanent<'a>(app: &'a str, database: &'a str) -> Permanent<'a> {
    permanent_in(RENAMED, app, database)
}

fn permanent_in<'a>(renamed: &[Renamed], app: &'a str, database: &'a str) -> Permanent<'a> {
    let row = |d: Option<&str>| renamed.iter().find(|r| r.app == app && r.database == d);
    let Some(row) = row(Some(database)).or_else(|| row(None)) else {
        return Permanent { app, database, key: database };
    };
    let first = row.first_database.unwrap_or(database);
    if row.key_only { Permanent { app, database, key: first } } else { Permanent { app: row.first_app, database: first, key: first } }
}

/// What an app's key in an epoch is sealed under is derived with: its permanent name.
fn app_key_info(app: &str) -> Vec<u8> {
    format!("one/app\0{}", permanent(app, "").app).into_bytes()
}

/// A database's key in an epoch, from its app's key `sealed` under the epoch's key (`app_key_name`).
pub fn open_database_key(epoch_key: &[u8; 32], app: &str, database: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    Ok(database_key(&open_app_key(epoch_key, app, sealed)?, app, database))
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(key.into())
}

pub fn public_of(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).0
}

/// The X25519 twin of an Ed25519 key (the recovery key), as age and backups use it: → (secret, public).
pub fn twin(seed: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let secret: [u8; 32] = Sha512::digest(seed)[..32].try_into().expect("64 bytes");
    (secret, public_of(&secret))
}

pub fn twin_public(public: &[u8; 32]) -> Result<[u8; 32]> {
    Ok(CompressedEdwardsY(*public).decompress().context("not an Ed25519 key")?.to_montgomery().0)
}

/// Seals `plain` to an X25519 public key: `FORMAT` ‖ share ‖ AES-256-GCM(HKDF(shared, share ‖ to), zero nonce, plain).
pub fn seal_to(to: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    sealed_to(NUMBERED, to, plain)
}

/// `seal_to` with no version: what a notification carries, since a phone's shell, which opens it with no window, changes only
/// when the App Store or TestFlight brings a new build. Nothing keeps these.
pub fn seal_unnumbered_to(to: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    sealed_to(UNNUMBERED, to, plain)
}

fn sealed_to(format: &'static [u8], to: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    let ephemeral: [u8; 32] = rand::random();
    let share = public_of(&ephemeral);
    let shared = MontgomeryPoint(*to).mul_clamped(ephemeral).0;
    ensure!(shared != [0; 32], "not a public key");
    let key = hkdf_salted(&shared, &[&share[..], to].concat(), b"one/sealed-to");
    let mut out = format.to_vec();
    out.extend(share);
    out.extend(cipher(&key).encrypt(&Default::default(), Payload { msg: plain, aad: format }).map_err(|_| anyhow!("sealing failed"))?);
    Ok(out)
}

pub fn open_sealed(secret: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 48, "not sealed");
    let mut wrong = None;
    for (aad, body) in forms(sealed).filter(|(_, body)| body.len() >= 48) {
        let share: [u8; 32] = body[..32].try_into()?;
        match opened(&agree(secret, &share), &public_of(secret), aad, body) {
            Ok(plain) => return Ok(plain),
            Err(err) => wrong = Some(err),
        }
    }
    Err(wrong.context("not sealed")?)
}

/// What opens what's sealed to a seal key: its secret, or where that can't be read (a browser's WebCrypto key), what
/// agrees a secret with it.
pub trait Opens {
    /// The seal key.
    fn public(&self) -> [u8; 32];
    /// The X25519 secret it shares with `share`.
    fn shared(&self, share: &[u8; 32]) -> impl Future<Output = Result<[u8; 32]>>;
}

impl Opens for [u8; 32] {
    fn public(&self) -> [u8; 32] {
        public_of(self)
    }

    async fn shared(&self, share: &[u8; 32]) -> Result<[u8; 32]> {
        Ok(agree(self, share))
    }
}

/// The secret `secret` (a seal key's) and `share` agree on.
pub fn agree(secret: &[u8; 32], share: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint(*share).mul_clamped(*secret).0
}

/// The secret a device's seal key (of its key `secret`) and `share` agree on.
pub fn agree_as(secret: &[u8; 32], share: &[u8; 32]) -> [u8; 32] {
    agree(&seal_secret(&SigningKey::from_bytes(secret)), share)
}

/// `secret`'s Ed25519 signature of `bytes`.
pub fn sign_with(secret: &[u8; 32], bytes: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(secret).sign(bytes).to_bytes()
}

/// `open_sealed`, by what opens it.
pub async fn open_sealed_by(opens: &impl Opens, sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 48, "not sealed");
    let mut wrong = None;
    for (aad, body) in forms(sealed).filter(|(_, body)| body.len() >= 48) {
        let share: [u8; 32] = body[..32].try_into()?;
        match opened(&opens.shared(&share).await?, &opens.public(), aad, body) {
            Ok(plain) => return Ok(plain),
            Err(err) => wrong = Some(err),
        }
    }
    Err(wrong.context("not sealed")?)
}

fn opened(shared: &[u8; 32], public: &[u8; 32], aad: &[u8], body: &[u8]) -> Result<Vec<u8>> {
    let key = hkdf_salted(shared, &[&body[..32], public].concat(), b"one/sealed-to");
    cipher(&key).decrypt(&Default::default(), Payload { msg: &body[32..], aad }).map_err(|_| anyhow!("that key doesn't open it"))
}

/// The id a copy sealed under `secret` is kept by.
pub fn id_of(secret: &[u8]) -> String {
    B64U.encode(&hkdf(secret, b"one/id"))
}

/// Seals `plain` under a secret (a refresh token): `FORMAT` ‖ nonce ‖ AES-256-GCM.
pub fn seal_under(secret: &[u8], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = NUMBERED.to_vec();
    out.extend(nonce);
    out.extend(cipher(&hkdf(secret, b"one/sealed-under")).encrypt(&nonce.into(), Payload { msg: plain, aad: NUMBERED }).expect("encrypting in memory"));
    out
}

pub fn open_under(secret: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 28, "not sealed");
    let key = cipher(&hkdf(secret, b"one/sealed-under"));
    forms(sealed).filter(|(_, body)| body.len() >= 28).find_map(|(aad, body)| key.decrypt(body[..12].into(), Payload { msg: &body[12..], aad }).ok()).ok_or_else(|| anyhow!("that secret doesn't open it"))
}

pub fn decode_key(text: &str) -> Result<[u8; 32]> {
    B64U.decode(text.as_bytes())?.try_into().map_err(|_| anyhow!("a key is 32 bytes"))
}

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

/// What signs as a device: its key, or where that can't be read (a browser's WebCrypto key), what signs with it.
pub trait Signs {
    /// Its Ed25519 public key.
    fn public(&self) -> [u8; 32];
    fn sign_bytes(&self, bytes: &[u8]) -> impl Future<Output = Result<[u8; 64]>>;
}

impl Signs for SigningKey {
    fn public(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        Ok(self.sign(bytes).to_bytes())
    }
}

/// `sign`, by what signs.
pub async fn sign_by(signs: &impl Signs, domain: &str, v: &Value) -> Result<[u8; 64]> {
    signs.sign_bytes(&signed_bytes(domain, v)).await
}

/// `signed_request`, by what signs.
pub async fn signed_request_by(signs: &impl Signs, domain: &str, mut fields: Value, at: u64) -> Result<Value> {
    fields["at"] = json!(at);
    fields["by"] = json!(HEXLOWER.encode(&signs.public()));
    let sig = sign_by(signs, domain, &fields).await?;
    fields["sig"] = json!(B64U.encode(&sig));
    Ok(fields)
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

/// How long to wait before signing again a request (signed `at`) the relay refused as `replayed`: a few milliseconds that
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
/// signed or from `now`, whichever is sooner, and not signed in the future. Who signed it is for the caller to check: it counts only if they're a device of the network.
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

/// What an `epoch` entry says of its key, so a device takes a key from the bucket as epoch n only if it's the one the
/// signed entry made, and app schedules of each app key they seal: HKDF-SHA256(key, "one/epoch-hash"), hex.
pub fn epoch_hash(key: &[u8; 32]) -> String {
    HEXLOWER.encode(&hkdf(key, b"one/epoch-hash"))
}

/// Whom an epoch's or a Vault's key is also kept sealed to, under a recovery key other than the one it was first sealed
/// to: the name beside `recovery`, by that key's twin (`twin_public`).
pub fn recovery_to(twin: &[u8; 32]) -> String {
    format!("recovery.{}", hex(twin))
}

/// The epoch `hash` names, out of what's kept for a recovery key, opened with its twin's `secret`.
pub fn opened_recovery_epoch(secret: &[u8; 32], hash: &str, kept: &[Vec<u8>]) -> Option<[u8; 32]> {
    kept.iter().find_map(|sealed| <[u8; 32]>::try_from(open_sealed(secret, sealed).ok()?).ok().filter(|k| epoch_hash(k) == hash))
}

const HELD_DOMAIN: &str = "one/recovery-held";

/// What's signed for a copy of the recovery key kept for a device: who it's for and what's sealed, so anyone may create
/// an object at its name (the URL signer lets any endpoint) and only the recovery key makes one that counts.
fn held_signed(to: &str, sealed: &[u8]) -> Value {
    serde_json::json!({ "to": to, "sealed": B64U.encode(sealed) })
}

/// A copy of the recovery key, sealed to the device `to` (its key in hex) on the machine with its file, as it's kept:
/// the recovery key's signature, then what's sealed.
pub fn held_copy(recovery: &SigningKey, to: &str, sealed: &[u8]) -> Vec<u8> {
    [sign(recovery, HELD_DOMAIN, &held_signed(to, sealed)).as_slice(), sealed].concat()
}

/// What's sealed in a copy kept for `to`, if `recovery` (its public half) signed it: none for anything else there.
pub fn held_sealed<'a>(recovery: &[u8; 32], to: &str, copy: &'a [u8]) -> Option<&'a [u8]> {
    let (signature, sealed) = (copy.get(..64)?, copy.get(64..)?);
    verify(recovery, HELD_DOMAIN, &held_signed(to, sealed), signature).ok().map(|()| sealed)
}

/// Seals an object under an epoch's key (or a key from it) and its name: `FORMAT` ‖ epoch (u32, big-endian) ‖ nonce ‖
/// AES-256-GCM(key, nonce, bytes, aad = `FORMAT` ‖ name), so it can't be read, changed or passed off as another object,
/// and a reader knows which epoch's key opens it before trying.
pub fn seal(epoch: u32, key: &[u8; 32], name: &str, bytes: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = NUMBERED.to_vec();
    out.extend(epoch.to_be_bytes());
    out.extend(nonce);
    out.extend(Aes256Gcm::new(key.into()).encrypt(&nonce.into(), Payload { msg: bytes, aad: &[NUMBERED, name.as_bytes()].concat() }).expect("encrypts"));
    out
}

/// What was sealed as an object, by its first byte, which isn't random there: an epoch's top byte is 0 in anything
/// sealed before formats were numbered. → what's authenticated with its format, and what follows its version.
fn object_form(sealed: &[u8]) -> (&'static [u8], &[u8]) {
    forms(sealed).next().expect("always at least the form with no version")
}

pub fn epoch_of(sealed: &[u8]) -> Result<u32> {
    let (_, body) = object_form(sealed);
    ensure!(body.len() >= 4 + 12 + 16, "too short to be sealed");
    Ok(u32::from_be_bytes(body[..4].try_into().expect("4 bytes")))
}

/// Opens what `seal` sealed, with its epoch's key.
pub fn open(key: &[u8; 32], name: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    epoch_of(sealed).with_context(|| format!("{name} isn't sealed"))?;
    let (aad, body) = object_form(sealed);
    let nonce: [u8; 12] = body[4..16].try_into().expect("12 bytes");
    Aes256Gcm::new(key.into()).decrypt(&nonce.into(), Payload { msg: &body[16..], aad: &[aad, name.as_bytes()].concat() }).map_err(|_| anyhow!("{name} didn't open with its key"))
}

/// The code a new device shows, which a device of its user types, scans or compares to approve it: the first 16 base32
/// characters (80 bits) of the sha256 of its key and seal key, so whoever passes on its request can't make another key
/// that shows it.
pub fn join_code(device: &Endpoint) -> String {
    BASE32_NOPAD.encode(&sha256(&[device.key, device.seal].concat()))[..16].to_string()
}

/// A join code in fours, as it's read.
pub fn spaced_code(code: &str) -> String {
    code.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c)).collect::<Vec<_>>().join(" ")
}

/// How much of a join code is compared where the request's own sign-in names its key (`settings::Joining`): the rest is
/// there for one that can't show that, as 40 bits alone can be found to match, offline, in minutes.
pub const JOIN_CODE_SHORT: usize = 8;

/// A newcomer's proof that their invite's secret was theirs: HKDF-SHA256(secret, "one/invite\0" ‖ their device's
/// key ‖ its seal key), hex.
pub fn invite_proof(secret: &[u8], device: &Endpoint) -> String {
    HEXLOWER.encode(&hkdf(secret, &[&b"one/invite\0"[..], &device.key, &device.seal].concat()))
}

/// Whether two secrets are the same, in time that doesn't depend on where they differ.
pub fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// A passkey's device keys, from its PRF output: (the Ed25519 seed it signs with, the X25519 secret sealed to). No build
/// reads a passkey's PRF output now, so nothing here is one's device; this stays because copies already sealed to a
/// passkey listed before that must still open (`tests/sealed.rs`).
pub fn passkey_keys(prf: &[u8]) -> (SigningKey, [u8; 32]) {
    let seed = hkdf(prf, b"one/passkey-device/sign");
    (SigningKey::from_bytes(&seed), hkdf(prf, b"one/passkey-device/seal"))
}

/// The network's recovery key, 64 hex characters, from the file at `at` on the machine of whoever runs this, or stdin
/// for `-`: never typed or pasted into a chat, so one given in a file's place is refused, and never printed.
pub fn recovery_key_in(at: &str) -> Result<String> {
    let looks_like_key = |t: &str| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(!looks_like_key(at.trim()), "give the file the recovery key is in, never the key itself");
    let text = match at {
        "-" => std::io::read_to_string(std::io::stdin())?,
        path => std::fs::read_to_string(path).with_context(|| format!("couldn't read {path}"))?,
    };
    let key = text.trim().to_ascii_lowercase();
    ensure!(looks_like_key(&key), "that file doesn't hold a recovery key: 64 hex characters");
    Ok(key)
}

/// The next recovery key, for replacing the one in use (`devices::cli`): the one already in the file at `at`, to finish
/// a replacement that stopped, or a new one, written there only if `write`, readable by this user alone.
pub fn next_recovery_key(at: &str, write: bool) -> Result<String> {
    if std::path::Path::new(at).exists() {
        return recovery_key_in(at);
    }
    let key = HEXLOWER.encode(&rand::random::<[u8; 32]>());
    if write {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(at).with_context(|| format!("couldn't write the new key to {at}"))?, format!("{key}\n").as_bytes())?;
    }
    Ok(key)
}

pub fn unhex(s: &str) -> Result<[u8; 32]> {
    HEXLOWER.decode(s.as_bytes()).ok().and_then(|b| b.try_into().ok()).context("not a key: 64 hex characters")
}

mod hex32 {
    pub fn serialize<S: serde::Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&data_encoding::HEXLOWER.encode(v))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        crate::unhex(&s).map_err(serde::de::Error::custom)
    }
}

pub mod b64key {
    pub fn serialize<S: serde::Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&data_encoding::BASE64URL_NOPAD.encode(v))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        data_encoding::BASE64URL_NOPAD.decode(s.as_bytes()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| serde::de::Error::custom("a key is 32 bytes, base64url"))
    }
}

/// The core's own steps and databases (`CoreDb`), beside apps' in each scope's folder, where no app can be called it.
/// Apps never see them. The core's keys are derived from it too (`keys::app_key`), so it stays.
pub const APP: &str = "node";

/// The core's own databases of a scope, under `APP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreDb {
    /// Its scope database, whose steps are who's who, permissions and keys (`steps`).
    Scope,
    /// Its frozen log: history, kept for backups and read by nothing else. The scope database's steps are sealed with
    /// its key, which they took over.
    Log,
    /// Its Vault's items (`vault`).
    Vault,
    /// The blobs its steps name (`bucket::blob_name`).
    Blobs,
    /// AI Gateway's rows: a retryKey's answers and the default models (`ai_gateway::SCHEMA`).
    AiGateway,
}

impl CoreDb {
    const ALL: [CoreDb; 5] = [CoreDb::Scope, CoreDb::Log, CoreDb::Vault, CoreDb::Blobs, CoreDb::AiGateway];

    pub const fn name(self) -> &'static str {
        match self {
            CoreDb::Scope => "scope",
            CoreDb::Log => "log",
            CoreDb::Vault => "vault",
            CoreDb::Blobs => "blobs",
            CoreDb::AiGateway => "ai-gateway",
        }
    }

    /// Which it is, if `app`'s `database` is one.
    pub fn of(app: &str, database: &str) -> Option<CoreDb> {
        CoreDb::ALL.into_iter().find(|c| app == APP && database == c.name())
    }


    /// What a user calls it.
    pub fn said(self) -> &'static str {
        match self {
            CoreDb::Scope => "who's who",
            CoreDb::Log => "history",
            CoreDb::Vault => "Vault",
            CoreDb::Blobs => "files",
            CoreDb::AiGateway => "AI Gateway",
        }
    }
}

/// What an app's own database is called: SQLite's name for a connection's first.
pub const APP_DATABASE: &str = "main";

#[cfg(test)]
mod tests {
    use crate::*;

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

    /// What's sealed in `FORMAT`, written here by hand, so the format is stated apart from the code that reads it.
    fn gcm(key: &[u8; 32], nonce: &[u8; 12], msg: &[u8], aad: &[u8]) -> Vec<u8> {
        cipher(key).encrypt(nonce.into(), Payload { msg, aad }).unwrap()
    }

    /// Sealed to `to` by hand, in `format`, from a share whose first byte is `first`, if any.
    fn by_hand_to(to: &[u8; 32], format: &[u8], first: Option<u8>, plain: &[u8]) -> Vec<u8> {
        loop {
            let ephemeral: [u8; 32] = rand::random();
            let share = public_of(&ephemeral);
            if first.is_some_and(|f| share[0] != f) {
                continue;
            }
            let key = hkdf_salted(&agree(&ephemeral, to), &[&share[..], to].concat(), b"one/sealed-to");
            return [format, &share[..], &gcm(&key, &[0; 12], plain, format)].concat();
        }
    }

    #[test]
    fn a_numbered_format_opens_and_what_came_before_still_does_whatever_its_first_byte() {
        let key: [u8; 32] = rand::random();
        let nonce: [u8; 12] = rand::random();
        let aad = [NUMBERED, b"a/b/1"].concat();
        let object = [NUMBERED, &7u32.to_be_bytes(), &nonce, &gcm(&key, &nonce, b"step", &aad)].concat();
        assert_eq!(epoch_of(&object).unwrap(), 7);
        assert_eq!(open(&key, "a/b/1", &object).unwrap(), b"step");
        assert!(open(&key, "a/b/2", &object).is_err(), "only as itself");
        assert!(open(&key, "a/b/1", &object[1..]).is_err(), "not with its version taken off");
        let before = [&7u32.to_be_bytes()[..], &nonce, &gcm(&key, &nonce, b"step", b"a/b/1")].concat();
        assert_eq!(epoch_of(&before).unwrap(), 7);
        assert_eq!(open(&key, "a/b/1", &before).unwrap(), b"step");
        assert!(open(&key, "a/b/1", &[NUMBERED, &before[..]].concat()).is_err(), "nor with one put on");

        let (secret, public) = twin(&rand::random());
        let numbered = by_hand_to(&public, NUMBERED, None, b"an epoch");
        assert_eq!(open_sealed(&secret, &numbered).unwrap(), b"an epoch");
        assert!(open_sealed(&secret, &numbered[1..]).is_err(), "not with its version taken off");
        let unlucky = by_hand_to(&public, UNNUMBERED, Some(FORMAT), b"an epoch");
        assert_eq!(open_sealed(&secret, &unlucky).unwrap(), b"an epoch", "sealed before, and starting with the version by chance");
        assert!(open_sealed(&secret, &[NUMBERED, &by_hand_to(&public, UNNUMBERED, None, b"x")[..]].concat()).is_err(), "not with a version put on");

        let under = hkdf(b"a token", b"one/sealed-under");
        let unlucky_nonce = [FORMAT, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        assert_eq!(open_under(b"a token", &[NUMBERED, &nonce, &gcm(&under, &nonce, b"copy", NUMBERED)].concat()).unwrap(), b"copy");
        assert_eq!(open_under(b"a token", &[&unlucky_nonce[..], &gcm(&under, &unlucky_nonce, b"copy", UNNUMBERED)].concat()).unwrap(), b"copy");

        let app = hkdf(&key, &app_key_info("tally"));
        let made: [u8; 32] = rand::random();
        assert_eq!(open_app_key(&key, "tally", &[NUMBERED, &nonce, &gcm(&app, &nonce, &made, NUMBERED)].concat()).unwrap(), made);
        assert_eq!(open_app_key(&key, "tally", &[&unlucky_nonce[..], &gcm(&app, &unlucky_nonce, &made, UNNUMBERED)].concat()).unwrap(), made);
    }

    #[test]
    fn a_rename_changes_nothing_a_thing_was_sealed_under() {
        let plain = |app, database| Permanent { app, database, key: database };
        assert_eq!(permanent("tally", "main"), plain("tally", "main"), "what was never renamed is its own name");
        let core = permanent(crate::APP, crate::CoreDb::Scope.name());
        assert_eq!((core.database, core.key), ("scope", "log"), "the scope database is where it was, under the log's key");
        let renamed = [Renamed { app: "counter", database: None, first_app: "tally", first_database: None, key_only: false }, Renamed { app: "notes", database: Some("pages"), first_app: "notes", first_database: Some("main"), key_only: false }];
        assert_eq!(permanent_in(&renamed, "counter", "main"), plain("tally", "main"), "an app renamed stays where it was sealed");
        assert_eq!(permanent_in(&renamed, "counter", ""), plain("tally", ""), "as does its key");
        assert_eq!(permanent_in(&renamed, "notes", "pages"), plain("notes", "main"), "a database renamed");
        assert_eq!(permanent_in(&renamed, "notes", "other"), plain("notes", "other"), "and not its neighbour");
        assert_eq!(permanent_in(&renamed, "tally", "main"), plain("tally", "main"), "its first name is still that name");
        let both = [Renamed { app: "counter", database: None, first_app: "tally", first_database: None, key_only: false }, Renamed { app: "counter", database: Some("pages"), first_app: "tally", first_database: Some("main"), key_only: false }];
        assert_eq!(permanent_in(&both, "counter", "pages"), plain("tally", "main"), "a database renamed in an app renamed, whichever row comes first");
    }

}

pub type Key = [u8; 32];

/// A sha256: a step's (of its header), or a network's root.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, std::hash::Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub const fn from_bytes(bytes: [u8; 32]) -> Hash {
        Hash(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }
    pub fn from_hex(s: &str) -> Result<Hash> {
        Ok(Hash(unhex(s)?))
    }
}

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl std::fmt::Debug for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..12])
    }
}

pub fn hex(key: &Key) -> String {
    HEXLOWER.encode(key)
}

/// A short, readable form of a key for users and logs.
pub fn short(key: &Key) -> String {
    hex(key)[..8].to_string()
}
