//! Devices joining and leaving: the keys sealed to each (`GET`/`POST /v1/keys`, which any of the install's devices may
//! read, sealed as they are), approving a device's request to join, and removing one.

use super::*;
use crate::bucket::Bucket;
use data_encoding::BASE64URL_NOPAD as B64U;

/// The most a sealed key may be: an epoch's key sealed to an endpoint, or an app's under an epoch, is far less.
const SEALED_MAX: usize = 256;

/// Whom an epoch's key is sealed to, in `keys::epoch_name`: an endpoint's key in hex, or `recovery`.
fn sealed_to(to: &str) -> Result<()> {
    ensure!(to == "recovery" || crate::keys::unhex(to).is_ok(), "to is an endpoint's key (64 hex characters) or recovery");
    Ok(())
}

/// Whether `user` may start a scope's new epoch, as `members` counts an `epoch` entry: an admin of the install for
/// `global`, of the group for a group's, and the person for their own.
fn starts_epochs(m: &Members, user: &str, scope: &Scope) -> bool {
    match scope {
        Scope::Global => m.is_admin(user),
        Scope::Group(g) => m.manages(user, g),
        Scope::User(u) => u == user,
    }
}

impl Node {
    /// What's sealed of a scope's keys, for any person of the install, since it opens only with keys they may hold:
    /// with `to`, each of its epochs sealed to that endpoint, `{ epoch, sealed: { n: base64url } }`, `epoch` the newest
    /// made; with `app`, the app's key in each epoch, sealed under that epoch's, `{ sealed: { n: base64url } }`.
    pub async fn sealed_keys(&self, caller: &Caller, scope: &Scope, to: Option<&str>, app: Option<&str>) -> Result<Value> {
        ensure!(self.is_user(&caller.user), "only the install's people read its keys");
        let bucket = self.bucket()?;
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        let mut sealed = serde_json::Map::new();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                for n in 1..=newest {
                    if let Some(o) = Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, to)).await? {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "epoch": newest, "sealed": sealed }))
            }
            (None, Some(app)) => {
                ensure!(members::valid_app_name(app), "{app} isn't an app's name");
                for n in 1..=newest {
                    if let Some(o) = Bucket::get(&bucket, &crate::keys::app_key_name(&name, app, n)).await? {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "sealed": sealed }))
            }
            _ => bail!("give to (an endpoint's key, or recovery) or app"),
        }
    }

    /// Keeps a key sealed by a device of `caller`'s, who may use the scope: epoch `n` sealed to `to`, an endpoint listed
    /// as someone's who may use the scope (or the recovery key, or a new epoch, only by one who may start it), only if
    /// it isn't there (→ `{}`); or an app's key in epoch `n`, if it isn't there (→ `{ sealed }`, whichever is there
    /// now, which every device then uses).
    pub async fn seal_key(&self, caller: &Caller, scope: &Scope, n: u32, to: Option<&str>, app: Option<&str>, sealed: &str) -> Result<Value> {
        let sealed = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("sealed is base64url"))?;
        ensure!(!sealed.is_empty() && sealed.len() <= SEALED_MAX, "a sealed key is at most {SEALED_MAX} bytes");
        let bucket = self.bucket()?;
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                self.with(|i| {
                    let m = &i.r.members;
                    ensure!(m.may_use(&caller.user, scope), "you don't use {scope}");
                    ensure!(n >= 1 && n <= newest + 1, "{scope} has no epoch {n}: its newest is {newest}");
                    ensure!(n <= newest || starts_epochs(m, &caller.user, scope), "only {scope}'s admins start its new epoch");
                    match to {
                        "recovery" => ensure!(starts_epochs(m, &caller.user, scope), "only {scope}'s admins seal its keys to the recovery key"),
                        to => {
                            let key = crate::keys::unhex(to)?;
                            let listed = m.machine(&key).context("that endpoint isn't one of the install's")?;
                            ensure!(m.may_use(&listed.user, scope), "that endpoint's person doesn't use {scope}");
                        }
                    }
                    Ok(())
                })?;
                ensure!(crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, to), &sealed).await?, "{scope}'s epoch {n} is sealed to {to} already");
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(members::valid_app_name(app), "{app} isn't an app's name");
                ensure!(self.with(|i| Ok(i.r.members.may_use(&caller.user, scope)))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = crate::keys::app_key_name(&name, app, n);
                let there = match crate::db::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            _ => bail!("give to (an endpoint's key, or recovery) or app"),
        }
    }
}
