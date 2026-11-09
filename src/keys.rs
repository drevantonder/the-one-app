//! Keys as devices hold them: each endpoint's own key, each scope's numbered epochs sealed to the endpoints that may
//! open it, what's sealed under an epoch, signed requests, and the keys a caller hands a node for one call.
//! `cloudflare/shared/crypto.js` writes the same formats, and `tests/contracts.rs` checks they agree.
//!
//! Every client is a device with its own key pair, never shared by a user's devices: a native app's in the Keychain,
//! keystore or Credential Locker, a browser's a non-extractable WebCrypto key in IndexedDB, a local agent's its
//! machine's. A new device gets in only when one of its user's devices approves it, from a code or QR that shows
//! which device it is, which seals it the keys of the scopes it may open. A user who lost a device is let back in by
//! another, by a copy sealed under a passkey's PRF (optional for most, kept because synced passkeys make it
//! self-service), or by the network's recovery key, kept offline by an admin, each use of it an entry in the log.
//! A scope gets a new epoch when a user, or a device that didn't sign itself out (lost or stolen), loses the right
//! to open it, never on sign-out, where the device deletes its own key: the keys are sealed to everyone who still
//! belongs. Epochs never go down along a database's positions (`src/database/stored.rs`).
//!
//! ```text
//! <folder>/_epochs/<n>/<endpoint key hex>   epoch n's key, sealed to that endpoint's seal key (`keys::seal_to`)
//! <folder>/_epochs/<n>/recovery             and to the network's recovery key
//! <folder>/<app>/_key/<n>                    the app's key in epoch n, sealed under epoch n's key (`keys::seal_app_key`)
//! <folder>/<app>/_app_schedules/<endpoint>/<n>  and sealed to an endpoint that runs its app schedules (`keys::seal_to`)
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
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::collections::BTreeMap;

/// How old a signed request may be when it's checked, in milliseconds, either way.
pub const FRESH: u64 = 60_000;

/// The first byte of everything sealed since formats were numbered, and authenticated with it, so taking it off, or
/// putting it on what was sealed before, opens nothing. What was sealed before has no version and opens forever
/// (`tests/sealed/`). A later format is one more, and reads one more in `database::READS`: an older build takes what it
/// can't read for junk and passes over it, and what it kept is read again (`steps::read_again`).
pub const FORMAT: u8 = 1;
/// What's authenticated with a format, before what each seal adds.
pub(crate) const NUMBERED: &[u8] = &[FORMAT];
const UNNUMBERED: &[u8] = &[];

/// The ways `sealed` may have been written, newest first: what's authenticated with that format, and what follows its
/// version. What was sealed before can start with `FORMAT` by chance, where it starts with random bytes, so each is
/// tried: AES-GCM says which it was.
pub(crate) fn forms(sealed: &[u8]) -> impl Iterator<Item = (&'static [u8], &[u8])> {
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

/// What a native app's device seals the last answers of its screens' calls under (client/src/device.rs): its own, from
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
    use crate::database::scopes::{APP, CoreDb};
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

/// An app's key in epoch `n` of a scope, from its copy in the bucket (`app_key_name`), sealed under that epoch's
/// key. The first to need it makes it, at random, unless `make` is false (an epoch nothing was written under yet,
/// where a reader has nothing to open): whichever is written first is the key.
pub async fn app_key(bucket: &impl crate::bucket::Bucket, epoch_key: &[u8; 32], scope: &str, app: &str, n: u32, make: bool) -> Result<Option<[u8; 32]>> {
    let name = app_key_name(scope, app, n);
    if let Some(o) = bucket.get(&name).await? {
        return open_app_key(epoch_key, app, &o.bytes).map(Some);
    }
    if !make {
        return Ok(None);
    }
    let key = rand::random();
    if bucket.create(&name, &seal_app_key(epoch_key, app, &key)).await? {
        return Ok(Some(key));
    }
    let made = bucket.get(&name).await?.context("an app's key went as it was made")?;
    open_app_key(epoch_key, app, &made.bytes).map(Some)
}

/// A database's keys in a scope, by epoch, from `epochs`, the scope's epochs' keys: each from its app's key there
/// (`app_key`), the newest epoch's made if `make` and it isn't yet. An epoch with no app key yet has none.
pub async fn database_keys(bucket: &impl crate::bucket::Bucket, scope: &str, app: &str, database: &str, epochs: &BTreeMap<u32, [u8; 32]>, make: bool) -> Result<BTreeMap<u32, [u8; 32]>> {
    let newest = epochs.keys().next_back().copied();
    let mut out = BTreeMap::new();
    for (n, epoch) in epochs {
        if let Some(key) = app_key(bucket, epoch, scope, app, *n, make && Some(*n) == newest).await? {
            out.insert(*n, database_key(&key, app, database));
        }
    }
    Ok(out)
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

/// `seal_to` with no version: what a push carries, since a phone's shell, which opens it with no window, changes only
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

/// Seals `plain` under a secret (a passkey's PRF output, a refresh token): `FORMAT` ‖ nonce ‖ AES-256-GCM.
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

/// The folder of a scope's epochs, one folder each (`epoch_name`).
pub fn epochs_folder(scope: &str) -> String {
    format!("{}/_epochs", crate::bucket::folder(scope))
}

/// Where epoch `n` of a scope is kept sealed to `to`: an endpoint key, or `recovery`.
pub fn epoch_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/{n}/{to}", epochs_folder(scope))
}

/// What an `epoch` entry says of its key, so a device takes a key from the bucket as epoch n only if it's the one the
/// signed entry made, and app schedules of each app key they seal: HKDF-SHA256(key, "one/epoch-hash"), hex.
pub fn epoch_hash(key: &[u8; 32]) -> String {
    HEXLOWER.encode(&hkdf(key, b"one/epoch-hash"))
}

/// The folder of an app's keys in a scope, one by epoch (`app_key_name`).
pub fn app_keys_folder(scope: &str, app: &str) -> String {
    format!("{}/{}/_key", crate::bucket::folder(scope), permanent(app, "").app)
}

/// Where an app's key in epoch `n` of a scope is kept, sealed under that epoch's key.
pub fn app_key_name(scope: &str, app: &str, n: u32) -> String {
    format!("{}/{n}", app_keys_folder(scope, app))
}

/// Where an app's key in epoch `n` of a scope is kept sealed to an endpoint that runs the app's schedules there: its
/// key in hex.
pub fn app_schedules_key_name(scope: &str, app: &str, endpoint: &str, n: u32) -> String {
    format!("{}/{}/_app_schedules/{endpoint}/{n}", crate::bucket::folder(scope), permanent(app, "").app)
}

/// Where a build from before app schedules had their name sealed it: read when it isn't under its own name.
pub fn app_schedules_key_name_before(scope: &str, app: &str, endpoint: &str, n: u32) -> String {
    format!("{}/{}/_grants/{endpoint}/{n}", crate::bucket::folder(scope), permanent(app, "").app)
}

/// Where a scope's Vault secret key `n` is kept sealed to `to`: a machine's key granted it, or `recovery`. Its public
/// half is in the scope's `vault` entry, and what's sealed to that opens only on those machines.
pub fn vault_key_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/{n}/{to}", vault_keys_folder(scope))
}

/// Whom an epoch's or a Vault's key is also kept sealed to, under a recovery key other than the one it was first sealed
/// to: the name beside `recovery`, by that key's twin (`twin_public`).
pub fn recovery_to(twin: &[u8; 32]) -> String {
    format!("recovery.{}", hex(twin))
}

/// Where a recovery key whose twin is `twin` finds epoch `n` of a scope: under its own name first, as it was handed
/// over to it, then at `recovery`.
pub fn recovery_epoch_names(scope: &str, n: u32, twin: &[u8; 32]) -> [String; 2] {
    [epoch_name(scope, n, &recovery_to(twin)), epoch_name(scope, n, "recovery")]
}

/// The epoch `hash` names, out of what's kept for a recovery key, opened with its twin's `secret`.
pub fn opened_recovery_epoch(secret: &[u8; 32], hash: &str, kept: &[Vec<u8>]) -> Option<[u8; 32]> {
    kept.iter().find_map(|sealed| <[u8; 32]>::try_from(open_sealed(secret, sealed).ok()?).ok().filter(|k| epoch_hash(k) == hash))
}

/// Where the network's recovery key (`recovery`, its public half) is kept sealed to a device (`to`) of someone who holds
/// it, so a new recovery key starts with none of the old one's.
pub fn recovery_held_name(recovery: &str, to: &str) -> String {
    format!("{}/{recovery}/{to}", recovery_held_folder())
}

/// Where each recovery key's copies are, one folder each.
pub fn recovery_held_folder() -> String {
    format!("{}/_recovery", crate::bucket::folder("global"))
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

/// The folder of a scope's Vault secret keys, one folder each (`vault_key_name`).
pub fn vault_keys_folder(scope: &str) -> String {
    format!("{}/_vault", crate::bucket::folder(scope))
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
    Ok(B64U.encode(&seal_to(to, &serde_json::to_vec(handoff)?)?))
}

/// Opens a handoff sealed to this node, with its seal secret.
pub fn open_handoff(secret: &[u8; 32], sealed: &str) -> Result<Handoff> {
    let bytes = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("the keys handed aren't base64url"))?;
    let plain = open_sealed(secret, &bytes).context("the keys handed weren't sealed to this device")?;
    serde_json::from_slice(&plain).context("the keys handed aren't a handoff")
}

/// What a node with no sign-in to pass on (the Cloudflare node, or a machine's passing on a call it was handed keys
/// for) asks another node to run a call with (`src/node/net.rs`): the call, naming `scopes`, the scopes it reaches,
/// the user it's for, and the keys of those scopes it was handed, sealed to that node's `seal` key for calls from
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
    Ok(json!({ "kind": crate::protocol::message::ASK, "body": { "call": call } }))
}

/// A key a call lacked, which its caller hands it when it sends the call again: what a node answers with 428.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Need {
    pub scope: String,
    pub epoch: u32,
    pub app: String,
    pub database: String,
}

/// What a request runs, which decides what a client hands a node with it (`may_hand`), and what it remembers each kind of
/// request needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// An app's call: `/v1/call` for it, or its own request (`/<app>/…`).
    Call(String),
    /// An app's screens.
    Screens(String),
    /// Any other path.
    Path(String),
}

impl Route {
    /// The route of a request for `path` (a query is left off); `call_app` names the app of a `/v1/call`, from its body.
    pub fn of(path: &str, call_app: impl FnOnce() -> String) -> Route {
        use crate::protocol::path;
        let at = path.split('?').next().unwrap_or(path);
        if let Some(app) = path::app_request(at) {
            return Route::Call(app.to_string());
        }
        match at.strip_prefix(path::SCREENS).and_then(|s| s.split('/').next()) {
            Some(app) => Route::Screens(app.to_string()),
            None if at == path::CALL => Route::Call(call_app()),
            None => Route::Path(at.to_string()),
        }
    }

    /// The app it runs, for a call or its screens.
    fn app(&self) -> Option<&str> {
        match self {
            Route::Call(app) | Route::Screens(app) => Some(app).filter(|a| !a.is_empty()).map(String::as_str),
            Route::Path(_) => None,
        }
    }
}

impl std::fmt::Display for Route {
    /// What a client remembers a kind of request by.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Call(app) => write!(f, "call {app}"),
            Route::Screens(app) => write!(f, "{}{app}", crate::protocol::path::SCREENS),
            Route::Path(path) => f.write_str(path),
        }
    }
}

/// Whether a client hands a node the key of `need`'s database with a kind of request (`route`), when the node asks (428):
/// only what that kind of request can use. One that runs an app, that app's databases; an
/// MCP request (`/mcp`), which may run any, or widgets (and the tool that shows them) and badges, which run every app's, apps' own databases; and each
/// of those the node's Vault and log of a scope too (which fill in its items, and say who's who there), and `global`'s
/// blobs, where apps' deployed code is (a node that keeps nothing, the Cloudflare node, reads it there). The list of
/// apps, which says what each does from its code, the log and that code. The Vault's routes, its Vault and log; any
/// other, the log alone.
///
/// `global`'s blobs are those its entries name (deploys' bundles, releases, apps' icons), which everyone in the network
/// may read already: any of them may add any app, and read its code. Another scope's are its own users' files, and
/// never go with a request.
pub fn may_hand(route: &Route, need: &Need) -> bool {
    use crate::{database::scopes::CoreDb::{self, Blobs as BLOBS, Log as LOG, Vault as VAULT}, protocol::path};
    let node = |of: &[CoreDb]| CoreDb::of(&need.app, &need.database).is_some_and(|c| of.contains(&c));
    let apps = need.app != crate::database::scopes::APP && need.database == crate::database::scopes::APP_DATABASE;
    let code = need.scope == "global" && node(&[BLOBS]);
    match (route.app(), route) {
        (Some(app), _) => need.app == app || node(&[VAULT, LOG]) || code,
        (None, Route::Path(p)) if matches!(p.as_str(), path::MCP | path::BADGES) || p == path::WIDGETS || p.starts_with(&format!("{}/", path::WIDGETS)) => apps || node(&[VAULT, LOG]) || code,
        (None, Route::Path(p)) if p == path::APPS || p == path::OPENAPI => node(&[LOG]) || code,
        (None, Route::Path(p)) if p == path::VAULT || p.starts_with(&format!("{}/", path::VAULT)) => node(&[VAULT, LOG]),
        (None, _) => node(&[LOG]),
    }
}

/// What `may_hand` lets a client hand with `route`, as the OpenAPI document says it: an app's own database (`app`, any
/// app's on what draws several), the Vault's, the log's, and the code apps run from, in `global`'s blobs.
pub fn hands(route: &Route) -> Vec<&'static str> {
    use crate::database::scopes::{APP, APP_DATABASE, CoreDb};
    let need = |scope: &str, app: &str, database: &str| Need { scope: scope.into(), epoch: 1, app: app.into(), database: database.into() };
    let kinds = [("app", need("user:x", route.app().unwrap_or("x"), APP_DATABASE)), ("vault", need("user:x", APP, CoreDb::Vault.name())), ("log", need("user:x", APP, CoreDb::Log.name())), ("code", need("global", APP, CoreDb::Blobs.name()))];
    kinds.into_iter().filter(|(_, need)| may_hand(route, need)).map(|(kind, _)| kind).collect()
}

/// The oldest epoch of `database`'s key a reader needs, of a scope whose newest epoch is `newest`: the one its newest
/// copy is under, as its newest hint (`hint`, if it has one) says, which holds all before it; every epoch from the first
/// while it has no copy; the newest if it has no hint, being new. Blobs have neither: each is kept under the epoch it
/// was put in, so every epoch from the first. The log, a scope's steps, needs every epoch from the first too: every
/// reader replays it from the first (`steps::Truth::read`), whatever copies there are. Nodes ask for no older, and
/// clients hand no older.
pub fn oldest_needed(database: &str, hint: Option<&[u8]>, newest: u32) -> u32 {
    use crate::database::scopes::CoreDb;
    if [CoreDb::Blobs, CoreDb::Log].map(CoreDb::name).contains(&database) {
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
/// sealed to the one the network's entries list for that node.
pub fn asked_for(route: &Route, node: &str, answer: &Value) -> Option<Vec<Need>> {
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
    /// Approved, in the network whose root (hex) the device that approved it named, if it named one: the only
    /// one it then takes; and once who's who is written as steps, where that device read them to, which this one
    /// starts from (`database::steps::Welcome`), if it said. It's in once the relay lets it.
    Approved { root: Option<String>, welcome: Option<String> },
    /// Gone from auth (approved and cleared, turned down, or waited too long): it's in if the relay lets it, else it
    /// asks again.
    Gone,
    /// Still waiting, or auth didn't answer.
    Waiting,
}

pub fn waited(status: u16, said: &Value) -> Waited {
    match status {
        401 => Waited::SignedOut,
        404 => Waited::Gone,
        200..=299 if said["approved"] == true => Waited::Approved {
            root: said["root"].as_str().filter(|g| g.len() == 64 && g.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).map(str::to_string),
            welcome: said["welcome"].as_str().filter(|w| crate::database::steps::Welcome::parse(w).is_some()).map(str::to_string),
        },
        _ => Waited::Waiting,
    }
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

/// A passkey's device keys, from its PRF output: (the Ed25519 seed it signs with, the X25519 secret sealed to).
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

    fn call(app: &str) -> Route {
        Route::Call(app.into())
    }

    fn at(path: &str) -> Route {
        Route::Path(path.into())
    }

    #[test]
    fn a_node_is_handed_only_the_keys_a_kind_of_request_uses() {
        let need = |app: &str, database: &str| Need { scope: "global".into(), epoch: 1, app: app.into(), database: database.into() };
        assert!(may_hand(&call("tally"), &need("tally", "main")) && !may_hand(&call("tally"), &need("notes", "main")));
        assert!(may_hand(&at("/v1/steps"), &need("node", "log")) && !may_hand(&at("/v1/steps"), &need("tally", "main")) && !may_hand(&at("/v1/state"), &need("node", "vault")));
        assert!(may_hand(&at("/v1/vault"), &need("node", "vault")) && !may_hand(&at("/v1/vault"), &need("tally", "main")));
        // Apps' code, `global`'s blobs, goes to a node running an app (the Cloudflare node keeps none); a scope's own
        // blobs, its users' files, never.
        let blobs = |scope: &str| Need { scope: scope.into(), epoch: 1, app: "node".into(), database: "blobs".into() };
        assert!(may_hand(&call("tally"), &blobs("global")) && may_hand(&Route::Screens("tally".into()), &blobs("global")) && may_hand(&at("/v1/badges"), &blobs("global")) && may_hand(&at("/v1/widgets/show"), &blobs("global")) && may_hand(&at("/v1/apps"), &blobs("global")) && may_hand(&at(crate::protocol::path::OPENAPI), &blobs("global")));
        assert!(!may_hand(&at("/v1/apps"), &need("tally", "main")) && !may_hand(&at("/v1/apps"), &blobs("user:ab")));
        assert!(!may_hand(&call("tally"), &blobs("user:ab")) && !may_hand(&at("/mcp"), &blobs("group:cd")) && !may_hand(&at("/v1/state"), &blobs("global")));
        assert_eq!(oldest_needed("main", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 2, "from the epoch its newest copy is under");
        assert_eq!(oldest_needed("log", Some(br#"{"at":9,"copy":7,"epoch":2}"#), 3), 1, "a scope's steps are read from the first");
        assert_eq!(oldest_needed("log", None, 3), 1, "a scope's steps are read from the first, before any hint");
        assert_eq!(oldest_needed("blobs", None, 3), 1, "a blob is kept under the epoch it was put in");
    }

    #[test]
    fn a_nodes_428_asks_only_for_what_the_request_uses_from_the_node_that_answered() {
        let node = "ab".repeat(32);
        let need = |app: &str, database: &str| json!({ "scope": "global", "epoch": 1, "app": app, "database": database });
        let every = |n: usize| (0..n).map(|i| need(&format!("app{i}"), "main")).collect::<Vec<_>>();
        let twice = json!({ "to": node, "need": [need("tally", "main"), need("tally", "main"), need("node", "log")] });
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("tally", "main")] })).is_some());
        assert!(asked_for(&call("tally"), &node, &twice).is_some_and(|n| n.len() == 2), "a key asked for twice is handed once");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": "cd".repeat(32), "need": [need("tally", "main")] })).is_none(), "only the node that answered");
        assert!(asked_for(&call("tally"), &node, &json!({ "need": [need("tally", "main")] })).is_none(), "naming itself");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [] })).is_none() && asked_for(&call("tally"), &node, &json!({ "to": node, "need": null })).is_none());
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("notes", "main")] })).is_none(), "another app's");
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [need("tally", "main"), { "scope": "global", "epoch": "1", "app": "tally", "database": "main" }] })).is_none());
        assert!(asked_for(&call("tally"), &node, &json!({ "to": node, "need": [{ "scope": "global", "epoch": -1, "app": "tally", "database": "main" }] })).is_none());
        assert!(asked_for(&at("/v1/badges"), &node, &json!({ "to": node, "need": every(MOST_NEEDED) })).is_some());
        assert!(asked_for(&at("/v1/badges"), &node, &json!({ "to": node, "need": every(MOST_NEEDED + 1) })).is_none());
    }

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
        let core = permanent(crate::database::scopes::APP, crate::database::scopes::CoreDb::Scope.name());
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
