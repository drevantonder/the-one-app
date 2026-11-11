//! Pairing, as the app runtime does it: a code that lets another computer join the network, and admitting the computer
//! that comes with one (its shapes are `onboarding`'s).

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use ::custom_build::allowed;
use ::custom_build::config::YOURS;
use ::custom_build::now_ms;
use ::custom_build::protocol::{Coded, PAIRING_LIFE, code};
use ::onboarding::{JoinReply, JoinRequest, Pairing, PairingCode};
use api::Caller;
use api::runtime::prelude::*;
use api::runtime::Runs;
use key::{Key, hex, unhex};
use scope::content;
use scope::scopes::{self, ScopeDb};

/// A computer, as it's listed when it joins or makes a network: its user's client, and a machine that runs calls.
pub const MACHINE: &[scopes::Kind] = &[scopes::Kind::Client, scopes::Kind::Compute];

pub fn runs() -> Runs {
    Runs { state: Box::new(State::default()), ..Runs::named("onboarding") }
}

/// The pairing codes this runtime made that haven't been used, read from its home the first time they're asked for.
#[derive(Default)]
pub struct State {
    pairings: Mutex<Option<HashMap<[u8; 16], Pairing>>>,
}

/// The pairing codes, as kept in a file, so one still works after the device that made it restarts.
fn pairings<R>(rt: &Rt, f: impl FnOnce(&mut HashMap<[u8; 16], Pairing>) -> R) -> R {
    let mut kept = rt.state::<State>().pairings.lock().unwrap();
    f(kept.get_or_insert_with(|| ::onboarding::pairings(rt.home())))
}

fn save(rt: &Rt, pairings: &HashMap<[u8; 16], Pairing>) -> Result<()> {
    let list: Vec<(String, &Pairing)> = pairings.iter().map(|(k, v)| (data_encoding::HEXLOWER.encode(k), v)).collect();
    api::home::write_private(&rt.home().join("pairings.json"), &serde_json::to_vec(&list)?)
}

fn used() -> anyhow::Error {
    Coded::error(code::PAIRING_USED, "this pairing code was already used or has expired")
}

/// Whether `who` may list a device of `user`'s that does `kinds`, as `rules` counts a `device` step: only of someone in
/// the network now (one removed, or not in it yet, is added with their first device, by an invite's approval); a
/// client or a service only from their own devices (`add-devices`), a machine that runs calls also by an admin
/// (`add-machines`), which runs only what that admin's listing may, never their own scope. The rows give the verbs;
/// which kind of device takes which is its rule's.
fn may_list(db: &ScopeDb, who: &str, user: &str, kinds: &[scopes::Kind]) -> Result<bool> {
    let theirs = kinds.iter().any(|k| matches!(k, scopes::Kind::Client | scopes::Kind::Service));
    Ok(db.user(user)?.is_some() && db.may(who, if theirs { "add-devices" } else { "add-machines" }, &scopes::user_principal(user)))
}

/// Whether a pairing code this runtime made still works: until one does, a computer that isn't the network's isn't
/// answered.
pub fn waiting(rt: &Rt) -> bool {
    pairings(rt, |p| p.values().any(|p| p.until > now_ms()))
}

/// Makes a one-time pairing code for another of the local user's machines, which joins as a device.
pub fn pair(rt: &Rt) -> Result<String> {
    Ok(pair_as(rt, &rt.caller()?, None)?.0)
}

/// Like `pair`, for `asker`, or for an admin, a code for another machine of `user` (an id or a name). With the id
/// of the user the code admits.
pub fn pair_as(rt: &Rt, asker: &Caller, user: Option<&str>) -> Result<(String, String)> {
    let id = rt.id();
    let user = user.map(|u| rt.read(|r| r.db().find_user(u))).transpose()?;
    rt.read(|r| {
        let m = r.db();
        ensure!(m.compute(&id)?.is_some(), "this device isn't in {YOURS} yet");
        let kinds = if user.as_ref().is_none_or(|u| *u == asker.user) { MACHINE } else { &[scopes::Kind::Compute][..] };
        allowed!(may_list(m, &asker.user, user.as_deref().unwrap_or(&asker.user), kinds)?, "only admins pair other users' machines");
        ensure!(m.serves(&id, &asker.user)?, "{} doesn't serve you", rt.device_name());
        Ok(())
    })?;
    let address = rt.read(|r| Ok(r.db().setting("address")?))?.with_context(|| format!("a computer joins through {YOURS}'s address, and it has none yet: an admin sets it with cloudflare use"))?;
    let joins_as = user.clone().unwrap_or_else(|| asker.user.clone());
    let secret: [u8; 16] = rand::random();
    let by = (asker.key != id).then(|| hex(&asker.key));
    let pairing = Pairing { user: asker.user.clone(), by, admits: user, until: now_ms() + PAIRING_LIFE, tried: None };
    pairings(rt, |p| {
        p.retain(|_, p| p.until > now_ms());
        p.insert(secret, pairing);
        save(rt, p)
    })?;
    // Told with the key the code names, which answers the computer joining with it: the relay lets that computer
    // reach only the key that told it of the code.
    let (rt2, fields) = (rt.clone(), json!({ "pairing": { "hash": ::onboarding::join::ticket_hash(&::onboarding::join::pairing_ticket(&secret)), "until": now_ms() + PAIRING_LIFE } }));
    tokio::spawn(async move {
        if let Err(err) = rt2.tell_relay(fields, false).await {
            tracing::warn!("telling {YOURS}'s relay of a pairing code: {err:#}");
        }
    });
    Ok((PairingCode { address, from: id, secret }.text()?, joins_as))
}

/// Joins a machine with a pairing code: used up once it's in, and kept for that machine alone when its join didn't
/// finish, so trying again with the same code finishes it rather than finding it listed already.
pub async fn admit(rt: &Rt, key: Key, request: JoinRequest) -> Result<JoinReply> {
    let taken = pairings(rt, |p| {
        let pairing = p.remove(&request.secret);
        if pairing.is_some() {
            save(rt, p)?;
        }
        anyhow::Ok(pairing)
    })?;
    let mut pairing = taken.filter(|p| p.until > now_ms()).ok_or_else(used)?;
    if pairing.tried.as_ref().is_some_and(|t| *t != hex(&key)) {
        keep(rt, request.secret, pairing);
        return Err(used());
    }
    let admitted = admit_with(rt, key, &request, &pairing).await;
    if admitted.is_err() {
        // Once it's listed, the code is that machine's to finish with; before, anyone's who has it.
        if rt.read(|r| Ok(r.db().machine(&key)?.is_some()))? {
            pairing.tried = Some(hex(&key));
        }
        keep(rt, request.secret, pairing);
    }
    admitted
}

fn keep(rt: &Rt, secret: [u8; 16], pairing: Pairing) {
    pairings(rt, |p| {
        p.insert(secret, pairing);
        if let Err(err) = save(rt, p) {
            tracing::warn!("keeping a half-finished pairing code: {err:#}");
        }
    })
}

async fn admit_with(rt: &Rt, key: Key, request: &JoinRequest, pairing: &Pairing) -> Result<JoinReply> {
    ensure!(request.as_ == "device", "only computers join with a pairing code: sign in at {YOURS}'s address instead");
    let user = pairing.admits.clone().unwrap_or_else(|| pairing.user.clone());
    let root = rt.read(|r| r.replica.root().map(|r| r.to_hex()).context("no network"))?;
    let bucket = rt.bucket_fields();
    let by = pairing.by.as_deref().map(unhex).transpose()?;
    let endpoint = key::Endpoint { key, seal: key::twin_public(&key)? };
    // A machine of the user's own is their device too. An admin lists another user's as compute, which runs
    // what that admin's listing may (`global`, groups) but never that user's own scope, until a device of theirs
    // lists it again (`scope::derive`'s runners).
    let roles = if user == pairing.user { MACHINE } else { &[scopes::Kind::Compute][..] };
    // Listed already by a join with this code that didn't finish: only the relay's left.
    if !rt.read(|r| Ok(r.db().machine(&key)?.is_some_and(|m| m.user == user)))? {
        let listed = content::device(&endpoint, &user, &request.name, roles, None);
        rt.as_member(&Caller::own(by.unwrap_or(rt.id()), pairing.user.clone()), listed).await.context("this device can no longer admit that machine")?;
    }
    // Waited for: until the relay lets it in, the machine that joined reaches nothing.
    if rt.address().is_ok() {
        let admin = rt.read(|r| Ok(r.db().may(&user, "invite", scopes::NETWORK)))?;
        rt.tell_relay(json!({ "add": hex(&key), "admin": admin, "role": "compute" }), true).await.with_context(|| format!("{YOURS}'s network didn't take this machine: try again with the same code"))?;
        // Auth learns whose it is only here, as it never signs in to ask: without this it just can't answer that
        // user's requests yet, so a refusal (a pairing device auth doesn't know either) doesn't stop the join.
        if user == pairing.user
            && let Err(err) = rt.ask_auth("POST", "/api/auth/devices/theirs", json!({ "key": hex(&key), "user": user, "name": request.name })).await
        {
            tracing::warn!("telling auth whose computer this is: {err:#}");
        }
    }
    Ok(JoinReply { root, user, bucket })
}
