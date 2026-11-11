//! A step's signature (`Sig`), in it or beside it, and the one check of a step and of a copy that every device makes.

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::stored::sql::{copied, state_hash};

/// What `copy_holds` says of a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyCheck {
    /// Whether a reader may start from it.
    pub holds: bool,
    /// Where its database's steps are signed in them from, as its step says.
    pub from: Option<u64>,
    /// Whether it comes to the state hash signed at its step, if one is.
    pub hash: Option<bool>,
}

/// The one check of a copy at `at`, as its bytes, beside its step opened as `step` with `signed` the signature in
/// or beside it, the same on every device and for every use: readers, `tidy`, backups, and the writer of the copy. A
/// copy holds only if it's a database; one whose step is signed in it only if that signature holds; and, once steps
/// are signed in them, only if it comes to the state hash signed. Before, it holds whatever that says.
pub fn copy_holds(at: u64, copy: &[u8], step: &[u8], signed: Option<&Result<Sig>>) -> CopyCheck {
    let inside = step.first() == Some(&SIGNED);
    let from = match signed {
        Some(Ok(sig)) if inside => sig.from.filter(|f| *f <= at),
        _ => None,
    };
    let database = copied(copy).ok();
    let hash = match signed {
        Some(Ok(Sig { hash: Some(hash), .. })) => Some(database.as_ref().is_some_and(|c| state_hash(c).is_ok_and(|h| h == *hash))),
        _ => None,
    };
    let holds = database.is_some() && !(inside && !matches!(signed, Some(Ok(_)))) && (from.is_none() || hash == Some(true));
    CopyCheck { holds, from, hash }
}

/// Where a database's steps are signed in them from, after the step at `at`, opened as `step` with `signed` the
/// signature in or beside it, given `before`, where the steps before said: the first signed in it that says a position
/// (`Sig::from`), its own on the first a writer signs in it, marks it, as a copy's step does (`copy_holds`).
pub fn signed_from(before: Option<u64>, at: u64, step: &[u8], signed: Option<&Result<Sig>>) -> Option<u64> {
    before.or(match signed {
        Some(Ok(sig)) if step.first() == Some(&SIGNED) => sig.from.filter(|f| *f <= at),
        _ => None,
    })
}

/// The one check of a step, the same on every device: whether the step at `at`, opened as `step` with `signed`
/// the signature in or beside it, is refused, in a database whose steps are signed in them from `from` (`signed_from`).
/// Before that position nothing is: older steps, and those signed beside them whose signature never landed, have none
/// for good. From it, a step counts only if it's signed in it, and its signature holds and says that position. Its
/// state hash is checked as it's applied.
pub fn refused(from: Option<u64>, at: u64, step: &[u8], signed: Option<&Result<Sig>>) -> bool {
    let Some(from) = from.filter(|f| at >= *f) else { return false };
    !(step.first() == Some(&SIGNED) && matches!(signed, Some(Ok(sig)) if sig.from == Some(from)))
}

/// A step's signature: its writer's endpoint key (`by`), and, on a step a copy follows, the state hash of the database
/// after it (`hash`). Beside the step (`<prefix>/<n>.sig`), it's `{"by", "hash"?, "sig"}`, sealed under the step's
/// epoch and bound to its own name, since who wrote what is as much the scope's as what they wrote: `by` hex, and `sig`
/// (base64url) the writer's signature, under `one/step`, over the canonical `{"hash"?, "name", "step"}`, the step's name
/// (its database and position) and the BLAKE3 of its sealed bytes (hex). In the step (`SIGNED`), the same JSON comes
/// before the step it signs, with `from` too, and `sig` is under `one/signed-step`, over `{"from"?, "hash"?, "name",
/// "step"}`, with `step` the BLAKE3 of the step it signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub by: [u8; 32],
    pub hash: Option<String>,
    /// In a step signed in it, where its database's steps are signed in them from: its own position on the first.
    pub from: Option<u64>,
}

impl Sig {
    const BESIDE: &str = "one/step";
    const IN: &str = "one/signed-step";

    fn signed(name: &str, what: &[u8], hash: Option<&str>, from: Option<u64>) -> Value {
        let mut signed = json!({ "name": name, "step": blake3::hash(what).to_hex().as_str() });
        if let Some(hash) = hash {
            signed["hash"] = json!(hash);
        }
        if let Some(from) = from {
            signed["from"] = json!(from);
        }
        signed
    }

    /// `{"by", "from"?, "hash"?, "sig"}`: `signer`'s signature, under `domain`, of `what` at `name`, saying `hash` and
    /// `from`.
    fn make(signer: &ed25519_dalek::SigningKey, domain: &str, name: &str, what: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
        let sig = key::sign(signer, domain, &Self::signed(name, what, hash, from));
        let mut made = json!({ "by": key::hex(signer.verifying_key().as_bytes()), "sig": data_encoding::BASE64URL_NOPAD.encode(&sig) });
        if let Some(hash) = hash {
            made["hash"] = json!(hash);
        }
        if let Some(from) = from {
            made["from"] = json!(from);
        }
        key::canonical(&made)
    }

    /// Reads `{"by", "hash"?, "sig"}` and checks it signs `what` at `name` under `domain`.
    fn read(bytes: &[u8], domain: &str, name: &str, what: &[u8]) -> Result<Sig> {
        let v: Value = serde_json::from_slice(bytes).context("a step's signature is JSON")?;
        let by: [u8; 32] = key::unhex(v["by"].as_str().context("a step's signature says who signed it")?)?;
        let hash = match &v["hash"] {
            Value::Null => None,
            hash => Some(hash.as_str().context("a step's state hash is text")?.to_string()),
        };
        let from = match &v["from"] {
            Value::Null => None,
            from => Some(from.as_u64().context("where steps are signed from is a position")?),
        };
        let signature = data_encoding::BASE64URL_NOPAD.decode(v["sig"].as_str().context("a step's signature has its signature")?.as_bytes())?;
        key::verify(&by, domain, &Self::signed(name, what, hash.as_deref(), from), &signature).with_context(|| format!("{name}'s signer"))?;
        Ok(Sig { by, hash, from })
    }

    /// Step `name`'s signature beside it, by `signer`, saying `hash`, sealed under `epoch`'s `key`.
    #[cfg(test)]
    pub fn seal(signer: &ed25519_dalek::SigningKey, epoch: u32, key: &[u8; 32], name: &str, sealed: &[u8], hash: Option<&str>) -> Vec<u8> {
        key::seal(epoch, key, &format!("{name}.sig"), &Self::make(signer, Self::BESIDE, name, sealed, hash, None))
    }

    /// Opens step `name`'s signature beside it with the key the step opened with, and checks it signs the step's
    /// `sealed` bytes there.
    pub fn check(key: &[u8; 32], name: &str, sealed: &[u8], sig: &[u8]) -> Result<Sig> {
        Self::read(&key::open(key, &format!("{name}.sig"), sig)?, Self::BESIDE, name, sealed)
    }

    /// `step`, to be written at `name`, with `signer`'s signature in it, saying `hash`, and that steps are signed in
    /// them from `from`.
    pub fn wrap(signer: &ed25519_dalek::SigningKey, name: &str, step: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
        let sig = Self::make(signer, Self::IN, name, step, hash, from);
        [&[SIGNED][..], &(sig.len() as u32).to_be_bytes(), &sig, step].concat()
    }

    /// The signature in step `name`, opened as `step`, if it's one signed in it.
    pub fn unwrap(name: &str, step: &[u8]) -> Option<Result<Sig>> {
        if step.first() != Some(&SIGNED) {
            return None;
        }
        Some(match split_signed(step) {
            Some((sig, inner)) => Self::read(sig, Self::IN, name, inner),
            None => Err(anyhow::anyhow!("{name} is a signed step that doesn't say where its signature ends")),
        })
    }
}

/// A step signed in it: `v`, its signature's length (4 bytes, big-endian), its signature (`Sig`), then the step it
/// signs, a changeset (`c`) or a schema change (`s`).
pub const SIGNED: u8 = b'v';

/// A step signed in it, opened: its signature's bytes, and the step it signs.
pub fn split_signed(step: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = step.strip_prefix(&[SIGNED])?;
    let len = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    let rest = &rest[4..];
    (rest.len() >= len).then(|| rest.split_at(len))
}

/// `step`, to be written at `name`: with `signer`'s signature in it, saying `hash` and `from`, if there's one.
pub fn signed(signer: Option<&ed25519_dalek::SigningKey>, name: &str, step: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
    match signer {
        Some(signer) => Sig::wrap(signer, name, step, hash, from),
        None => step.to_vec(),
    }
}
