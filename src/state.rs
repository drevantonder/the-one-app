//! `GET /v1/state`: the install as one person sees it, from `global`'s scope database, the same from a node on a
//! machine and from the Cloudflare node. What only the node answering knows (which devices are online and how loaded,
//! who is looking) it says in `Here`.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::database::scopes::{INSTALL, ScopeDb, object};
use crate::entry::hex;
use crate::members::{Kind, Machine};
use crate::rank;

/// What the node answering knows that the scope database doesn't.
pub struct Here<'a> {
    /// Who is looking: `{ key, name, user }`.
    pub me: Value,
    /// The client looking, if it's a device and not this one.
    pub client: Option<Value>,
    /// The node answering: `{ key, name, ready, guide }`.
    pub device: Value,
    /// Whether that node is one of the install's devices.
    pub member: bool,
    /// How many entries it holds.
    pub entries: u64,
    /// Requests for an item waiting for this person to answer.
    pub waiting: usize,
    /// How this person arranged each of their pages.
    pub arrangements: Value,
    /// What's wrong with backups, for an admin.
    pub backups: Option<Value>,
    /// Where an agent reads how to connect itself, when the install has no address.
    pub guide: Option<String>,
    /// Whether people manage passkeys at the install's address: not in a throwaway network, whose sign-in lets its person in
    /// with none.
    pub passkeys: bool,
    /// What it knows of each device now: `kind` (as it last said), and whatever else it says of it.
    pub device_now: &'a dyn Fn(&Machine) -> Map<String, Value>,
    /// What it knows of each client now.
    pub client_now: &'a dyn Fn(&Machine) -> Map<String, Value>,
}

/// The install as `user` sees it, with `here` from the node answering; None for `install` if it has none.
pub fn state(m: Option<&ScopeDb>, user: &str, here: Here) -> Value {
    let settings = settings(&m.map(ScopeDb::settings).unwrap_or_default());
    let install = m.map(|m| install(m, user, &settings, &here));
    let address = settings.get("address").and_then(|a| a.as_str()).filter(|a| !a.is_empty());
    let passkeys = address.filter(|_| here.passkeys).map(|a| format!("{}/passkeys", a.trim_end_matches('/')));
    let settings_items = m.map(|m| settings_items(m, user, here.waiting, here.backups.is_some(), settings["store"].as_str().unwrap_or(""), passkeys.as_deref())).unwrap_or_default();
    // `push` is for phones from before notes, which register with a node while it's set (src/push.rs).
    let config = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "store": settings["store"], "push": crate::push::NAME, "install": install_line() });
    let mut state = json!({
        "config": config,
        "me": here.me,
        "device": here.device,
        "user": m.and_then(|m| m.user(user)).map(|u| json!({ "id": u.id, "name": u.name, "role": u.role })),
        "install": install,
        // Requests for an item waiting for this user to answer (see GET /v1/vault/requests).
        "vault_requests": here.waiting,
        // How this user arranged each of their pages (see `arrange`).
        "arrangements": here.arrangements,
        // What's wrong with backups, for an admin: a badge on Settings until a run works again.
        "backups": here.backups,
        // What Settings shows this user, in order (see `settings_items`).
        "settings_items": settings_items,
        // What to paste into an AI so it connects itself, for Connect Your AI.
        "agent_prompt": agent_prompt(address, here.guide.as_deref()),
    });
    if let Some(client) = here.client {
        state["client"] = client;
    }
    state
}

fn install(m: &ScopeDb, me: &str, settings: &Map<String, Value>, here: &Here) -> Value {
    let set = rank::set(&m.settings());
    let switches = m.switches();
    // Service endpoints are listed as devices too, offering their service.
    let mut devices: Vec<Value> = m
        .machines()
        .iter()
        .filter(|s| !s.removed && (s.is(Kind::Compute) || s.is(Kind::Service)))
        .map(|s| {
            let mut now = (here.device_now)(s);
            // A device that hasn't said what it is, as one that's always on.
            let kind = now.remove("kind").and_then(|k| k.as_str().filter(|k| !k.is_empty()).map(str::to_string)).unwrap_or_else(|| "always_on".into());
            let mut device = json!({
                "kind": kind, "priority": rank::priority(&set, &hex(&s.key)).to_value(), "far": false, "since": s.since,
                "key": hex(&s.key), "seal": hex(&s.seal), "name": s.name, "user": s.user, "me": false, "here": false, "online": false,
                "roles": s.roles, "service": s.service, "device": s.device.as_ref().map(hex), "serves_me": m.serves(&s.key, me),
                "runs_for": m.run_by(&s.key).iter().map(|scope| scope.to_string()).collect::<Vec<_>>(),
                "build": null, "target": null, "services": [], "installed_services": [],
                "off_services": switches.iter().filter(|((k, _), d)| *k == s.key && !d.on).map(|((_, name), d)| json!({ "name": name, "time": d.time })).collect::<Vec<_>>(),
                "load": null,
            });
            device.as_object_mut().unwrap().extend(now);
            device
        })
        .collect();
    // Each device's place in run priority's order, 0 first, for services that pick one themselves; on the same number,
    // by name.
    let mut by_order: Vec<usize> = (0..devices.len()).collect();
    by_order.sort_by_cached_key(|&n| {
        let s = &devices[n];
        (rank::priority(&set, s["key"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase())
    });
    for (place, n) in by_order.into_iter().enumerate() {
        devices[n]["order"] = json!(place);
    }
    // Devices that also run calls are listed with the devices.
    let clients: Vec<Value> = m
        .clients()
        .iter()
        .filter(|c| !c.is(Kind::Compute))
        .map(|c| {
            let mut client = json!({ "key": hex(&c.key), "seal": hex(&c.seal), "name": c.name, "user": c.user, "roles": c.roles, "passkey": c.passkey, "device": c.device.as_ref().map(hex), "since": c.since, "place": c.place, "me": false });
            // What it signed in with, for its own person only: their Sign-ins.
            if c.user == me && c.sign_in.is_some() {
                client["sign_in"] = json!(c.sign_in);
            }
            client.as_object_mut().unwrap().extend((here.client_now)(c));
            client
        })
        .collect();
    let users: Vec<Value> = m.users().iter().filter(|u| !u.removed).map(|u| json!({ "id": u.id, "name": u.name, "handle": u.handle, "role": u.role, "me": u.id == me })).collect();
    let groups: Vec<Value> = m
        .groups()
        .iter()
        .map(|g| {
            let members: Vec<Value> = g.members.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
            let invited: Vec<Value> = g.invited.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
            json!({ "id": g.id, "name": g.name, "handle": g.handle, "role": m.group_role(me, &g.id), "members": members, "invited": invited })
        })
        .collect();
    json!({
        "name": m.name(), "root": m.root().map(|r| r.to_hex()), "user": me, "role": m.user(me).map(|u| u.role),
        // What a device seals a scope's new epoch to, beside its devices' seal keys.
        "recovery": m.recovery().as_ref().map(hex),
        // Each scope's newest epoch the viewer may use, and its epochs' hashes, which a key from the bucket must match.
        "epochs": m.epochs().into_iter().filter(|(s, _)| m.may(me, "use", &object(s))).map(|(s, n)| {
            let hashes: Map<String, Value> = m.epoch_hashes(&s).into_iter().map(|(n, h)| (n.to_string(), json!(h))).collect();
            (s.to_string(), json!({ "n": n, "hashes": hashes }))
        }).collect::<Map<String, Value>>(),
        // Where the viewer's own things run, as their own device chose: the machines' keys, or null if it hasn't yet.
        "runs": m.runs(me).map(|on| on.iter().map(hex).collect::<Vec<_>>()),
        // When the install's recovery key last let a device in, and whose.
        "recovery_used": m.recovery_used().map(|(time, user)| json!({ "time": time, "user": user })),
        // When the install's recovery key was set.
        "recovery_set": m.recovery_set(),
        "member": here.member,
        "users": users, "groups": groups, "devices": devices, "devices_at": m.devices_at(), "clients": clients,
        // Standing work in the scopes this person may grant it in: which endpoint keeps which app running in which
        // scope, until when, and who granted it. One whose endpoint is gone, or whose granter may grant no more, is over.
        "grants": m.keeps().iter().filter(|((k, s, _), g)| m.may(me, "keep", &object(s)) && m.machine(k).is_some() && m.may(&g.by, "keep", &object(s))).map(|((k, s, a), g)| json!({ "endpoint": hex(k), "scope": s.to_string(), "app": a, "until": g.until, "by": g.by, "keys": g.hashes })).collect::<Vec<_>>(),
        "entries": here.entries, "settings": settings,
        // The Cloudflare node, once an admin deployed one (`cloudflare node deploy` sets `serverless`, which clients read here by that name).
        "serverless": m.cloudflare_node().map(|key| json!({ "priority": rank::priority(&set, rank::CLOUDFLARE).to_value(), "key": hex(&key), "runs_for": m.run_by(&key).iter().map(crate::entry::Scope::to_string).collect::<Vec<_>>() })),
    })
}

/// The install's settings: the custom build's defaults (config.toml's `[settings]`), and what its admins set over them
/// (`ScopeDb::settings`).
pub fn settings(set: &BTreeMap<String, String>) -> Map<String, Value> {
    let mut out: Map<String, Value> = crate::config::SETTINGS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    out.extend(set.iter().map(|(k, v)| (k.clone(), json!(v))));
    out.entry("store").or_insert(json!(""));
    out
}

/// The one line that installs this custom build's latest release on a machine, if it publishes releases.
pub fn install_line() -> Option<String> {
    let repo = crate::config::REPO;
    (!repo.is_empty()).then(|| format!("curl -fsSL https://github.com/{repo}/releases/latest/download/install.sh | sh"))
}

/// What to paste into an AI so it connects itself, for Connect Your AI: the guide at the install's address, where it
/// has one; else the one a device serves on its tailnet (`guide`), or, where it serves none, the command line's guide.
pub fn agent_prompt(address: Option<&str>, guide: Option<&str>) -> String {
    let (yours, cli) = (crate::config::YOURS, crate::config::CLI);
    match (address.filter(|a| !a.is_empty()), guide) {
        (Some(address), _) => format!("Read {}/llms.txt and connect yourself to {yours}.", address.trim_end_matches('/')),
        (None, Some(guide)) => format!("Read {guide} and connect yourself to {yours}."),
        (None, None) => format!("Run \"{cli} guide\" in a terminal, read what it says, and connect yourself to {yours}."),
    }
}

/// What Settings shows `user`, in order, with a badge on Requests for `items_asked` and their other requests, and on
/// Backups when `backups` are failing. Passkeys opens `passkeys` in a browser, where they're kept.
fn settings_items(m: &ScopeDb, user: &str, items_asked: usize, backups: bool, store: &str, passkeys: Option<&str>) -> Vec<Value> {
    let Some(me) = m.user(user) else { return Vec::new() };
    let item = |name: &str, title: &str, icon: &str, color: &str| json!({ "name": name, "title": title, "icon": icon, "color": color });
    let with = |mut item: Value, key: &str, value: Value| {
        item[key] = value;
        item
    };
    let invites = m.groups().iter().filter(|g| g.invited.contains_key(user)).count();
    let mut out: Vec<Value> = passkeys.map(|url| with(item("passkeys", "Passkeys", "key", "#8e8e93"), "url", json!(url))).into_iter().collect();
    let requests = invites + items_asked;
    if requests > 0 {
        out.push(with(item("requests", "Requests", "tray", "#ff2d55"), "badge", json!(requests)));
    }
    if m.app(store).is_some_and(|a| !a.erased && a.deleted.is_none()) {
        out.push(with(item("store", "App Store", "squares-four", "#007aff"), "app", json!(store)));
    }
    out.extend([
        item("users", "Users", "user-gear", "#30b0c7"),
        item("groups", "Groups", "users-three", "#34c759"),
        item("devices", "Devices", "devices", "#8e8e93"),
        item("connect-ai", "Connect Your AI", "plugs-connected", "#ff2d55"),
        item("vault", "Vault", "lock", "#5856d6"),
        item("appearance", "Appearance", "circle-half", "#007aff"),
    ]);
    if m.may(&me.id, "change-settings", INSTALL) {
        let backed = item("backups", "Backups", "hard-drives", "#ff9500");
        out.push(if backups { with(backed, "badge", json!(1)) } else { backed });
        out.push(item("labs", "Labs", "flask", "#af52de"));
        out.push(item("developer", "Developer", "code", "#8e8e93"));
    }
    out
}

