//! The network's Workers on Cloudflare (`cloudflare/`: auth, the relay, the URL signer) as nodes meet them:
//! checking access tokens, and building and running the Workers in workerd.

#[cfg(feature = "node")]
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::Deserialize;
use serde_json::Value;

/// What an access token says.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    pub iss: String,
    /// The user's id.
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
    /// The user's name, as auth knows them.
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

/// An access token's claims, if a key in `jwks` (auth's) signed it, `iss` issued it, and it hasn't expired
/// at `now` (milliseconds).
pub fn verify(token: &str, jwks: &Value, iss: &str, now: u64) -> Result<Claims> {
    let parts: Vec<&str> = token.split('.').collect();
    ensure!(parts.len() == 3, "not a token");
    let header: Value = serde_json::from_slice(&B64U.decode(parts[0].as_bytes())?)?;
    ensure!(header["alg"] == "EdDSA", "not a token signed with Ed25519");
    let jwk = published_key(jwks, &header).context("signed by a key sign-in doesn't publish")?;
    let public: [u8; 32] = B64U.decode(jwk["x"].as_str().unwrap_or("").as_bytes())?.try_into().map_err(|_| anyhow!("not an Ed25519 key"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&public)?;
    let signature = ed25519_dalek::Signature::from_slice(&B64U.decode(parts[2].as_bytes())?)?;
    key.verify_strict(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature).map_err(|_| anyhow!("its signature is wrong"))?;
    let claims: Claims = serde_json::from_slice(&B64U.decode(parts[1].as_bytes())?).context("its claims aren't an access token's")?;
    ensure!(claims.iss == iss, "issued by {}, not {iss}", claims.iss);
    ensure!(claims.exp * 1000 > now, "it has expired");
    Ok(claims)
}

/// The key in `jwks` that signed a token with this header, if sign-in publishes it.
fn published_key<'a>(jwks: &'a Value, header: &Value) -> Option<&'a Value> {
    jwks["keys"].as_array()?.iter().find(|k| k["kid"] == header["kid"] && k["kty"] == "OKP" && k["crv"] == "Ed25519")
}

/// Whether `jwks` holds the key `token` names; nothing for what isn't a token.
#[cfg(feature = "node")]
fn names_published_key(jwks: &Value, token: &str) -> Option<bool> {
    let header: Value = serde_json::from_slice(&B64U.decode(token.split('.').next()?.as_bytes()).ok()?).ok()?;
    Some(published_key(jwks, &header).is_some())
}

/// Sign-in at a network's address, as nodes check its tokens: its keys fetched and kept five minutes, or
/// until a token names a key that isn't among them (as `cloudflare/shared/token.js`'s `Issuer` does).
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
                    let jwks: Value = agent.get(format!("{}/jwks", self.iss)).call().context("sign-in didn't give its keys")?.body_mut().read_json()?;
                    *self.keys.lock().unwrap() = Some((jwks.clone(), std::time::Instant::now()));
                    jwks
                }
            };
            if fresh || names_published_key(&jwks, token) != Some(false) {
                return verify(token, &jwks, &self.iss, now);
            }
        }
        unreachable!("the second try returns")
    }
}

#[cfg(feature = "node")]
pub mod cli;
#[cfg(feature = "node")]
pub mod node;
#[cfg(feature = "node")]
mod workers;
#[cfg(feature = "node")]
pub use workers::*;
