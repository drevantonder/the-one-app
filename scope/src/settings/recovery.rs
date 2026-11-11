//! What the network's recovery key does on the computer where its file is read, never asked over the network: letting
//! back in someone who lost every device, replacing the key, and keeping a holder's sealed copy of it. Every computer
//! runs this one copy as its device (`Me`): its node, and its onet.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use crate::settings::Me;
use crate::content;
use crate::scopes::{self, Kind, Scope, ScopeDb};
use ::key::{Endpoint, Key, SigningKey, hex};
use ::custom_build::{protocol::{Coded, code}, allowed};

/// The most a holder's copy of the recovery key may be.
const COPY_MAX: usize = 256 + 64;

/// Lets back in someone who lost every device, with the network's recovery key: the device whose join code is `code`,
/// theirs, is listed by an entry the recovery key signs (which counts as an admin's approval, and is listed as its use),
/// sealed each epoch of each scope they may open (opened through the recovery key's copy), and let onto the network by
/// the recovery key. Without `confirm`, says whose it is. → what it is, or what was done, and `said`, in words.
pub async fn approve(m: &impl Me, code: &str, recovery: &SigningKey, confirm: bool) -> Result<Value> {
    let at = recovery.verifying_key().to_bytes();
    ensure!(m.who(|db| db.recovery())? == Some(at), "that isn't {}'s recovery key", ::custom_build::config::YOURS);
    let r = crate::settings::asked_with(m, code).await?;
    let (endpoint, user, name) = (*r.endpoint(), r.user().to_string(), r.name().to_string());
    let theirs = format!("the recovery key lets in a device of someone already in {}", ::custom_build::config::YOURS);
    ensure!(!r.new(), "{theirs}");
    let user_name = m.who(|db| Ok(db.user(&user)?.map(|u| u.name)))?.context(theirs)?;
    if !confirm {
        return Ok(said(json!({ "device": name, "user": user_name, "recovery": true })));
    }
    m.read().await?;
    // Listed already by an approval that didn't finish: the rest is done again, each step once.
    if !m.who(|db| Ok(db.client(&endpoint.key)?.is_some_and(|c| c.user == user && c.seal == endpoint.seal)))? {
        m.write_as(recovery, &user, content::device(&endpoint, &user, &name, &[Kind::Client], None)).await?;
    }
    let admin = m.who(|db| Ok(db.may(&user, "invite", scopes::NETWORK)))?;
    let relay = crate::settings::tell(m, recovery, json!({ "add": hex(&endpoint.key), "role": "client", "admin": admin })).await.err().map(|err| format!("{err:#}"));
    let scopes: Vec<Scope> = m.who(|db| db.may_scopes(&user, "use"))?;
    let (mut sealed, mut not) = (Vec::new(), Vec::new());
    for scope in scopes {
        match seal_from_recovery(m, &scope, recovery, &endpoint).await {
            Ok(0) => {}
            Ok(_) => sealed.push(scope.to_string()),
            Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
        }
    }
    if let Err(err) = crate::settings::mark_approved(m, &endpoint.key).await {
        tracing::warn!("marking a request approved at auth: {err:#}");
    }
    Ok(said(json!({ "device": name, "user": user_name, "recovery": true, "sealed": sealed, "not_sealed": not, "relay": relay })))
}

fn said(mut approved: Value) -> Value {
    approved["said"] = json!(crate::settings::said_approved(&approved));
    approved
}

/// Seals each epoch of `scope` its entries made, opened through the recovery key's copy, to `to` where it isn't yet.
/// → how many it opened.
async fn seal_from_recovery(m: &impl Me, scope: &Scope, recovery: &SigningKey, to: &Endpoint) -> Result<usize> {
    let name = scope.to_string();
    let hashes: BTreeMap<u32, String> = m.who(|db| db.epoch_hashes(scope))?;
    let secret = ::key::seal_secret(recovery);
    let twin = ::key::twin_public(&recovery.verifying_key().to_bytes())?;
    let mut opened = 0;
    for (n, hash) in hashes {
        let Some(key) = opened_for(m, &name, n, &hash, &secret, &twin).await? else {
            tracing::warn!("{scope}'s epoch {n} isn't sealed to the recovery key, or isn't the key its entry made: passed over");
            continue;
        };
        let at = database::keys::epoch_name(&name, n, &hex(&to.key));
        if m.stored(&at).await?.is_none() {
            m.create(&at, &::key::seal_to(&to.seal, &key)?).await?;
        }
        opened += 1;
    }
    Ok(opened)
}

/// Epoch `n` of the scope named `name`, opened with the recovery key whose twin's secret and public are given.
async fn opened_for(m: &impl Me, name: &str, n: u32, hash: &str, secret: &[u8; 32], twin: &[u8; 32]) -> Result<Option<[u8; 32]>> {
    let mut kept = Vec::new();
    for at in database::keys::recovery_epoch_names(name, n, twin) {
        kept.extend(m.stored(&at).await?);
    }
    Ok(::key::opened_recovery_epoch(secret, hash, &kept))
}

/// Replaces the recovery key, from its file, the only way to take one that was seen or stolen out of use: every epoch
/// and Vault key it opens is sealed to the next too, so nothing already there is lost to it; the next is named; and each
/// scope still in use gets a new epoch, and each Vault new keys, sealed to the next and not the old, so what the old one
/// opens stops growing. Copies held under the old key stop counting, as they're kept under its name, and nothing already
/// sealed is touched. Run again (with `old` that was, or is now, the network's key) it finishes what it didn't. Without
/// `confirm`, says what it would do. → what was done.
pub async fn replace(m: &impl Me, old: &SigningKey, new: &SigningKey, confirm: bool) -> Result<Value> {
    m.read().await?;
    let (old_public, new_public) = (old.verifying_key().to_bytes(), new.verifying_key().to_bytes());
    ensure!(old_public != new_public, "The new recovery key is the same as the one it replaces.");
    let user = m.user()?;
    let (now, scopes, holders) = m.who(|db| {
        let scopes: Vec<Scope> = db.scopes()?.into_iter().filter(|s| db.epoch(s).is_ok_and(|n| n > 0)).filter(|s| in_use(db, s)).collect();
        Ok((db.recovery()?, scopes, db.recovery_holders()?.len()))
    })?;
    ensure!(now == Some(old_public) || now == Some(new_public), "That file doesn't hold {}'s recovery key.", ::custom_build::config::YOURS);
    if !confirm {
        return Ok(json!({ "said": format!("Would replace {}'s recovery key: {} scopes get new keys, and whatever {holders} holder(s) were sealed stops counting until the new key is sealed to them. Run it again with --confirm.", ::custom_build::config::YOURS, scopes.len()) }));
    }
    let not_carried = carry_to_next(m, old, new).await?;
    if now == Some(old_public) {
        m.write_as(old, &user, content::recovery(&new_public)).await?;
        m.read().await?;
    }
    ensure!(m.who(|db| db.recovery())? == Some(new_public), "The next recovery key wasn't taken. Nothing else changed: run this again.");
    let (renewed, not_renewed) = renew_under_next(m, new, &user).await?;
    let said = format!(
        "{} has a new recovery key: {} scopes have new keys, and old holders' copies no longer count. Seal the new key to its holders with `{} devices holders seal` when you're ready.{}{}",
        ::custom_build::config::YOURS,
        renewed.len(),
        ::custom_build::config::CLI,
        if not_carried.is_empty() { String::new() } else { format!(" Couldn't carry over: {}.", not_carried.join(", ")) },
        if not_renewed.is_empty() { String::new() } else { format!(" Couldn't renew {}: run this again.", not_renewed.iter().map(|n| n["scope"].as_str().unwrap_or("")).collect::<Vec<_>>().join(", ")) },
    );
    Ok(json!({ "said": said, "renewed": renewed, "not_renewed": not_renewed, "not_carried": not_carried }))
}

/// Seals every epoch and Vault key `old` opens to `new`, under `new`'s own name, beside what's there: → what it couldn't.
async fn carry_to_next(m: &impl Me, old: &SigningKey, new: &SigningKey) -> Result<Vec<String>> {
    let (old_secret, old_twin) = (::key::seal_secret(old), ::key::twin_public(&old.verifying_key().to_bytes())?);
    let new_twin = ::key::twin_public(&new.verifying_key().to_bytes())?;
    let to = ::key::recovery_to(&new_twin);
    let (hashes, vaults) = m.who(|db| {
        let scopes = db.scopes()?;
        let hashes: Vec<(Scope, BTreeMap<u32, String>)> = scopes.iter().map(|s| Ok::<_, anyhow::Error>((s.clone(), db.epoch_hashes(s)?))).collect::<Result<_>>()?;
        let vaults: Vec<(Scope, BTreeMap<u32, Key>)> = scopes.iter().map(|s| Ok::<_, anyhow::Error>((s.clone(), db.vault_publics(s)?))).collect::<Result<_>>()?;
        Ok((hashes, vaults))
    })?;
    let mut not = Vec::new();
    for (scope, epochs) in hashes {
        let name = scope.to_string();
        for (n, hash) in epochs {
            // Under the new key already: made since a handover that stopped.
            if opened_for(m, &name, n, &hash, &::key::seal_secret(new), &new_twin).await?.is_some() {
                continue;
            }
            match opened_for(m, &name, n, &hash, &old_secret, &old_twin).await? {
                Some(key) => drop(m.create(&database::keys::epoch_name(&name, n, &to), &::key::seal_to(&new_twin, &key)?).await?),
                None => not.push(format!("{scope}'s epoch {n}")),
            }
        }
    }
    let opens = |sealed: &[u8], secret: &[u8; 32], public: &Key| ::key::open_sealed(secret, sealed).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| ::key::public_of(k) == *public);
    for (scope, publics) in vaults {
        let name = scope.to_string();
        for (n, public) in publics {
            if m.stored(&database::keys::vault_key_name(&name, n, "recovery")).await?.is_some_and(|o| opens(&o, &::key::seal_secret(new), &public).is_some()) {
                continue;
            }
            let mut secret = None;
            for at in [database::keys::vault_key_name(&name, n, &::key::recovery_to(&old_twin)), database::keys::vault_key_name(&name, n, "recovery")] {
                if let Some(k) = m.stored(&at).await?.and_then(|o| opens(&o, &old_secret, &public)) {
                    secret = Some(k);
                    break;
                }
            }
            match secret {
                Some(k) => drop(m.create(&database::keys::vault_key_name(&name, n, &to), &::key::seal_to(&new_twin, &k)?).await?),
                None => not.push(format!("{scope}'s Vault key {n}")),
            }
        }
    }
    Ok(not)
}

/// A new epoch of each scope still in use, and new Vault keys, signed by the network's recovery key `new`, sealed to it
/// and to whoever may open them: each one already sealed to it is left as it is. → what was done, and what wasn't.
async fn renew_under_next(m: &impl Me, new: &SigningKey, user: &str) -> Result<(Vec<String>, Vec<Value>)> {
    let (secret, twin) = (::key::seal_secret(new), ::key::twin_public(&new.verifying_key().to_bytes())?);
    let (scopes, vaults) = m.who(|db| Ok((db.scopes()?.into_iter().filter(|s| in_use(db, s)).collect::<Vec<_>>(), db.vaults()?)))?;
    let (mut renewed, mut not) = (Vec::new(), Vec::new());
    for scope in scopes {
        let name = scope.to_string();
        let (newest, hash) = m.who(|db| {
            let n = db.epoch(&scope)?;
            Ok((n, db.epoch_hash(&scope, n)?))
        })?;
        if newest == 0 {
            continue;
        }
        // Sealed to the new key under `recovery`, as only an epoch made since the handover is: a run that finishes one
        // that stopped. A carried-over copy, under its own name, doesn't count.
        let held = m.stored(&database::keys::epoch_name(&name, newest, "recovery")).await?;
        if hash.is_some_and(|h| ::key::opened_recovery_epoch(&secret, &h, &held.into_iter().collect::<Vec<_>>()).is_some()) {
            continue;
        }
        match epoch_by_recovery(m, new, user, &scope, newest + 1, &twin).await {
            Ok(()) => renewed.push(name),
            Err(err) => not.push(json!({ "scope": name, "why": format!("{err:#}") })),
        }
    }
    for (scope, vault) in vaults.into_iter().filter(|(s, _)| m.who(|db| Ok(in_use(db, s))).unwrap_or(false)) {
        let name = scope.to_string();
        let held = m.stored(&database::keys::vault_key_name(&name, vault.n, "recovery")).await?;
        if held.and_then(|o| ::key::open_sealed(&secret, &o).ok()).and_then(|k| <[u8; 32]>::try_from(k).ok()).is_some_and(|k| ::key::public_of(&k) == vault.public) {
            continue;
        }
        if let Err(err) = vault_by_recovery(m, new, user, &scope, vault.n + 1, &vault.to, &twin).await {
            not.push(json!({ "scope": format!("{name}'s Vault"), "why": format!("{err:#}") }));
        }
    }
    Ok((renewed, not))
}

/// Starts epoch `n` of `scope`, signed by the network's recovery key: its step first, as a new epoch always is, then
/// sealed to the recovery key's twin and to each client of whoever may open it.
async fn epoch_by_recovery(m: &impl Me, recovery: &SigningKey, user: &str, scope: &Scope, n: u32, twin: &[u8; 32]) -> Result<()> {
    let to: Vec<Endpoint> = m.who(|db| Ok(db.openers(scope)?.into_iter().filter(|c| c.seal != c.key).map(|c| Endpoint { key: c.key, seal: c.seal }).collect()))?;
    let key: [u8; 32] = rand::random();
    let hash = ::key::epoch_hash(&key);
    m.write_as(recovery, user, content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
    m.read().await?;
    ensure!(m.who(|db| db.epoch_hash(scope, n))? == Some(hash), "Another device made new keys for {scope} just now. Run this again.");
    let name = scope.to_string();
    m.create(&database::keys::epoch_name(&name, n, "recovery"), &::key::seal_to(twin, &key)?).await?;
    for e in &to {
        m.create(&database::keys::epoch_name(&name, n, &hex(&e.key)), &::key::seal_to(&e.seal, &key)?).await?;
    }
    m.made_epoch(scope, n, key);
    Ok(())
}

/// New keys for a Vault, signed by the network's recovery key, sealed to it and to the machines that open it now.
async fn vault_by_recovery(m: &impl Me, recovery: &SigningKey, user: &str, scope: &Scope, n: u32, to: &[Key], twin: &[u8; 32]) -> Result<()> {
    let secret: [u8; 32] = rand::random();
    let public = ::key::public_of(&secret);
    m.write_as(recovery, user, content::vault(scope, n, &public, to)).await?;
    m.read().await?;
    let seals = m.who(|db| {
        ensure!(db.vault_publics(scope)?.get(&n) == Some(&public), "Another device made new keys for this Vault just now. Run this again.");
        let mut seals = Vec::new();
        for k in to {
            seals.extend(db.seal_key(k)?.map(|s| (hex(k), s)));
        }
        Ok(seals)
    })?;
    let name = scope.to_string();
    m.create(&database::keys::vault_key_name(&name, n, "recovery"), &::key::seal_to(twin, &secret)?).await?;
    for (at, seal) in seals {
        m.create(&database::keys::vault_key_name(&name, n, &at), &::key::seal_to(&seal, &secret)?).await?;
    }
    Ok(())
}

/// Whether a scope still has someone to use it: not a removed user's own, nor a group that's gone.
fn in_use(db: &ScopeDb, scope: &Scope) -> bool {
    match scope {
        Scope::Global => true,
        Scope::User(u) => db.user(u).is_ok_and(|u| u.is_some()),
        Scope::Group(g) => db.group(g).is_ok_and(|g| g.is_some()),
    }
}

/// Keeps a copy of the recovery key, sealed and signed by whoever has its file on this computer, for `to`: only a device
/// who's who says may hold it, of someone an admin named (`ScopeDb::recovery_devices`), and only by an admin's own
/// computer. This device never sees the key: what's sealed goes by. One there already stays, unless the recovery key
/// never signed it: then it's anyone's who can reach the bucket, and it goes for this one.
pub async fn seal(m: &impl Me, to: &str, copy: &str) -> Result<()> {
    let copy = B64U.decode(copy.trim().as_bytes()).map_err(|_| anyhow!("copy is base64url"))?;
    ensure!(!copy.is_empty() && copy.len() <= COPY_MAX, "a sealed key is at most {} bytes", COPY_MAX - 64);
    let key = ::key::unhex(to)?;
    let me = m.user()?;
    let recovery = m.who(|db| {
        allowed!(db.may(&me, "change-settings", scopes::NETWORK), "only an admin seals the recovery key to someone who holds it");
        let may_hold = db.recovery_holders()?.into_iter().try_fold(false, |found, user| Ok::<_, anyhow::Error>(found || db.recovery_devices(&user)?.iter().any(|d| d.key == key)))?;
        ensure!(may_hold, "the recovery key is sealed only to a device of someone an admin named, and not to a browser, a node or a passkey");
        db.recovery()?.context("this network has no recovery key to seal")
    })?;
    ensure!(::key::held_sealed(&recovery, to, &copy).is_some(), "that copy isn't signed by the recovery key");
    let name = database::keys::recovery_held_name(&hex(&recovery), to);
    if let Some(there) = m.stored(&name).await? {
        if ::key::held_sealed(&recovery, to, &there).is_some() {
            return Err(Coded::error(code::ALREADY, "that device has the recovery key already"));
        }
        m.delete(&name).await?;
    }
    if !m.create(&name, &copy).await? {
        return Err(Coded::error(code::ALREADY, "that device has the recovery key already"));
    }
    Ok(())
}
