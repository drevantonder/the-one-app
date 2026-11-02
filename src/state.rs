//! `GET /v1/state`: the install as one person sees it, from its members, the same from a node on a machine and from
//! the serverless node. What only the node answering knows (which devices are online and how loaded, who is looking)
//! it says in `Here`.

use serde_json::{Map, Value, json};

use crate::entry::hex;
use crate::members::{Machine, Members, Principal, Role};
use crate::rank;

/// What the node answering knows that the members don't.
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
    /// What it knows of each device now: `kind` (as it last said), and whatever else it says of it.
    pub device_now: &'a dyn Fn(&Machine) -> Map<String, Value>,
    /// What it knows of each client now.
    pub client_now: &'a dyn Fn(&Machine) -> Map<String, Value>,
}

/// The install as `user` sees it, with `here` from the node answering; None for `install` if it has none.
pub fn state(m: Option<&Members>, user: &str, here: Here) -> Value {
    let empty = Members::default();
    let settings = settings(m.unwrap_or(&empty));
    let install = m.map(|m| install(m, user, &settings, &here));
    let settings_items = m.map(|m| settings_items(m, user, here.waiting, here.backups.is_some(), settings["store"].as_str().unwrap_or(""))).unwrap_or_default();
    let config = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "accent": crate::config::ACCENT, "store": settings["store"], "push": settings["push"] });
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
        // What to paste into an agent so it connects itself, for Ask Your Agent.
        "agent_prompt": agent_prompt(settings.get("address").and_then(|a| a.as_str()), here.guide.as_deref()),
    });
    if let Some(client) = here.client {
        state["client"] = client;
    }
    state
}

fn install(m: &Members, me: &str, settings: &Map<String, Value>, here: &Here) -> Value {
    let list = |ps: Vec<Principal>| ps.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    let set = rank::set(&m.settings);
    let mut devices: Vec<Value> = m
        .devices()
        .map(|s| {
            let mut now = (here.device_now)(s);
            // A device that hasn't said what it is, as one that's always on.
            let kind = now.remove("kind").and_then(|k| k.as_str().filter(|k| !k.is_empty()).map(str::to_string)).unwrap_or_else(|| "always_on".into());
            let mut device = json!({
                "kind": kind, "use": rank::place(&set, &hex(&s.key), &kind), "far": false,
                "key": hex(&s.key), "name": s.name, "user": s.user, "me": false, "here": false, "online": false,
                "owners": list(m.owners(&s.key)), "users": list(m.users_of(&s.key)), "privileged": m.privileged(&s.key),
                "trusted_by": m.users.keys().filter(|u| m.trusts(u, &s.key)).collect::<Vec<_>>(),
                "serves_me": m.serves(&s.key, me),
                "build": null, "target": null, "sidecars": [], "installed_sidecars": [],
                "removed_sidecars": m.sidecars.iter().filter(|((k, _), d)| *k == s.key && d.removed()).map(|((_, name), d)| json!({ "name": name, "by": m.users.get(&d.by).map(|u| &u.name), "time": d.time })).collect::<Vec<_>>(),
                "load": null,
            });
            device.as_object_mut().unwrap().extend(now);
            device
        })
        .collect();
    // Each device's place in where things run first, 0 first, for sidecars that pick one themselves.
    let mut by_order: Vec<usize> = (0..devices.len()).collect();
    by_order.sort_by_cached_key(|&n| {
        let s = &devices[n];
        (rank::order(&set, s["key"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase())
    });
    for (place, n) in by_order.into_iter().enumerate() {
        devices[n]["order"] = json!(place);
    }
    let clients: Vec<Value> = m
        .clients()
        .map(|c| {
            let mut client = json!({ "key": hex(&c.key), "name": c.name, "user": c.user, "me": false });
            client.as_object_mut().unwrap().extend((here.client_now)(c));
            client
        })
        .collect();
    let users: Vec<Value> = m.users.values().filter(|u| !u.removed).map(|u| json!({ "id": u.id, "name": u.name, "role": u.role, "me": u.id == me })).collect();
    let groups: Vec<Value> = m
        .groups
        .values()
        .map(|g| {
            let members: Vec<Value> = g.members.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
            let invited: Vec<Value> = g.invited.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
            json!({ "id": g.id, "name": g.name, "role": m.group_role(me, &g.id), "members": members, "invited": invited })
        })
        .collect();
    json!({
        "name": m.name, "root": m.root.map(|r| r.to_hex()), "user": me, "role": m.user(me).map(|u| u.role),
        "member": here.member,
        "users": users, "groups": groups, "devices": devices, "devices_at": m.devices_at, "clients": clients,
        "entries": here.entries, "settings": settings,
        // The serverless node, once an admin deployed one (`serverless deploy` sets `serverless`).
        "serverless": settings.contains_key("serverless").then(|| json!({ "use": rank::place(&set, rank::CLOUDFLARE, rank::CLOUDFLARE) })),
    })
}

/// The install's settings: the white label's defaults (config.toml's `[settings]`), and what its admins set over them.
pub fn settings(m: &Members) -> Map<String, Value> {
    let mut out: Map<String, Value> = crate::config::SETTINGS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    out.extend(m.settings.iter().map(|(k, v)| (k.clone(), json!(v))));
    for key in ["store", "push"] {
        out.entry(key).or_insert(json!(""));
    }
    out
}

/// What to paste into an agent so it connects itself: the MCP server at the install's address, where it has one; else
/// the skills a device serves on its tailnet (`guide`), or, where it serves none, the command line's guide.
pub fn agent_prompt(address: Option<&str>, guide: Option<&str>) -> String {
    let (name, cli) = (crate::config::NAME, crate::config::CLI);
    match (address.filter(|a| !a.is_empty()), guide) {
        (Some(address), _) => format!("Connect yourself to {name}: add the MCP server {}/mcp, and I'll sign in when it asks.", address.trim_end_matches('/')),
        (None, Some(guide)) => format!("Connect yourself to {name}: read {guide} and set it up for me."),
        (None, None) => format!("Connect yourself to {name}: run \"{cli} guide\" in a terminal and set it up for me."),
    }
}

/// What Settings shows `user`, in order, with a badge on Requests for `items_asked` and their other requests, and on
/// Backups when `backups` are failing.
fn settings_items(m: &Members, user: &str, items_asked: usize, backups: bool, store: &str) -> Vec<Value> {
    let Some(me) = m.user(user) else { return Vec::new() };
    let item = |name: &str, title: &str, icon: &str, color: &str| json!({ "name": name, "title": title, "icon": icon, "color": color });
    let with = |mut item: Value, key: &str, value: Value| {
        item[key] = value;
        item
    };
    let invites = m.groups.values().filter(|g| g.invited.contains_key(user)).count();
    let offers = m
        .devices()
        .filter(|s| !m.privileged(&s.key) && !m.owners(&s.key).contains(&Principal::User(user.into())) && !m.trusts(user, &s.key))
        .filter(|s| m.users_of(&s.key).iter().any(|p| matches!(p, Principal::User(u) if u == user) || matches!(p, Principal::Group(g) if m.group_role(user, g).is_some())))
        .count();
    let mut out = vec![item("account", &me.name, "user-circle", "#8e8e93")];
    let requests = invites + offers + items_asked;
    if requests > 0 {
        out.push(with(item("requests", "Requests", "tray", "#ff2d55"), "badge", json!(requests)));
    }
    if m.apps.get(store).is_some_and(|a| !a.erased && a.deleted.is_none()) {
        out.push(with(item("store", "App Store", "squares-four", "#007aff"), "app", json!(store)));
    }
    out.extend([
        item("groups", "Groups", "users-three", "#34c759"),
        item("devices", "Devices", "devices", "#8e8e93"),
        item("agents", "Agents", "sparkle", "#ff2d55"),
        item("vault", "Vault", "lock", "#5856d6"),
        item("appearance", "Appearance", "circle-half", "#007aff"),
    ]);
    if me.role == Role::Admin {
        out.extend([item("users", "Users", "user-gear", "#30b0c7"), item("general", "General", "sliders-horizontal", "#8e8e93")]);
        let backed = item("backups", "Backups", "hard-drives", "#ff9500");
        out.push(if backups { with(backed, "badge", json!(1)) } else { backed });
    }
    out
}

