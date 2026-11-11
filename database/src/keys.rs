//! Where keys are kept in the bucket, and opening them from it: each scope's epochs, its apps' keys and the keys made
//! from them for each database, the keys held for app schedules and the Vault, and the recovery key's copies.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use key::*;

/// An app's key in epoch `n` of a scope, from its copy in the bucket (`app_key_name`), sealed under that epoch's
/// key. The first to need it makes it, at random, unless `make` is false (an epoch nothing was written under yet,
/// where a reader has nothing to open): whichever is written first is the key.
pub async fn app_key(bucket: &impl bucket::Bucket, epoch_key: &[u8; 32], scope: &str, app: &str, n: u32, make: bool) -> Result<Option<[u8; 32]>> {
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
pub async fn database_keys(bucket: &impl bucket::Bucket, scope: &str, app: &str, database: &str, epochs: &BTreeMap<u32, [u8; 32]>, make: bool) -> Result<BTreeMap<u32, [u8; 32]>> {
    let newest = epochs.keys().next_back().copied();
    let mut out = BTreeMap::new();
    for (n, epoch) in epochs {
        if let Some(key) = app_key(bucket, epoch, scope, app, *n, make && Some(*n) == newest).await? {
            out.insert(*n, database_key(&key, app, database));
        }
    }
    Ok(out)
}

/// The folder of a scope's epochs, one folder each (`epoch_name`).
pub fn epochs_folder(scope: &str) -> String {
    format!("{}/_epochs", bucket::folder(scope))
}

/// Where epoch `n` of a scope is kept sealed to `to`: an endpoint key, or `recovery`.
pub fn epoch_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/{n}/{to}", epochs_folder(scope))
}

/// The folder of an app's keys in a scope, one by epoch (`app_key_name`).
pub fn app_keys_folder(scope: &str, app: &str) -> String {
    format!("{}/{}/_key", bucket::folder(scope), permanent(app, "").app)
}

/// Where an app's key in epoch `n` of a scope is kept, sealed under that epoch's key.
pub fn app_key_name(scope: &str, app: &str, n: u32) -> String {
    format!("{}/{n}", app_keys_folder(scope, app))
}

/// Where an app's key in epoch `n` of a scope is kept sealed to an endpoint that runs the app's schedules there: its
/// key in hex.
pub fn app_schedules_key_name(scope: &str, app: &str, endpoint: &str, n: u32) -> String {
    format!("{}/{}/_app_schedules/{endpoint}/{n}", bucket::folder(scope), permanent(app, "").app)
}

/// Where the Vault's database's key in epoch `n` of a scope is kept for app schedules of `app`, sealed under the app's
/// key in that epoch: what opens the Vault's items for whatever runs them, as a call of the app is handed them.
pub fn app_schedules_vault_name(scope: &str, app: &str, n: u32) -> String {
    format!("{}/{}/_app_schedules/vault/{n}", bucket::folder(scope), permanent(app, "").app)
}

/// Where a build from before app schedules had their name sealed it: read when it isn't under its own name.
pub fn app_schedules_key_name_before(scope: &str, app: &str, endpoint: &str, n: u32) -> String {
    format!("{}/{}/_grants/{endpoint}/{n}", bucket::folder(scope), permanent(app, "").app)
}

/// Where a scope's Vault secret key `n` is kept sealed to `to`: a machine's key granted it, or `recovery`. Its public
/// half is in the scope's `vault` entry, and what's sealed to that opens only on those machines.
pub fn vault_key_name(scope: &str, n: u32, to: &str) -> String {
    format!("{}/{n}/{to}", vault_keys_folder(scope))
}

/// Where a recovery key whose twin is `twin` finds epoch `n` of a scope: under its own name first, as it was handed
/// over to it, then at `recovery`.
pub fn recovery_epoch_names(scope: &str, n: u32, twin: &[u8; 32]) -> [String; 2] {
    [epoch_name(scope, n, &recovery_to(twin)), epoch_name(scope, n, "recovery")]
}

/// Where the network's recovery key (`recovery`, its public half) is kept sealed to a device (`to`) of someone who holds
/// it, so a new recovery key starts with none of the old one's.
pub fn recovery_held_name(recovery: &str, to: &str) -> String {
    format!("{}/{recovery}/{to}", recovery_held_folder())
}

/// Where each recovery key's copies are, one folder each.
pub fn recovery_held_folder() -> String {
    format!("{}/_recovery", bucket::folder("global"))
}

/// The folder of a scope's Vault secret keys, one folder each (`vault_key_name`).
pub fn vault_keys_folder(scope: &str) -> String {
    format!("{}/_vault", bucket::folder(scope))
}
