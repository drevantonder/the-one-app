//! The kernel services (`kernel/`: auth, the relay, the URL signer) as nodes meet them: the formats they agree on
//! (`kernel/shared/crypto.js` writes the same ones, and `tests/contracts.rs` checks they do), checking access tokens,
//! and building and running the services in workerd.

#[cfg(feature = "node")]
use std::time::Duration;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::{Context, Result, anyhow, ensure};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};

// Keys.

fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    hkdf::Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

/// An app's key in a scope's epoch is random, kept sealed under the epoch's key (`keys::app_key_name`): nonce ‖
/// AES-256-GCM(HKDF(epoch key, "one/app\0<app>"), nonce, app key). Erasing the app deletes it, which destroys the key
/// and so the app's data.
pub fn seal_app_key(epoch_key: &[u8; 32], app: &str, app_key: &[u8; 32]) -> Vec<u8> {
    let nonce: [u8; 12] = rand::random();
    let mut out = nonce.to_vec();
    out.extend(cipher(&hkdf(epoch_key, &[], format!("one/app\0{app}").as_bytes())).encrypt(&nonce.into(), &app_key[..]).expect("encrypting in memory"));
    out
}

/// An app's key, from its sealed copy and its epoch's key.
pub fn open_app_key(epoch_key: &[u8; 32], app: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    ensure!(sealed.len() == 60, "not a sealed app key");
    let plain = cipher(&hkdf(epoch_key, &[], format!("one/app\0{app}").as_bytes())).decrypt(sealed[..12].into(), &sealed[12..]).map_err(|_| anyhow!("that epoch's key doesn't open {app}'s"))?;
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

/// Seals `plain` under a secret (a passkey's PRF output, a refresh token): nonce ‖ AES-256-GCM.
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
    /// The person's name, as the auth service knows them.
    #[serde(default)]
    pub name: String,
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
#[cfg(feature = "node")]
pub struct Issuer {
    pub iss: String,
    keys: std::sync::Mutex<Option<(Value, std::time::Instant)>>,
}

#[cfg(feature = "node")]
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

#[cfg(feature = "node")]
mod services;
#[cfg(feature = "node")]
pub use services::*;
