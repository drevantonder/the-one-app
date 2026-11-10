//! What the device's own key opens and signs, which a node asks of its holder rather than taking for its own: the
//! epochs of the scopes the device opens, anything else sealed to it, and its steps in `global`'s scope database, which
//! only a user's own devices sign. A computer's node is its device today, so its holder is its own key. Once the app
//! runtime is an endpoint of its own key, the device's `onet` holds it, and the runtime asks it for these three.

use super::*;
use crate::database::stored::Keys;

pub(super) struct Holds {
    key: SigningKey,
}

impl Holds {
    pub(super) fn new(key: SigningKey) -> Holds {
        Holds { key }
    }

    /// The device whose key this is.
    pub(super) fn device(&self) -> Key {
        self.key.verifying_key().to_bytes()
    }

    /// The key the device signs its steps in `global` with.
    pub(super) fn signs_global(&self) -> &SigningKey {
        &self.key
    }

    /// The epochs of `scope` sealed to the device in the bucket, of those `hashes` names by number, each kept only if
    /// it's the key its entry made: one that isn't is someone's junk, passed over.
    pub(super) async fn epochs(&self, bucket: &Copied, scope: &Scope, hashes: &BTreeMap<u32, String>) -> Result<Keys> {
        let mut out = Keys::new();
        for (n, hash) in hashes {
            let name = crate::keys::epoch_name(&scope.to_string(), *n, &hex(&self.device()));
            let Some(o) = crate::bucket::Bucket::get(bucket, &name).await? else { continue };
            match crate::keys::open_sealed_by(self, &o.bytes).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == *hash) {
                Some(key) => drop(out.insert(*n, key)),
                None => tracing::warn!("{name} isn't the key {scope}'s epoch {n} entry made: passed over"),
            }
        }
        Ok(out)
    }
}

impl crate::keys::Opens for Holds {
    fn public(&self) -> [u8; 32] {
        crate::keys::public_of(&crate::keys::seal_secret(&self.key))
    }

    async fn shared(&self, share: &[u8; 32]) -> Result<[u8; 32]> {
        Ok(crate::keys::agree(&crate::keys::seal_secret(&self.key), share))
    }
}

impl crate::keys::Signs for Holds {
    fn public(&self) -> [u8; 32] {
        self.device()
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        self.key.sign_bytes(bytes).await
    }
}
