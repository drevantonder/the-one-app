//! `GET /v1/state`: the network as one person sees it, from `global`'s scope database, the same from a node on a
//! machine and from the Cloudflare node. What only the node answering knows (which devices are online and how loaded,
//! who is looking) it says in `Here`.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::database::scopes::{NETWORK, ScopeDb, object};
use crate::keys::hex;
use crate::database::scopes;
use crate::database::scopes::{App, Kind, Machine, Role, Scope, ScopeState};
use crate::rank;

/// What the node answering knows that the scope database doesn't.
pub struct Here<'a> {
    /// Who is looking: `{ key, name, user }`.
    pub me: Value,
    /// The client looking, if it's a device and not this one.
    pub client: Option<Value>,
    /// The node answering: `{ key, name, ready, guide }`.
    pub device: Value,
    /// Whether that node is one of the network's devices.
    pub member: bool,
    pub steps: u64,
    /// Requests for an item waiting for this person to answer.
    pub waiting: usize,
    /// How this person arranged each of their pages.
    pub arrangements: Value,
    /// What's wrong with backups, for an admin.
    pub backups: Option<Value>,
    /// Where an agent reads how to connect itself, when the network has no address.
    pub guide: Option<String>,
    /// Whether this is a throwaway network, whose sign-in lets its person in with no passkey. Every other network's people
    /// manage passkeys at its address, a Google one's too: its admins sign in only with one.
    pub throwaway: bool,
    /// What it knows of each device now: `kind` (as it last said), and whatever else it says of it.
    pub device_now: &'a dyn Fn(&Machine) -> Result<Map<String, Value>>,
    /// What it knows of each client now.
    pub client_now: &'a dyn Fn(&Machine) -> Map<String, Value>,
}

/// The network as `user` sees it, with `here` from the node answering; None for `network` if it has none.
pub fn state(m: Option<&ScopeDb>, user: &str, here: Here) -> Result<Value> {
    let settings = settings(&m.map(ScopeDb::settings).transpose()?.unwrap_or_default());
    let network = m.map(|m| network(m, user, &settings, &here)).transpose()?;
    let address = settings.get("address").and_then(|a| a.as_str()).filter(|a| !a.is_empty());
    let passkeys = address.filter(|_| !here.throwaway).map(|a| format!("{}/passkeys", a.trim_end_matches('/')));
    let settings_items = m.map(|m| settings_items(m, user, here.waiting, here.backups.is_some(), settings["store"].as_str().unwrap_or(""), passkeys.as_deref())).transpose()?.unwrap_or_default();
    let me = m.map(|m| m.user(user)).transpose()?.flatten();
    let config = json!({ "name": crate::config::NAME, "cli": crate::config::CLI, "store": settings["store"], "install": install_line() });
    let mut state = json!({
        "config": config,
        "me": here.me,
        "device": here.device,
        "user": me.map(|u| json!({ "id": u.id, "name": u.name, "role": u.role })),
        "network": network,
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
    Ok(state)
}

fn network(m: &ScopeDb, me: &str, settings: &Map<String, Value>, here: &Here) -> Result<Value> {
    let set = rank::set(&m.settings()?);
    let switches = m.switches()?;
    // Only devices: a service with an endpoint of its own is part of one, and its device says it runs it.
    let mut devices: Vec<Value> = Vec::new();
    for s in m.machines()?.iter().filter(|s| !s.removed && s.is(Kind::Compute)) {
        let mut now = (here.device_now)(s)?;
        // A device that hasn't said what it is, as one that's always on.
        let kind = now.remove("kind").and_then(|k| k.as_str().filter(|k| !k.is_empty()).map(str::to_string)).unwrap_or_else(|| "always_on".into());
        let mut device = json!({
            "kind": kind, "priority": rank::priority(&set, &hex(&s.key)).to_value(), "far": false, "since": s.since,
            "key": hex(&s.key), "seal": hex(&s.seal), "name": s.name, "user": s.user, "me": false, "here": false, "online": false,
            "roles": s.roles, "serves_me": m.serves(&s.key, me)?,
            "runs_for": m.run_by(&s.key)?.iter().map(|scope| scope.to_string()).collect::<Vec<_>>(),
            "build": null, "target": null, "services": [], "installed_services": [],
            "off_services": switches.iter().filter(|((k, _), d)| *k == s.key && !d.on).map(|((_, name), d)| json!({ "name": name, "time": d.time })).collect::<Vec<_>>(),
            "load": null,
        });
        device.as_object_mut().unwrap().extend(now);
        devices.push(device);
    }
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
        .clients()?
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
    let users: Vec<Value> = m.users()?.iter().filter(|u| !u.removed).map(|u| json!({ "id": u.id, "name": u.name, "handle": u.handle, "role": u.role, "me": u.id == me })).collect();
    let mut groups: Vec<Value> = Vec::new();
    for g in m.groups()? {
        let members: Vec<Value> = g.members.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
        let invited: Vec<Value> = g.invited.iter().map(|(u, r)| json!({ "user": u, "role": r })).collect();
        groups.push(json!({ "id": g.id, "name": g.name, "handle": g.handle, "role": m.group_role(me, &g.id)?, "members": members, "invited": invited }));
    }
    let mut epochs = Map::new();
    for (s, n) in m.epochs()?.into_iter().filter(|(s, _)| m.may(me, "use", &object(s))) {
        let hashes: Map<String, Value> = m.epoch_hashes(&s)?.into_iter().map(|(n, h)| (n.to_string(), json!(h))).collect();
        epochs.insert(s.to_string(), json!({ "n": n, "hashes": hashes }));
    }
    let mut app_schedules = Vec::new();
    for ((k, s, a), g) in m.app_schedules()? {
        if m.may(me, "choose-devices", &object(&s)) && m.machine(&k)?.is_some() && m.may(&g.by, "choose-devices", &object(&s)) {
            app_schedules.push(json!({ "endpoint": hex(&k), "scope": s.to_string(), "app": a, "until": g.until, "by": g.by, "keys": g.hashes }));
        }
    }
    let cloudflare_node = match m.cloudflare_node()? {
        Some(key) => {
            let mut parts = vec![json!({ "name": scopes::APPS_PART, "on": true, "fixed": true })];
            for (name, _) in crate::config::CLOUDFLARE_SERVICES.iter() {
                parts.push(json!({ "name": name, "on": m.switch(&key, name)?.is_some_and(|s| s.on) }));
            }
            Some(json!({ "priority": rank::priority(&set, rank::CLOUDFLARE).to_value(), "key": hex(&key), "runs_for": m.run_by(&key)?.iter().map(crate::database::scopes::Scope::to_string).collect::<Vec<_>>(), "parts": parts }))
        }
        None => None,
    };
    Ok(json!({
        "name": m.name()?, "root": m.root()?.map(|r| r.to_hex()), "user": me, "role": m.user(me)?.map(|u| u.role),
        // What a device seals a scope's new epoch to, beside its devices' seal keys.
        "recovery": m.recovery()?.as_ref().map(hex),
        // Each scope's newest epoch the viewer may use, and its epochs' hashes, which a key from the bucket must match.
        "epochs": epochs,
        // Where the viewer's own things run, as their own device chose: the machines' keys, or null if it hasn't yet.
        "runs": m.runs(me)?.map(|on| on.iter().map(hex).collect::<Vec<_>>()),
        // When the network's recovery key last let a device in, and whose.
        "recovery_used": m.recovery_used()?.map(|(time, user)| json!({ "time": time, "user": user })),
        "recovery_set": m.recovery_set()?,
        "member": here.member,
        "users": users, "groups": groups, "devices": devices, "devices_at": m.devices_at()?, "clients": clients,
        // App schedules in the scopes this person may set them in: which endpoint runs which app's schedules in which
        // scope, until when, and who set them. Ones whose endpoint is gone, or whose setter may set them no more, are
        // over.
        "app_schedules": app_schedules,
        "steps": here.steps, "settings": settings,
        // The Cloudflare node, once an admin deployed one (`cloudflare node deploy` sets it).
        // Its parts, as a device's oned lists its own: it always runs apps, and an admin switches on each service built
        // for it that it carries (a built-in app's `service/cloudflare.js`).
        "cloudflare_node": cloudflare_node,
    }))
}

/// The network's settings: the custom build's defaults (config.toml's `[settings]`), and what its admins set over them
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

/// What to paste into an AI so it connects itself, for Connect Your AI: the guide at the network's address, where it
/// has one; else the one a device serves on its tailnet (`guide`), or, where it serves none, the command line's guide.
pub fn agent_prompt(address: Option<&str>, guide: Option<&str>) -> String {
    let (yours, cli) = (crate::config::YOURS, crate::config::CLI);
    match (address.filter(|a| !a.is_empty()), guide) {
        (Some(address), _) => format!("Read {}/llms.txt and connect yourself to {yours}.", address.trim_end_matches('/')),
        (None, Some(guide)) => format!("Read {guide} and connect yourself to {yours}."),
        (None, None) => format!("Run \"{cli} help\" in a terminal, do what it says, and connect yourself to {yours}."),
    }
}

/// What Settings shows `user`, in order, with a badge on Requests for `items_asked` and their other requests, and on
/// Backups when `backups` are failing. Passkeys opens `passkeys` in a browser, where they're kept.
fn settings_items(m: &ScopeDb, user: &str, items_asked: usize, backups: bool, store: &str, passkeys: Option<&str>) -> Result<Vec<Value>> {
    let Some(me) = m.user(user)? else { return Ok(Vec::new()) };
    let item = |name: &str, title: &str, icon: &str, color: &str| json!({ "name": name, "title": title, "icon": icon, "color": color });
    let with = |mut item: Value, key: &str, value: Value| {
        item[key] = value;
        item
    };
    let invites = m.groups()?.iter().filter(|g| g.invited.contains_key(user)).count();
    let mut out: Vec<Value> = passkeys.map(|url| with(item("passkeys", "Passkeys", "key", "#8e8e93"), "url", json!(url))).into_iter().collect();
    let requests = invites + items_asked;
    if requests > 0 {
        out.push(with(item("requests", "Requests", "tray", "#ff2d55"), "badge", json!(requests)));
    }
    if m.app(store)?.is_some_and(|a| !a.erased && a.deleted.is_none()) {
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
    if m.may(&me.id, "change-settings", NETWORK) {
        let backed = item("backups", "Backups", "hard-drives", "#ff9500");
        out.push(if backups { with(backed, "badge", json!(1)) } else { backed });
        out.push(item("labs", "Labs", "flask", "#af52de"));
        out.push(item("developer", "Developer", "code", "#8e8e93"));
    }
    Ok(out)
}


// What a node tells an app and a person of the network beyond `state`, the same from a node on a machine and from the
// Cloudflare node: each gives who's who (`global`'s scope database) and what it read of each scope (`ScopeState`).

/// Who a call, widget or hook runs for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Me {
    pub key: String,
    pub user: String,
    pub name: String,
    pub role: Option<Role>,
    /// The user's name.
    pub user_name: String,
    /// The user's handle (`scopes::valid_handle`).
    #[serde(default)]
    pub handle: String,
    /// The agent that asked (an MCP client's name, like "Claude Code"), when one did.
    pub agent: Option<String>,
}

/// `user` as an app is told of them, on the device `key` (hex) named `name`.
pub fn me(m: &ScopeDb, user: &str, key: String, name: String, agent: Option<String>) -> Result<Me> {
    let u = m.user(user)?;
    Ok(Me {
        key,
        user: user.to_string(),
        name,
        role: u.as_ref().map(|u| u.role),
        user_name: u.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
        handle: u.map(|u| u.handle).unwrap_or_default(),
        agent,
    })
}

/// A scope a call may reach, and the caller's role there.
#[derive(Clone, Debug, Serialize)]
pub struct ScopeHandle {
    pub id: String,
    pub name: String,
    pub role: Option<Role>,
}

pub fn handles(m: &ScopeDb, scopes: &[Scope], user: &str) -> Result<Vec<ScopeHandle>> {
    scopes.iter().map(|s| Ok(ScopeHandle { id: s.to_string(), name: whose(m, s)?, role: m.role_in(user, s)? })).collect()
}

/// A scope's name: the network's, a group's, or a person's, removed people's too.
pub fn whose(m: &ScopeDb, scope: &Scope) -> Result<String> {
    Ok(match scope {
        Scope::Global => m.name()?,
        Scope::Group(g) => m.group(g)?.map(|g| g.name).unwrap_or_default(),
        Scope::User(u) => m.user_ever(u)?.map(|u| u.name).unwrap_or_default(),
    })
}

/// The scopes a call of an app as `user` reaches: their own if it's added for them at all, each of their groups it's
/// added for (in the group or in `global`), and `global` if it's added there. `added` says whether the app is added in
/// a scope, or None for one the node answering doesn't run, and so can't read: such a group counts, so that the call
/// goes to a node that runs it, which answers for it, and what a call reaches never depends on which node answers. A
/// scope the node runs but hasn't read yet has nothing added.
pub fn reachable(m: &ScopeDb, user: &str, added: impl Fn(&Scope) -> Option<bool>) -> Result<Vec<Scope>> {
    let is = |s: &Scope| added(s) == Some(true);
    let global = is(&Scope::Global);
    let mut out = Vec::new();
    for g in m.groups_of(user)?.into_keys() {
        let s = Scope::Group(g);
        if global || added(&s) != Some(false) {
            out.push(s);
        }
    }
    let mine = Scope::User(user.into());
    if global || is(&mine) || out.iter().any(is) {
        out.insert(0, mine);
    }
    if global {
        out.push(Scope::Global);
    }
    Ok(out)
}

/// How long an app's or agent's request for an item waits for an answer.
pub const REQUEST_LIFE: u64 = 86_400_000;

/// How many requests for an item `user` may answer are waiting, of the scopes read (`states`), at `now`.
pub fn waiting<'a>(m: &ScopeDb, user: &str, states: impl IntoIterator<Item = (&'a Scope, &'a ScopeState)>, now: u64) -> usize {
    states
        .into_iter()
        .filter(|(s, _)| m.may(user, "add-apps", &object(s)))
        .map(|(_, st)| st.requests.values().filter(|r| r.answer.is_none() && now.saturating_sub(r.time) <= REQUEST_LIFE).count())
        .sum()
}

/// The scopes `viewer` may use that `app` is added in, of those read (`states`): theirs first, everyone's last.
pub fn added<'a>(m: &ScopeDb, app: &str, viewer: &str, states: impl IntoIterator<Item = (&'a Scope, &'a ScopeState)>) -> Vec<String> {
    let mut added: Vec<String> = states.into_iter().filter(|(s, st)| st.added.contains(app) && m.may(viewer, "use", &object(s))).map(|(s, _)| s.to_string()).collect();
    added.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
    added
}

/// What `GET /v1/apps` says of `a`, of every app in the network (`every`), to `viewer`, from who's who alone: owner,
/// `from`, forks, deploys, bundle, history, the scopes it's `added` in (`added`), its source if it was deployed from
/// one, and, deleted, when and by whom, added nowhere.
pub fn app_info(m: &ScopeDb, a: &App, every: &[App], viewer: &str, added: Vec<String>) -> Result<Value> {
    let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
    let forks: Vec<&String> = every.iter().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
    let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "added": added });
    if let Some(link) = a.source.as_ref().filter(|l| !l.is_empty()) {
        info["source"] = json!(link);
    }
    // A deleted app leaves every page; its owners see it in Recently Deleted.
    if let Some((time, by)) = &a.deleted {
        info["added"] = json!([]);
        info["deleted"] = json!({ "time": time, "by": m.user_ever(by)?.map(|u| u.name), "yours": m.may(viewer, "delete", &scopes::app(&a.name)) });
    }
    Ok(info)
}
