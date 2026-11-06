//! What a step says, as a device writes it: the content of each kind of the node's steps (`rules` says who may
//! write each, and what it does). The web client builds the same content (cloudflare/web/members.js), and
//! tests/contracts.rs checks it does.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::keys::{Key, hex};

use crate::database::scopes::Scope;

use super::scopes::{APP, APP_SCHEDULES, Kind, Role};

/// What a step says: its app, its kind, what it says, and the blobs it names.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Content {
    pub app: String,
    pub kind: String,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blobs: Vec<String>,
}

impl Content {
    pub fn new(app: &str, kind: &str, data: Value) -> Self {
        Content { app: app.into(), kind: kind.into(), data, blobs: Vec::new() }
    }
}

pub fn node(kind: &str, data: Value) -> Content {
    Content::new(APP, kind, data)
}

/// What a network starts with, as its first freeze's copy holds it (`steps::first_freeze`): its name, its first person
/// and their device, the recovery key, and the hash of `global`'s first epoch (`keys::epoch_hash`), made with it.
pub fn first(name: &str, user: &str, user_name: &str, device: &crate::keys::Endpoint, device_name: &str, roles: &[Kind], recovery: &Key, epoch: &str) -> Value {
    let device = json!({ "key": hex(&device.key), "seal": hex(&device.seal), "name": device_name, "roles": roles });
    json!({ "name": name, "user": user, "user_name": user_name, "device": device, "recovery": hex(recovery), "epoch": epoch })
}

/// Lists an endpoint as `user`'s, and with `new_user` lets them in, by an admin's device.
pub fn device(endpoint: &crate::keys::Endpoint, user: &str, name: &str, roles: &[Kind], new_user: Option<(&str, Role)>) -> Content {
    let mut data = json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "user": user, "name": name, "roles": roles });
    if let Some((user_name, role)) = new_user {
        data["user_name"] = json!(user_name);
        data["role"] = json!(role);
    }
    node("device", data)
}

/// Lists a passkey as `user`'s device, by the credential it is: what's sealed to its keys (from its PRF) lets them back
/// in on a new device.
pub fn passkey(endpoint: &crate::keys::Endpoint, user: &str, name: &str, credential: &str) -> Content {
    let mut content = device(endpoint, user, name, &[Kind::Client], None);
    content.data["passkey"] = json!(credential);
    content
}


pub fn role(user: &str, role: Role) -> Content {
    node("role", json!({ "user": user, "role": role }))
}

pub fn remove_user(user: &str) -> Content {
    node("remove", json!({ "user": user }))
}

pub fn remove_device(key: &Key) -> Content {
    node("remove", json!({ "key": hex(key) }))
}

/// One of the network's settings; `None` puts back the custom build's default.
pub fn setting(name: &str, value: Option<&str>) -> Content {
    node("setting", json!({ "name": name, "value": value }))
}

/// A new group, its writer its first admin.
pub fn group(id: &str, name: &str) -> Content {
    node("group", json!({ "group": id, "name": name }))
}

pub fn invite(group: &str, user: &str, role: Role) -> Content {
    node("invite", json!({ "group": group, "user": user, "role": role }))
}

pub fn accept(group: &str) -> Content {
    node("accept", json!({ "group": group }))
}

/// A group member's role, or `None` to take them out.
pub fn member(group: &str, user: &str, role: Option<Role>) -> Content {
    node("member", json!({ "group": group, "user": user, "role": role }))
}

/// Turns down a device that asked to join as this person's, before it's listed ("Not Me"): every device of theirs
/// then refuses it.
pub fn refuse(endpoint: &crate::keys::Endpoint, name: &str) -> Content {
    node("refuse", json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "name": name }))
}

/// Forgets what one of this person's devices signed in with (`Machine::sign_in`).
pub fn forget_sign_in(key: &Key) -> Content {
    node("forget-sign-in", json!({ "key": hex(key) }))
}

/// Renames a device (`key`), a person (`user`) or a group (`group`): `what` is which, `id` its key (hex) or id.
pub fn rename(what: &str, id: &str, name: &str) -> Content {
    let mut data = json!({ "name": name });
    data[what] = json!(id);
    node("rename", data)
}

/// Changes a person's (`what` user) or a group's (`group`) handle.
pub fn handle(what: &str, id: &str, handle: &str) -> Content {
    let mut data = json!({ "handle": handle });
    data[what] = json!(id);
    node("rename", data)
}

/// A scope's new epoch, sealed to `to`, its key's `keys::epoch_hash` as `hash`.
pub fn epoch(scope: &Scope, n: u32, to: &[Key], hash: &str) -> Content {
    node("epoch", json!({ "scope": scope.to_string(), "n": n, "to": to.iter().map(hex).collect::<Vec<_>>(), "hash": hash }))
}

/// A scope's Vault key `n`, its public half `public`, sealed to the machines `to`.
pub fn vault(scope: &Scope, n: u32, public: &[u8; 32], to: &[Key]) -> Content {
    node("vault", json!({ "scope": scope.to_string(), "n": n, "public": hex(public), "to": to.iter().map(hex).collect::<Vec<_>>() }))
}

/// A part of a device's oned (a service, or `apps`, the app runtime) switched on or off there; a service, for the app
/// added in `scope`.
pub fn service(device: &Key, name: &str, on: bool, scope: Option<&Scope>) -> Content {
    let mut data = json!({ "device": hex(device), "name": name, "on": on });
    if let Some(scope) = scope {
        data["scope"] = json!(scope.to_string());
    }
    node("service", data)
}

/// Where its writer's own things run besides their own machines: the network's others that run their own scope.
pub fn runs(on: &[Key]) -> Content {
    node("runs", json!({ "on": on.iter().map(hex).collect::<Vec<_>>() }))
}

/// App schedules: `endpoint` runs `app`'s schedules in `scope` until `until`, or no longer with 0.
pub fn app_schedules(endpoint: &Key, scope: &Scope, app: &str, until: u64, keys: &BTreeMap<u32, String>) -> Content {
    let keys: serde_json::Map<String, Value> = keys.iter().map(|(n, h)| (n.to_string(), json!(h))).collect();
    node(APP_SCHEDULES, json!({ "endpoint": hex(endpoint), "scope": scope.to_string(), "app": app, "until": until, "keys": keys }))
}

/// Deletes an app (`deleted`), restores it (`restored`) or erases it (`erased`), as its owner (`owner`) says.
pub fn retire(app: &str, owner: &str, how: &str) -> Result<Content> {
    let mut data = json!({ "name": app, "owner": owner });
    match how {
        "deleted" => data["deleted"] = json!(true),
        "restored" => data["deleted"] = json!(false),
        "erased" => data["erased"] = json!(true),
        other => bail!("an app is deleted, restored or erased, not {other}"),
    }
    Ok(node("app", data))
}

/// Puts an app on a scope's page, or takes it off.
pub fn install(app: &str, installed: bool) -> Content {
    node(if installed { "install" } else { "uninstall" }, json!({ "app": app }))
}

/// Lists a service endpoint offering `service` as `user`'s, a part of `device`, by that device: `shared` if it serves
/// everyone it may, not only them.
pub fn service_endpoint(endpoint: &crate::keys::Endpoint, user: &str, name: &str, service: &str, shared: bool, device: &Key) -> Content {
    let mut data = json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "user": user, "name": name, "roles": [Kind::Service], "service": service, "device": hex(device) });
    if shared {
        data["shared"] = json!(true);
    }
    node("device", data)
}
