//! What's encrypted to a machine, by the Ed25519 key it's reached by, as Encryption encrypts to one
//! (`apps/encryption/client/native/src/cipher.rs`): HPKE (RFC 9180) in its base mode, with X25519, HKDF-SHA256 and
//! ChaCha20-Poly1305, to that key's X25519 form, as libsodium and age do. A server encrypts to a device what it
//! answered a write, which any server may hand back and only the device opens (`OneCore.Writes`).

use anyhow::{Context, Result, anyhow};
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use iroh::{PublicKey, SecretKey};

type Encapped = <X25519HkdfSha256 as Kem>::EncappedKey;

/// Encrypts `data` to the machine whose key is `to`, for what `about` says, which it's bound to.
pub fn seal(to: &PublicKey, about: &str, data: &[u8]) -> Result<Vec<u8>> {
    let to = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(to.as_verifying_key().to_montgomery().as_bytes())
        .map_err(|_| anyhow!("that isn't a key to encrypt to"))?;
    let (encapped, sealed) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeS::Base,
        &to,
        about.as_bytes(),
        data,
        b"",
    )
    .map_err(|_| anyhow!("couldn't encrypt to that key"))?;
    Ok([encapped.to_bytes().as_slice(), &sealed].concat())
}

/// What was encrypted to this machine's `key` for what `about` says. It opens with no other key, for nothing else.
pub fn open(key: &SecretKey, about: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    let (encapped, sealed) = sealed.split_at_checked(Encapped::size()).context("that isn't encrypted to a key")?;
    let own = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(&key.as_signing_key().to_scalar_bytes())
        .map_err(|_| anyhow!("no key"))?;
    let encapped = Encapped::from_bytes(encapped).map_err(|_| anyhow!("that isn't encrypted to a key"))?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &own,
        &encapped,
        about.as_bytes(),
        sealed,
        b"",
    )
    .map_err(|_| anyhow!("this key doesn't open that"))
}
