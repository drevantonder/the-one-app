//! The kernel services (`kernel/`: auth, vault, schedules) as nodes meet them: the formats they agree on
//! (`kernel/shared/crypto.js` writes the same ones, and `tests/contracts.rs` checks they do), checking access tokens,
//! asking the vault for keys and bucket URLs, and building and running the servers in workerd.

#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, ensure};
#[cfg(not(target_family = "wasm"))]
use anyhow::bail;
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

// Keys.

fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

/// An app's key in a scope is random, kept sealed under the scope's key: nonce ‖ AES-256-GCM(HKDF(scope key,
/// "one/app\0<app>"), nonce, app key). Erasing the app deletes it, which destroys the key and so the app's data.
pub fn seal_app_key(scope_key: &[u8; 32], app: &str, app_key: &[u8; 32]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher(&hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())).encrypt(&nonce.into(), &app_key[..]).expect("encrypting in memory"));
    out
}

/// The key an app's data in a scope was sealed with before app keys were random: derived from the scope's key. Only
/// for data written then (`db::app_key`).
pub fn derived_app_key(scope_key: &[u8; 32], app: &str) -> [u8; 32] {
    hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())
}

/// Where an app's sealed key is kept in the bucket, beside its databases: `_key` in its folder, which no database
/// can be named.
pub fn app_key_name(scope: &str, app: &str) -> String {
    format!("{}/{app}/_key", crate::bucket::folder(scope))
}

/// An app's key, from its sealed copy and its scope's key.
pub fn open_app_key(scope_key: &[u8; 32], app: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    ensure!(sealed.len() == 60, "not a sealed app key");
    let plain = cipher(&hkdf(scope_key, &[], format!("one/app\0{app}").as_bytes())).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("that scope's key doesn't open {app}'s"))?;
    plain.try_into().map_err(|_| anyhow!("an app's key is 32 bytes"))
}

/// A database's key in an app, from the app's key.
pub fn database_key(app_key: &[u8; 32], database: &str) -> [u8; 32] {
    hkdf(app_key, &[], format!("one/database\0{database}").as_bytes())
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(key.into())
}

/// The public half of an X25519 secret.
pub fn public_of(secret: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint::mul_base_clamped(*secret).0
}

/// The X25519 twin of an Ed25519 key (the recovery key), as age and backups use it: → (secret, public).
pub fn twin(seed: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let secret: [u8; 32] = Sha512::digest(seed)[..32].try_into().expect("64 bytes");
    (secret, public_of(&secret))
}

/// The X25519 twin of an Ed25519 public key.
pub fn twin_public(public: &[u8; 32]) -> Result<[u8; 32]> {
    Ok(CompressedEdwardsY(*public).decompress().context("not an Ed25519 key")?.to_montgomery().0)
}

/// Seals `plain` to an X25519 public key: share ‖ AES-256-GCM(HKDF(shared, share ‖ to), zero nonce, plain).
pub fn seal_to(to: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>> {
    let ephemeral: [u8; 32] = rand::random();
    let share = public_of(&ephemeral);
    let shared = MontgomeryPoint(*to).mul_clamped(ephemeral).0;
    ensure!(shared != [0; 32], "not a public key");
    let key = hkdf(&shared, &[&share[..], to].concat(), b"one/sealed-to");
    let mut out = share.to_vec();
    out.extend(cipher(&key).encrypt(&Default::default(), plain).map_err(|_| anyhow!("sealing failed"))?);
    Ok(out)
}

/// Opens what was sealed to the public half of `secret`.
pub fn open_sealed(secret: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 48, "not sealed");
    let share: [u8; 32] = sealed[..32].try_into()?;
    let shared = MontgomeryPoint(share).mul_clamped(*secret).0;
    let key = hkdf(&shared, &[&share[..], &public_of(secret)].concat(), b"one/sealed-to");
    cipher(&key).decrypt(&Default::default(), &sealed[32..]).map_err(|_| anyhow!("that key doesn't open it"))
}

/// The id a copy sealed under `secret` is kept by.
pub fn id_of(secret: &[u8]) -> String {
    B64U.encode(&hkdf(secret, &[], b"one/id"))
}

/// Seals `plain` under a secret (text, as the vault is given it): nonce ‖ AES-256-GCM.
pub fn seal_under(secret: &[u8], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher(&hkdf(secret, &[], b"one/sealed-under")).encrypt(&nonce.into(), plain).expect("encrypting in memory"));
    out
}

/// Opens what was sealed under `secret`.
pub fn open_under(secret: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    ensure!(sealed.len() >= 28, "not sealed");
    cipher(&hkdf(secret, &[], b"one/sealed-under")).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("that secret doesn't open it"))
}

pub fn decode_key(text: &str) -> Result<[u8; 32]> {
    B64U.decode(text.as_bytes())?.try_into().map_err(|_| anyhow!("a key is 32 bytes"))
}

// Access tokens.

/// What an access token says.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub iss: String,
    /// The person's user id.
    pub sub: String,
    /// The client it was issued to.
    pub azp: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub aud: Value,
    pub exp: u64,
    #[serde(default)]
    pub jti: String,
}

impl Claims {
    /// Whether it was issued for `resource`.
    pub fn for_resource(&self, resource: &str) -> bool {
        match &self.aud {
            Value::String(a) => a == resource,
            Value::Array(list) => list.iter().any(|a| a == resource),
            _ => false,
        }
    }
}

/// An access token's claims, if a key in `jwks` (the auth service's) signed it, `iss` issued it, and it hasn't expired
/// at `now` (milliseconds).
pub fn verify(token: &str, jwks: &Value, iss: &str, now: u64) -> Result<Claims> {
    let parts: Vec<&str> = token.split('.').collect();
    ensure!(parts.len() == 3, "not a token");
    let header: Value = serde_json::from_slice(&B64U.decode(parts[0].as_bytes())?)?;
    ensure!(header["alg"] == "EdDSA", "not a token signed with Ed25519");
    let jwk = jwks["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["kid"] == header["kid"] && k["kty"] == "OKP" && k["crv"] == "Ed25519")
        .context("signed by a key the auth service doesn't publish")?;
    let public: [u8; 32] = B64U.decode(jwk["x"].as_str().unwrap_or("").as_bytes())?.try_into().map_err(|_| anyhow!("not an Ed25519 key"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&public)?;
    let signature = ed25519_dalek::Signature::from_slice(&B64U.decode(parts[2].as_bytes())?)?;
    key.verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature).map_err(|_| anyhow!("its signature is wrong"))?;
    let claims: Claims = serde_json::from_slice(&B64U.decode(parts[1].as_bytes())?).context("its claims aren't an access token's")?;
    ensure!(claims.iss == iss, "issued by {}, not {iss}", claims.iss);
    ensure!(claims.exp * 1000 > now, "it has expired");
    Ok(claims)
}

/// The auth service at an install's address, as nodes check its tokens: its keys fetched and kept five minutes, or
/// until a token names a key that isn't among them (as `kernel/shared/token.js`'s `Issuer` does).
#[cfg(not(target_family = "wasm"))]
pub struct Issuer {
    pub iss: String,
    keys: std::sync::Mutex<Option<(Value, std::time::Instant)>>,
}

#[cfg(not(target_family = "wasm"))]
impl Issuer {
    pub fn new(address: &str) -> Issuer {
        Issuer { iss: format!("{}/api/auth", address.trim_end_matches('/')), keys: Default::default() }
    }

    /// A token's claims, as `verify` checks them, at `now` (milliseconds).
    pub fn verify(&self, token: &str, now: u64) -> Result<Claims> {
        for fresh in [false, true] {
            let kept = self.keys.lock().unwrap().clone().filter(|(_, at)| !fresh && at.elapsed() < Duration::from_secs(300));
            let jwks = match kept {
                Some((jwks, _)) => jwks,
                None => {
                    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
                    let jwks: Value = agent.get(format!("{}/jwks", self.iss)).call().context("the auth service didn't give its keys")?.body_mut().read_json()?;
                    *self.keys.lock().unwrap() = Some((jwks.clone(), std::time::Instant::now()));
                    jwks
                }
            };
            match verify(token, &jwks, &self.iss, now) {
                Err(err) if !fresh && err.to_string().contains("key the auth service") => continue,
                other => return other,
            }
        }
        unreachable!("the second try returns")
    }
}

// The vault, as nodes ask it.

/// A key a call names, and (from the vault) the key itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub scope: String,
    pub app: String,
    pub database: Option<String>,
    pub key: [u8; 32],
}

/// Presigned URLs, by object name, and until when (milliseconds) they work.
#[derive(Debug, Default, Deserialize)]
pub struct Urls {
    pub read: std::collections::BTreeMap<String, String>,
    /// Each works only with `If-None-Match: *`, so only while nothing has that name.
    pub create: std::collections::BTreeMap<String, String>,
    /// Each works only with `If-Match: <the ETag it was asked with>`.
    pub replace: std::collections::BTreeMap<String, String>,
    pub until: u64,
}

/// A token's session at the vault: whose it is.
#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub user: String,
    /// The client it's for: an OAuth client's id, or `schedules` for a scheduled run.
    pub client: Option<String>,
    /// The client's name, as its person sees it ("Chrome on Mac").
    #[serde(default)]
    pub name: Option<String>,
    pub until: u64,
}

/// A run the schedules service hands a node: `POST <node>/kernel/run`, its token as the bearer. `run` names
/// it, the same each time it's sent, so a node that leases by it runs it once.
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub run: String,
    pub app: String,
    pub schedule: String,
    pub op: String,
    pub input: Value,
    pub scope: String,
}

/// The vault at `url`, asked with a caller's access token.
#[cfg(not(target_family = "wasm"))]
pub struct Vault {
    pub url: String,
}

#[cfg(not(target_family = "wasm"))]
impl Vault {
    fn post(&self, token: &str, path: &str, body: Value) -> Result<Value> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut resp = agent.post(format!("{}{path}", self.url.trim_end_matches('/'))).header("authorization", format!("Bearer {token}")).send_json(body)?;
        let status = resp.status().as_u16();
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if status != 200 {
            bail!("the vault said: {}", out["error"].as_str().unwrap_or("nothing"));
        }
        Ok(out)
    }

    /// Whose a token is, while its session lasts: a node asks before it runs anything as its person.
    pub fn session(&self, token: &str) -> Result<Session> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut resp = agent.get(format!("{}/session", self.url.trim_end_matches('/'))).header("authorization", format!("Bearer {token}")).call()?;
        let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        ensure!(resp.status() == 200, "the vault said: {}", out["error"].as_str().unwrap_or("nothing"));
        Ok(serde_json::from_value(out)?)
    }

    /// The keys a call names: (scope, app, database?) each.
    pub fn keys(&self, token: &str, want: &[(&str, &str, Option<&str>)]) -> Result<Vec<Key>> {
        keys_given(&self.post(token, "/keys", keys_asked(want))?)
    }

    /// URLs to read and create objects named `<scope's folder>/<app>/…` (or `log/…`, the install's log), and to
    /// replace a database's `…/newest` while it's the version an ETag names: (name, etag) each.
    pub fn urls(&self, token: &str, read: &[&str], create: &[&str], replace: &[(&str, &str)]) -> Result<Urls> {
        Ok(serde_json::from_value(self.post(token, "/urls", urls_asked(read, create, replace))?)?)
    }
}

/// What a node posts to the vault's `/keys` for the keys a call names: (scope, app, database?) each.
pub fn keys_asked(want: &[(&str, &str, Option<&str>)]) -> Value {
    let keys: Vec<Value> = want.iter().map(|(s, a, d)| if let Some(d) = d { json!({ "scope": s, "app": a, "database": d }) } else { json!({ "scope": s, "app": a }) }).collect();
    json!({ "keys": keys })
}

/// The keys in the vault's answer to `/keys`.
pub fn keys_given(out: &Value) -> Result<Vec<Key>> {
    out["keys"]
        .as_array()
        .context("the vault gave no keys")?
        .iter()
        .map(|k| {
            Ok(Key {
                scope: k["scope"].as_str().context("a key's scope")?.into(),
                app: k["app"].as_str().context("a key's app")?.into(),
                database: k["database"].as_str().map(str::to_string),
                key: decode_key(k["key"].as_str().context("a key")?)?,
            })
        })
        .collect()
}

/// What a node posts to the vault's `/urls`: names to read and create, and (name, etag) to replace.
pub fn urls_asked(read: &[&str], create: &[&str], replace: &[(&str, &str)]) -> Value {
    let replace: Vec<Value> = replace.iter().map(|(name, etag)| json!({ "name": name, "etag": etag })).collect();
    json!({ "read": read, "create": create, "replace": replace })
}

#[cfg(not(target_family = "wasm"))]
mod services;
#[cfg(not(target_family = "wasm"))]
pub use services::*;
