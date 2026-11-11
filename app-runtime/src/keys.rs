//! Scopes' keys: those sealed to each device (`GET`/`POST /api/keys`, which any of the network's devices may read,
//! sealed as they are), new keys a scope is owed, and the relay told of a new recovery key. Letting devices in and
//! taking them out is what this machine signs as its own device (`settings.rs`).

use ::custom_build::allowed;
use super::*;

use ::bucket::Bucket;
use data_encoding::BASE64URL_NOPAD as B64U;

/// The most a sealed key may be: an epoch's key sealed to an endpoint, or an app's under an epoch, is far less.
const SEALED_MAX: usize = 256;

/// Whom an epoch's key is sealed to, in `keys::epoch_name`: an endpoint's key in hex, or `recovery`.
fn sealed_to(to: &str) -> Result<()> {
    ensure!(to == "recovery" || key::unhex(to).is_ok(), "to is an endpoint's key (64 hex characters) or recovery");
    Ok(())
}

/// A scope's keys that may need making again: its epoch, or its Vault's key pair.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owing {
    Epoch(Scope),
    Vault(Scope),
}

/// What `Host::start_owed_keys` knows between its looks.
#[derive(Default)]
pub(super) struct Owed {
    /// What owed new keys at the last look.
    seen: BTreeSet<Owing>,
    /// What it's making new keys of now.
    starting: BTreeSet<Owing>,
    /// Keys whose entry lists this device that are sealed to it, by number, known so they're asked of the bucket once.
    sealed: BTreeSet<(Owing, u32)>,
    /// Those that weren't, at the last look.
    unsealed: BTreeSet<(Owing, u32)>,
}

impl Host {
    /// What's sealed of a scope's keys, for any user of the network, since it opens only with keys they may hold:
    /// with `to`, each of its epochs sealed to that endpoint, `{ epoch, sealed: { n: base64url } }`, `epoch` the newest
    /// made; with `app`, the app's key in each epoch, sealed under that epoch's, `{ sealed: { n: base64url } }`.
    pub async fn sealed_keys(&self, caller: &Caller, scope: &Scope, to: Option<&str>, app: Option<&str>) -> Result<Value> {
        ensure!(self.is_user(&caller.user), "only the network's users read its keys");
        let bucket = self.bucket()?;
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        let mut sealed = serde_json::Map::new();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                // Every epoch at once: one wait on the bucket, not one each.
                let got = futures_util::future::try_join_all((1..=newest).map(|n| {
                    let (bucket, at) = (&bucket, database::keys::epoch_name(&name, n, to));
                    async move { Bucket::get(bucket, &at).await }
                }))
                .await?;
                for (n, o) in (1..=newest).zip(got) {
                    if let Some(o) = o {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "epoch": newest, "sealed": sealed }))
            }
            // One the bucket hadn't is asked for again only once who's who has moved on, or this node keeps one: a
            // device that wants it makes it, and is given whichever is there (`seal_key`).
            (None, Some(app)) => {
                ensure!(app == key::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
                let at = self.with(|i| Ok(i.r.at()))?;
                let id = |n: u32| (scope.clone(), app.to_string(), n);
                let asked: Vec<u32> = (1..=newest).filter(|n| self.0.no_app_keys.lock().unwrap().get(&id(*n)) != Some(&at)).collect();
                let got = futures_util::future::try_join_all(asked.iter().map(|n| {
                    let (bucket, at) = (&bucket, database::keys::app_key_name(&name, app, *n));
                    async move { Bucket::get(bucket, &at).await }
                }))
                .await?;
                let mut missing = self.0.no_app_keys.lock().unwrap();
                for (n, o) in asked.into_iter().zip(got) {
                    match o {
                        Some(o) => drop(sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)))),
                        None => drop(missing.insert(id(n), at)),
                    }
                }
                Ok(json!({ "sealed": sealed }))
            }
            _ => bail!("give to (an endpoint's key, or recovery) or app"),
        }
    }

    /// Keeps a key sealed by a device of `caller`'s, who may use the scope: epoch `n` sealed to `to`, an endpoint listed
    /// as someone's who may use the scope (or the recovery key, or a new epoch, only by one who may start it), only if
    /// it isn't there (→ `{}`); an app's key in epoch `n`, if it isn't there (→ `{ sealed }`, whichever is there
    /// now, which every device then uses); or with both, an app's key in epoch `n` sealed to `to`, an endpoint of the
    /// network that will run the app's schedules there (`keys::app_schedules_key_name`), by one who may set them (→ `{}`,
    /// there already or not: the app schedules name the hash of the one they mean); or with `vault`, a Vault's newest key
    /// sealed to a machine it opens on, or the recovery key, by one who may say which those are; or with `vault` and an
    /// app, the Vault's database's key in epoch `n` sealed under that app's, for its app schedules
    /// (`keys::app_schedules_vault_name`), by one who may set them (→ `{}`, there already or not: only the app's key opens it).
    pub async fn seal_key(&self, caller: &Caller, scope: &Scope, n: u32, to: Option<&str>, app: Option<&str>, vault: bool, sealed: &str) -> Result<Value> {
        let sealed = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("sealed is base64url"))?;
        ensure!(!sealed.is_empty() && sealed.len() <= SEALED_MAX, "a sealed key is at most {SEALED_MAX} bytes");
        let bucket = self.bucket()?;
        if vault && app.is_none() {
            let Some(to) = to else { bail!("a Vault key is sealed to a machine, or recovery") };
            self.with(|i| {
                let db = i.r.db();
                allowed!(db.may(&caller.user, "choose-devices", &scopes::object(scope)), "only {scope}'s user, or its admins, say which machines open its Vault");
                let newest = db.vault(scope)?.filter(|v| v.n == n).with_context(|| format!("{scope}'s newest Vault key isn't {n}"))?;
                ensure!(to == "recovery" || newest.to.contains(&key::unhex(to)?), "{scope}'s Vault key {n} doesn't open on that machine");
                Ok(())
            })?;
            if !database::stored::created(&bucket, &database::keys::vault_key_name(&scope.to_string(), n, to), &sealed).await? {
                return Err(::custom_build::protocol::Coded::error(::custom_build::protocol::code::ALREADY, format!("{scope}'s Vault key {n} is sealed to {to} already")));
            }
            return Ok(json!({}));
        }
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        match (to, app) {
            (None, Some(app)) if vault => {
                ensure!(scopes::valid_app_name(app), "{app} isn't an app's name");
                allowed!(self.with(|i| Ok(i.r.db().may(&caller.user, "choose-devices", &scopes::object(scope))))?, "only {scope}'s user, or its group's admins, choose where its apps' schedules run");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                database::stored::created(&bucket, &database::keys::app_schedules_vault_name(&name, app, n), &sealed).await?;
                Ok(json!({}))
            }
            (Some(_), Some(_)) if vault => bail!("the Vault's key for an app's schedules is kept under the app's, and sealed to no device"),
            (Some(to), None) => {
                sealed_to(to)?;
                self.with(|i| {
                    let db = i.r.db();
                    // Whoever holds the recovery key can open any scope's keys, so may seal them to the user they're for.
                    allowed!(db.may(&caller.user, "use", &scopes::object(scope)) || (n <= newest && db.recovery_holders()?.contains(&caller.user)), "you don't use {scope}");
                    ensure!(n >= 1 && n <= newest + 1, "{scope} has no epoch {n}: its newest is {newest}");
                    allowed!(n <= newest || db.starts_epochs(&caller.user, scope)?, "only {scope}'s admins start its new epoch");
                    match to {
                        "recovery" => allowed!(db.starts_epochs(&caller.user, scope)?, "only {scope}'s admins seal its keys to the recovery key"),
                        to => {
                            let key = key::unhex(to)?;
                            // A scope's epochs are its users' client devices' alone: a node is handed a call's keys,
                            // and a service holds none, reading nothing and asking its user's node to write.
                            let listed = db.client(&key)?.context("a scope's keys are sealed only to client devices and the recovery key")?;
                            allowed!(db.may(&listed.user, "use", &scopes::object(scope)), "that endpoint's user doesn't use {scope}");
                            ensure!(db.service(&key)?.is_none(), "a service is sealed no keys");
                        }
                    }
                    Ok(())
                })?;
                if !database::stored::created(&bucket, &database::keys::epoch_name(&name, n, to), &sealed).await? {
                    return Err(::custom_build::protocol::Coded::error(::custom_build::protocol::code::ALREADY, format!("{scope}'s epoch {n} is sealed to {to} already")));
                }
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(app == key::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
                allowed!(self.with(|i| Ok(i.r.db().may(&caller.user, "use", &scopes::object(scope))))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = database::keys::app_key_name(&name, app, n);
                self.0.no_app_keys.lock().unwrap().remove(&(scope.clone(), app.to_string(), n));
                let there = match database::stored::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            (Some(to), Some(app)) => {
                ensure!(scopes::valid_app_name(app), "{app} isn't an app's name");
                let key = key::unhex(to)?;
                self.with(|i| {
                    let db = i.r.db();
                    allowed!(db.may(&caller.user, "choose-devices", &scopes::object(scope)), "only {scope}'s user, or its group's admins, choose where its apps' schedules run");
                    ensure!(db.machine(&key)?.is_some(), "that endpoint isn't one of the network's");
                    Ok(())
                })?;
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                database::stored::created(&bucket, &database::keys::app_schedules_key_name(&name, app, to, n), &sealed).await?;
                Ok(json!({}))
            }
            _ => bail!("give to (an endpoint's key, or recovery), app, or both"),
        }
    }
}

impl Host {
    /// Once the network's recovery key changed, an admin's device tells the relay to let the old one in no more and the
    /// new one in as a client: once each since this device started.
    pub(super) fn follow_recovery(&self) {
        let told = self.with(|i| {
            let db = i.r.db();
            let admins = db.machine(&self.0.id)?.is_some_and(|d| db.may(&d.user, "invite", scopes::NETWORK));
            let was = db.recovery_was()?;
            Ok(db.recovery()?.filter(|_| admins && !was.is_empty()).map(|now| (now, was)))
        });
        let Ok(Some((now, was))) = told else { return };
        let mut done = self.0.recovery_told.lock().unwrap();
        for old in was.into_iter().filter(|k| done.insert(*k)) {
            self.tell_relay(json!({ "remove": hex(&old) }));
        }
        if done.insert(now) {
            self.tell_relay(json!({ "add": hex(&now), "role": "client", "recovery": true }));
        }
    }

    /// Makes, in the background, each scope's keys that are owed, of what this device's user makes them of: a new
    /// epoch a removal or stopped app schedules asked for (`ScopeDb::owed`) that the device writing it didn't make (it failed,
    /// or went off first); or a new epoch or Vault key pair where the newest's entry lists this device but it was never
    /// sealed to it, as when the device making it stopped between its entry and its seals, so nobody holds it. Only
    /// once one has been owed at two looks in a row, so a removal's own new epoch, and every seal, goes first.
    pub(super) fn start_owed_keys(&self) {
        let Ok(me) = self.user() else { return };
        let owing = self
            .with(|i| {
                let db = i.r.db();
                if db.client(&self.0.holds.device())?.is_none_or(|c| c.user != me) {
                    return Ok(Default::default());
                }
                let mut owed = BTreeSet::new();
                for s in db.owed()? {
                    if db.starts_epochs(&me, &s)? {
                        owed.insert(Owing::Epoch(s));
                    }
                }
                let mut epochs = Vec::new();
                for (s, n) in db.epochs()? {
                    if db.sealed_to(&s)?.contains(&self.0.holds.device()) && db.starts_epochs(&me, &s)? {
                        epochs.push((Owing::Epoch(s), n));
                    }
                }
                let vaults: BTreeMap<Scope, (u32, Vec<Key>)> = db.vaults()?.into_iter().filter(|(s, v)| v.to.contains(&self.0.holds.device()) && db.may(&me, "choose-devices", &scopes::object(s))).map(|(s, v)| (s, (v.n, v.to))).collect();
                let listed: Vec<(Owing, u32)> = epochs.into_iter().chain(vaults.iter().map(|(s, (n, _))| (Owing::Vault(s.clone()), *n))).collect();
                Ok((owed, listed, vaults))
            });
        // A failed look is tried again at the next, which comes on its own.
        let (mut owed, listed, vaults) = match owing {
            Ok(owing) => owing,
            Err(err) => {
                tracing::warn!("looking for epochs this device owes: {err:#}");
                return;
            }
        };
        let (due, unsure) = {
            let mut held = self.0.owed.lock().unwrap();
            owed.extend(held.unsealed.iter().filter(|e| listed.contains(e)).map(|(o, _)| o.clone()));
            let due = owed.intersection(&held.seen).filter(|o| !held.starting.contains(*o)).cloned().collect::<Vec<_>>();
            held.starting.extend(due.iter().cloned());
            held.seen = owed;
            let unsure: Vec<(Owing, u32)> = listed.into_iter().filter(|e| !held.sealed.contains(e)).collect();
            (due, unsure)
        };
        if !unsure.is_empty() {
            let host = self.clone();
            tokio::spawn(async move {
                let Ok(bucket) = host.bucket() else { return };
                let mut unsealed = BTreeSet::new();
                for (owing, n) in unsure {
                    let name = match &owing {
                        Owing::Epoch(s) => database::keys::epoch_name(&s.to_string(), n, &hex(&host.0.holds.device())),
                        Owing::Vault(s) => database::keys::vault_key_name(&s.to_string(), n, &hex(&host.0.holds.device())),
                    };
                    match Bucket::get(&bucket, &name).await {
                        Ok(Some(_)) => drop(host.0.owed.lock().unwrap().sealed.insert((owing, n))),
                        Ok(None) => drop(unsealed.insert((owing, n))),
                        Err(err) => tracing::debug!("looking in the bucket for {name}: {err:#}"),
                    }
                }
                host.0.owed.lock().unwrap().unsealed = unsealed;
            });
        }
        for owing in due {
            let (host, vaults) = (self.clone(), vaults.clone());
            tokio::spawn(async move {
                let made = match &owing {
                    Owing::Epoch(scope) => match host.newest_epoch(scope).await {
                        Ok(newest) => scope::settings::new_epoch(&host, scope, newest + 1, &[]).await,
                        Err(err) => Err(err),
                    },
                    Owing::Vault(scope) => match vaults.get(scope) {
                        Some((n, to)) => scope::settings::new_vault_key(&host, scope, n + 1, to).await,
                        None => Ok(()),
                    },
                };
                match made {
                    Ok(()) => tracing::info!("made {owing:?}'s new keys, which it owed"),
                    Err(err) => tracing::warn!("making {owing:?}'s new keys, which it owed: {err:#}"),
                }
                host.0.owed.lock().unwrap().starting.remove(&owing);
            });
        }
    }
}
