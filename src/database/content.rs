//! What a step says, as a device writes it: each kind of the core's own steps (`Change`), and what each says (`rules`
//! says who may write each, and what it does).

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::keys::{Key, hex};

use crate::database::scopes::Scope;

use super::scopes::{APP, Kind, Role};

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

    /// Which of the core's kinds it is: none for an app's own step, or one of a kind this release doesn't know.
    pub fn change(&self) -> Option<Change> {
        if self.app != APP {
            return None;
        }
        NAMES.iter().find(|(_, name)| *name == self.kind).map(|(c, _)| *c)
    }

    pub fn is(&self, change: Change) -> bool {
        self.change() == Some(change)
    }
}

/// Each kind of the core's own steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Freezes the log (`steps::Truth::first`): `{ hash, logs }`, with its copy. The first the network's recovery key
    /// signed is who's who from there; a network made since starts with it (`steps::first_freeze`).
    Freeze,
    /// Starts a scope's database (not `global`'s) whose frozen log the freeze names, at position 0: `{ hash }`, with its
    /// copy, what that log's entries added up to at the freeze. Written once, by a device that holds the scope's keys
    /// (it's sealed under them, as its steps are).
    Start,
    /// Moves a database on to a newer release's types, roles, rules and tables (`permissions::DEFAULTS`,
    /// `scope::SCHEMA`): `{ reads, types, roles, rules }`, `reads` that release's `READS`, which the database keeps as
    /// its version. An admin's device writes one once every device reads it, and only above the database's version: a
    /// release never puts back what a newer one said, and two on different releases never write turn about. Readers
    /// never run their own release's: each database keeps the rows its freeze and steps said. One that reads less stops
    /// at it (`steps::Newer`): update it.
    Defaults,
    Device,
    Refuse,
    ForgetSignIn,
    Role,
    Remove,
    Epoch,
    Runs,
    AppSchedules,
    Vault,
    Group,
    Invite,
    Accept,
    Member,
    Rename,
    Setting,
    Recovery,
    App,
    Deploy,
    Service,
    NodeRelease,
    ServiceRelease,
    AddApp,
    RemoveApp,
    Schedule,
    VaultRequest,
    Keep,
    Forget,
    Arrange,
}

const NAMES: [(Change, &str); 31] = [
    (Change::Freeze, "freeze"),
    (Change::Start, "start"),
    (Change::Defaults, "defaults"),
    (Change::Device, "device"),
    (Change::Refuse, "refuse"),
    (Change::ForgetSignIn, "forget-sign-in"),
    (Change::Role, "role"),
    (Change::Remove, "remove"),
    (Change::Epoch, "epoch"),
    (Change::Runs, "runs"),
    (Change::AppSchedules, "app-schedules"),
    (Change::Vault, "vault"),
    (Change::Group, "group"),
    (Change::Invite, "invite"),
    (Change::Accept, "accept"),
    (Change::Member, "member"),
    (Change::Rename, "rename"),
    (Change::Setting, "setting"),
    (Change::Recovery, "recovery"),
    (Change::App, "app"),
    (Change::Deploy, "deploy"),
    (Change::Service, "service"),
    (Change::NodeRelease, "node-release"),
    (Change::ServiceRelease, "service-release"),
    (Change::AddApp, "add-app"),
    (Change::RemoveApp, "remove-app"),
    (Change::Schedule, "schedule"),
    (Change::VaultRequest, "vault-request"),
    (Change::Keep, "keep"),
    (Change::Forget, "forget"),
    (Change::Arrange, "arrange"),
];

impl Change {
    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(c, _)| *c == self).expect("every kind is named").1
    }

    /// Whether it changes only its own scope's state, wherever it's written; every other kind written in `global` is
    /// who's who, which only its person's own devices write.
    pub fn own(self) -> bool {
        matches!(self, Change::Schedule | Change::VaultRequest | Change::Keep | Change::Forget | Change::Arrange)
    }
}

pub fn node(change: Change, data: Value) -> Content {
    Content::new(APP, change.name(), data)
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
    node(Change::Device, data)
}

/// Lists a passkey as `user`'s device, by the credential it is: what's sealed to its keys (from its PRF) lets them back
/// in on a new device.
pub fn passkey(endpoint: &crate::keys::Endpoint, user: &str, name: &str, credential: &str) -> Content {
    let mut content = device(endpoint, user, name, &[Kind::Client], None);
    content.data["passkey"] = json!(credential);
    content
}


pub fn role(user: &str, role: Role) -> Content {
    node(Change::Role, json!({ "user": user, "role": role }))
}

pub fn remove_user(user: &str) -> Content {
    node(Change::Remove, json!({ "user": user }))
}

pub fn remove_device(key: &Key) -> Content {
    node(Change::Remove, json!({ "key": hex(key) }))
}

/// One of the network's settings; `None` puts back the custom build's default.
pub fn setting(name: &str, value: Option<&str>) -> Content {
    node(Change::Setting, json!({ "name": name, "value": value }))
}

/// A new group, its writer its first admin.
pub fn group(id: &str, name: &str) -> Content {
    node(Change::Group, json!({ "group": id, "name": name }))
}

pub fn invite(group: &str, user: &str, role: Role) -> Content {
    node(Change::Invite, json!({ "group": group, "user": user, "role": role }))
}

pub fn accept(group: &str) -> Content {
    node(Change::Accept, json!({ "group": group }))
}

/// A group member's role, or `None` to take them out.
pub fn member(group: &str, user: &str, role: Option<Role>) -> Content {
    node(Change::Member, json!({ "group": group, "user": user, "role": role }))
}

/// Turns down a device that asked to join as this person's, before it's listed ("Not Me"): every device of theirs
/// then refuses it.
pub fn refuse(endpoint: &crate::keys::Endpoint, name: &str) -> Content {
    node(Change::Refuse, json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "name": name }))
}

/// Forgets what one of this person's devices signed in with (`Machine::sign_in`).
pub fn forget_sign_in(key: &Key) -> Content {
    node(Change::ForgetSignIn, json!({ "key": hex(key) }))
}

/// Renames a device (`key`), a person (`user`) or a group (`group`): `what` is which, `id` its key (hex) or id.
pub fn rename(what: &str, id: &str, name: &str) -> Content {
    let mut data = json!({ "name": name });
    data[what] = json!(id);
    node(Change::Rename, data)
}

/// Changes a person's (`what` user) or a group's (`group`) handle.
pub fn handle(what: &str, id: &str, handle: &str) -> Content {
    let mut data = json!({ "handle": handle });
    data[what] = json!(id);
    node(Change::Rename, data)
}

/// A scope's new epoch, sealed to `to`, its key's `keys::epoch_hash` as `hash`.
pub fn epoch(scope: &Scope, n: u32, to: &[Key], hash: &str) -> Content {
    node(Change::Epoch, json!({ "scope": scope.to_string(), "n": n, "to": to.iter().map(hex).collect::<Vec<_>>(), "hash": hash }))
}

/// A scope's Vault key `n`, its public half `public`, sealed to the machines `to`.
pub fn vault(scope: &Scope, n: u32, public: &[u8; 32], to: &[Key]) -> Content {
    node(Change::Vault, json!({ "scope": scope.to_string(), "n": n, "public": hex(public), "to": to.iter().map(hex).collect::<Vec<_>>() }))
}

/// A part of a device's oned (a service, or `apps`, the app runtime) switched on or off there; a service, for the app
/// added in `scope`.
pub fn service(device: &Key, name: &str, on: bool, scope: Option<&Scope>) -> Content {
    let mut data = json!({ "device": hex(device), "name": name, "on": on });
    if let Some(scope) = scope {
        data["scope"] = json!(scope.to_string());
    }
    node(Change::Service, data)
}

/// Where its writer's own things run besides their own machines: the network's others that run their own scope.
pub fn runs(on: &[Key]) -> Content {
    node(Change::Runs, json!({ "on": on.iter().map(hex).collect::<Vec<_>>() }))
}

/// App schedules: `endpoint` runs `app`'s schedules in `scope` until `until`, or no longer with 0.
pub fn app_schedules(endpoint: &Key, scope: &Scope, app: &str, until: u64, keys: &BTreeMap<u32, String>) -> Content {
    let keys: serde_json::Map<String, Value> = keys.iter().map(|(n, h)| (n.to_string(), json!(h))).collect();
    node(Change::AppSchedules, json!({ "endpoint": hex(endpoint), "scope": scope.to_string(), "app": app, "until": until, "keys": keys }))
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
    Ok(node(Change::App, data))
}

/// Adds an app to a scope, or removes it.
pub fn added(app: &str, added: bool) -> Content {
    node(if added { Change::AddApp } else { Change::RemoveApp }, json!({ "app": app }))
}

/// A new app, or an app given to a new owner (`owner`, as `Principal` writes it).
pub fn app(name: &str, owner: &str) -> Content {
    node(Change::App, json!({ "name": name, "owner": owner }))
}

/// A new app `name`, a copy of `from`.
pub fn fork(name: &str, owner: &str, from: &str) -> Content {
    node(Change::App, json!({ "name": name, "owner": owner, "from": from }))
}

/// Where an app's code is kept (`scopes::source_link`), or nowhere with "".
pub fn source(name: &str, owner: &str, link: &str) -> Content {
    node(Change::App, json!({ "name": name, "owner": owner, "source": link }))
}

/// A deploy of `app`: the bundle at blob `bundle`, and the builds of its service it names (`bundle::SERVICES`), each a
/// blob too, so a node that runs it finds it.
pub fn deploy(app: &str, bundle: &str, builds: impl IntoIterator<Item = String>) -> Content {
    let blobs = std::iter::once(bundle.to_string()).chain(builds).collect();
    Content { blobs, ..node(Change::Deploy, json!({ "app": app, "bundle": bundle })) }
}

/// Switches a schedule of an app added in the step's scope on or off.
pub fn schedule(app: &str, schedule: &str, on: bool) -> Content {
    node(Change::Schedule, json!({ "app": app, "schedule": schedule, "on": on }))
}

/// The blobs its scope keeps.
pub fn keep(blobs: Vec<String>) -> Content {
    Content { blobs, ..node(Change::Keep, json!({})) }
}

/// A blob its scope no longer needs (its hash, hex).
pub fn forget(blob: &str) -> Content {
    node(Change::Forget, json!({ "blob": blob.to_lowercase() }))
}

/// A page arranged by its person, in their own scope: its items, the apps taken off it, and with their own page
/// (`user`), `pages`, the order of their groups' pages and those they hid.
pub fn arrange(page: &str, items: Value, hidden: Value, pages: Value) -> Content {
    let mut data = json!({ "page": page, "items": items, "hidden": if hidden.is_null() { json!([]) } else { hidden } });
    if page == "user" && !pages.is_null() {
        data["pages"] = pages;
    }
    node(Change::Arrange, data)
}

/// What a Vault request asks for: `id`'s item for `app`, its `title`, `fields` (`{ name: { secret } }`) and `hosts`, the
/// `need` the app declares it as, if it does, `why` for the person, and the agent that asked.
pub struct Request<'a> {
    pub id: &'a str,
    pub app: &'a str,
    pub title: Value,
    pub fields: Value,
    pub hosts: Value,
    pub need: Option<&'a str>,
    pub why: &'a str,
    pub agent: Option<&'a str>,
}

/// A Vault request made.
pub fn vault_request(r: Request) -> Content {
    node(Change::VaultRequest, json!({ "id": r.id, "app": r.app, "title": r.title, "fields": r.fields, "hosts": r.hosts, "need": r.need, "why": r.why, "agent": r.agent }))
}

/// A Vault request answered: declined, or set once its item is in the scope's Vault.
pub fn vault_answer(id: &str, declined: bool) -> Content {
    let mut data = json!({ "id": id });
    data[if declined { "declined" } else { "set" }] = json!(true);
    node(Change::VaultRequest, data)
}

/// Lists a service endpoint offering `service` as `user`'s, a part of `device`, by that device.
pub fn service_endpoint(endpoint: &crate::keys::Endpoint, user: &str, name: &str, service: &str, device: &Key) -> Content {
    node(Change::Device, json!({ "key": hex(&endpoint.key), "seal": hex(&endpoint.seal), "user": user, "name": name, "roles": [Kind::Service], "service": service, "device": hex(device) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind of the core's steps, built with fixed inputs, as the bytes its step's body signs: a step a release
    /// writes must say exactly what the one before wrote, or a reader of either judges it differently.
    #[test]
    fn each_kind_says_what_it_always_said() {
        let (user, other, g) = ("a000000000000001", "b000000000000002", "c000000000000003");
        let endpoint = crate::keys::Endpoint { key: [1; 32], seal: [2; 32] };
        let scope = Scope::Group(g.into());
        let built = [
            ("device", device(&endpoint, user, "mac", &[Kind::Client], None)),
            ("device, new person", device(&endpoint, other, "phone", &[Kind::Client, Kind::Compute], Some(("Bob", Role::Member)))),
            ("passkey", passkey(&endpoint, user, "Passkey", "cred")),
            ("service endpoint", service_endpoint(&endpoint, user, "whatsapp", "whatsapp", &[4; 32])),
            ("role", role(other, Role::Admin)),
            ("remove person", remove_user(other)),
            ("remove device", remove_device(&[5; 32])),
            ("setting", setting("name", Some("Ours"))),
            ("setting back", setting("name", None)),
            ("group", group(g, "Band")),
            ("invite", invite(g, other, Role::Member)),
            ("accept", accept(g)),
            ("member", member(g, other, Some(Role::Admin))),
            ("member out", member(g, other, None)),
            ("refuse", refuse(&endpoint, "stranger")),
            ("forget sign-in", forget_sign_in(&[5; 32])),
            ("rename device", rename("key", &hex(&[5; 32]), "laptop")),
            ("rename person", rename("user", other, "Robert")),
            ("rename group", rename("group", g, "The Band")),
            ("handle person", handle("user", other, "bob")),
            ("handle group", handle("group", g, "band")),
            ("epoch", epoch(&scope, 2, &[[6; 32], [7; 32]], &"f".repeat(64))),
            ("vault", vault(&Scope::User(user.into()), 1, &[8; 32], &[[6; 32]])),
            ("service on", service(&[4; 32], "whatsapp", true, Some(&scope))),
            ("service off", service(&[4; 32], "apps", false, None)),
            ("runs", runs(&[[6; 32], [7; 32]])),
            ("app schedules", app_schedules(&[6; 32], &scope, "notes", 99, &BTreeMap::from([(1, "a".repeat(64)), (2, "b".repeat(64))]))),
            ("app schedules off", app_schedules(&[6; 32], &Scope::Global, "notes", 0, &BTreeMap::new())),
            ("deleted", retire("notes", "admins", "deleted").unwrap()),
            ("restored", retire("notes", "admins", "restored").unwrap()),
            ("erased", retire("notes", &format!("group:{g}"), "erased").unwrap()),
            ("added", added("notes", true)),
            ("removed", added("notes", false)),
            ("app", app("notes", "admins")),
            ("fork", fork("tally2", &format!("user:{user}"), "tally")),
            ("source", source("notes", "admins", "https://example.com/notes.git")),
            ("no source", source("notes", "admins", "")),
            ("deploy", deploy("notes", &"d".repeat(64), ["e".repeat(64)])),
            ("schedule", schedule("notes", "daily", true)),
            ("keep", keep(vec!["c".repeat(64)])),
            ("keep nothing", keep(Vec::new())),
            ("forget", forget(&"C".repeat(64))),
            ("arrange", arrange("user", json!([{ "app": "notes" }]), Value::Null, json!({ "order": [format!("group:{g}")], "hidden": [] }))),
            ("arrange a group's", arrange(&format!("group:{g}"), json!([{ "app": "notes", "widget": "w", "id": "1" }]), json!(["tally"]), json!({ "order": [] }))),
            (
                "vault request",
                vault_request(Request { id: "e000000000000008", app: "notes", title: json!("Token"), fields: json!({ "token": { "secret": true } }), hosts: json!(["api.example.com"]), need: Some("token"), why: "for notes", agent: Some("Claude") }),
            ),
            ("vault request, plain", vault_request(Request { id: "e000000000000009", app: "ai-agents", title: json!("Key"), fields: json!({}), hosts: Value::Null, need: None, why: "", agent: None })),
            ("vault declined", vault_answer("e000000000000008", true)),
            ("vault set", vault_answer("e000000000000008", false)),
            ("freeze", crate::database::steps::Freeze { at: 2, hash: "a".repeat(64), logs: BTreeMap::from([("global".into(), 2), (scope.to_string(), 0)]) }.content()),
            ("release", crate::release::content(&crate::release::Release { commit: "abc".into(), target: "x86_64-unknown-linux-gnu".into(), blob: "d".repeat(64), size: 9, committed: 1, signature: "sig".into(), service: None })),
            (
                "service release",
                crate::release::content(&crate::release::Release { commit: "abc".into(), target: "x86_64-unknown-linux-gnu".into(), blob: "d".repeat(64), size: 9, committed: 1, signature: "sig".into(), service: Some("whatsapp".into()) }),
            ),
        ];
        let first = first("Home", user, "Andre", &endpoint, "mac", &[Kind::Client], &[3; 32], &"e".repeat(64));
        let said: String = std::iter::once(format!("first: {first}\n")).chain(built.iter().map(|(name, c)| format!("{name}: {}\n", serde_json::to_string(c).unwrap()))).collect();
        assert_eq!(said, GOLDEN, "\n{said}");
    }

    const GOLDEN: &str = r#"first: {"device":{"key":"0101010101010101010101010101010101010101010101010101010101010101","name":"mac","roles":["client"],"seal":"0202020202020202020202020202020202020202020202020202020202020202"},"epoch":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","name":"Home","recovery":"0303030303030303030303030303030303030303030303030303030303030303","user":"a000000000000001","user_name":"Andre"}
device: {"app":"node","kind":"device","data":{"key":"0101010101010101010101010101010101010101010101010101010101010101","name":"mac","roles":["client"],"seal":"0202020202020202020202020202020202020202020202020202020202020202","user":"a000000000000001"}}
device, new person: {"app":"node","kind":"device","data":{"key":"0101010101010101010101010101010101010101010101010101010101010101","name":"phone","role":"member","roles":["client","compute"],"seal":"0202020202020202020202020202020202020202020202020202020202020202","user":"b000000000000002","user_name":"Bob"}}
passkey: {"app":"node","kind":"device","data":{"key":"0101010101010101010101010101010101010101010101010101010101010101","name":"Passkey","passkey":"cred","roles":["client"],"seal":"0202020202020202020202020202020202020202020202020202020202020202","user":"a000000000000001"}}
service endpoint: {"app":"node","kind":"device","data":{"device":"0404040404040404040404040404040404040404040404040404040404040404","key":"0101010101010101010101010101010101010101010101010101010101010101","name":"whatsapp","roles":["service"],"seal":"0202020202020202020202020202020202020202020202020202020202020202","service":"whatsapp","user":"a000000000000001"}}
role: {"app":"node","kind":"role","data":{"role":"admin","user":"b000000000000002"}}
remove person: {"app":"node","kind":"remove","data":{"user":"b000000000000002"}}
remove device: {"app":"node","kind":"remove","data":{"key":"0505050505050505050505050505050505050505050505050505050505050505"}}
setting: {"app":"node","kind":"setting","data":{"name":"name","value":"Ours"}}
setting back: {"app":"node","kind":"setting","data":{"name":"name","value":null}}
group: {"app":"node","kind":"group","data":{"group":"c000000000000003","name":"Band"}}
invite: {"app":"node","kind":"invite","data":{"group":"c000000000000003","role":"member","user":"b000000000000002"}}
accept: {"app":"node","kind":"accept","data":{"group":"c000000000000003"}}
member: {"app":"node","kind":"member","data":{"group":"c000000000000003","role":"admin","user":"b000000000000002"}}
member out: {"app":"node","kind":"member","data":{"group":"c000000000000003","role":null,"user":"b000000000000002"}}
refuse: {"app":"node","kind":"refuse","data":{"key":"0101010101010101010101010101010101010101010101010101010101010101","name":"stranger","seal":"0202020202020202020202020202020202020202020202020202020202020202"}}
forget sign-in: {"app":"node","kind":"forget-sign-in","data":{"key":"0505050505050505050505050505050505050505050505050505050505050505"}}
rename device: {"app":"node","kind":"rename","data":{"key":"0505050505050505050505050505050505050505050505050505050505050505","name":"laptop"}}
rename person: {"app":"node","kind":"rename","data":{"name":"Robert","user":"b000000000000002"}}
rename group: {"app":"node","kind":"rename","data":{"group":"c000000000000003","name":"The Band"}}
handle person: {"app":"node","kind":"rename","data":{"handle":"bob","user":"b000000000000002"}}
handle group: {"app":"node","kind":"rename","data":{"group":"c000000000000003","handle":"band"}}
epoch: {"app":"node","kind":"epoch","data":{"hash":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","n":2,"scope":"group:c000000000000003","to":["0606060606060606060606060606060606060606060606060606060606060606","0707070707070707070707070707070707070707070707070707070707070707"]}}
vault: {"app":"node","kind":"vault","data":{"n":1,"public":"0808080808080808080808080808080808080808080808080808080808080808","scope":"user:a000000000000001","to":["0606060606060606060606060606060606060606060606060606060606060606"]}}
service on: {"app":"node","kind":"service","data":{"device":"0404040404040404040404040404040404040404040404040404040404040404","name":"whatsapp","on":true,"scope":"group:c000000000000003"}}
service off: {"app":"node","kind":"service","data":{"device":"0404040404040404040404040404040404040404040404040404040404040404","name":"apps","on":false}}
runs: {"app":"node","kind":"runs","data":{"on":["0606060606060606060606060606060606060606060606060606060606060606","0707070707070707070707070707070707070707070707070707070707070707"]}}
app schedules: {"app":"node","kind":"app-schedules","data":{"app":"notes","endpoint":"0606060606060606060606060606060606060606060606060606060606060606","keys":{"1":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","2":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"scope":"group:c000000000000003","until":99}}
app schedules off: {"app":"node","kind":"app-schedules","data":{"app":"notes","endpoint":"0606060606060606060606060606060606060606060606060606060606060606","keys":{},"scope":"global","until":0}}
deleted: {"app":"node","kind":"app","data":{"deleted":true,"name":"notes","owner":"admins"}}
restored: {"app":"node","kind":"app","data":{"deleted":false,"name":"notes","owner":"admins"}}
erased: {"app":"node","kind":"app","data":{"erased":true,"name":"notes","owner":"group:c000000000000003"}}
added: {"app":"node","kind":"add-app","data":{"app":"notes"}}
removed: {"app":"node","kind":"remove-app","data":{"app":"notes"}}
app: {"app":"node","kind":"app","data":{"name":"notes","owner":"admins"}}
fork: {"app":"node","kind":"app","data":{"from":"tally","name":"tally2","owner":"user:a000000000000001"}}
source: {"app":"node","kind":"app","data":{"name":"notes","owner":"admins","source":"https://example.com/notes.git"}}
no source: {"app":"node","kind":"app","data":{"name":"notes","owner":"admins","source":""}}
deploy: {"app":"node","kind":"deploy","data":{"app":"notes","bundle":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"},"blobs":["dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"]}
schedule: {"app":"node","kind":"schedule","data":{"app":"notes","on":true,"schedule":"daily"}}
keep: {"app":"node","kind":"keep","data":{},"blobs":["cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"]}
keep nothing: {"app":"node","kind":"keep","data":{}}
forget: {"app":"node","kind":"forget","data":{"blob":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}
arrange: {"app":"node","kind":"arrange","data":{"hidden":[],"items":[{"app":"notes"}],"page":"user","pages":{"hidden":[],"order":["group:c000000000000003"]}}}
arrange a group's: {"app":"node","kind":"arrange","data":{"hidden":["tally"],"items":[{"app":"notes","id":"1","widget":"w"}],"page":"group:c000000000000003"}}
vault request: {"app":"node","kind":"vault-request","data":{"agent":"Claude","app":"notes","fields":{"token":{"secret":true}},"hosts":["api.example.com"],"id":"e000000000000008","need":"token","title":"Token","why":"for notes"}}
vault request, plain: {"app":"node","kind":"vault-request","data":{"agent":null,"app":"ai-agents","fields":{},"hosts":null,"id":"e000000000000009","need":null,"title":"Key","why":""}}
vault declined: {"app":"node","kind":"vault-request","data":{"declined":true,"id":"e000000000000008"}}
vault set: {"app":"node","kind":"vault-request","data":{"id":"e000000000000008","set":true}}
freeze: {"app":"node","kind":"freeze","data":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","logs":{"global":2,"group:c000000000000003":0}}}
release: {"app":"node","kind":"node-release","data":{"blob":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","commit":"abc","committed":1,"signature":"sig","size":9,"target":"x86_64-unknown-linux-gnu"},"blobs":["dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"]}
service release: {"app":"node","kind":"service-release","data":{"blob":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","commit":"abc","committed":1,"service":"whatsapp","signature":"sig","size":9,"target":"x86_64-unknown-linux-gnu"},"blobs":["dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"]}
"#;
}
