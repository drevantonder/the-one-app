//! Devices' keys and parts: the keys sealed to each (`GET`/`POST /v1/keys`, which any of the network's devices may read,
//! sealed as they are), new keys a scope is owed, a device's services listed as its parts, and erasing what a device
//! keeps. Letting devices in and taking them out is what this machine signs as its own device (`settings.rs`).

use crate::protocol::allowed;
use crate::config::YOURS;
use super::*;

pub(crate) mod routes;
use crate::bucket::Bucket;
use data_encoding::BASE64URL_NOPAD as B64U;

/// The most a sealed key may be: an epoch's key sealed to an endpoint, or an app's under an epoch, is far less.
const SEALED_MAX: usize = 256;

/// Whom an epoch's key is sealed to, in `keys::epoch_name`: an endpoint's key in hex, or `recovery`.
fn sealed_to(to: &str) -> Result<()> {
    ensure!(to == "recovery" || crate::keys::unhex(to).is_ok(), "to is an endpoint's key (64 hex characters) or recovery");
    Ok(())
}

/// A scope's keys that may need making again: its epoch, or its Vault's key pair.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owing {
    Epoch(Scope),
    Vault(Scope),
}

/// What `Node::start_owed_keys` knows between its looks.
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

/// Whether `who` may list a device of `user`'s that does `kinds`, as `rules` counts a `device` step: only of someone in
/// the network now (one removed, or not in it yet, is added with their first device, by an invite's approval); a
/// client or a service only from their own devices (`add-devices`), a machine that runs calls also by an admin
/// (`add-machines`), which runs only what that admin's listing may, never their own scope. The rows give the verbs;
/// which kind of device takes which is its rule's.
pub(super) fn may_list(db: &ScopeDb, who: &str, user: &str, kinds: &[scopes::Kind]) -> Result<bool> {
    let theirs = kinds.iter().any(|k| matches!(k, scopes::Kind::Client | scopes::Kind::Service));
    Ok(db.user(user)?.is_some() && db.may(who, if theirs { "add-devices" } else { "add-machines" }, &scopes::user_principal(user)))
}

impl Node {
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
                    let (bucket, at) = (&bucket, crate::keys::epoch_name(&name, n, to));
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
            (None, Some(app)) => {
                ensure!(app == scopes::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
                let got = futures_util::future::try_join_all((1..=newest).map(|n| {
                    let (bucket, at) = (&bucket, crate::keys::app_key_name(&name, app, n));
                    async move { Bucket::get(bucket, &at).await }
                }))
                .await?;
                for (n, o) in (1..=newest).zip(got) {
                    if let Some(o) = o {
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
    /// it isn't there (→ `{}`); an app's key in epoch `n`, if it isn't there (→ `{ sealed }`, whichever is there
    /// now, which every device then uses); or with both, an app's key in epoch `n` sealed to `to`, an endpoint of the
    /// network that will run the app's schedules there (`keys::app_schedules_key_name`), by one who may set them (→ `{}`,
    /// there already or not: the app schedules name the hash of the one they mean); or with `vault`, a Vault's newest key
    /// sealed to a machine it opens on, or the recovery key, by one who may say which those are.
    pub async fn seal_key(&self, caller: &Caller, scope: &Scope, n: u32, to: Option<&str>, app: Option<&str>, vault: bool, sealed: &str) -> Result<Value> {
        let sealed = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("sealed is base64url"))?;
        ensure!(!sealed.is_empty() && sealed.len() <= SEALED_MAX, "a sealed key is at most {SEALED_MAX} bytes");
        let bucket = self.bucket()?;
        if vault {
            let (Some(to), None) = (to, app) else { bail!("a Vault key is sealed to a machine, or recovery, and is no app's") };
            self.with(|i| {
                let db = i.r.db();
                allowed!(db.may(&caller.user, "choose-devices", &scopes::object(scope)), "only {scope}'s user, or its admins, say which machines open its Vault");
                let newest = db.vault(scope)?.filter(|v| v.n == n).with_context(|| format!("{scope}'s newest Vault key isn't {n}"))?;
                ensure!(to == "recovery" || newest.to.contains(&crate::keys::unhex(to)?), "{scope}'s Vault key {n} doesn't open on that machine");
                Ok(())
            })?;
            if !crate::database::stored::created(&bucket, &crate::keys::vault_key_name(&scope.to_string(), n, to), &sealed).await? {
                return Err(crate::protocol::Coded::error(crate::protocol::code::ALREADY, format!("{scope}'s Vault key {n} is sealed to {to} already")));
            }
            return Ok(json!({}));
        }
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                self.with(|i| {
                    let db = i.r.db();
                    allowed!(db.may(&caller.user, "use", &scopes::object(scope)), "you don't use {scope}");
                    ensure!(n >= 1 && n <= newest + 1, "{scope} has no epoch {n}: its newest is {newest}");
                    allowed!(n <= newest || db.starts_epochs(&caller.user, scope)?, "only {scope}'s admins start its new epoch");
                    match to {
                        "recovery" => allowed!(db.starts_epochs(&caller.user, scope)?, "only {scope}'s admins seal its keys to the recovery key"),
                        to => {
                            let key = crate::keys::unhex(to)?;
                            // A scope's epochs are its users' client devices' alone: a node is handed a call's keys,
                            // and a service holds none, reading nothing and asking its user's node to write.
                            let listed = db.client(&key)?.context("a scope's keys are sealed only to client devices and the recovery key")?;
                            allowed!(db.may(&listed.user, "use", &scopes::object(scope)), "that endpoint's user doesn't use {scope}");
                            ensure!(db.service(&key)?.is_none(), "a service is sealed no keys");
                        }
                    }
                    Ok(())
                })?;
                if !crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, to), &sealed).await? {
                    return Err(crate::protocol::Coded::error(crate::protocol::code::ALREADY, format!("{scope}'s epoch {n} is sealed to {to} already")));
                }
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(app == scopes::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
                allowed!(self.with(|i| Ok(i.r.db().may(&caller.user, "use", &scopes::object(scope))))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = crate::keys::app_key_name(&name, app, n);
                let there = match crate::database::stored::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            (Some(to), Some(app)) => {
                ensure!(scopes::valid_app_name(app), "{app} isn't an app's name");
                let key = crate::keys::unhex(to)?;
                self.with(|i| {
                    let db = i.r.db();
                    allowed!(db.may(&caller.user, "choose-devices", &scopes::object(scope)), "only {scope}'s user, or its group's admins, choose where its apps' schedules run");
                    ensure!(db.machine(&key)?.is_some(), "that endpoint isn't one of the network's");
                    Ok(())
                })?;
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                crate::database::stored::created(&bucket, &crate::keys::app_schedules_key_name(&name, app, to, n), &sealed).await?;
                Ok(json!({}))
            }
            _ => bail!("give to (an endpoint's key, or recovery), app, or both"),
        }
    }
}

impl Node {
    /// Who holds the recovery key, for any user of the network, as who's who says it does and what the bucket has for
    /// them: `{ recovery, holders: [{ user, name, devices: [{ key, seal, name, holds }] }] }`. A device with `holds` false
    /// waits for someone with the key to seal it (`devices::cli`).
    pub async fn recovery_holders(&self, caller: &Caller) -> Result<Value> {
        ensure!(self.is_user(&caller.user), "only the network's users read who holds its recovery key");
        let bucket = self.bucket()?;
        let (recovery, holders) = self.with(|i| {
            let db = i.r.db();
            let mut holders = Vec::new();
            for user in db.recovery_holders()? {
                let name = db.user(&user)?.map(|u| u.name).unwrap_or_default();
                holders.push((user.clone(), name, db.recovery_devices(&user)?));
            }
            Ok((db.recovery()?.as_ref().map(hex), holders))
        })?;
        let mut out = Vec::new();
        for (user, name, devices) in holders {
            let mut listed = Vec::new();
            for d in devices {
                let holds = match &recovery {
                    Some(r) => Bucket::get(&bucket, &crate::keys::recovery_held_name(r, &hex(&d.key))).await?.is_some(),
                    None => false,
                };
                listed.push(json!({ "key": hex(&d.key), "seal": hex(&d.seal), "name": d.name, "holds": holds }));
            }
            out.push(json!({ "user": user, "name": name, "devices": listed }));
        }
        Ok(json!({ "recovery": recovery, "holders": out }))
    }

    /// Keeps the recovery key, sealed by whoever has its file on this machine, for `to`: only a device who's who says may
    /// hold it, of someone an admin named (`ScopeDb::recovery_devices`), and only by an admin's own machine. This node
    /// never sees the key: what's sealed goes by. One there already stays.
    pub async fn seal_recovery(&self, to: &str, sealed: &str) -> Result<()> {
        let sealed = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("sealed is base64url"))?;
        ensure!(!sealed.is_empty() && sealed.len() <= SEALED_MAX, "a sealed key is at most {SEALED_MAX} bytes");
        let key = crate::keys::unhex(to)?;
        let me = self.user()?;
        let recovery = self.with(|i| {
            let db = i.r.db();
            allowed!(db.may(&me, "change-settings", scopes::NETWORK), "only an admin seals the recovery key to someone who holds it");
            let may_hold = db.recovery_holders()?.into_iter().try_fold(false, |found, user| Ok::<_, anyhow::Error>(found || db.recovery_devices(&user)?.iter().any(|d| d.key == key)))?;
            ensure!(may_hold, "the recovery key is sealed only to a device of someone an admin named, and not to a browser, a node or a passkey");
            db.recovery()?.context("this network has no recovery key to seal")
        })?;
        if !crate::database::stored::created(&self.bucket()?, &crate::keys::recovery_held_name(&hex(&recovery), to), &sealed).await? {
            return Err(crate::protocol::Coded::error(crate::protocol::code::ALREADY, "that device has the recovery key already"));
        }
        Ok(())
    }
}

impl Node {
    /// The network's address, where users sign in.
    pub fn address(&self) -> Result<String> {
        self.with(|i| super::calls::address(i.r.db()))?.with_context(|| format!("{YOURS} has no address yet, so it has no sign-ins or requests to join"))
    }

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
            self.tell_relay(json!({ "add": hex(&now), "role": "client" }));
        }
    }

    /// Lists one of this device's parts, the service `service`, as its user's, a part of this device (oned turned it
    /// on, and it asked with its endpoint), and tells the relay to let it in. It holds no
    /// keys: the nodes that call it bring it the network's entries, which say who may. A part listed before it named its
    /// device is listed again, naming it.
    pub async fn list_part(&self, endpoint: &crate::keys::Endpoint, service: &str) -> Result<()> {
        let user = self.user()?;
        let caller = Caller::own(self.0.id, user.clone());
        let listed = self.with(|i| {
            let db = i.r.db();
            ensure!(db.service(&endpoint.key)?.is_some() || db.machine_ever(&endpoint.key)?.is_none(), "that key is listed already, and not as a service");
            Ok(db.service(&endpoint.key)?.map(|s| s.device == Some(self.0.id)))
        })?;
        if listed.is_none() {
            // Told first: a service the relay won't let in is never reached.
            if let Err(err) = crate::settings::tell(self, self, json!({ "add": hex(&endpoint.key), "admin": false, "role": "service" })).await {
                ensure!(crate::protocol::Coded::of(&err).as_deref() == Some(crate::protocol::code::ALREADY), "{YOURS}'s relay won't let {service} in: {err:#}");
            }
        }
        if listed != Some(true) {
            let name = self.device_name();
            self.member_content(&caller, content::service_endpoint(endpoint, &user, &name, service, &self.0.id)).await?;
        }
        // Its first hello brings it the entries that say it's theirs: as soon as the relay lets it in.
        let (node, key) = (self.clone(), endpoint.key);
        tokio::spawn(async move {
            for _ in 0..20 {
                if node.hello_service_now(&key).await {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            tracing::warn!("service {} was listed but this device hasn't reached it", crate::keys::short(&key));
        });
        Ok(())
    }

    /// Erases what this device keeps (by name or key), for its user or an admin asking from one of their own devices,
    /// directly: asked of another device, it says to ask that one, if it's on. Once it has answered, its oned stops its
    /// parts and deletes its home (its copies of the network's data, the pages it kept, and each service's folder and
    /// data, WhatsApp's link among them), then stops. Removing it next takes it out. → what's erasing.
    pub async fn erase(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let name = self.with(|i| Ok(i.r.db().machine(&key)?.map(|d| d.name).unwrap_or_default()))?;
        if key != self.0.id {
            ensure!(self.with(|i| Ok(i.online(&key)))?, "{name} isn't on, so it can't erase what it keeps");
            return Err(super::calls::Elsewhere { why: format!("only {name} erases what it keeps"), nodes: vec![hex(&key)] }.into());
        }
        self.with(|i| {
            let db = i.r.db();
            allowed!(caller.key != self.0.id && db.client(&caller.key)?.is_some() && db.may(&caller.user, "remove", &scopes::device(&key)), "only {name}'s user or an admin erases it, from another of their devices");
            Ok(())
        })?;
        tracing::warn!("asked to erase what {name} keeps: its oned does, then stops");
        self.0.parts.erase();
        Ok(json!({ "erasing": name }))
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
                if db.client(&self.0.id)?.is_none_or(|c| c.user != me) {
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
                    if db.sealed_to(&s)?.contains(&self.0.id) && db.starts_epochs(&me, &s)? {
                        epochs.push((Owing::Epoch(s), n));
                    }
                }
                let vaults: BTreeMap<Scope, (u32, Vec<Key>)> = db.vaults()?.into_iter().filter(|(s, v)| v.to.contains(&self.0.id) && db.may(&me, "choose-devices", &scopes::object(s))).map(|(s, v)| (s, (v.n, v.to))).collect();
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
            let node = self.clone();
            tokio::spawn(async move {
                let Ok(bucket) = node.bucket() else { return };
                let mut unsealed = BTreeSet::new();
                for (owing, n) in unsure {
                    let name = match &owing {
                        Owing::Epoch(s) => crate::keys::epoch_name(&s.to_string(), n, &hex(&node.0.id)),
                        Owing::Vault(s) => crate::keys::vault_key_name(&s.to_string(), n, &hex(&node.0.id)),
                    };
                    match Bucket::get(&bucket, &name).await {
                        Ok(Some(_)) => drop(node.0.owed.lock().unwrap().sealed.insert((owing, n))),
                        Ok(None) => drop(unsealed.insert((owing, n))),
                        Err(err) => tracing::debug!("looking in the bucket for {name}: {err:#}"),
                    }
                }
                node.0.owed.lock().unwrap().unsealed = unsealed;
            });
        }
        for owing in due {
            let (node, vaults) = (self.clone(), vaults.clone());
            tokio::spawn(async move {
                let made = match &owing {
                    Owing::Epoch(scope) => match node.newest_epoch(scope).await {
                        Ok(newest) => crate::settings::new_epoch(&node, scope, newest + 1, &[]).await,
                        Err(err) => Err(err),
                    },
                    Owing::Vault(scope) => match vaults.get(scope) {
                        Some((n, to)) => crate::settings::new_vault_key(&node, scope, n + 1, to).await,
                        None => Ok(()),
                    },
                };
                match made {
                    Ok(()) => tracing::info!("made {owing:?}'s new keys, which it owed"),
                    Err(err) => tracing::warn!("making {owing:?}'s new keys, which it owed: {err:#}"),
                }
                node.0.owed.lock().unwrap().starting.remove(&owing);
            });
        }
    }
}
