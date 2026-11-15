//! What encryption is made of, for a client and for a server's NIF (`plugins/encryption/server/nif/encryption`), so
//! both do the same thing with the same code. Nothing here is a construction of our own:
//!
//! - **To a machine's key** (`seal`, `open`): HPKE (RFC 9180) in its base mode, with X25519, HKDF-SHA256 and
//!   ChaCha20-Poly1305. Every server and client has an Ed25519 key, the one it's reached by, and what's encrypted to
//!   it is encrypted to that key's X25519 form, as libsodium and age do. So naming a machine is enough to encrypt
//!   to it, and no second key has to be published and believed.
//! - **An owner's key** is 32 random bytes, made on a device of theirs. It's shown to them once more as its
//!   recovery key, which is the key itself in base32 (RFC 4648).
//! - **An artifact's key** is derived from its owner's with HKDF-SHA256 (RFC 5869), from the artifact's name and
//!   an epoch: taking a server off an artifact starts a new epoch, whose key that server was never given.
//! - **A blob** (`wrap`, `unwrap`) is encrypted with XChaCha20-Poly1305 under its artifact's newest key, with a
//!   random nonce, and says which epoch's. Its name in the bucket is authenticated with it, so a blob put in
//!   another's place doesn't open.
//! - **A backup** is encrypted to a key of its own, derived from the owner's key of whoever added where it goes
//!   (`backups`), with HPKE as above: a server is given only its public half, so whatever backs up can't read a
//!   backup, and the recovery key opens every one.
//! - **A notification** is encrypted to a key of its device's, derived from the device's own (`notifications`), with
//!   HPKE as above, so the push service that carries it reads nothing, and what shows it on a phone (its notification
//!   extension) holds only that key.
//!
//! What each is encrypted for (the `for_` functions) a server's plugin says too, in its Elixir, as what it hands the
//! NIF (`about`): the same text in both, which the proofs with a real device show agree (`plugins/vault/test/vault.sh`,
//! `plugins/backups/test/backups.sh`).

use anyhow::{Context, Result, anyhow, bail};
use chacha20poly1305::aead::{Aead, Generate, Payload};
use chacha20poly1305::{Key as BlobKey, KeyInit, XChaCha20Poly1305, XNonce};
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use hkdf::Hkdf;
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};
use iroh_base::{PublicKey, SecretKey};
use sha2::Sha256;
use zeroize::Zeroizing;

/// A key: an owner's, or an artifact's for one epoch.
pub type Key = Zeroizing<[u8; 32]>;

/// What a blob starts with: how it's encrypted, so that can change, and then its key's epoch.
const BLOB: u8 = 1;
/// A blob's nonce, in bytes.
const NONCE: usize = 24;

/// What something encrypted to a machine is for, which HPKE binds it to: an owner's key for a device of theirs.
pub fn for_device(owner: &str, device: &str) -> String {
    format!("one/encryption/1/owner\n{owner}\n{device}")
}

/// An artifact's keys for a server chosen to open it.
pub fn for_server(artifact: &str, server: &str) -> String {
    format!("one/encryption/1/keys\n{artifact}\n{server}")
}

/// What a person typed on their device, for a server that opens what it's kept in.
pub fn for_secret(server: &str) -> String {
    format!("one/encryption/1/secret\n{server}")
}

/// A new owner's key, from the system's random numbers.
pub fn owner() -> Key {
    Zeroizing::new(BlobKey::generate().into())
}

fn derived(owner: &Key, what: &str) -> Key {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(None, owner.as_slice()).expand(what.as_bytes(), key.as_mut_slice()).expect("32 bytes fit");
    key
}

/// An artifact's keys for every epoch up to `epochs`, the first first, from its owner's key.
pub fn artifact(owner: &Key, artifact: &str, epochs: u32) -> Vec<Key> {
    (1..=epochs).map(|epoch| derived(owner, &format!("one/encryption/1/artifact\n{artifact}\n{epoch}"))).collect()
}

/// The key backups are encrypted to, from the owner's key of whoever added where they go. A server holds only its
/// public half, which is what a device hands over.
pub fn backups(owner: &Key) -> SecretKey {
    SecretKey::from_bytes(&derived(owner, "one/backups/1/key"))
}

/// What a backup's object is encrypted for, which only that object opens as: its name where the backup goes.
pub fn for_object(object: &str) -> String {
    format!("one/backups/1/object\n{object}")
}

/// What a backup's list of blobs is encrypted for: its name where the backup goes.
pub fn for_backup(backup: &str) -> String {
    format!("one/backups/1/backup\n{backup}")
}

/// The backup key, for a server that restores from a backup while a device asks it to.
pub fn for_restoring(server: &str) -> String {
    format!("one/backups/1/restoring\n{server}")
}

/// The key a device's notifications are encrypted to, from the device's own key. Its public half is what the device
/// registers with its push token.
pub fn notifications(device: &SecretKey) -> SecretKey {
    SecretKey::from_bytes(&derived(&Zeroizing::new(device.to_bytes()), "one/notifications/1/key"))
}

/// What a notification is encrypted for: the key it's encrypted to, by its name. A server seals it so
/// (`plugins/notifications/server/notifications.ex`).
pub fn for_notification(to: &str) -> String {
    format!("one/notifications/1/notification\n{to}")
}

/// A notification as the device it was sent to opens it, with its own key.
pub fn open_notification(device: &SecretKey, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let key = notifications(device);
    open(&key, &for_notification(&key.public().to_string()), sealed)
}

/// What tells an owner's key from another without giving it away: kept beside what's encrypted to their devices,
/// so a recovery key that was mistyped is found out on the device.
pub fn check(owner: &Key) -> String {
    HEXLOWER.encode(derived(owner, "one/encryption/1/check").as_slice())
}

/// An owner's key as its recovery key, which a person writes down: base32, in fours.
pub fn recovery(owner: &Key) -> String {
    let text = BASE32_NOPAD.encode(owner.as_slice());
    text.as_bytes().chunks(4).map(|four| String::from_utf8_lossy(four)).collect::<Vec<_>>().join("-")
}

/// The owner's key a recovery key is, however it was typed: in either case, with or without its hyphens and spaces.
pub fn recovered(text: &str) -> Result<Key> {
    let typed: String = text.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    let bytes =
        Zeroizing::new(BASE32_NOPAD.decode(typed.as_bytes()).map_err(|_| anyhow!("That isn't a recovery key."))?);
    key(&bytes).map_err(|_| anyhow!("That isn't a recovery key."))
}

type Sealed = <X25519HkdfSha256 as Kem>::EncappedKey;

/// Encrypts `data` to the machine whose key is `to`, for what `about` says.
pub fn seal(to: &PublicKey, about: &str, data: &[u8]) -> Result<Vec<u8>> {
    let to = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(to.as_verifying_key().to_montgomery().as_bytes())
        .map_err(|_| anyhow!("that isn't a key to encrypt to"))?;
    let (key, sealed) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeS::Base,
        &to,
        about.as_bytes(),
        data,
        b"",
    )
    .map_err(|_| anyhow!("couldn't encrypt to that key"))?;
    Ok([key.to_bytes().as_slice(), &sealed].concat())
}

/// What was encrypted to this machine's `key`, for what `about` says. It doesn't open with another key, or for
/// anything else.
pub fn open(key: &SecretKey, about: &str, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let (encapped, sealed) = sealed.split_at_checked(Sealed::size()).context("that isn't encrypted to a key")?;
    let own = Zeroizing::new(key.as_signing_key().to_scalar_bytes());
    let own = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(own.as_slice()).map_err(|_| anyhow!("no key"))?;
    let encapped = Sealed::from_bytes(encapped).map_err(|_| anyhow!("that isn't encrypted to a key"))?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &own,
        &encapped,
        about.as_bytes(),
        sealed,
        b"",
    )
    .map(Zeroizing::new)
    .map_err(|_| anyhow!("This key doesn't open that"))
}

/// A key from its 32 bytes.
pub fn key(bytes: &[u8]) -> Result<Key> {
    Ok(Zeroizing::new(bytes.try_into().map_err(|_| anyhow!("that isn't a key"))?))
}

/// An artifact's keys as they're handed to a server: one after another, the first epoch's first.
pub fn joined(keys: &[Key]) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(keys.iter().flat_map(|key| key.iter().copied()).collect())
}

/// The keys that were handed over as `joined` gives them.
pub fn split(joined: &[u8]) -> Result<Vec<Key>> {
    if joined.is_empty() || !joined.len().is_multiple_of(32) {
        bail!("those aren't an artifact's keys")
    }
    joined.chunks(32).map(key).collect()
}

/// Encrypts a blob under its artifact's newest key. `name` is the blob's whole name in the bucket.
pub fn wrap(keys: &[Key], name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let key = keys.last().context("this server holds no key for that artifact")?;
    let nonce = XNonce::generate();
    let encrypted = XChaCha20Poly1305::new(key.as_slice().try_into().expect("32 bytes"))
        .encrypt(&nonce, Payload { msg: data, aad: name.as_bytes() })
        .map_err(|_| anyhow!("couldn't encrypt {name}"))?;
    let epoch = u32::try_from(keys.len()).expect("an artifact has few epochs");
    Ok([&[BLOB][..], &epoch.to_be_bytes(), &nonce, &encrypted].concat())
}

/// A blob as it was before `wrap`. One that was changed, was written under another name, or is of an epoch this
/// server was never given, doesn't open.
pub fn unwrap(keys: &[Key], name: &str, blob: &[u8]) -> Result<Vec<u8>> {
    let not = || anyhow!("{name} isn't a blob this server's keys open");
    let (&[BLOB], rest) = blob.split_at_checked(1).ok_or_else(not)? else { return Err(not()) };
    let (epoch, rest) = rest.split_at_checked(4).ok_or_else(not)?;
    let (nonce, encrypted) = rest.split_at_checked(NONCE).ok_or_else(not)?;
    let epoch = u32::from_be_bytes(epoch.try_into().expect("4 bytes")) as usize;
    let key = epoch.checked_sub(1).and_then(|epoch| keys.get(epoch)).ok_or_else(not)?;
    XChaCha20Poly1305::new(key.as_slice().try_into().expect("32 bytes"))
        .decrypt(&XNonce::try_from(nonce).map_err(|_| not())?, Payload { msg: encrypted, aad: name.as_bytes() })
        .map_err(|_| not())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_key_its_for_opens_it_and_only_for_what_it_says() {
        let (server, other) = (SecretKey::generate(), SecretKey::generate());
        let about = for_secret(&server.public().to_string());
        let sealed = seal(&server.public(), &about, b"typed").unwrap();
        assert_eq!(open(&server, &about, &sealed).unwrap().as_slice(), b"typed");
        assert!(open(&other, &about, &sealed).is_err());
        assert!(
            open(&server, &for_server("users/alice/made/vault/items", &server.public().to_string()), &sealed).is_err()
        );
    }

    #[test]
    fn a_blob_opens_under_its_own_name_with_its_epochs_key() {
        let owner = owner();
        let keys = artifact(&owner, "users/alice/made/vault/items", 2);
        let blob = wrap(&keys, "users/alice/made/vault/items/log/1", b"a secret").unwrap();
        assert!(!blob.windows(8).any(|part| part == b"a secret"));
        assert_eq!(unwrap(&keys, "users/alice/made/vault/items/log/1", &blob).unwrap(), b"a secret");
        assert!(unwrap(&keys, "users/alice/made/vault/items/log/2", &blob).is_err());
        // A server taken off before the second epoch holds only the first's key.
        assert!(unwrap(&keys[..1], "users/alice/made/vault/items/log/1", &blob).is_err());
        assert!(
            unwrap(&artifact(&owner, "users/bob/made/vault/items", 2), "users/alice/made/vault/items/log/1", &blob)
                .is_err()
        );
        let first = wrap(&keys[..1], "name", b"before").unwrap();
        assert_eq!(unwrap(&keys, "name", &first).unwrap(), b"before");
    }

    #[test]
    fn a_backup_opens_with_the_key_its_owners_recovery_key_gives_and_no_other() {
        let owner = owner();
        let to = backups(&owner).public();
        let sealed = seal(&to, &for_object("ab"), b"a blob").unwrap();
        let recovered = backups(&recovered(&recovery(&owner)).unwrap());
        assert_eq!(open(&recovered, &for_object("ab"), &sealed).unwrap().as_slice(), b"a blob");
        assert!(open(&recovered, &for_object("cd"), &sealed).is_err());
        assert!(open(&backups(&super::owner()), &for_object("ab"), &sealed).is_err());
    }

    #[test]
    fn a_device_opens_a_notification_sent_to_it_and_nothing_else_does() {
        let (device, other) = (SecretKey::generate(), SecretKey::generate());
        let said = br#"{"title":"Lunch is here","text":"At the door","app":"meals"}"#;
        let to = notifications(&device).public();
        let sealed = seal(&to, &for_notification(&to.to_string()), said).unwrap();
        assert!(!sealed.windows(5).any(|part| part == b"Lunch"));
        assert_eq!(open_notification(&device, &sealed).unwrap().as_slice(), said);
        assert!(open_notification(&other, &sealed).is_err());
        // Not with the device's own key: only the key derived for its notifications.
        assert!(open(&device, &for_notification(&notifications(&device).public().to_string()), &sealed).is_err());
    }

    #[test]
    fn a_recovery_key_is_the_owners_key_however_its_typed() {
        let owner = owner();
        let written = recovery(&owner);
        assert_eq!(recovered(&written).unwrap(), owner);
        assert_eq!(recovered(&written.to_lowercase().replace('-', " ")).unwrap(), owner);
        assert!(recovered("not one").is_err());
        assert_ne!(check(&owner), check(&super::owner()));
        assert_eq!(split(&joined(&artifact(&owner, "a", 3))).unwrap(), artifact(&owner, "a", 3));
    }
}
