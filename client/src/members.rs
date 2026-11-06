//! Who's who as the network's own signed steps say it, checked on the device with the nodes' own Rust
//! (`one::database::steps`), never as a node says it: a node holds no keys, and a hacked one could list a seal key of
//! its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device seals,
//! and to whom it hands keys, it takes only from here.
//!
//! A device reads who's who as every reader does (`one::database::steps::Reader`): from the network's first freeze,
//! signed by the recovery key its copy names, of the network it joined, or from where the device that let it in had
//! read to (its Welcome), applying each step after itself, opened with its own keys. What it asks of who's who it asks
//! `global`'s scope database and `may` over it (`one::database::scopes::ScopeDb`, as nodes ask theirs): native apps
//! directly, browsers as JSON (`view_of`).

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{Map, Value, json};

pub use one::database::scopes::ScopeDb;
use one::database::steps;
use one::database::scopes::{NETWORK, device, object, person};
use one::keys::hex;
use one::database::scopes::Scope;
use one::database::scopes::{Kind, Role};

/// `global`'s step key in each of `epochs` given to this device (raw, by number), from `apps`, the node's key sealed
/// under each: unchecked, so a reader uses each only once who's who says its epoch is the one it made
/// (`steps::Reader::add`).
pub fn step_keys(epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> one::database::stored::Keys {
    let mut keys = one::database::stored::Keys::new();
    for (e, epoch) in epochs {
        if let Some(key) = apps.get(e).and_then(|a| one::keys::open_database_key(epoch, one::database::scopes::APP, steps::NAME, a).ok()) {
            keys.insert(*e, key);
        }
    }
    keys
}

/// What a browser seals, hands and signs by, as JSON for it to keep and look up (`cloudflare/web/device.js`,
/// `net-worker.js`), each answer worked out here:
///
/// - `root`, `name`, `recovery` (hex, what each epoch is sealed to besides devices)
/// - `devices`: each listed now, `{ key, seal, user, name, roles, passkey, device }`, and the Cloudflare node as
///   `{ key, seal, roles: ["compute"] }`
/// - `removed`: the keys taken out, or turned down before they were listed, which are never sealed anything again
/// - `sign_in_clients`: the OIDC clients the network's people sign in with, its setting (`sign_in_proof::CLIENTS`)
/// - `openers`: each scope's `{ key, seal }` its epochs are sealed to
/// - `runs_for`: each node's scopes whose calls it runs: the only nodes handed a scope's keys
/// - `epochs`: each scope's newest epoch `n` and every epoch's hash
/// - `people`: each person's `uses` and `starts` (scopes they open, and whose new epochs they start), `nodes` (those
///   that run their calls), `runs_until_chosen`, `approves` (whose new devices they let in), `invites` (someone new),
///   `removes_people`, `removes` (the devices they may take out), and `app_schedules`: theirs on a device still
///   listed, `{ endpoint, scope, app, until, keys }` (each epoch's key hash), the only ones their browser renews
pub fn view_of(db: &ScopeDb) -> Result<Value> {
    let scopes = db.scopes();
    let listed = db.machines().into_iter().filter(|d| !d.removed).collect::<Vec<_>>();
    let mut devices: Vec<Value> = listed.iter().map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal), "user": d.user, "name": d.name, "roles": d.roles, "passkey": d.passkey, "device": d.device.as_ref().map(hex), "since": d.since, "place": d.place })).collect();
    let nodes = db.nodes();
    if let Some(k) = db.cloudflare_node()
        && let Some((_, seal)) = nodes.iter().find(|(n, _)| *n == k)
    {
        devices.push(json!({ "key": hex(&k), "seal": hex(seal), "roles": ["compute"] }));
    }
    let people: Vec<String> = db.users().into_iter().filter(|u| !u.removed).map(|u| u.id).collect();
    let uses = |u: &str| scopes.iter().filter(|s| db.may(u, "use", &object(s))).collect::<Vec<_>>();
    let uses: BTreeMap<&str, Vec<&Scope>> = people.iter().map(|u| (u.as_str(), uses(u))).collect();
    let mut openers = Map::new();
    for s in scopes.iter().filter(|s| uses.values().any(|on| on.contains(s))) {
        let to: Vec<Value> = listed.iter().filter(|d| d.is(Kind::Client) && uses.get(d.user.as_str()).is_some_and(|on| on.contains(&s))).map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal) })).collect();
        openers.insert(s.to_string(), json!(to));
    }
    let runs_for: Map<String, Value> = nodes.iter().map(|(k, _)| (hex(k), json!(scopes.iter().filter(|s| db.runs_for(k, s)).map(Scope::to_string).collect::<Vec<_>>()))).collect();
    let mut epochs = Map::new();
    for (s, n) in db.epochs() {
        epochs.insert(s.to_string(), json!({ "n": n, "hashes": db.epoch_hashes(&s).into_iter().map(|(n, h)| (n.to_string(), json!(h))).collect::<Map<_, _>>() }));
    }
    let schedules = db.app_schedules();
    let mut answers = Map::new();
    for u in &people {
        let own: Vec<Value> = schedules
            .iter()
            .filter(|((k, s, _), g)| g.by == *u && db.machine(k).is_some() && db.may(u, "choose-devices", &object(s)))
            .map(|((k, s, a), g)| json!({ "endpoint": hex(k), "scope": s.to_string(), "app": a, "until": g.until, "keys": g.hashes }))
            .collect();
        answers.insert(
            u.clone(),
            json!({
                "uses": uses[u.as_str()].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "starts": scopes.iter().filter(|s| db.role_in(u, s) == Some(Role::Admin)).map(Scope::to_string).collect::<Vec<_>>(),
                "nodes": nodes.iter().filter(|(k, _)| db.serves(k, u)).map(|(k, _)| hex(k)).collect::<Vec<_>>(),
                "runs_until_chosen": db.runs_until_chosen(u).iter().map(hex).collect::<Vec<_>>(),
                "approves": people.iter().filter(|of| db.may(u, "add-devices", &person(of))).collect::<Vec<_>>(),
                "invites": db.may(u, "invite", NETWORK),
                "removes_people": db.may(u, "remove", NETWORK),
                "removes": listed.iter().filter(|d| db.may(u, "remove", &device(&d.key))).map(|d| hex(&d.key)).collect::<Vec<_>>(),
                "app_schedules": own,
                "sign_ins": db.sign_ins(u),
            }),
        );
    }
    Ok(json!({
        "root": db.root().map(|r| r.to_hex()), "name": db.name(), "recovery": db.recovery().as_ref().map(hex),
        "devices": devices, "openers": openers,
        "removed": db.machines().iter().filter(|d| d.removed).map(|d| hex(&d.key)).collect::<Vec<_>>(),
        "sign_in_clients": db.setting(one::sign_in_proof::CLIENTS), "runs_for": runs_for, "epochs": epochs, "people": answers,
    }))
}
