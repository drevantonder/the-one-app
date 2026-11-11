//! `GET /api/state`: the network as one user sees it, from `global`'s scope database, the same from an app runtime on a
//! machine and from oned on Cloudflare. What only the app runtime answering knows (which devices are online and how loaded,
//! who is looking) it says in `Here`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::scopes::{NETWORK, ScopeDb, object};
use ::key::{Key, hex};
use crate::scopes;
use crate::scopes::{App, Kind, Machine, Role, Scope, ScopeState};
use crate::rank::{self, Priority};

/// `GET /api/state`'s answer, as every reader (the screens, the web client, widgets, the CLI, oned on Cloudflare's
/// Worker) reads it: the field names are the wire's.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct State {
    pub config: Config,
    /// Who is looking.
    pub me: Viewer,
    /// The app runtime answering.
    pub device: Answering,
    /// The user looking, if the network lists them.
    pub user: Option<User>,
    /// None until a device joins one.
    pub network: Option<Network>,
    /// The pages this user can read, as every client lists them (see `pages`).
    pub pages: Vec<Page>,
    /// How this user arranged each of their pages (see `arrange`).
    pub arrangements: BTreeMap<String, Value>,
    /// What needs this user's attention, by feature, as each says it (`Runs.problem`): a badge on Settings (`Badge`).
    pub features: BTreeMap<String, Value>,
    /// What Settings shows this user, in order (see `settings_items`).
    pub settings_items: Vec<SettingsItem>,
    /// What to paste into an AI so it connects itself, for Connect Your AI.
    pub agent_prompt: String,
    /// The client looking, if it's a device and not the one answering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientHere>,
}

impl State {
    /// Where the network is reached from outside, once an admin set it.
    pub fn address(&self) -> Option<&str> {
        self.network.as_ref()?.settings.get("address").map(String::as_str)
    }

    /// The user looking, by name; with none listed, who the app runtime says is looking.
    pub fn looking(&self) -> &str {
        self.user.as_ref().map_or(self.me.name.as_str(), |u| u.name.as_str())
    }
}

/// The custom build the network runs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Config {
    pub name: String,
    pub cli: String,
    pub store: String,
    pub install: Option<String>,
}

/// Who is looking, and as which device or sign-in (`key` empty for a sign-in, which is no device).
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Viewer {
    pub key: String,
    pub name: String,
    pub user: String,
}

/// The client looking, when it's one of the user's devices.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientHere {
    pub key: String,
    pub name: String,
}

/// The app runtime answering.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Answering {
    pub key: String,
    pub name: String,
    pub ready: bool,
    /// Where an agent reads how to connect itself, when the network has no address.
    pub guide: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: Role,
}

/// One of a user's pages: theirs, a group's or everyone's.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Page {
    /// `user`, `group:<id>` or `global`.
    pub scope: String,
    pub title: String,
    /// Who the page is for and who chooses its apps; for only them, empty.
    pub subtitle: String,
    /// A group's page they switched off: whoever lists pages to show leaves it out.
    pub hidden: bool,
}

/// One entry of Settings' menu.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SettingsItem {
    pub name: String,
    pub title: String,
    /// A Phosphor icon's name.
    pub icon: String,
    pub color: String,
    /// Opened in a browser, where it's kept (Passkeys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<usize>,
    /// The app that opens (the store).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// The network as its scope database says it, and what the app runtime answering adds to it.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Network {
    pub name: String,
    pub root: Option<String>,
    /// The viewer's id and role.
    pub user: String,
    pub role: Option<Role>,
    /// What a device seals a scope's new epoch to, beside its devices' seal keys.
    pub recovery: Option<String>,
    /// Each scope's newest epoch the viewer may use, and its epochs' hashes, which a key from the bucket must match.
    pub epochs: BTreeMap<String, Epoch>,
    /// Where the viewer's own things run, as their own device chose: the machines' keys, or null if it hasn't yet.
    pub runs: Option<Vec<String>>,
    /// When the network's recovery key last let a device in, and whose.
    pub recovery_used: Option<RecoveryUsed>,
    pub recovery_set: u64,
    /// Whether the app runtime answering is one of the network's devices.
    pub member: bool,
    pub users: Vec<NetworkUser>,
    /// For an admin: who's invited by email and hasn't joined yet.
    #[serde(default)]
    pub email_invites: Vec<EmailInvited>,
    pub groups: Vec<Group>,
    pub devices: Vec<Device>,
    pub devices_at: u64,
    pub clients: Vec<Client>,
    /// App schedules in the scopes this user may set them in: which endpoint runs which app's schedules in which
    /// scope, until when, and who set them. Ones whose endpoint is gone, or whose setter may set them no more, are
    /// over.
    pub app_schedules: Vec<AppSchedule>,
    /// `global`'s steps the app runtime answering read.
    pub steps: u64,
    pub settings: BTreeMap<String, String>,
    /// oned on Cloudflare, once an admin deployed one (`cloudflare oned deploy` sets it).
    pub cloudflare_node: Option<CloudflareNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Epoch {
    pub n: u32,
    pub hashes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RecoveryUsed {
    pub time: u64,
    pub user: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NetworkUser {
    pub id: String,
    pub name: String,
    pub handle: String,
    pub role: Role,
    pub me: bool,
    /// Whether an admin named them as holding the recovery key.
    #[serde(default)]
    pub recovery_holder: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EmailInvited {
    pub user: String,
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub handle: String,
    /// The viewer's.
    pub role: Option<Role>,
    pub members: Vec<Member>,
    pub invited: Vec<Member>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Member {
    pub user: String,
    pub role: Role,
}

/// A device that runs calls.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Device {
    pub key: String,
    pub seal: String,
    pub name: String,
    /// Its user's id.
    pub user: String,
    pub roles: BTreeSet<Kind>,
    pub since: u64,
    /// The device this is the app runtime of, if it is: what's recorded of it (a grant, a priority) is that device's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// What it is, for its icon: `laptop`, `cloud` or `always_on`.
    pub kind: String,
    pub priority: Priority,
    /// Its place in run priority's order, 0 first, for services that pick one themselves; on the same number, by name.
    pub order: usize,
    pub serves_me: bool,
    pub runs_for: Vec<String>,
    /// Services its user switched off there.
    pub off_services: Vec<Switched>,
    /// Whether the viewer is looking from it.
    pub me: bool,
    /// What only the app runtime answering knows of it.
    #[serde(flatten)]
    pub now: DeviceNow,
}

/// What the app runtime answering knows of a device that the scope database doesn't: nothing, for oned on Cloudflare.
#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeviceNow {
    /// The app runtime answering.
    #[serde(default)]
    pub here: bool,
    /// Its socket is open at the relay.
    #[serde(default)]
    pub online: bool,
    /// When it was last seen (ms), if it ever was: the relay's record and what this app runtime itself exchanged with it.
    #[serde(default)]
    pub seen: Option<u64>,
    #[serde(default)]
    pub far: bool,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    /// The services serving there.
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub installed_services: Vec<InstalledService>,
    /// What it can run, for its user and the admins to see; None where it doesn't say.
    #[serde(default)]
    pub parts: Option<Vec<Part>>,
    #[serde(default)]
    pub load: Option<crate::state::Load>,
    /// What it is, as it last said, for its icon: `Device::kind`.
    #[serde(skip)]
    pub kind: Option<String>,
    /// The commit it should be on, as the record at the network's address says (`oned_update::latest`), if it's a build it could take.
    #[serde(default)]
    pub latest: Option<String>,
    /// Why it hasn't taken it, if it knows: its app runtime's words.
    #[serde(default)]
    pub updating: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Switched {
    pub name: String,
    pub time: u64,
}

/// What a device says of a service serving there: what it is, its version (a service's own), whom it serves, and who
/// installed it.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstalledService {
    pub name: String,
    pub title: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub domain: Option<String>,
    pub image: Option<String>,
    pub image_kind: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub built_in: bool,
    pub installed_by: Option<String>,
    pub installed: Option<u64>,
    pub users: Vec<String>,
}

/// A part of a device's oned: `apps`, the app service, or a service, with whether it's switched on and, while it isn't
/// simply on or off, what it's doing or what went wrong.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Part {
    pub name: String,
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `apps` on oned on Cloudflare, which an admin can't switch off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fixed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// How it runs, as its app declares: without its sandbox, saying why, or as the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub user: bool,
}

/// A device that doesn't run calls: a browser or an app signed in.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Client {
    pub key: String,
    pub seal: String,
    pub name: String,
    pub user: String,
    pub roles: BTreeSet<Kind>,
    pub passkey: Option<String>,
    /// What it said it's built for, once it has: "browser", "iphone" and the like.
    pub platform: Option<String>,
    /// The device it's on, when it's a native app's sign-in.
    pub device: Option<String>,
    pub since: u64,
    pub place: Option<String>,
    pub me: bool,
    /// What it signed in with, for its own user only: their Sign-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<crate::sign_in_proof::Account>,
    /// What else they sign in with, linked from this device, for its own user only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked: Vec<crate::sign_in_proof::Account>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AppSchedule {
    pub endpoint: String,
    pub scope: String,
    pub app: String,
    pub until: u64,
    pub by: String,
    /// The hash of the app's key in each epoch it sealed to the endpoint.
    pub keys: BTreeMap<u32, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CloudflareNode {
    pub priority: Priority,
    pub key: String,
    pub runs_for: Vec<String>,
}

/// What the app runtime answering knows that the scope database doesn't.
pub struct Here<'a> {
    /// Who is looking.
    pub me: Viewer,
    /// The client looking, if it's a device and not this one.
    pub client: Option<ClientHere>,
    /// The app runtime answering.
    pub device: Answering,
    /// The device the viewer is looking from, to say "This device": none from a sign-in.
    pub looking: Option<Key>,
    /// Whether that app runtime is one of the network's devices.
    pub member: bool,
    pub steps: u64,
    /// How this user arranged each of their pages.
    pub arrangements: BTreeMap<String, Value>,
    /// What needs this user's attention, by feature.
    pub features: BTreeMap<String, Value>,
    /// Where an agent reads how to connect itself, when the network has no address.
    pub guide: Option<String>,
    /// Whether this is a throwaway network, whose sign-in lets its user in with no passkey. Every other network's users
    /// manage passkeys at its address, a Google one's too: its admins sign in only with one.
    pub throwaway: bool,
    /// What it knows of each device now.
    pub device_now: &'a dyn Fn(&Machine) -> Result<DeviceNow>,
}

/// The network as `user` sees it, with `here` from the app runtime answering; None for `network` if it has none.
pub fn state(m: Option<&ScopeDb>, user: &str, here: Here) -> Result<State> {
    let settings = settings(&m.map(ScopeDb::settings).transpose()?.unwrap_or_default());
    let network = m.map(|m| network(m, user, &settings, &here)).transpose()?;
    let address = settings.get("address").filter(|a| !a.is_empty());
    let passkeys = address.filter(|_| !here.throwaway).map(|a| format!("{}/passkeys", a.trim_end_matches('/')));
    let store = settings["store"].clone();
    let settings_items = m.map(|m| settings_items(m, user, &here.features, &store, passkeys.as_deref())).transpose()?.unwrap_or_default();
    let me = m.map(|m| m.user(user)).transpose()?.flatten();
    Ok(State {
        config: Config { name: custom_build::config::NAME.into(), cli: custom_build::config::CLI.into(), store, install: install_line() },
        me: here.me,
        device: here.device,
        user: me.map(|u| User { id: u.id, name: u.name, role: u.role }),
        network,
        pages: m.map(|m| pages(m, user, &here.arrangements)).transpose()?.unwrap_or_default(),
        arrangements: here.arrangements,
        features: here.features,
        settings_items,
        agent_prompt: agent_prompt(address.map(String::as_str), here.guide.as_deref()),
        client: here.client,
    })
}

/// The pages `user` can read, as every client lists them, each `{ scope, title, subtitle, hidden }`: theirs, their
/// groups' in the order they chose (Edit Pages…: any they haven't ordered after, by name), everyone's last. `hidden` is
/// a group's page they switched off; whoever lists pages to show leaves it out. `subtitle` says who a page is for and
/// who chooses its apps: a few users by name, more as how many; for only them, nothing.
fn pages(m: &ScopeDb, user: &str, arrangements: &BTreeMap<String, Value>) -> Result<Vec<Page>> {
    let users = m.users()?;
    let name = |id: &str| if id == user { "You".to_string() } else { users.iter().find(|u| u.id == id && !u.removed).map_or("Someone".to_string(), |u| u.name.clone()) };
    // Users by name, you last, as "you".
    let named = |ids: &[&str]| {
        let mut ids = ids.to_vec();
        ids.sort_by_cached_key(|id| (*id == user, name(id).to_lowercase()));
        ids.into_iter().map(|id| if id == user { "you".to_string() } else { name(id) }).collect::<Vec<_>>()
    };
    let and = |names: Vec<String>| match names.as_slice() {
        [] | [_] => names.join(""),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let chosen_by = |admins: &[&str]| match named(admins).as_slice() {
        [] => "apps chosen by admins".to_string(),
        [one] => format!("apps chosen by {one}"),
        [a, b] => format!("apps chosen by {a} and {b}"),
        many => format!("apps chosen by {} admins", many.len()),
    };
    let whose = |ids: &[&str], admins: &[&str]| {
        if ids == [user] {
            return String::new();
        }
        let who = if ids.len() <= 3 { and(named(ids)) } else { format!("{} users", ids.len()) };
        let who = who.strip_prefix('y').map_or(who.clone(), |rest| format!("Y{rest}"));
        format!("{who} · {}", chosen_by(admins))
    };
    let kept = arrangements.get("user").map_or(&Value::Null, |u| &u["pages"]);
    let strings = |k: &str| kept[k].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>();
    let (order, hidden) = (strings("order"), strings("hidden"));
    let mut groups: Vec<_> = m.groups()?.into_iter().filter(|g| g.members.contains_key(user)).collect();
    groups.sort_by_cached_key(|g| g.name.to_lowercase());
    let rank = |scope: &str, by_name: usize| order.iter().position(|s| s == scope).unwrap_or(order.len() + by_name);
    let mut ranked: Vec<_> = groups.iter().enumerate().map(|(n, g)| (rank(&format!("group:{}", g.id), n), g)).collect();
    ranked.sort_by_key(|(r, _)| *r);
    let mut out = vec![Page { scope: "user".into(), title: "You".into(), subtitle: String::new(), hidden: false }];
    for (_, g) in ranked {
        let scope = format!("group:{}", g.id);
        let ids: Vec<&str> = g.members.keys().map(String::as_str).collect();
        let admins: Vec<&str> = g.members.iter().filter(|(_, r)| **r == scopes::Role::Admin).map(|(u, _)| u.as_str()).collect();
        out.push(Page { hidden: hidden.contains(&scope), scope, title: g.name.clone(), subtitle: whose(&ids, &admins) });
    }
    let everyone: Vec<&str> = users.iter().filter(|u| !u.removed).map(|u| u.id.as_str()).collect();
    let admins: Vec<&str> = users.iter().filter(|u| !u.removed && u.role == scopes::Role::Admin).map(|u| u.id.as_str()).collect();
    out.push(Page { scope: "global".into(), title: "Everyone".into(), subtitle: whose(&everyone, &admins), hidden: false });
    Ok(out)
}

fn network(m: &ScopeDb, me: &str, settings: &BTreeMap<String, String>, here: &Here) -> Result<Network> {
    let set = rank::set(&m.settings()?);
    let switches = m.switches()?;
    // Only devices: a service is part of one, and its device says it runs it.
    let mut devices: Vec<Device> = Vec::new();
    for s in m.machines()?.iter().filter(|s| !s.removed && s.is(Kind::Compute)) {
        let mut now = (here.device_now)(s)?;
        // A device that hasn't said what it is, as one that's always on.
        let kind = now.kind.take().filter(|k| !k.is_empty()).unwrap_or_else(|| "always_on".into());
        devices.push(Device {
            key: hex(&s.key),
            seal: hex(&s.seal),
            name: s.name.clone(),
            user: s.user.clone(),
            roles: s.roles.clone(),
            since: s.since,
            device: s.runtime_of().as_ref().map(hex),
            kind,
            priority: rank::priority(&set, &hex(&m.device_of(&s.key))),
            order: 0,
            serves_me: m.serves(&s.key, me)?,
            runs_for: m.run_by(&s.key)?.iter().map(|scope| scope.to_string()).collect(),
            off_services: switches.iter().filter(|((k, _), d)| *k == m.device_of(&s.key) && !d.on).map(|((_, name), d)| Switched { name: name.clone(), time: d.time }).collect(),
            me: Some(s.key) == here.looking,
            now,
        });
    }
    // Each device's place in run priority's order, 0 first, for services that pick one themselves; on the same number,
    // by name.
    let mut by_order: Vec<usize> = (0..devices.len()).collect();
    by_order.sort_by_cached_key(|&n| (devices[n].priority, devices[n].name.to_lowercase()));
    for (place, n) in by_order.into_iter().enumerate() {
        devices[n].order = place;
    }
    // Devices that also run calls are listed with the devices.
    let clients: Vec<Client> = m
        .clients()?
        .iter()
        .filter(|c| !c.is(Kind::Compute))
        .map(|c| Client {
            key: hex(&c.key),
            seal: hex(&c.seal),
            name: c.name.clone(),
            user: c.user.clone(),
            roles: c.roles.clone(),
            passkey: c.passkey.clone(),
            platform: c.platform.clone(),
            device: c.device.as_ref().map(hex),
            since: c.since,
            place: c.place.clone(),
            me: Some(c.key) == here.looking,
            sign_in: c.sign_in.clone().filter(|_| c.user == me),
            linked: if c.user == me { m.linked_sign_ins(&c.key).unwrap_or_default() } else { Vec::new() },
        })
        .collect();
    let holders = m.recovery_holders()?;
    let users = m.users()?.iter().filter(|u| !u.removed).map(|u| NetworkUser { id: u.id.clone(), name: u.name.clone(), handle: u.handle.clone(), role: u.role, me: u.id == me, recovery_holder: holders.contains(&u.id) }).collect();
    let mut groups = Vec::new();
    for g in m.groups()? {
        let members = |of: &BTreeMap<String, Role>| of.iter().map(|(user, role)| Member { user: user.clone(), role: *role }).collect();
        groups.push(Group { id: g.id.clone(), name: g.name.clone(), handle: g.handle.clone(), role: m.group_role(me, &g.id)?, members: members(&g.members), invited: members(&g.invited) });
    }
    let mut epochs = BTreeMap::new();
    for (s, n) in m.epochs()?.into_iter().filter(|(s, _)| m.may(me, "use", &object(s))) {
        let hashes = m.epoch_hashes(&s)?.into_iter().map(|(n, h)| (n.to_string(), h)).collect();
        epochs.insert(s.to_string(), Epoch { n, hashes });
    }
    let mut app_schedules = Vec::new();
    for ((k, s, a), g) in m.app_schedules()? {
        if m.may(me, "choose-devices", &object(&s)) && m.machine(&k)?.is_some() && m.may(&g.by, "choose-devices", &object(&s)) {
            app_schedules.push(AppSchedule { endpoint: hex(&k), scope: s.to_string(), app: a, until: g.until, by: g.by, keys: g.hashes });
        }
    }
    let cloudflare_node = match m.cloudflare_node()? {
        Some(key) => Some(CloudflareNode { priority: rank::priority(&set, rank::CLOUDFLARE), key: hex(&key), runs_for: m.run_by(&key)?.iter().map(Scope::to_string).collect() }),
        None => None,
    };
    Ok(Network {
        name: m.name()?,
        root: m.root()?.map(|r| r.to_hex()),
        user: me.to_string(),
        role: m.user(me)?.map(|u| u.role),
        recovery: m.recovery()?.as_ref().map(hex),
        epochs,
        runs: m.runs(me)?.map(|on| on.iter().map(hex).collect()),
        recovery_used: m.recovery_used()?.map(|(time, user)| RecoveryUsed { time, user }),
        recovery_set: m.recovery_set()?,
        member: here.member,
        users,
        email_invites: if m.may(me, "invite", scopes::NETWORK) { m.email_invites()?.into_iter().filter(|i| !i.used && i.until > m.now()).map(|i| EmailInvited { user: i.user, name: i.name, email: i.email }).collect() } else { Vec::new() },
        groups,
        devices,
        devices_at: m.devices_at()?,
        clients,
        app_schedules,
        steps: here.steps,
        settings: settings.clone(),
        cloudflare_node,
    })
}

impl Part {
    /// A part with only its name, switched off.
    pub fn off(name: &str) -> Part {
        Part { name: name.into(), on: false, title: None, fixed: false, doing: None, problem: None, sandbox: None, user: false }
    }
}

/// The network's settings: the custom build's defaults (config.toml's `[settings]`), and what its admins set over them
/// (`ScopeDb::settings`).
pub fn settings(set: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = custom_build::config::SETTINGS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    out.extend(set.clone());
    out.entry("store".into()).or_default();
    out
}

/// What install.sh sets as `name` (`repo`, `cli`, `key`): it is shell, which can't read config.toml, so it says it again.
pub fn installer_says(name: &str) -> Option<&'static str> {
    include_str!("../../install.sh").lines().find_map(|l| l.strip_prefix(&format!("{name}="))).map(|v| v.trim_matches('"'))
}

/// Whether install.sh, which gets releases from GitHub, installs this custom build. Only that custom build is released there.
pub fn installer_is_ours() -> bool {
    installer_says("cli") == Some(custom_build::config::CLI) && installer_says("repo") == Some(custom_build::config::REPO)
}

/// The one line that installs this custom build's latest release on a machine, if it publishes releases.
pub fn install_line() -> Option<String> {
    installer_is_ours().then(|| format!("curl -fsSL {}/releases/latest/download/install.sh | sh", custom_build::config::REPO))
}

/// What to paste into an AI so it connects itself, for Connect Your AI: the guide at the network's address, where it
/// has one; else the one a device serves on its tailnet (`guide`), or, where it serves none, the command line's guide.
pub fn agent_prompt(address: Option<&str>, guide: Option<&str>) -> String {
    // Said by the user to their AI, so "my", where the screens' words are "your".
    let (yours, cli) = (custom_build::config::YOURS.replacen("your ", "my ", 1), custom_build::config::CLI);
    match (address.filter(|a| !a.is_empty()), guide) {
        (Some(address), _) => {
            let address = address.trim_end_matches('/');
            format!("Read {address}/llms.txt and connect yourself to {yours}. If you can't open that link, show me, one step at a time, how to add {address}/mcp as a connector here.")
        }
        (None, Some(guide)) => format!("Read {guide} and connect yourself to {yours}."),
        (None, None) => format!("Run \"{cli} help\" in a terminal, do what it says, and connect yourself to {yours}."),
    }
}

/// What a feature says needs a user's attention (`state.features`) badges a Settings item: the one named `badge` in it,
/// else the feature's own, by `count` (1 if it says none).
#[derive(Debug, Default, Deserialize)]
pub struct Badge {
    pub badge: Option<String>,
    pub count: Option<usize>,
}

/// What Settings shows `user`, in order, each badged by what `features` say needs their attention, Requests also by the
/// groups they're invited to. Passkeys opens `passkeys` in a browser, where they're kept.
fn settings_items(m: &ScopeDb, user: &str, features: &BTreeMap<String, Value>, store: &str, passkeys: Option<&str>) -> Result<Vec<SettingsItem>> {
    let Some(me) = m.user(user)? else { return Ok(Vec::new()) };
    let mut badges: BTreeMap<String, usize> = BTreeMap::new();
    for (feature, said) in features {
        let b: Badge = serde_json::from_value(said.clone()).unwrap_or_default();
        *badges.entry(b.badge.unwrap_or_else(|| feature.clone())).or_default() += b.count.unwrap_or(1);
    }
    let item = |name: &str, title: &str, icon: &str, color: &str| SettingsItem { name: name.into(), title: title.into(), icon: icon.into(), color: color.into(), url: None, badge: badges.get(name).copied().filter(|n| *n > 0), app: None };
    let invites = m.groups()?.iter().filter(|g| g.invited.contains_key(user)).count();
    let mut out: Vec<SettingsItem> = passkeys.map(|url| SettingsItem { url: Some(url.into()), ..item("passkeys", "Passkeys", "key", "#8e8e93") }).into_iter().collect();
    let requests = invites + badges.get("requests").copied().unwrap_or(0);
    if requests > 0 {
        out.push(SettingsItem { badge: Some(requests), ..item("requests", "Requests", "tray", "#ff2d55") });
    }
    if m.app(store)?.is_some_and(|a| !a.erased && a.deleted.is_none()) {
        out.push(SettingsItem { app: Some(store.into()), ..item("store", "App Store", "squares-four", "#007aff") });
    }
    out.extend([
        item("connect-ai", "Connect Your AI", "plugs-connected", "#ff2d55"),
        item("ai-gateway", "AI Gateway", "sparkle", "#af52de"),
        item("vault", "Vault", "lock", "#5856d6"),
        item("users", "Users", "user-gear", "#30b0c7"),
        item("groups", "Groups", "users-three", "#34c759"),
        item("devices", "Devices", "devices", "#8e8e93"),
        item("insights", "Insights", "chart-line", "#5ac8fa"),
        item("appearance", "Appearance", "circle-half", "#007aff"),
    ]);
    if m.may(&me.id, "change-settings", NETWORK) {
        out.push(item("backups", "Backups", "hard-drives", "#ff9500"));
        out.push(item("labs", "Labs", "flask", "#af52de"));
    }
    Ok(out)
}


// What an app runtime tells an app and a user of the network beyond `state`, the same from an app runtime on a computer and from the
// oned on Cloudflare: each gives who's who (`global`'s scope database) and what it read of each scope (`ScopeState`).

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

/// A scope's name: the network's, a group's, or a user's, removed users' too.
pub fn whose(m: &ScopeDb, scope: &Scope) -> Result<String> {
    Ok(match scope {
        Scope::Global => m.name()?,
        Scope::Group(g) => m.group(g)?.map(|g| g.name).unwrap_or_default(),
        Scope::User(u) => m.user_ever(u)?.map(|u| u.name).unwrap_or_default(),
    })
}

/// The scopes a call of an app as `user` reaches: their own if it's added for them at all, each of their groups it's
/// added for (in the group or in `global`), and `global` if it's added there. `added` says whether the app is added in
/// a scope, or None for one the app runtime answering doesn't run, and so can't read: such a group counts, so that the call
/// goes to an app runtime that runs it, which answers for it, and what a call reaches never depends on which app runtime answers. A
/// scope the app runtime runs but hasn't read yet has nothing added.
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

/// What `GET /api/apps` says of `a`, of every app in the network (`every`), to `viewer`, from who's who alone: owner,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Load {
    /// Used memory: what the OS can't hand to a new program without swapping. Linux's total minus available;
    /// Activity Monitor's "Memory Used" on a Mac (app memory, wired and compressed, not cached files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Space>,
    /// The volume user files live on; total is used plus what's free to a user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<Space>,
    /// Percent busy, oldest first, one sample every 5 seconds.
    pub cpu: Vec<u8>,
    /// The busiest GPU's percent busy, like `cpu`; empty where none reports, or in a container (it sees its host's).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gpu: Vec<u8>,
    /// Whether this machine is itself a container.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub container: bool,
    /// The containers running here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<Guest>,
    /// What this machine is, for its icon: `laptop`, `cloud` or `always_on`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

/// Bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Space {
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Guest {
    pub name: String,
    /// `system` (an Incus or LXD container) or `podman`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Space>,
}
