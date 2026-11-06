//! A sign-in a person's own devices check themselves, without trusting the network's Workers, before one of them lets a new device
//! in with no one tapping anything (docs/plan-device-approval.md). The new device's sign-in names it: an OIDC
//! provider's ID token carries `nonce(device)`, which sign-in puts in the request it sends the provider, and a passkey's
//! assertion signs `challenge(device, time)`, which sign-in makes the passkey prompt's challenge. Sign-in passes the token or
//! the assertion on with the device's request to join, unchanged (`Proof`), and a device of the person's checks it:
//!
//! - an ID token: signed by a key its issuer publishes (`keys_at`, only issuers named here), for one of the OIDC
//!   clients the network's admins said its people sign in with (`CLIENTS`, a setting their device signs, never auth's
//!   word), issued just now (`FRESH`), and naming this device in this network;
//! - a passkey's assertion: of this network's address, the person present, naming this device in this network just now,
//!   and signed by the passkey's public key.
//!
//! Both name the network by its root (`Network::root`), so a sign-in made for another network lets no device in by itself:
//! it's approved by its code.
//!
//! Only a device that signed in itself is let in by its sign-in. One whose sign-in was finished from a link opened
//! elsewhere (the CLI's sign-in link, or any link sent to the person) could be anyone's who sent it, so sign-in passes its
//! proof on as `Proof::ByLink`, which is checked to record what it signed in with but never vouches: it's approved by a
//! tap on its card, which names it and where it is. Auth tells the two apart by where the sign-in that asked came back
//! to, which phishing can't change; a tampered sign-in could lie, which is the Cloudflare account's trust, as below.
//!
//! What it signs in with (`Account`) is recorded in the `device` entry that lists the device (`sign_in`), so the next
//! device's proof is checked against the accounts the signed entries record for that person (`vouches`), never against
//! what sign-in says. The first is recorded when a person, or for a newcomer the inviting admin, approves a device by its
//! code: what that device signed in with is then theirs, and their devices show it (and forget it) under its Sign-ins.
//! A passkey's public key then comes from auth, which keeps it: passkeys are made on auth's own page, so no client sees
//! one made without auth's code in between. Email links and passwords make no proof: only auth can check those, so
//! they're approved by code.
//!
//! What's checked needs `sign-in-proof` (ring), which the Cloudflare node, approving no one, goes without.

#[cfg(feature = "sign-in-proof")]
use anyhow::{Context, Result, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::keys::{Endpoint, sha256};

/// What a nonce and a challenge begin with.
const DOMAIN: &[u8] = b"one/join\0";

/// How long after it's made a proof lets a device in: a request to join waits 15 minutes, and a sign-in is fresh for
/// less.
pub const FRESH: u64 = 10 * 60_000;

/// How far ahead of this device's clock a proof's time may be.
#[cfg(feature = "sign-in-proof")]
const SKEW: u64 = 60_000;

/// The OIDC `nonce` a sign-in for `device` in the network whose root is `root` asks its provider to
/// put in its ID token: base64url(sha256("one/join\0" ‖ root ‖ key ‖ seal key)).
pub fn nonce(root: &[u8; 32], device: &Endpoint) -> String {
    B64U.encode(&sha256(&[DOMAIN, root, &device.key, &device.seal].concat()))
}

/// The challenge a passkey signs for `device` in the network `root` at `at` (ms): "one/join\0" ‖ root ‖ key ‖
/// seal key ‖ `at` (8 bytes, big-endian).
pub fn challenge(root: &[u8; 32], device: &Endpoint, at: u64) -> Vec<u8> {
    [DOMAIN, root, &device.key, &device.seal, &at.to_be_bytes()].concat()
}

/// The setting naming the OIDC clients a network's people sign in with, "<issuer> <client id>", one a line: an ID
/// token for any other counts for nothing, however it's signed. The device that deploys to Cloudflare writes it, as an
/// admin's (`cloudflare deploy`, and `one throwaway` for its own).
pub const CLIENTS: &str = "sign_in_clients";

/// How many days an invite link works.
pub const INVITE_DAYS: u64 = 3;

/// The network a sign-in must be for, as the checking device's own signed entries say.
#[derive(Clone, Debug)]
pub struct Network {
    /// Its address, whose host is its passkeys' relying party.
    pub address: String,
    pub root: [u8; 32],
    /// The OIDC clients its people sign in with, `(issuer, client id)`, from its `CLIENTS` setting.
    pub clients: Vec<(String, String)>,
}

impl Network {
    pub fn new(address: &str, root: [u8; 32], setting: Option<&str>) -> Network {
        let clients = setting.unwrap_or("").lines().filter_map(|l| l.trim().split_once(char::is_whitespace)).map(|(i, a)| (i.trim().to_string(), a.trim().to_string())).collect();
        Network { address: address.trim_end_matches('/').to_string(), root, clients }
    }
}

/// What a person signs in with, as the `device` entry that listed one of their devices records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// An account at an OIDC provider: who it is (`subject`) at `issuer`, signed in for `audience`, the network's
    /// client there, and the email its token gave, to show it by.
    Provider {
        issuer: String,
        audience: String,
        subject: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
    },
    /// A passkey: its credential id and its COSE public key, each base64url.
    Passkey { credential: String, public_key: String },
}

impl Account {
    /// Whether it's the same account as `other`, whatever email each shows.
    pub fn same(&self, other: &Account) -> bool {
        match (self, other) {
            (Account::Provider { issuer, audience, subject, .. }, Account::Provider { issuer: i, audience: a, subject: s, .. }) => (issuer, audience, subject) == (i, a, s),
            _ => self == other,
        }
    }
}

/// A new device's sign-in, as auth passes it on with its request to join.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proof {
    IdToken(String),
    /// A passkey's assertion, each part base64url as WebAuthn's JSON has it, and the public key auth holds for it,
    /// which counts only when a person approves the device by its code.
    Passkey { credential: String, authenticator_data: String, client_data: String, signature: String, public_key: String },
    /// A sign-in finished from a link opened on another device, not on the device asking: what it shows counts only for
    /// a device approved by a tap.
    ByLink(Box<Proof>),
}

impl Proof {
    /// The issuer an ID token says it's from, unchecked, to find its keys by.
    pub fn issuer(&self) -> Option<String> {
        let token = match self {
            Proof::IdToken(token) => token,
            Proof::ByLink(proof) => return proof.issuer(),
            Proof::Passkey { .. } => return None,
        };
        let claims = token.split('.').nth(1)?;
        let claims: Value = serde_json::from_slice(&B64U.decode(claims.as_bytes()).ok()?).ok()?;
        claims["iss"].as_str().map(str::to_string)
    }
}

/// Where an issuer publishes the keys it signs ID tokens with, for the issuers a person's devices take: Google,
/// Apple, Microsoft (any tenant), and, for a throwaway network (`one throwaway`) at `address`, a loopback address only it has, its
/// own sign-in. None for any other.
pub fn keys_at(issuer: &str, address: &str) -> Option<String> {
    match issuer {
        "https://accounts.google.com" => Some("https://www.googleapis.com/oauth2/v3/certs".into()),
        "https://appleid.apple.com" => Some("https://appleid.apple.com/auth/keys".into()),
        _ => {
            if let Some(tenant) = issuer.strip_prefix("https://login.microsoftonline.com/").and_then(|t| t.strip_suffix("/v2.0")) {
                return (!tenant.is_empty() && tenant.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| format!("https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys"));
            }
            let own = format!("{}/api/auth", address.trim_end_matches('/'));
            (address.starts_with("http://127.0.0.1:") && issuer == own).then(|| format!("{issuer}/jwks"))
        }
    }
}

/// What `proof` shows `device` signed in with, checked: an ID token against its issuer's keys `jwks` and the network's
/// own clients, a passkey's assertion against the public key sign-in gave with it. For a device approved by its code,
/// whose account is then recorded.
#[cfg(feature = "sign-in-proof")]
pub fn check(proof: &Proof, device: &Endpoint, network: &Network, jwks: Option<&Value>, now: u64) -> Result<Account> {
    match proof {
        Proof::IdToken(token) => {
            let account = id_token(token, device, network, jwks.context("no keys to check its ID token with")?, now)?;
            let Account::Provider { issuer, audience, .. } = &account else { unreachable!() };
            ensure!(network.clients.iter().any(|(i, a)| i == issuer && a == audience), "that ID token is for {audience} at {issuer}, not a client this network's people sign in with");
            Ok(account)
        }
        Proof::Passkey { credential, public_key, .. } => {
            assertion(proof, device, network, public_key, now)?;
            Ok(Account::Passkey { credential: credential.clone(), public_key: public_key.clone() })
        }
        Proof::ByLink(proof) => check(proof, device, network, jwks, now),
    }
}

/// Whether `proof` lets `device` in with no one tapping: it's checked, and what it signs in with is one of `accounts`,
/// the person's as the signed entries record them, a passkey checked against the public key recorded for it. → that
/// account.
#[cfg(feature = "sign-in-proof")]
pub fn vouches(proof: &Proof, device: &Endpoint, network: &Network, jwks: Option<&Value>, accounts: &[Account], now: u64) -> Result<Account> {
    match proof {
        Proof::IdToken(_) => {
            let account = check(proof, device, network, jwks, now)?;
            let at = |a: &Account| match a {
                Account::Provider { issuer, audience, .. } => Some((issuer.clone(), audience.clone())),
                Account::Passkey { .. } => None,
            };
            let (issuer, audience) = at(&account).context("an ID token signs in with a provider account")?;
            let theirs: Vec<&Account> = accounts.iter().filter(|a| at(a).is_some()).collect();
            ensure!(!theirs.is_empty(), "they've signed in with no provider account before");
            ensure!(theirs.iter().any(|a| at(a) == Some((issuer.clone(), audience.clone()))), "that ID token is from {issuer} for {audience}, not where they sign in");
            ensure!(theirs.iter().any(|a| a.same(&account)), "that ID token is another account's");
            Ok(account)
        }
        Proof::Passkey { credential, .. } => {
            let recorded = accounts.iter().find_map(|a| match a {
                Account::Passkey { credential: c, public_key } if c == credential => Some(public_key),
                _ => None,
            });
            let public_key = recorded.context("that passkey isn't one they've signed in with before")?;
            assertion(proof, device, network, public_key, now)?;
            Ok(Account::Passkey { credential: credential.clone(), public_key: public_key.clone() })
        }
        Proof::ByLink(_) => bail!("that sign-in was finished from a link opened elsewhere, so it's let in by a tap"),
    }
}

/// A time a proof was made at, in ms, if it's fresh at `now`.
#[cfg(feature = "sign-in-proof")]
fn fresh(at: u64, now: u64) -> Result<()> {
    ensure!(at <= now + SKEW, "that sign-in is dated ahead of this device's clock");
    ensure!(now.saturating_sub(at) <= FRESH, "that sign-in is too old: it was {} minutes ago", now.saturating_sub(at) / 60_000);
    Ok(())
}

#[cfg(feature = "sign-in-proof")]
fn b64(s: &str) -> Result<Vec<u8>> {
    B64U.decode(s.trim_end_matches('=').as_bytes()).context("that isn't base64url")
}

/// An ID token's account, if `jwks` has the key that signed it and it names `device`, just now.
#[cfg(feature = "sign-in-proof")]
fn id_token(token: &str, device: &Endpoint, network: &Network, jwks: &Value, now: u64) -> Result<Account> {
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { bail!("that isn't a JWT") };
    let header: Value = serde_json::from_slice(&b64(head)?).context("its header isn't JSON")?;
    let claims: Value = serde_json::from_slice(&b64(body)?).context("its claims aren't JSON")?;
    let issuer = claims["iss"].as_str().context("it names no issuer")?;
    ensure!(keys_at(issuer, &network.address).is_some(), "{issuer} isn't a provider a device takes sign-ins from");
    let kid = header["kid"].as_str();
    let alg = header["alg"].as_str().unwrap_or("");
    let keys = jwks["keys"].as_array().context("those aren't its issuer's keys")?;
    let jwk = keys.iter().find(|k| kid.is_none_or(|kid| k["kid"].as_str() == Some(kid)) && k["alg"].as_str().is_none_or(|a| a == alg)).context("its issuer has no key it was signed with")?;
    let signed = format!("{head}.{body}");
    let sig = b64(sig)?;
    match alg {
        "RS256" => {
            ensure!(jwk["kty"] == "RSA", "its key isn't an RSA key");
            let (n, e) = (b64(jwk["n"].as_str().context("no n")?)?, b64(jwk["e"].as_str().context("no e")?)?);
            ring::signature::RsaPublicKeyComponents { n, e }.verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &sig).ok().context("its signature isn't its issuer's")?;
        }
        "EdDSA" => {
            ensure!(jwk["kty"] == "OKP" && jwk["crv"] == "Ed25519", "its key isn't an Ed25519 key");
            ed25519(&b64(jwk["x"].as_str().context("no x")?)?, signed.as_bytes(), &sig).context("its signature isn't its issuer's")?;
        }
        other => bail!("it's signed with {other}, which no provider here uses"),
    }
    let audience = match &claims["aud"] {
        Value::String(a) => a.clone(),
        Value::Array(a) if a.len() == 1 => a[0].as_str().context("its audience isn't text")?.to_string(),
        _ => bail!("it's for no one audience"),
    };
    let subject = claims["sub"].as_str().filter(|s| !s.is_empty()).context("it names no one")?.to_string();
    let issued = claims["iat"].as_u64().context("it says no time it was issued")?.saturating_mul(1000);
    fresh(issued, now)?;
    if let Some(exp) = claims["exp"].as_u64() {
        ensure!(exp.saturating_mul(1000) > now, "that ID token has expired");
    }
    ensure!(claims["nonce"].as_str() == Some(nonce(&network.root, device).as_str()), "that sign-in was for another device");
    let email = claims["email"].as_str().map(|e| e.chars().take(254).collect());
    Ok(Account::Provider { issuer: issuer.to_string(), audience, subject, email })
}

#[cfg(feature = "sign-in-proof")]
fn ed25519(public: &[u8], message: &[u8], sig: &[u8]) -> Result<()> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(public.try_into().ok().context("not an Ed25519 key")?)?;
    key.verify_strict(message, &ed25519_dalek::Signature::from_slice(sig)?)?;
    Ok(())
}

/// Whether a passkey's assertion names `device`, just now, at the network's `address`, with the person present, and
/// `public_key` (COSE, base64url) signed it.
#[cfg(feature = "sign-in-proof")]
fn assertion(proof: &Proof, device: &Endpoint, network: &Network, public_key: &str, now: u64) -> Result<()> {
    let Proof::Passkey { authenticator_data, client_data, signature, .. } = proof else { bail!("not a passkey's") };
    let (auth_data, client_json, sig) = (b64(authenticator_data)?, b64(client_data)?, b64(signature)?);
    let client: Value = serde_json::from_slice(&client_json).context("its client data isn't JSON")?;
    ensure!(client["type"] == "webauthn.get", "it isn't a sign-in's");
    let origin = network.address.as_str();
    ensure!(client["origin"].as_str() == Some(origin), "it was made at {}, not {origin}", client["origin"].as_str().unwrap_or("nowhere"));
    let asked = b64(client["challenge"].as_str().context("it has no challenge")?)?;
    let at = asked.strip_prefix(DOMAIN).and_then(|rest| rest.strip_prefix(&network.root[..])).and_then(|rest| rest.strip_prefix(&device.key[..])).and_then(|rest| rest.strip_prefix(&device.seal[..])).context("that sign-in was for another device")?;
    let at = u64::from_be_bytes(at.try_into().ok().context("that sign-in was for another device")?);
    fresh(at, now)?;
    let host = origin.split_once("://").map_or(origin, |(_, h)| h).split(['/', ':']).next().unwrap_or("");
    ensure!(auth_data.len() >= 37 && auth_data[..32] == sha256(host.as_bytes()), "it's a passkey for another site");
    ensure!(auth_data[32] & 0x01 != 0, "the person wasn't there to tap it");
    let signed = [&auth_data[..], &sha256(&client_json)].concat();
    cose_verify(&b64(public_key)?, &signed, &sig).context("its signature isn't that passkey's")
}

/// Checks `sig` of `message` with a COSE public key: ES256 (P-256), EdDSA (Ed25519) or RS256, the kinds passkeys are.
#[cfg(feature = "sign-in-proof")]
fn cose_verify(key: &[u8], message: &[u8], sig: &[u8]) -> Result<()> {
    use ciborium::Value as C;
    let map: C = ciborium::from_reader(key).context("its public key isn't COSE")?;
    let map = map.as_map().context("its public key isn't COSE")?;
    let get = |label: i64| map.iter().find(|(k, _)| k.as_integer() == Some(label.into())).map(|(_, v)| v);
    let int = |label: i64| get(label).and_then(C::as_integer).and_then(|i| i64::try_from(i).ok());
    let bytes = |label: i64| get(label).and_then(C::as_bytes).cloned().context("its public key is missing a part");
    match (int(1), int(3)) {
        (Some(2), Some(-7)) => {
            ensure!(int(-1) == Some(1), "not a P-256 key");
            let point = [&[4u8][..], &bytes(-2)?, &bytes(-3)?].concat();
            ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point).verify(message, sig).ok().context("bad signature")
        }
        (Some(1), Some(-8)) => {
            ensure!(int(-1) == Some(6), "not an Ed25519 key");
            ed25519(&bytes(-2)?, message, sig)
        }
        (Some(3), Some(-257)) => {
            let (n, e) = (bytes(-1)?, bytes(-2)?);
            ring::signature::RsaPublicKeyComponents { n, e }.verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, message, sig).ok().context("bad signature")
        }
        (kty, alg) => bail!("a passkey of a kind no device checks (kty {kty:?}, alg {alg:?})"),
    }
}

#[cfg(all(test, feature = "sign-in-proof"))]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_SHA256, RsaKeyPair, RsaPublicKeyComponents};
    use serde_json::json;

    /// A 2048-bit RSA key for tests only, PKCS#8 (`openssl genpkey -algorithm RSA`), as a provider signs with.
    const RSA: &str = include_str!("sign_in_proof/test-rsa.b64");

    const NOW: u64 = 1_800_000_000_000;
    const ADDRESS: &str = "https://one.example.com";
    const GOOGLE: &str = "https://accounts.google.com";
    const OURS: &str = "1234-ours.apps.googleusercontent.com";
    const ROOT: [u8; 32] = [3; 32];

    /// The network checking, whose people sign in with Google for `OURS`.
    fn ours() -> Network {
        Network::new(ADDRESS, ROOT, Some(&format!("{GOOGLE} {OURS}")))
    }

    fn device(n: u8) -> Endpoint {
        Endpoint::of(&SigningKey::from_bytes(&[n; 32]))
    }

    struct Provider {
        key: RsaKeyPair,
    }

    impl Provider {
        fn new() -> Provider {
            let der = data_encoding::BASE64.decode(RSA.trim().as_bytes()).unwrap();
            Provider { key: RsaKeyPair::from_pkcs8(&der).unwrap() }
        }
        fn jwks(&self) -> Value {
            let c: RsaPublicKeyComponents<Vec<u8>> = self.key.public().into();
            json!({ "keys": [{ "kty": "RSA", "kid": "k1", "alg": "RS256", "n": B64U.encode(&c.n), "e": B64U.encode(&c.e) }] })
        }
        fn token(&self, claims: Value) -> String {
            let head = B64U.encode(json!({ "alg": "RS256", "kid": "k1", "typ": "JWT" }).to_string().as_bytes());
            let body = B64U.encode(claims.to_string().as_bytes());
            let mut sig = vec![0; self.key.public().modulus_len()];
            self.key.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), format!("{head}.{body}").as_bytes(), &mut sig).unwrap();
            format!("{head}.{body}.{}", B64U.encode(&sig))
        }
        fn claims(d: &Endpoint) -> Value {
            json!({ "iss": GOOGLE, "aud": OURS, "sub": "1001", "iat": NOW / 1000 - 30, "exp": NOW / 1000 + 3000, "nonce": nonce(&ROOT, d), "email": "a@example.com" })
        }
    }

    fn theirs() -> Vec<Account> {
        vec![Account::Provider { issuer: GOOGLE.into(), audience: OURS.into(), subject: "1001".into(), email: None }]
    }

    /// An ID token for this device, from their account, for our client, just now, lets it in; and every other refuses.
    #[test]
    fn an_id_token_lets_in_only_its_own_device() {
        let (p, d) = (Provider::new(), device(1));
        let jwks = p.jwks();
        let try_ = |claims: Value| vouches(&Proof::IdToken(p.token(claims)), &d, &ours(), Some(&jwks), &theirs(), NOW);
        let refused = |claims: Value, why: &str| {
            let err = try_(claims).expect_err(why).to_string();
            assert!(err.contains(why), "{err} says {why}");
        };
        assert!(try_(Provider::claims(&d)).unwrap().same(&theirs()[0]));
        // The same sign-in finished from a link opened elsewhere lets nothing in by itself, and is recorded as it's
        // approved by a tap.
        let linked = Proof::ByLink(Box::new(Proof::IdToken(p.token(Provider::claims(&d)))));
        let err = vouches(&linked, &d, &ours(), Some(&jwks), &theirs(), NOW).unwrap_err().to_string();
        assert!(err.contains("from a link"), "{err}");
        assert!(check(&linked, &d, &ours(), Some(&jwks), NOW).unwrap().same(&theirs()[0]));
        assert_eq!(linked.issuer().as_deref(), Some(GOOGLE));
        let with = |k: &str, v: Value| {
            let mut c = Provider::claims(&d);
            c[k] = v;
            c
        };
        refused(with("sub", json!("2002")), "another account's");
        refused(with("aud", json!("someone-else.apps.googleusercontent.com")), "not a client this network's people sign in with");
        refused(with("nonce", json!(nonce(&ROOT, &device(2)))), "for another device");
        refused(with("nonce", json!(nonce(&[4; 32], &d))), "for another device");
        refused(with("nonce", Value::Null), "for another device");
        refused(with("iat", json!(NOW / 1000 - 3600)), "too old");
        refused(with("iat", json!(NOW / 1000 + 600)), "ahead of this device's clock");
        refused(with("exp", json!(NOW / 1000 - 1)), "expired");
        refused(with("iss", json!("https://evil.example.com")), "isn't a provider");
        // Signed by someone else's key, or changed after it was signed.
        let other = Provider::claims(&d);
        let token = p.token(other.clone());
        let (head, _) = token.split_once('.').unwrap();
        let sig = token.rsplit_once('.').unwrap().1;
        let forged = format!("{head}.{}.{sig}", B64U.encode(with("sub", json!("1001 ")).to_string().as_bytes()));
        let err = vouches(&Proof::IdToken(forged), &d, &ours(), Some(&jwks), &theirs(), NOW).unwrap_err().to_string();
        assert!(err.contains("signature"), "{err}");
        let stranger = SigningKey::from_bytes(&[9; 32]);
        let head = B64U.encode(json!({ "alg": "EdDSA", "kid": "k1" }).to_string().as_bytes());
        let body = B64U.encode(other.to_string().as_bytes());
        let sig = stranger.sign(format!("{head}.{body}").as_bytes());
        let err = vouches(&Proof::IdToken(format!("{head}.{body}.{}", B64U.encode(&sig.to_bytes()))), &d, &ours(), Some(&jwks), &theirs(), NOW).unwrap_err().to_string();
        assert!(err.contains("no key it was signed with") || err.contains("isn't an Ed25519 key"), "{err}");
        // Someone with no account recorded is approved by code.
        let err = vouches(&Proof::IdToken(p.token(Provider::claims(&d))), &d, &ours(), Some(&jwks), &[], NOW).unwrap_err().to_string();
        assert!(err.contains("no provider account"), "{err}");
        // Recorded as it's approved by code only for the network's own client, whatever its signature: a token for
        // another app's client (an attacker's own), or with the network naming no client, is recorded as nothing.
        let theirs_app = p.token(with("aud", json!("attackers-app.apps.googleusercontent.com")));
        let err = check(&Proof::IdToken(theirs_app), &d, &ours(), Some(&jwks), NOW).unwrap_err().to_string();
        assert!(err.contains("not a client this network's people sign in with"), "{err}");
        let none = Network::new(ADDRESS, ROOT, None);
        assert!(check(&Proof::IdToken(p.token(Provider::claims(&d))), &d, &none, Some(&jwks), NOW).is_err());
        let recorded = check(&Proof::IdToken(p.token(Provider::claims(&d))), &d, &ours(), Some(&jwks), NOW).unwrap();
        assert!(matches!(&recorded, Account::Provider { email: Some(e), .. } if e == "a@example.com") && recorded.same(&theirs()[0]));
        // Nor does one recorded before for another client let a device in, once the network names its own.
        let elsewhere = vec![Account::Provider { issuer: GOOGLE.into(), audience: "attackers-app.apps.googleusercontent.com".into(), subject: "1001".into(), email: None }];
        assert!(vouches(&Proof::IdToken(p.token(with("aud", json!("attackers-app.apps.googleusercontent.com")))), &d, &ours(), Some(&jwks), &elsewhere, NOW).is_err());
        // A throwaway network's sign-in signs with Ed25519, and only one on this machine is taken.
        let sign_in = SigningKey::from_bytes(&[7; 32]);
        let issuer = "http://127.0.0.1:8787/api/auth";
        let sign_in_keys = json!({ "keys": [{ "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "kid": "t", "x": B64U.encode(sign_in.verifying_key().as_bytes()) }] });
        let throwaway = |iss: &str, address: &str| {
            let head = B64U.encode(json!({ "alg": "EdDSA", "kid": "t" }).to_string().as_bytes());
            let body = B64U.encode(json!({ "iss": iss, "aud": "one-throwaway", "sub": "a000000000000001", "iat": NOW / 1000, "nonce": nonce(&ROOT, &d) }).to_string().as_bytes());
            let sig = sign_in.sign(format!("{head}.{body}").as_bytes());
            let network = Network::new(address, ROOT, Some(&format!("{iss} one-throwaway")));
            check(&Proof::IdToken(format!("{head}.{body}.{}", B64U.encode(&sig.to_bytes()))), &d, &network, Some(&sign_in_keys), NOW)
        };
        assert!(throwaway(issuer, "http://127.0.0.1:8787").is_ok());
        assert!(throwaway(issuer, ADDRESS).is_err(), "a real network takes no loopback issuer");
        assert!(throwaway(issuer, "http://127.0.0.1:9999").is_err(), "nor another throwaway network's");
        assert!(throwaway("http://one.example.com:8787/api/auth", "http://one.example.com:8787").is_err());
        assert_eq!(keys_at("https://login.microsoftonline.com/9188040d-6c67-4c5b-b112-36a304b66dad/v2.0", ADDRESS).as_deref(), Some("https://login.microsoftonline.com/9188040d-6c67-4c5b-b112-36a304b66dad/discovery/v2.0/keys"));
        assert_eq!(keys_at("https://login.microsoftonline.com/../v2.0", ADDRESS), None);
    }

    /// A passkey as a browser makes an assertion with it, for the site at `address`.
    struct Passkey {
        key: EcdsaKeyPair,
        credential: String,
    }

    impl Passkey {
        fn new() -> Passkey {
            let rng = SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            Passkey { key: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap(), credential: B64U.encode(b"credential-1") }
        }
        fn public_key(&self) -> String {
            use ciborium::Value as C;
            let point = self.key.public_key().as_ref();
            let map = C::Map(vec![
                (C::Integer(1.into()), C::Integer(2.into())),
                (C::Integer(3.into()), C::Integer((-7).into())),
                (C::Integer((-1).into()), C::Integer(1.into())),
                (C::Integer((-2).into()), C::Bytes(point[1..33].to_vec())),
                (C::Integer((-3).into()), C::Bytes(point[33..].to_vec())),
            ]);
            let mut out = Vec::new();
            ciborium::into_writer(&map, &mut out).unwrap();
            B64U.encode(&out)
        }
        fn assert(&self, challenge: &[u8], origin: &str, rp: &str, flags: u8) -> Proof {
            let client = json!({ "type": "webauthn.get", "challenge": B64U.encode(challenge), "origin": origin, "crossOrigin": false }).to_string();
            let auth = [&sha256(rp.as_bytes())[..], &[flags], &[0, 0, 0, 7]].concat();
            let sig = self.key.sign(&SystemRandom::new(), &[&auth[..], &sha256(client.as_bytes())].concat()).unwrap();
            Proof::Passkey { credential: self.credential.clone(), authenticator_data: B64U.encode(&auth), client_data: B64U.encode(client.as_bytes()), signature: B64U.encode(sig.as_ref()), public_key: self.public_key() }
        }
    }

    /// A passkey they signed in with before, naming this device just now at this network, with them there, lets it in;
    /// and every other refuses.
    #[test]
    fn a_passkey_lets_in_only_its_own_device() {
        let (pk, d) = (Passkey::new(), device(1));
        let accounts = vec![Account::Passkey { credential: pk.credential.clone(), public_key: pk.public_key() }];
        let ok = pk.assert(&challenge(&ROOT, &d, NOW - 5_000), ADDRESS, "one.example.com", 0x05);
        assert_eq!(vouches(&ok, &d, &ours(), None, &accounts, NOW).unwrap(), accounts[0]);
        let refused = |proof: Proof, accounts: &[Account], why: &str| {
            let err = vouches(&proof, &d, &ours(), None, accounts, NOW).expect_err(why).to_string();
            assert!(err.contains(why), "{err} says {why}");
        };
        refused(pk.assert(&challenge(&ROOT, &device(2), NOW), ADDRESS, "one.example.com", 0x05), &accounts, "for another device");
        refused(pk.assert(&challenge(&[4; 32], &d, NOW), ADDRESS, "one.example.com", 0x05), &accounts, "for another device");
        refused(pk.assert(&challenge(&ROOT, &d, NOW - FRESH - 1), ADDRESS, "one.example.com", 0x05), &accounts, "too old");
        refused(pk.assert(&challenge(&ROOT, &d, NOW), "https://evil.example.com", "one.example.com", 0x05), &accounts, "not https://one.example.com");
        refused(pk.assert(&challenge(&ROOT, &d, NOW), ADDRESS, "evil.example.com", 0x05), &accounts, "another site");
        refused(pk.assert(&challenge(&ROOT, &d, NOW), ADDRESS, "one.example.com", 0x04), &accounts, "wasn't there");
        refused(ok.clone(), &[], "isn't one they've signed in with");
        refused(Proof::ByLink(Box::new(ok.clone())), &accounts, "from a link");
        assert_eq!(check(&Proof::ByLink(Box::new(ok.clone())), &d, &ours(), None, NOW).unwrap(), accounts[0]);
        // Another passkey's key recorded for that credential, as auth might say it has: its signature isn't that key's.
        let other = Passkey::new();
        refused(ok.clone(), &[Account::Passkey { credential: pk.credential.clone(), public_key: other.public_key() }], "signature");
        // Changed after it was signed.
        let Proof::Passkey { credential, authenticator_data, signature, public_key, .. } = ok.clone() else { unreachable!() };
        let client = json!({ "type": "webauthn.get", "challenge": B64U.encode(&challenge(&ROOT, &device(2), NOW)), "origin": ADDRESS }).to_string();
        let swapped = Proof::Passkey { credential, authenticator_data, client_data: B64U.encode(client.as_bytes()), signature, public_key };
        refused(swapped, &accounts, "for another device");
        let mut changed = ok.clone();
        if let Proof::Passkey { authenticator_data, .. } = &mut changed {
            let mut a = b64(authenticator_data).unwrap();
            a[36] ^= 1;
            *authenticator_data = B64U.encode(&a);
        }
        refused(changed, &accounts, "signature");
        // Approved by code, it's checked with the key sign-in gave, and that's what's recorded.
        assert_eq!(check(&ok, &d, &ours(), None, NOW).unwrap(), accounts[0]);
    }

    /// Older devices read a `device` entry's `sign_in` as nothing, and these read what a newer one might add as nothing.
    #[test]
    fn accounts_read_as_recorded() {
        let a: Account = serde_json::from_value(json!({ "provider": { "issuer": GOOGLE, "audience": OURS, "subject": "1" } })).unwrap();
        assert_eq!(a, Account::Provider { issuer: GOOGLE.into(), audience: OURS.into(), subject: "1".into(), email: None });
        assert!(serde_json::from_value::<Account>(json!({ "password": {} })).is_err());
        let p: Proof = serde_json::from_value(json!({ "id_token": "a.b.c" })).unwrap();
        assert_eq!(p, Proof::IdToken("a.b.c".into()));
        // As auth passes on a sign-in finished from a link.
        let p: Proof = serde_json::from_value(json!({ "by_link": { "id_token": "a.b.c" } })).unwrap();
        assert_eq!(p, Proof::ByLink(Box::new(Proof::IdToken("a.b.c".into()))));
    }
}
