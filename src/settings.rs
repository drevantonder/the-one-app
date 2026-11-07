//! What a device changes of who can do what, signed by itself as its person's device: Settings' changes (roles, groups,
//! handles, names, settings, removing a device or a person, the new epochs that go with it), letting devices in or
//! turning them down, inviting someone new, and app schedules. Every device runs this one copy (`Me`): each client
//! through its keyring (client/src/settings.rs), and a machine with a node as its own device, which its CLI and MCP
//! use (src/node/settings.rs). Each is asked as a node's route of the same path (`ROUTES`).
//!
//! What it signs, it signs with what each device has (`keys::Signs`): a native app's key, a browser's WebCrypto key,
//! which can't be read, or a node's own. Who may do what is judged by the steps' own rules wherever they're read
//! (src/database/rules.rs): what's checked here only says why sooner.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::YOURS;
use crate::database::content::{self, Content};
use crate::database::scopes::{self, Kind, Role, Scope, ScopeDb};
use crate::database::steps::Welcome;
use crate::keys::{self, Endpoint, Key, Signs, hex, unhex};
use crate::protocol::{Coded, code};

/// How long app schedules last, unless their person renews them as they use the app.
pub const APP_SCHEDULES_DAYS: u64 = 30;
const DAY: u64 = 86_400_000;
/// The invites this device made, by who they're for: their secret, name, role and until when.
const INVITES: &str = "invites.json";

/// A device signing who can do what as its person's.
pub trait Me: Signs {
    /// What `f` finds in who's who as this device last read it.
    fn who<R>(&self, f: impl FnOnce(&ScopeDb) -> Result<R>) -> Result<R>;
    /// Its person: `code::NOT_LET_IN` before who's who lists it.
    fn user(&self) -> Result<String>;
    /// Its seal key's public half, which what's sealed to it is sealed to.
    fn seal(&self) -> [u8; 32];
    /// Where it read who's who to, which a device it lets in starts from.
    fn welcome(&self) -> Option<Welcome>;
    /// Reads what's new of who's who.
    fn read(&self) -> impl Future<Output = Result<()>>;
    /// Signs `content` as the next step of `global`'s scope database, and writes it.
    fn write(&self, content: Content) -> impl Future<Output = Result<()>>;
    /// Each epoch of `scope` sealed to it, opened, each the one its entry names.
    fn held(&self, scope: &Scope) -> impl Future<Output = Result<Vec<(u32, [u8; 32])>>>;
    /// Keeps a key it sealed, as `POST /v1/keys` takes it: one there already fails with `code::ALREADY`.
    fn keep_key(&self, sealed: Value) -> impl Future<Output = Result<()>>;
    /// `app`'s key in epoch `n` of `scope`.
    fn app_key(&self, scope: &Scope, app: &str, n: u32) -> impl Future<Output = Result<[u8; 32]>>;
    /// An epoch it just made, whose hash who's who names.
    fn made_epoch(&self, scope: &Scope, n: u32, key: [u8; 32]);
    /// The network's address, `https://…`: none for a network that has none yet, which has no auth or relay.
    fn address(&self) -> Option<String>;
    /// A request to `url`, JSON in and out, with `header` if there's one: → its status and JSON (null if it wasn't).
    fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> impl Future<Output = Result<(u16, Value)>>;
    /// An OIDC issuer's published keys, at `url`.
    fn issuer_keys(&self, url: &str) -> impl Future<Output = Option<Value>>;
    /// What it keeps of its own by `name` (`INVITES`).
    fn kept(&self, name: &str) -> impl Future<Output = Option<Vec<u8>>>;
    fn keep(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<()>>;
    /// Now, in ms since 1970, on the clock of its runtime.
    fn now(&self) -> u64;
    /// Waits `ms`, on its runtime.
    fn wait(&self, ms: u64) -> impl Future<Output = ()>;
    /// Takes itself out of its person's devices, as it signs out.
    fn leave(&self) -> impl Future<Output = Result<()>>;
}

/// What a device answers itself of a node's routes, signing what they change.
#[derive(Clone, Copy)]
enum Route {
    Role,
    Remove,
    Settings,
    Group,
    Invite,
    Accept,
    Member,
    Handle,
    Name,
    Runs,
    Service,
    Retire,
    ForgetSignIn,
    InviteLink,
    AppSchedules,
    Requests,
    Approve,
    Refuse,
    Passkey,
    Seen,
    VaultGrant,
}

/// The only list of them: a route of a path and method, which `answer` runs (its `match` fails to compile for one
/// missing here or there), and `answered` tells a request that comes by of; and whether a machine with a node answers
/// it with this alone (`on_a_node`), false where the node serves that path itself: its own name, and services, which it serves for
/// more than its own programs.
const ROUTES: &[(&str, &str, Route, bool)] = &[
    ("POST", "/v1/role", Route::Role, true),
    ("POST", "/v1/remove", Route::Remove, true),
    ("POST", "/v1/settings", Route::Settings, true),
    ("POST", "/v1/group", Route::Group, true),
    ("POST", "/v1/invite", Route::Invite, true),
    ("POST", "/v1/accept", Route::Accept, true),
    ("POST", "/v1/member", Route::Member, true),
    ("POST", "/v1/handle", Route::Handle, true),
    ("POST", "/v1/name", Route::Name, false),
    ("POST", "/v1/runs", Route::Runs, true),
    ("POST", "/v1/service", Route::Service, false),
    ("POST", "/v1/retire", Route::Retire, true),
    ("POST", "/v1/devices/forget-sign-in", Route::ForgetSignIn, true),
    ("POST", "/v1/invite-link", Route::InviteLink, true),
    ("POST", "/v1/app-schedules", Route::AppSchedules, true),
    ("GET", "/v1/devices/requests", Route::Requests, true),
    ("POST", "/v1/devices/approve", Route::Approve, true),
    ("POST", "/v1/devices/refuse", Route::Refuse, true),
    ("POST", "/v1/devices/passkey", Route::Passkey, true),
    ("GET", "/v1/devices/seen", Route::Seen, true),
    // A machine with a node that opens the Vault seals what's in it to the new key at once.
    ("POST", "/v1/vault/grant", Route::VaultGrant, false),
];

/// Whether a device answers the node's route of this path itself, whatever the method.
pub fn answered(path: &str) -> bool {
    ROUTES.iter().any(|(_, p, _, _)| *p == path)
}

/// The routes a machine with a node answers as its own device, by method and path.
pub fn on_a_node() -> impl Iterator<Item = (&'static str, &'static str)> {
    ROUTES.iter().filter(|r| r.3).map(|(m, p, _, _)| (*m, *p))
}

/// `body[k]` as text, "" if it isn't.
fn text(body: &Value, k: &str) -> String {
    body[k].as_str().unwrap_or("").to_string()
}

/// A request to join, as auth lists it.
pub struct Asked {
    said: Value,
    endpoint: Endpoint,
    user: String,
    name: String,
    /// "ABCD EFGH", as the device asking shows it.
    code: String,
}

impl Asked {
    /// A request as auth lists it: `{ key, seal, user, name, person?, proof?, device?, made?, place?, sign_in? }`.
    pub fn of(r: &Value) -> Option<Asked> {
        let endpoint = Endpoint { key: keys::unhex(r["key"].as_str()?).ok()?, seal: keys::unhex(r["seal"].as_str()?).ok()? };
        let code = keys::join_code(&endpoint);
        Some(Asked { said: r.clone(), endpoint, user: text(r, "user"), name: text(r, "name"), code: format!("{} {}", &code[..4], &code[4..]) })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Its person, by id.
    pub fn user(&self) -> &str {
        &self.user
    }

    /// The device's name, as it asked.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it's someone new's, who brings their invite's proof.
    pub fn new(&self) -> bool {
        self.said["proof"].is_string()
    }
}

/// An invite a device made: the newcomer's first device brings the proof its secret makes with its key.
#[derive(Clone, Serialize, Deserialize)]
pub struct Invited {
    secret: String,
    name: String,
    role: Role,
    until: u64,
}

/// The answer to a request a device answers itself, as a node's route of the same path would (`/v1/role`, `/v1/remove`,
/// `/v1/devices/approve`, …): none for any other, which goes to a node. Its body is the route's own (`body`), every
/// field known and of its kind, or it's refused.
pub async fn answer(m: &impl Me, method: &str, path: &str, body: &[u8]) -> Option<Result<Value>> {
    let path = path.split('?').next().unwrap_or(path);
    let (_, _, route, _) = ROUTES.iter().find(|(m, p, _, _)| *m == method && *p == path)?;
    // What a request names (a group, a device) may be newer than this device last read: it reads what's new first.
    if let Err(err) = m.read().await {
        return Some(Err(err));
    }
    Some(answered_by(m, *route, path, body).await)
}

/// A route's body, as `T`: none is `{}`.
fn read<T: serde::de::DeserializeOwned>(path: &str, body: &[u8]) -> Result<T> {
    let body = if body.is_empty() { b"{}".as_slice() } else { body };
    serde_json::from_slice(body).map_err(|err| {
        let said = err.to_string();
        anyhow!("{path} didn't take that: {}.", said.split(" at line ").next().unwrap_or(&said))
    })
}

/// Each route's body, as its callers send it.
mod body {
    use serde::Deserialize;

    use crate::database::scopes;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Role {
        pub user: String,
        pub role: scopes::Role,
    }

    /// A device by its key or name, or a person by their id or name, with all their devices.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Remove {
        pub key: Option<String>,
        pub user: Option<String>,
    }

    /// `value` none sets it back to what it is by default.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Setting {
        pub name: String,
        pub value: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Named {
        pub name: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Invite {
        pub group: String,
        pub user: String,
        pub role: scopes::Role,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Group {
        pub group: String,
    }

    /// `role` none takes them out of the group.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Member {
        pub group: String,
        pub user: String,
        pub role: Option<scopes::Role>,
    }

    /// A group's handle, or with no `group`, this person's.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Handle {
        pub handle: String,
        pub group: Option<String>,
    }

    /// Another device's name, or with no `key`, this one's.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Name {
        pub key: Option<String>,
        pub name: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Runs {
        pub on: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Service {
        pub device: String,
        pub name: String,
        pub on: bool,
    }

    /// `how`: deleted, restored or erased.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Retire {
        pub app: String,
        pub how: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Key {
        pub key: String,
    }

    /// `user` invites someone in the network already; `email` is the Google account a newcomer signs up with.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct InviteLink {
        pub name: String,
        #[serde(default)]
        pub admin: bool,
        pub user: Option<String>,
        pub email: Option<String>,
    }

    /// `device` by its name or key; `scope` user (theirs, the default), global, or a group by its name or id; `days`,
    /// 30 at most, 0 to stop.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct AppSchedules {
        pub app: String,
        pub device: String,
        pub scope: Option<String>,
        pub days: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Code {
        pub code: String,
    }

    /// Without `confirm`, whose the device showing `code` is.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Approve {
        pub code: String,
        #[serde(default)]
        pub confirm: bool,
    }

    /// The passkey's PRF output (base64url), which only the page it signed in on has, and its credential's id.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Passkey {
        pub prf: Option<String>,
        pub credential: String,
    }

    /// `device` added to the machines that open `scope`'s Vault (user, the default; global; or a group by its name or
    /// id), or with `on: false`, taken off them.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct VaultGrant {
        pub scope: Option<String>,
        pub device: String,
        #[serde(default = "yes")]
        pub on: bool,
    }

    fn yes() -> bool {
        true
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Nothing {}
}

async fn answered_by(m: &impl Me, route: Route, path: &str, raw: &[u8]) -> Result<Value> {
    let done = json!({});
    Ok(match route {
        Route::Role => {
            let body: body::Role = read(path, raw)?;
            m.write(content::role(&m.who(|db| db.find_user(&body.user))?, body.role)).await?;
            done
        }
        Route::Remove => match read(path, raw)? {
            body::Remove { key: Some(key), user: None } => remove(m, &key).await?,
            body::Remove { key: None, user: Some(user) } => remove_person(m, &user).await?,
            _ => bail!("Give a device's key or a person, not both."),
        },
        Route::Settings => {
            let body: body::Setting = read(path, raw)?;
            m.write(content::setting(&body.name, body.value.as_deref())).await?;
            done
        }
        Route::Group => group(m, &read::<body::Named>(path, raw)?.name).await?,
        Route::Invite => {
            let body: body::Invite = read(path, raw)?;
            let (group, user) = m.who(|db| Ok((db.find_group(&body.group)?, db.find_user(&body.user)?)))?;
            m.write(content::invite(&group, &user, body.role)).await?;
            done
        }
        Route::Accept => {
            let body: body::Group = read(path, raw)?;
            m.write(content::accept(&m.who(|db| db.find_group(&body.group))?)).await?;
            done
        }
        Route::Member => {
            let body: body::Member = read(path, raw)?;
            let (group, user) = m.who(|db| Ok((db.find_group(&body.group)?, db.find_user(&body.user)?)))?;
            m.write(content::member(&group, &user, body.role)).await?;
            done
        }
        Route::Handle => {
            let body: body::Handle = read(path, raw)?;
            let (what, id) = match &body.group {
                Some(g) => ("group", m.who(|db| db.find_group(g))?),
                None => ("user", m.user()?),
            };
            m.write(content::handle(what, &id, body.handle.trim())).await?;
            done
        }
        Route::Name => {
            let body: body::Name = read(path, raw)?;
            let key = match &body.key {
                Some(k) => m.who(|db| db.find_machine(k))?,
                None => m.public(),
            };
            m.write(content::rename("key", &hex(&key), body.name.trim())).await?;
            done
        }
        Route::Runs => {
            let body: body::Runs = read(path, raw)?;
            let machines = body.on.iter().map(|d| m.who(|db| db.find_machine(d))).collect::<Result<Vec<_>>>()?;
            m.write(content::runs(&machines)).await?;
            done
        }
        Route::Service => {
            let body: body::Service = read(path, raw)?;
            let user = m.user()?;
            m.write(m.who(|db| service(db, &user, &body.device, &body.name, body.on, None))?).await?;
            done
        }
        Route::Retire => {
            let body: body::Retire = read(path, raw)?;
            let user = m.user()?;
            m.write(m.who(|db| retire(db, &user, &body.app, &body.how, &body.app))?).await?;
            done
        }
        Route::ForgetSignIn => {
            let body: body::Key = read(path, raw)?;
            let user = m.user()?;
            let key = m.who(|db| {
                let key = db.find_machine(&body.key)?;
                ensure!(db.machine(&key)?.is_some_and(|d| d.user == user && d.sign_in.is_some()), "That isn't one of your devices with a sign-in to forget.");
                Ok(key)
            })?;
            m.write(content::forget_sign_in(&key)).await?;
            done
        }
        Route::InviteLink => {
            let body: body::InviteLink = read(path, raw)?;
            invite(m, &body.name, if body.admin { Role::Admin } else { Role::Member }, body.user.as_deref(), body.email.as_deref()).await?
        }
        Route::AppSchedules => app_schedules_asked(m, read(path, raw)?).await?,
        Route::Requests => {
            read::<body::Nothing>(path, raw)?;
            requests(m).await?
        }
        Route::Approve => {
            let body: body::Approve = read(path, raw)?;
            approve_code(m, &body.code, body.confirm).await?
        }
        Route::Refuse => refuse(m, &read::<body::Code>(path, raw)?.code).await?,
        Route::Passkey => {
            let body: body::Passkey = read(path, raw)?;
            let prf = body.prf.and_then(|p| B64U.decode(p.trim_end_matches('=').as_bytes()).ok()).filter(|p| !p.is_empty()).ok_or_else(|| anyhow!("Sign in with your passkey first."))?;
            list_passkey(m, &prf, &body.credential).await?;
            done
        }
        Route::Seen => {
            read::<body::Nothing>(path, raw)?;
            seen(m).await?
        }
        Route::VaultGrant => vault_grant(m, read(path, raw)?).await?,
    })
}

/// Says which machines open a Vault: `device` added to them or taken off, by its person or its admins. Always a new
/// key pair (`new_vault_key`); what's sealed to the old one is sealed to the new by a machine that opens both, at its
/// next write there. → `{ said }`.
async fn vault_grant(m: &impl Me, body: body::VaultGrant) -> Result<Value> {
    let user = m.user()?;
    let given = body.scope.as_deref().unwrap_or("user");
    let (scope, endpoint, name, current) = m.who(|who| {
        let scope = Scope::parse_for(given, &user).or_else(|_| who.find_group(given).map(Scope::Group))?;
        ensure!(who.may(&user, "choose-devices", &scopes::object(&scope)), "Only this Vault's person, or its admins, say which machines open it.");
        let endpoint = who.find_node(&body.device)?;
        let current = who.vault(&scope)?.unwrap_or_default();
        Ok((scope, endpoint, who.machine(&endpoint)?.map(|d| d.name).unwrap_or_default(), current))
    })?;
    let mut to: Vec<[u8; 32]> = current.to.into_iter().filter(|k| *k != endpoint).collect();
    if body.on {
        to.push(endpoint);
    }
    new_vault_key(m, &scope, current.n + 1, &to).await?;
    Ok(json!({ "said": match body.on {
        true => format!("{name} opens this Vault now, and fills its items into requests to their sites."),
        false => format!("{name} no longer opens this Vault: what's set from now on is sealed to a key it doesn't have."),
    } }))
}

/// Makes `scope`'s Vault key `n`, for the machines `to`: its `vault` step first, then its secret half sealed to each of
/// them and to the recovery key.
pub async fn new_vault_key(m: &impl Me, scope: &Scope, n: u32, to: &[[u8; 32]]) -> Result<()> {
    let secret: [u8; 32] = rand::random();
    let public = keys::public_of(&secret);
    m.write(content::vault(scope, n, &public, to)).await?;
    m.read().await?;
    let (recovery, seals) = m.who(|who| {
        ensure!(who.vault_publics(scope)?.get(&n) == Some(&public), "Another device made new keys for this Vault just now. Try again.");
        let mut seals = Vec::new();
        for k in to {
            seals.extend(who.seal_key(k)?.map(|s| (hex(k), s)));
        }
        Ok((who.recovery()?.ok_or_else(|| anyhow!("{YOURS} has no recovery key to seal its Vault to."))?, seals))
    })?;
    for (at, seal) in std::iter::once(("recovery".to_string(), keys::twin_public(&recovery)?)).chain(seals) {
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": at, "vault": true, "sealed": B64U.encode(&keys::seal_to(&seal, &secret)?) })).await?;
    }
    Ok(())
}

/// The step that deletes `app` (`deleted`), restores it (`restored`) or erases it for good (`erased`), as `user`, who
/// acts for its owner, says it: whoever writes it, from any device or an app's call. `title` is what people call it.
/// A deleted app stops running and leaves every page, but keeps its data until it's erased, which deletes its keys in
/// the bucket as each node reads the step (`Node::follow_erased`).
pub fn retire(who: &ScopeDb, user: &str, app: &str, how: &str, title: &str) -> Result<Content> {
    let a = who.app(app)?.ok_or_else(|| anyhow!("no app named {app}"))?;
    ensure!(who.may(user, "delete", &scopes::app(app)), "Only {title}'s owners can delete, restore or erase it.");
    ensure!(how != "erased" || a.deleted.is_some(), "Delete {title} before erasing it.");
    content::retire(app, &a.owner.to_string(), how)
}

/// The `service` entry that switches a part of a device's oned on or off, the device found by name or key prefix; for
/// the device's person, or to switch it off, for who may add apps where it ran. A service runs for the app added in
/// `scope`, which its writer may add apps to.
pub fn service(db: &ScopeDb, user: &str, device: &str, name: &str, on: bool, scope: Option<&Scope>) -> Result<Content> {
    if let Some(s) = scope.filter(|_| on) {
        ensure!(db.may(user, "add-apps", &scopes::object(s)), "only who may add apps there runs a service for them");
    }
    let managed = |k: &Key| -> Result<bool> { Ok(!on && db.switch(k, name)?.and_then(|s| s.scope).and_then(|s| Scope::parse(&s).ok()).is_some_and(|s| db.may(user, "add-apps", &scopes::object(&s)))) };
    // The Cloudflare node, by its name in run priority or its key: its admins switch its parts.
    if let Some(k) = db.cloudflare_node()?.filter(|k| device.eq_ignore_ascii_case(crate::rank::CLOUDFLARE) || (!device.is_empty() && hex(k).starts_with(device))) {
        ensure!(managed(&k)? || db.may(user, "change-settings", scopes::NETWORK), "only an admin switches what the Cloudflare node runs");
        // It always runs apps; what else it can run is the services built for it that it carries.
        ensure!(!on || crate::config::CLOUDFLARE_SERVICES.iter().any(|(n, _)| *n == name), "the Cloudflare node can't run {name}: it runs apps, and only the services built for Cloudflare");
        return Ok(content::service(&k, name, on, scope));
    }
    let mut found: Vec<scopes::Machine> = db.machines()?.into_iter().filter(|s| !s.removed).filter(|s| s.name.eq_ignore_ascii_case(device) || (!device.is_empty() && hex(&s.key).starts_with(device))).collect();
    // A machine's services are named for it: by its name, it's its node.
    if found.len() > 1 {
        found.retain(|s| s.is(scopes::Kind::Compute));
    }
    let key = match found.as_slice() {
        [s] => s.key,
        // A node that was removed still has its records, which who added the app may switch off.
        _ => match unhex(device).ok() {
            Some(k) if managed(&k)? => k,
            _ => bail!("no one device called {device}"),
        },
    };
    ensure!(managed(&key)? || db.may(user, "switch", &scopes::device(&key)), "only {device}'s person switches what it runs");
    Ok(content::service(&key, name, on, scope))
}

/// A group called `name`, with this person its first admin: → `{ group }`, its id.
async fn group(m: &impl Me, name: &str) -> Result<Value> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give the group a name.");
    let group = scopes::new_id();
    m.write(content::group(&group, name)).await?;
    Ok(json!({ "group": group }))
}

/// As each start of a device does: says where its person's own things run (the machines that run everyone's), if they
/// haven't; names the device it's on in its listing, if it was listed before sign-ins named it (`on_device`); and makes
/// their own scope's first epoch, if it has none. What's to do is looked at again in what it reads fresh, as another
/// device of theirs may have done it.
pub async fn settle_own(m: &impl Me, on_device: Option<&str>) -> Result<()> {
    let me = m.public();
    let todo = || {
        m.who(|who| {
            let mine = who.machine(&me)?;
            let user = mine.as_ref().map(|d| d.user.clone()).unwrap_or_default();
            let runs = who.runs_until_chosen(&user)?;
            let unnamed = mine.filter(|d| d.device.is_none() && d.passkey.is_none() && on_device.is_some()).map(|d| (d.name, d.roles.into_iter().collect::<Vec<_>>()));
            let first = who.epoch(&Scope::User(user.clone()))? == 0;
            Ok((!runs.is_empty() || unnamed.is_some() || first).then_some((user, runs, unnamed, first)))
        })
    };
    if todo()?.is_none() {
        return Ok(());
    }
    m.read().await?;
    let Some((user, runs, unnamed, first)) = todo()? else { return Ok(()) };
    if let (Some(device), Some((name, roles))) = (on_device, unnamed) {
        let mut listed = content::device(&Endpoint { key: me, seal: m.seal() }, &user, &name, &roles, None);
        listed.data["device"] = json!(device);
        m.write(listed).await?;
    }
    if !runs.is_empty() {
        m.write(content::runs(&runs)).await?;
    }
    if first {
        new_epoch(m, &Scope::User(user), 1, &[]).await?;
    }
    Ok(())
}

/// Starts epoch `n` of a scope: a random key, its `epoch` step naming its hash written first (which, first in causal
/// order, makes it), then, once who's who says it did, sealed to the recovery key and to each client device of the
/// people who open it but `except` (keys being taken out), this device among them unless it's going.
pub async fn new_epoch(m: &impl Me, scope: &Scope, n: u32, except: &[[u8; 32]]) -> Result<()> {
    let me = Endpoint { key: m.public(), seal: m.seal() };
    let (recovery, to) = m.who(|who| {
        let mut to: Vec<Endpoint> = who.openers(scope)?.into_iter().filter(|c| c.seal != c.key && !except.contains(&c.key)).map(|c| Endpoint { key: c.key, seal: c.seal }).collect();
        if !to.iter().any(|e| e.key == me.key) && !except.contains(&me.key) && who.client(&me.key)?.is_some() {
            to.push(me);
        }
        Ok((who.recovery()?, to))
    })?;
    let key: [u8; 32] = rand::random();
    let hash = keys::epoch_hash(&key);
    m.write(content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
    m.read().await?;
    ensure!(m.who(|who| who.epoch_hash(scope, n))? == Some(hash), "Another device made new keys for this just now. Try again.");
    let sealed = |to: &[u8; 32]| keys::seal_to(to, &key).map(|s| B64U.encode(&s));
    // Not while there's none (the one there was forked `global`), until an admin names the next.
    if let Some(recovery) = recovery {
        let recovery = keys::twin_public(&recovery)?;
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": "recovery", "sealed": sealed(&recovery)? })).await?;
    }
    for e in &to {
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": hex(&e.key), "sealed": sealed(&e.seal)? })).await?;
    }
    m.made_epoch(scope, n, key);
    Ok(())
}

/// A new epoch of each scope this person may start one of, sealed to none of `except`: → those it left to one of their
/// admins, by name.
async fn renew(m: &impl Me, scopes: &[Scope], except: &[[u8; 32]]) -> Result<Vec<String>> {
    let user = m.user()?;
    let mut left = Vec::new();
    for scope in scopes {
        let (starts, newest) = m.who(|who| Ok((who.starts_epochs(&user, scope)?, who.epoch(scope)?)))?;
        if !starts {
            left.push(m.who(|who| whose(who, scope))?);
        } else if newest > 0 {
            new_epoch(m, scope, newest + 1, except).await?;
        }
    }
    Ok(left)
}

/// Tells the relay something (`{ add, admin? }` or `{ remove }`), signed by `by`: this device, or on the machine it's
/// kept on, the network's recovery key. A network with no address has no relay to tell.
pub async fn tell(m: &impl Me, by: &impl Signs, fields: Value) -> Result<()> {
    let Some(address) = m.address() else { return Ok(()) };
    let url = format!("{address}/relay/tell");
    let mut again = true;
    loop {
        let at = keys::next_at(m.now());
        let signed = keys::signed_request_by(by, crate::protocol::RELAY, fields.clone(), at).await?;
        let (status, out) = m.ask("POST", &url, None, Some(signed)).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
        if again && out["code"] == code::REPLAYED {
            again = false;
            m.wait(keys::again_in(at).as_millis() as u64).await;
            continue;
        }
        let said = error(&out).unwrap_or_else(|| format!("{YOURS} didn't take that ({status})."));
        return Err(match out["code"].as_str() {
            Some(c) => Coded::error(c, said),
            None => anyhow!(said),
        });
    }
}

/// Takes a key off the relay's list; one it doesn't list is out already.
pub async fn take_out(m: &impl Me, key: &[u8; 32]) -> Result<()> {
    match tell(m, m, json!({ "remove": hex(key) })).await {
        Err(err) if Coded::of(&err).as_deref() != Some(code::NOT_LISTED) => Err(err),
        _ => Ok(()),
    }
}

/// When each of the network's devices was last let onto it, as the relay says: `{ key: ms }`.
async fn seen(m: &impl Me) -> Result<Value> {
    let Some(address) = m.address() else { return Ok(json!({})) };
    let signed = keys::signed_request_by(m, crate::protocol::RELAY, json!({ "seen": true }), keys::next_at(m.now())).await?;
    let (status, out) = m.ask("POST", &format!("{address}/relay/seen"), None, Some(signed)).await?;
    Ok(if status == 200 { out["seen"].clone() } else { json!({}) })
}

/// Takes one of this person's devices out (by its key or name), or for an admin, anyone's: off the network first,
/// then unlisted, it and its parts; then each scope its person opens that this person may move on gets a new epoch
/// it doesn't have. Itself, it leaves (`Me::leave`).
async fn remove(m: &impl Me, device: &str) -> Result<Value> {
    let key = m.who(|who| who.find_machine(device))?;
    if key == m.public() {
        return m.leave().await.map(|()| json!({ "removed": "this device" }));
    }
    let user = m.user()?;
    let (name, keys, scopes) = m.who(|who| {
        let it = who.machine(&key)?.filter(|d| !d.removed).ok_or_else(|| anyhow!("That device isn’t listed."))?;
        ensure!(who.may(&user, "remove", &scopes::device(&key)), "Only its person or an admin takes a device out.");
        let keys: Vec<[u8; 32]> = std::iter::once(key).chain(who.machines()?.into_iter().filter(|d| d.device == Some(key) && d.key != key && !d.removed).map(|d| d.key)).collect();
        let scopes = who.scopes()?.into_iter().filter(|s| who.may(&it.user, "use", &scopes::object(s))).collect::<Vec<_>>();
        Ok((it.name, keys, scopes))
    })?;
    for k in &keys {
        take_out(m, k).await?;
        m.write(content::remove_device(k)).await?;
    }
    let left = renew(m, &scopes, &keys).await?;
    Ok(json!({ "removed": name, "not_renewed": left }))
}

/// Takes a person out, for an admin: each of their devices off the network first, then them, then each scope they
/// could open but their own gets a new epoch they don't have.
async fn remove_person(m: &impl Me, person: &str) -> Result<Value> {
    let user = m.user()?;
    let me = m.public();
    let (gone, name, keys, scopes) = m.who(|who| {
        let gone = who.find_user(person)?;
        ensure!(gone != user, "You can’t remove yourself. Another admin can.");
        ensure!(who.may(&user, "remove", scopes::NETWORK), "Only an admin takes someone out.");
        let name = who.user(&gone)?.map(|u| u.name).unwrap_or_default();
        let keys: Vec<[u8; 32]> = who.machines()?.into_iter().filter(|d| d.user == gone && !d.removed).map(|d| d.key).collect();
        ensure!(!keys.contains(&me), "This device is {name}'s: remove them from another.");
        let own = Scope::User(gone.clone());
        let scopes = who.scopes()?.into_iter().filter(|s| *s != own && who.may(&gone, "use", &scopes::object(s))).collect::<Vec<_>>();
        Ok((gone, name, keys, scopes))
    })?;
    for k in &keys {
        take_out(m, k).await?;
    }
    m.write(content::remove_user(&gone)).await?;
    let left = renew(m, &scopes, &keys).await?;
    Ok(json!({ "removed": name, "not_renewed": left }))
}

/// Asks the network's auth something as this device, signed with its key: a GET's or DELETE's in `x-one-request`,
/// signed for `path` without its query; a POST's as its body, with `fields`.
pub async fn ask_auth(m: &impl Me, method: &str, path: &str, fields: Value) -> Result<Value> {
    let address = m.address().ok_or_else(|| anyhow!("{YOURS} has no address yet, so it has no sign-ins or requests to join."))?;
    let mut fields = fields;
    fields["route"] = json!(format!("{method} {}", path.split('?').next().unwrap_or_default()));
    let signed = keys::signed_request_by(m, crate::protocol::AUTH_DOMAIN, fields, m.now()).await?;
    let url = format!("{address}{path}");
    let (status, out) = match method {
        "POST" => m.ask(method, &url, None, Some(signed)).await?,
        _ => m.ask(method, &url, Some((crate::protocol::header::REQUEST, B64U.encode(&serde_json::to_vec(&signed)?))), None).await?,
    };
    ensure!((200..300).contains(&status), "{}", error(&out).unwrap_or_else(|| format!("{YOURS}'s auth answered {status}.")));
    Ok(out)
}

/// The invites this device made that still work, by who they're for: an error if what it keeps of them can't be read,
/// which the next invite would otherwise write over.
async fn invites(m: &impl Me) -> Result<BTreeMap<String, Invited>> {
    let Some(kept) = m.kept(INVITES).await else { return Ok(BTreeMap::new()) };
    let kept: BTreeMap<String, Invited> = serde_json::from_slice(&kept).map_err(|err| anyhow!("The invites this device made can't be read ({INVITES}: {err}), so none was changed."))?;
    let now = m.now();
    Ok(kept.into_iter().filter(|(_, i)| i.until > now).collect())
}

async fn keep_invites(m: &impl Me, invites: &BTreeMap<String, Invited>) -> Result<()> {
    m.keep(INVITES, &serde_json::to_vec(invites)?).await
}

/// An invite link for someone called `name`, as `role`, made by this admin's device: its secret stays here, to check
/// the newcomer's device by when it asks to be let in; auth is told only its hash. A new person gets a new id; `user`
/// invites someone who's in the network already, as who they are, and `email` is the Google account people sign up
/// with where they sign in with Google. → `{ link, until }`.
async fn invite(m: &impl Me, name: &str, role: Role, user: Option<&str>, email: Option<&str>) -> Result<Value> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give their name.");
    ensure!(crate::config::SIGN_IN != crate::config::SignIn::Google || email.is_some(), "People here sign in with Google: give the Google account they sign up with.");
    let secret = B64U.encode(&rand::random::<[u8; 32]>());
    let new = user.is_none();
    let user = user.map(str::to_string).unwrap_or_else(scopes::new_id);
    let until = m.now() + crate::sign_in_proof::INVITE_DAYS * DAY;
    let mut fields = json!({ "id": hex(&keys::sha256(secret.as_bytes())), "name": name, "user": user, "until": until });
    if let Some(email) = email {
        fields["email"] = json!(email);
    }
    let mut kept = invites(m).await?;
    ask_auth(m, "POST", "/api/auth/invites", fields).await?;
    if new {
        kept.insert(user, Invited { secret: secret.clone(), name: name.to_string(), role, until });
        keep_invites(m, &kept).await?;
    }
    Ok(json!({ "link": format!("{}/invite#{secret}", m.address().unwrap_or_default()), "until": until }))
}

/// Devices waiting that this device may let in: its person's own, and someone new from an invite made here, whose
/// proof is that invite's; but those taken out or turned down. Each with the invite it came from, if it did.
async fn waiting(m: &impl Me) -> Result<Vec<(Asked, Option<Invited>)>> {
    let user = m.user()?;
    let asked = asked(m).await?;
    let invites = invites(m).await?;
    let mut out = Vec::new();
    for r in asked {
        let gone = m.who(|who| Ok(who.machine_ever(&r.endpoint.key)?.is_some_and(|d| d.removed)))?;
        if r.endpoint.key == m.public() || r.said["service"].is_string() || gone {
            continue;
        }
        if r.user == user {
            out.push((r, None));
        } else if let Some(invite) = invites.get(&r.user).filter(|i| r.said["proof"].as_str() == Some(&keys::invite_proof(i.secret.as_bytes(), &r.endpoint))) {
            out.push((r, Some(invite.clone())));
        }
    }
    Ok(out)
}

/// Every request to join auth lists to this device.
async fn asked(m: &impl Me) -> Result<Vec<Asked>> {
    let asked = ask_auth(m, "GET", "/api/auth/devices/requests", json!({})).await?;
    Ok(asked["requests"].as_array().into_iter().flatten().filter_map(Asked::of).collect())
}

/// The one of `found` whose code is `code`, as typed: none if two are, as a device made to show another's code would be.
fn the_one<T>(found: impl IntoIterator<Item = T>, code_of: impl Fn(&T) -> &str, code: &str) -> Result<T> {
    let code: String = code.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let mut found: Vec<T> = found.into_iter().filter(|r| code_of(r).replace(' ', "") == code).collect();
    match found.len() {
        0 => bail!("No device is waiting with that code. Check it, or start again on that device."),
        1 => Ok(found.remove(0)),
        _ => bail!("Two devices are waiting with that code, so neither was let in. Start again on yours, for a new code."),
    }
}

/// The one waiting with `code` that this device may let in.
async fn waiting_with(m: &impl Me, code: &str) -> Result<(Asked, Option<Invited>)> {
    the_one(waiting(m).await?, |(r, _)| &r.code, code)
}

/// The one asking with `code`, whoever's it is: for the network's recovery key, which lets in anyone's.
pub async fn asked_with(m: &impl Me, code: &str) -> Result<Asked> {
    the_one(asked(m).await?, |r| &r.code, code)
}

/// Marks a request approved at auth, for the device waiting on it to see, with the network it's in (its root), the
/// only one that device then takes, and where this device read who's who to, which that device starts from.
pub async fn mark_approved(m: &impl Me, key: &[u8; 32]) -> Result<()> {
    let root = m.who(|who| who.root())?.ok_or_else(|| Coded::error(code::NOT_LET_IN, "This device hasn't been let in yet."))?;
    let welcome = m.welcome().map(|w| format!("&welcome={w}")).unwrap_or_default();
    ask_auth(m, "DELETE", &format!("/api/auth/devices/requests/{}?approved&root={}{welcome}", hex(key), root.to_hex()), json!({})).await.map(|_| ())
}

/// Devices waiting that this device may let in, as Settings › Devices shows them, each with its code and what
/// approving it records as what they sign in with. One of its person's own whose sign-in shows it's theirs (one of
/// the accounts their devices record) is let in by it, not shown.
pub async fn requests(m: &impl Me) -> Result<Value> {
    let user = m.user()?;
    let accounts = m.who(|who| who.sign_ins(&user))?;
    let mut out = Vec::new();
    for (r, invite) in waiting(m).await? {
        if invite.is_none() && !accounts.is_empty() && signed_in_with(m, &r, Some(&accounts)).await.is_some() {
            match approve(m, &r, None).await {
                Ok(_) => continue,
                Err(err) => tracing::warn!("letting {} in by its sign-in: {err:#}", r.name),
            }
        }
        let mut shown = json!({ "device": r.name, "person": invite.as_ref().map_or(r.said["person"].clone(), |i| json!(i.name)), "code": r.code, "made": r.said["made"], "place": r.said["place"] });
        shown[if invite.is_some() { "new" } else { "yours" }] = json!(true);
        if let Some(account) = signed_in_with(m, &r, None).await {
            shown["sign_in"] = json!(account);
        }
        out.push(shown);
    }
    Ok(json!(out))
}

/// What `r`'s sign-in shows it signed in with, checked here (`sign_in_proof`): against `accounts`, its person's as
/// their devices record them, for one let in by it, or with none, for one approved by its code. None if it shows
/// nothing that checks out.
async fn signed_in_with(m: &impl Me, r: &Asked, accounts: Option<&[crate::sign_in_proof::Account]>) -> Option<crate::sign_in_proof::Account> {
    let proof: crate::sign_in_proof::Proof = serde_json::from_value(r.said["sign_in"].clone()).ok()?;
    let address = m.address()?;
    let (root, clients) = m.who(|who| Ok((who.root()?, who.setting(crate::sign_in_proof::CLIENTS)?))).ok()?;
    let network = crate::sign_in_proof::Network::new(&address, *root?.as_bytes(), clients.as_deref());
    let jwks = match proof.issuer().and_then(|i| crate::sign_in_proof::keys_at(&i, &address)) {
        Some(at) => Some(m.issuer_keys(&at).await?),
        None => None,
    };
    match accounts {
        Some(theirs) => crate::sign_in_proof::vouches(&proof, &r.endpoint, &network, jwks.as_ref(), theirs, m.now()).ok(),
        None => crate::sign_in_proof::check(&proof, &r.endpoint, &network, jwks.as_ref(), m.now()).ok(),
    }
}

/// Without `confirm`, whose the device showing `code` is; with, it's let in, its request marked approved at auth.
async fn approve_code(m: &impl Me, code: &str, confirm: bool) -> Result<Value> {
    let (r, invite) = waiting_with(m, code).await?;
    if !confirm {
        let person = invite.as_ref().map_or(r.said["person"].clone(), |i| json!(i.name));
        return Ok(json!({ "device": r.name, "person": person, "new": invite.is_some() }));
    }
    approve(m, &r, invite.as_ref()).await
}

/// Lets in a device that asked: lists it (and for a newcomer, them), with what it signed in with, the device it's
/// on and where it asked from; seals it each epoch this device opens of the scopes its person uses; takes their
/// sign-in before on the same device out; lets it onto the network; and marks its request approved with where this
/// device read who's who to, which the device waiting starts from. → `{ device, sealed, not_sealed }`.
pub async fn approve(m: &impl Me, r: &Asked, invite: Option<&Invited>) -> Result<Value> {
    m.read().await?;
    let user = m.user()?;
    let e = &r.endpoint;
    let device = r.said["device"].as_str().map(keys::unhex).transpose().map_err(|_| anyhow!("The device it's on is 64 hex characters."))?;
    let (removed, listed, may_invite) = m.who(|who| {
        let removed = who.machine_ever(&e.key)?.is_some_and(|d| d.removed);
        let listed = who.client(&e.key)?.is_some_and(|c| c.user == r.user && c.seal == e.seal);
        Ok((removed, listed, who.may(&user, "invite", scopes::NETWORK)))
    })?;
    // Taken out, or turned down: never sealed anything again, whatever a node says.
    ensure!(!removed, "{} was turned down or taken out, so it isn’t let in again. Start again on it, for a new code.", r.name);
    ensure!(invite.is_none() || may_invite, "Only an admin lets someone new in.");
    ensure!(invite.is_some() || r.user == user, "{} is someone else's: only their own devices approve one of theirs.", r.name);
    if let Some(i) = invite {
        ensure!(r.said["proof"].as_str() == Some(&keys::invite_proof(i.secret.as_bytes(), e)), "{}'s proof isn't from {}'s invite.", r.name, i.name);
    }
    if !listed {
        let mut listing = content::device(e, &r.user, &r.name, &[Kind::Client], invite.map(|i| (i.name.as_str(), i.role)));
        if let Some(on) = &device {
            listing.data["device"] = json!(hex(on));
        }
        if let Some(account) = signed_in_with(m, r, None).await {
            listing.data["sign_in"] = json!(account);
        }
        if r.said["place"].is_string() {
            listing.data["place"] = r.said["place"].clone();
        }
        m.write(listing).await?;
        m.read().await?;
    }
    // A newcomer's own scope is theirs: their device makes its first epoch.
    let (scopes, admin, before) = m.who(|who| {
        let own = Scope::User(r.user.clone());
        let scopes = who.scopes()?.into_iter().filter(|s| who.may(&r.user, "use", &scopes::object(s)) && (invite.is_none() || *s != own)).collect::<Vec<_>>();
        let before: Vec<[u8; 32]> = match device {
            Some(d) => who.clients()?.into_iter().filter(|c| c.user == r.user && c.device == Some(d) && c.key != e.key && c.passkey.is_none()).map(|c| c.key).collect(),
            None => Vec::new(),
        };
        Ok((scopes, who.may(&r.user, "invite", scopes::NETWORK), before))
    })?;
    let (mut sealed, mut not_sealed) = (Vec::new(), Vec::new());
    for scope in &scopes {
        match seal_held(m, scope, e).await {
            Ok(()) => sealed.push(scope.to_string()),
            Err(err) => not_sealed.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
        }
    }
    match tell(m, m, json!({ "add": hex(&e.key), "role": "client", "admin": admin })).await {
        Err(err) if !(listed && Coded::of(&err).as_deref() == Some(code::ALREADY)) => return Err(err),
        _ => {}
    }
    // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
    // keep their keys. One the relay wouldn't take out stays listed, for the next sign-in there to try again.
    for old in before {
        if let Err(err) = take_out(m, &old).await {
            tracing::warn!("{}'s earlier sign-in is still let in: {err:#}", r.name);
            continue;
        }
        m.write(content::remove_device(&old)).await?;
    }
    if m.address().is_some() {
        if invite.is_some_and(|i| i.role == Role::Admin)
            && let Err(err) = ask_auth(m, "POST", "/api/auth/people/admin", json!({ "user": r.user, "admin": true })).await
        {
            tracing::warn!("telling auth {} is an admin: {err:#}", r.name);
        }
        mark_approved(m, &e.key).await?;
    }
    if invite.is_some() {
        let mut kept = invites(m).await?;
        kept.remove(&r.user);
        keep_invites(m, &kept).await?;
    }
    Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not_sealed }))
}

/// Seals each epoch of `scope` sealed to this device to `to`, but those it has.
async fn seal_held(m: &impl Me, scope: &Scope, to: &Endpoint) -> Result<()> {
    for (n, key) in m.held(scope).await? {
        let body = json!({ "scope": scope.to_string(), "n": n, "to": hex(&to.key), "sealed": B64U.encode(&keys::seal_to(&to.seal, &key)?) });
        match m.keep_key(body).await {
            Err(err) if Coded::of(&err).as_deref() != Some(code::ALREADY) => return Err(err),
            _ => {}
        }
    }
    Ok(())
}

/// Turns down the device waiting with `code`: one of its person's own by a step, so none of their devices lets it
/// in after; either way its request is forgotten at auth. → which it was.
async fn refuse(m: &impl Me, code: &str) -> Result<Value> {
    let (r, invite) = waiting_with(m, code).await?;
    if invite.is_none() {
        m.write(content::refuse(&r.endpoint, &r.name)).await?;
    }
    ask_auth(m, "DELETE", &format!("/api/auth/devices/requests/{}", hex(&r.endpoint.key)), json!({})).await?;
    Ok(json!({ "device": r.name }))
}

/// Lists a passkey (its PRF output, `keys::passkey_keys`) as a way back in: it's sealed what this device opens, and
/// for an admin's, is an admin's device at the relay too, so the devices it lets in are let in as theirs.
async fn list_passkey(m: &impl Me, prf: &[u8], credential: &str) -> Result<()> {
    let (sign, seal) = keys::passkey_keys(prf);
    let pk = Endpoint { key: sign.public(), seal: keys::public_of(&seal) };
    let user = m.user()?;
    m.write(content::passkey(&pk, &user, "Passkey", credential)).await?;
    m.read().await?;
    let (scopes, admin) = m.who(|who| Ok((who.scopes()?.into_iter().filter(|s| who.may(&user, "use", &scopes::object(s))).collect::<Vec<_>>(), who.may(&user, "invite", scopes::NETWORK))))?;
    for scope in &scopes {
        seal_held(m, scope, &pk).await?;
    }
    tell(m, m, json!({ "add": hex(&pk.key), "role": "client", "admin": admin })).await
}

/// `/v1/app-schedules`.
async fn app_schedules_asked(m: &impl Me, body: body::AppSchedules) -> Result<Value> {
    let user = m.user()?;
    let (app, given) = (body.app, body.scope.as_deref().unwrap_or("user"));
    let (endpoint, scope, name) = m.who(|who| {
        let endpoint = who.find_node(&body.device)?;
        let scope = Scope::parse_for(given, &user).or_else(|_| who.find_group(given).map(Scope::Group))?;
        ensure!(who.app(&app)?.is_some(), "no app called {app}");
        Ok((endpoint, scope, who.machine(&endpoint)?.map(|d| d.name).unwrap_or_default()))
    })?;
    let days = body.days.unwrap_or(APP_SCHEDULES_DAYS);
    let until = set_app_schedules(m, &endpoint, &app, &scope, days).await?;
    Ok(match until {
        0 => json!({ "said": format!("{name} no longer runs {app}'s schedules, and what's written from now on is under new keys it doesn't have.") }),
        until => json!({ "said": format!("{name} runs {app}'s schedules for you, for {} days, and longer as you use it.", days.min(APP_SCHEDULES_DAYS)), "until": until }),
    })
}

/// Runs `app`'s schedules in `scope` on one of its person's devices (`endpoint`) for `days`, 30 at most (0 stops
/// them): the app's key in each epoch of the scope sealed to it first, then the app schedules naming each one's
/// hash. Stopping them starts a new epoch of the scope, which the device isn't sealed, so the keys it held open
/// nothing written after. → until when.
async fn set_app_schedules(m: &impl Me, endpoint: &[u8; 32], app: &str, scope: &Scope, days: u64) -> Result<u64> {
    let (seal, made) = m.who(|who| {
        let seal = who.machine(endpoint)?.filter(|d| !d.user.is_empty()).map(|d| d.seal).ok_or_else(|| anyhow!("That device isn’t one of yours."))?;
        Ok((seal, who.epoch_hashes(scope)?.into_keys().collect::<Vec<_>>()))
    })?;
    let until = if days == 0 { 0 } else { m.now() + days.min(APP_SCHEDULES_DAYS) * DAY };
    let mut hashes = BTreeMap::new();
    if until > 0 {
        for n in if made.is_empty() { vec![1] } else { made } {
            let key = m.app_key(scope, app, n).await?;
            m.keep_key(json!({ "scope": scope.to_string(), "n": n, "app": app, "to": hex(endpoint), "sealed": B64U.encode(&keys::seal_to(&seal, &key)?) })).await?;
            hashes.insert(n, keys::epoch_hash(&key));
        }
    }
    m.write(content::app_schedules(endpoint, scope, app, until, &hashes)).await?;
    m.read().await?;
    if until == 0 {
        renew(m, std::slice::from_ref(scope), &[]).await?;
    }
    Ok(until)
}

/// Renews this person's app schedules of `app` as they use it: once a day at most, and at once when the scope has
/// an epoch their keys don't reach; one of theirs that ran out is stopped, as Stop does. Only those the network's
/// steps hold, never those a node lists: a hacked one could list itself.
pub async fn renew_app_schedules(m: &impl Me, app: &str) {
    let Ok(user) = m.user() else { return };
    let now = m.now();
    let due = m.who(|who| {
        let mut due = Vec::new();
        for ((endpoint, scope, of), s) in who.app_schedules()? {
            if s.by != user || s.until == 0 || who.machine(&endpoint)?.is_none() {
                continue;
            }
            let reaches = s.hashes.keys().next_back().copied().unwrap_or(0);
            if s.until <= now {
                due.push((endpoint, scope, of, 0));
            } else if of == app && (s.until <= now + (APP_SCHEDULES_DAYS - 1) * DAY || reaches < who.epoch(&scope)?) {
                due.push((endpoint, scope, of, APP_SCHEDULES_DAYS));
            }
        }
        Ok(due)
    });
    for (endpoint, scope, app, days) in due.unwrap_or_default() {
        if let Err(err) = set_app_schedules(m, &endpoint, &app, &scope, days).await {
            tracing::warn!("renewing {app}'s schedules: {err:#}");
        }
    }
}

/// Whose a scope is, as a person reads it: everyone's, a group's or someone's, by name.
fn whose(who: &ScopeDb, scope: &Scope) -> Result<String> {
    Ok(match scope {
        Scope::Global => "everyone".into(),
        Scope::Group(g) => who.group(g)?.map(|g| g.name).unwrap_or_else(|| g.clone()),
        Scope::User(u) => who.user_ever(u)?.map(|u| u.name).unwrap_or_else(|| u.clone()),
    })
}

/// What a JSON answer says went wrong, if it does.
pub fn error(out: &Value) -> Option<String> {
    out["error"].as_str().map(str::to_string)
}
