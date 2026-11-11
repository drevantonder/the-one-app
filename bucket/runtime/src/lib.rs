//! Where the network keeps its data: its bucket. It starts as a folder on the device that made the network, which no
//! other machine reaches; an admin moves it to S3-compatible storage, whose credentials only the URL signer
//! holds, with the `storage` setting (an entry only an admin's device writes) that says only where it is, and each
//! device follows: it reaches it through the URL signer, copies there what it reaches of the old one, and keeps its
//! data there from then on. A folder is only ever a machine's own choice, never taken from what others write.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use ::bucket::{Place, STORAGE};
use ::custom_build::allowed;
use api::Caller;
use api::call::Event;
use api::home::{NETWORK_OBJECT, bucket_root};
use api::runtime::prelude::*;
use api::runtime::{Happened, Runs};
use scope::scopes::{self, Scope, ScopeDb};

mod routes;

pub fn runs() -> Runs {
    Runs { routes: routes::routes(), state: Box::new(State::default()), on: Some(on), ..Runs::named("storage") }
}

#[derive(Default)]
pub struct State {
    /// Whether this device is moving to the network's new storage now.
    moving: AtomicBool,
}

/// Who's who changed: the `storage` setting may have.
fn on(rt: &Rt, happened: &Happened) {
    let Happened::Steps { scope: Scope::Global } = happened else { return };
    tokio::spawn(follow(rt.clone()));
}

/// The network's bucket as its `storage` setting says, if it has one: S3's address alone, never a folder's path.
fn storage_setting(m: &ScopeDb) -> Result<Option<BTreeMap<String, String>>> {
    let Some(value) = m.setting(STORAGE)? else { return Ok(None) };
    let fields: BTreeMap<String, String> = serde_json::from_str(&value).context("the storage setting isn't an S3 address")?;
    ensure!(fields.keys().all(|k| ["endpoint", "bucket", "region"].contains(&k.as_str())), "the storage setting names only S3's endpoint, bucket and region");
    ensure!(fields.contains_key("endpoint") && fields.contains_key("bucket"), "the storage setting names S3's endpoint and bucket");
    Ok(Some(fields))
}

/// Where this device keeps the network's data, as a user sees it.
pub fn storage(rt: &Rt) -> Value {
    match rt.bucket() {
        Ok(Place::Folder(path)) => json!({ "kind": "folder", "where": path.display().to_string(), "device": rt.device_name() }),
        Ok(Place::S3(s3)) => json!({ "kind": "s3", "where": s3.bucket, "endpoint": s3.endpoint }),
        Err(_) => json!({ "kind": null }),
    }
}

/// Moves the network's data, for an admin, to S3-compatible storage (`endpoint`, `bucket`, `region`), whose key the
/// URL signer holds: reaches it through the signer first, and makes it the network's `storage` setting if
/// it can.
pub async fn set(rt: &Rt, caller: &Caller, body: &Value) -> Result<Value> {
    rt.read(|r| {
        allowed!(r.db().may(&caller.user, "change-settings", scopes::NETWORK), "only admins move the network's data");
        Ok(())
    })?;
    let fields: BTreeMap<String, String> = ["endpoint", "bucket", "region"]
        .into_iter()
        .filter_map(|k| Some((k.to_string(), body[k].as_str().map(str::trim).filter(|v| !v.is_empty())?.to_string())))
        .collect();
    let place = rt.place(&fields)?;
    let Place::S3(_) = &place else { bail!("the network's data moves to S3-compatible storage, given its endpoint") };
    place.can_hold_a_network()?;
    let reached = {
        let place = place.clone();
        tokio::task::spawn_blocking(move || bucket_root(&place)).await?
    };
    let steps = vec![json!({ "step": "Reached it through the URL signer", "error": reached.as_ref().err().map(|e| format!("{e:#}")) })];
    if reached.is_err() {
        return Ok(json!({ "tried": steps }));
    }
    let setting = serde_json::to_string(&fields)?;
    rt.as_member(caller, scope::content::setting(STORAGE, Some(&setting))).await?;
    Ok(json!({ "tried": steps, "kept": true }))
}

/// Follows the network's `storage` setting, when this device keeps its data anywhere else.
async fn follow(rt: Rt) {
    let now = rt.bucket_fields();
    let wanted = rt.read(|r| Ok(storage_setting(r.db())?.filter(|f| now.as_ref().is_none_or(|now| now.iter().filter(|(k, _)| k.as_str() != "address").ne(f.iter())))));
    let fields = match wanted {
        Ok(Some(fields)) => fields,
        Ok(None) => return,
        Err(err) => return tracing::warn!("storage: {err:#}"),
    };
    let moving = &rt.state::<State>().moving;
    if moving.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(err) = move_to(&rt, fields).await {
        tracing::warn!("moving the network's data: {err:#}");
    }
    moving.store(false, Ordering::SeqCst);
}

/// Moves this device to the bucket `fields` says, once it reaches it: copies there each object of the bucket it had
/// that the new one lacks, then keeps the network's data there.
async fn move_to(rt: &Rt, fields: BTreeMap<String, String>) -> Result<()> {
    let to = rt.place(&fields)?;
    let from = rt.bucket().ok();
    let root = rt.read(|r| r.replica.root().map(|r| r.to_hex()).context("no network"))?;
    let (place, theirs) = (to.clone(), root.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        place.can_hold_a_network()?;
        if let Some(other) = bucket_root(&place)?.filter(|r| r != theirs.as_bytes()) {
            bail!("that bucket holds another network's data ({})", String::from_utf8_lossy(&other));
        }
        Ok(())
    })
    .await??;
    let place = to.clone();
    let copy = Box::pin(async move {
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(from) = from.filter(|f| f.present()) {
                for name in to_move(&from.names()?) {
                    if place.get(&name)?.is_none() {
                        if let Some(bytes) = from.get(&name)? {
                            place.create(&name, &bytes)?;
                        }
                    }
                }
            }
            place.create(NETWORK_OBJECT, root.as_bytes())?;
            Ok(())
        })
        .await?
    });
    rt.keep_in(to, fields, copy).await?;
    tracing::info!("the network's data is kept in {} now", storage(rt)["where"].as_str().unwrap_or(""));
    rt.emit(Event::Changed { apps: Vec::new() });
    Ok(())
}

/// Of `names`, every object of the bucket being moved from, sorted, those moving copies, in that order: each database's
/// and log's steps only up to the first position with nothing there, with the signature beside each and the copies
/// after them; everything else as it is. The URL signer creates a step only once the one before it is there (or the
/// copy up to it), and a copy only once its step is, and no reader reads past a position with nothing: what an older
/// writer left past one stays behind, and the next step is written there. A database whose older steps were deleted
/// (`stored::tidied_name`) moves whole, that first: its readers go on past a gap from a copy that holds, and the URL signer
/// creates any step of it.
fn to_move(names: &[String]) -> Vec<String> {
    // A database's prefix and position, of its copy (true), or of its step or the signature beside one: a copy's name is
    // a step's of a prefix ending in `copy`, never taken for one.
    fn of(name: &str) -> Option<(&str, u64, bool)> {
        if let Some((folder, last)) = name.rsplit_once('/')
            && let Some(prefix) = folder.strip_suffix("/copy")
            && let Ok(n) = last.parse()
            && bucket::names::copy_name(prefix, n) == name
        {
            return Some((prefix, n, true));
        }
        bucket::names::position(name).map(|(prefix, n)| (prefix, n, false))
    }
    let steps: BTreeSet<(&str, u64)> = names.iter().filter_map(|n| of(n).filter(|(p, at, copy)| !copy && bucket::names::step_name(p, *at) == *n)).map(|(p, at, _)| (p, at)).collect();
    let tidied: BTreeSet<&str> = names.iter().filter_map(|n| n.strip_suffix("/tidied")).collect();
    let mut ends: BTreeMap<&str, u64> = BTreeMap::new();
    for (prefix, _) in steps.iter().filter(|(p, _)| !tidied.contains(p)) {
        let end = ends.entry(prefix).or_insert(0);
        while steps.contains(&(prefix, *end + 1)) {
            *end += 1;
        }
    }
    let moving = names.iter().filter(|name| of(name).is_none_or(|(prefix, at, _)| tidied.contains(prefix) || ends.get(prefix).is_some_and(|end| at <= *end)));
    let (first, rest): (Vec<&String>, Vec<&String>) = moving.partition(|n| n.ends_with("/tidied"));
    first.into_iter().chain(rest).cloned().collect()
}

#[cfg(test)]
mod tests {
    /// Moving copies what every reader reads, in an order the URL signer takes: no step or copy past a position with
    /// nothing, which an older writer left, and which no reader reads past; and all of a database whose older steps were
    /// deleted, which its readers read on past from a copy.
    #[test]
    fn moving_copies_each_database_s_steps_up_to_a_gap() {
        let (log, db, old) = ("global/one/log", "user:a/notes/notes", "user:a/notes/old");
        let mut names: Vec<String> = [
            bucket::names::step_name(log, 1),
            bucket::names::step_name(log, 2),
            bucket::names::step_name(log, 4),
            bucket::names::copy_name(log, 2),
            bucket::names::copy_name(log, 4),
            bucket::names::newest_name(log),
            bucket::names::step_name(db, 1),
            bucket::names::sig_name(db, 1),
            bucket::names::sig_name(db, 3),
            bucket::names::step_name(db, 3),
            bucket::names::step_name(db, 1_000_000),
            bucket::names::tidied_name(old),
            bucket::names::step_name(old, 200),
            bucket::names::sig_name(old, 200),
            bucket::names::step_name(old, 201),
            bucket::names::copy_name(old, 200),
            bucket::names::newest_name(old),
            "user:a/blobs/abc".to_string(),
            "network".to_string(),
        ]
        .into();
        names.sort();
        let moved = super::to_move(&names);
        let want = [bucket::names::step_name(log, 1), bucket::names::step_name(log, 2), bucket::names::copy_name(log, 2), bucket::names::newest_name(log), bucket::names::step_name(db, 1), bucket::names::sig_name(db, 1), "user:a/blobs/abc".to_string(), "network".to_string()];
        let whole = names.iter().filter(|n| n.starts_with(&format!("{old}/")));
        assert_eq!(moved.iter().collect::<std::collections::BTreeSet<_>>(), want.iter().chain(whole).collect(), "and a database whose older steps were deleted, whole");
        // As the URL signer creates them (`url-signer/worker.js`'s `after`), one after another in that order.
        let mut there = std::collections::BTreeSet::new();
        for name in &moved {
            let due = match (name.rsplit_once("/copy/"), bucket::names::position(name).filter(|(p, at)| bucket::names::step_name(p, *at) == *name)) {
                (Some((prefix, n)), _) => there.contains(&bucket::names::step_name(prefix, n.parse().unwrap())),
                (None, Some((prefix, at))) => at == 1 || [bucket::names::step_name(prefix, at - 1), bucket::names::copy_name(prefix, at - 1), bucket::names::tidied_name(prefix)].iter().any(|n| there.contains(n)),
                (None, None) => true,
            };
            assert!(due, "the URL signer refuses {name} then");
            there.insert(name.clone());
        }
    }
}
