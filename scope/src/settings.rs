//! What a device changes of who can do what, signed by itself as its user's device: Settings' changes (roles, groups,
//! handles, names, settings, removing a device or a user, the new epochs that go with it), letting devices in or
//! turning them down, inviting someone new, and app schedules. Every device runs this one copy (`Me`): each client
//! through its keyring (onet/src/settings.rs), and a machine with an app runtime as its own device, which its CLI and MCP
//! use (app-runtime/src/settings.rs). Each is asked as an app runtime's route of the same path (`ROUTES`).
//!
//! What it signs, it signs with what each device has (`keys::Signs`): a native app's key, a browser's WebCrypto key,
//! which can't be read, or an app runtime's own. Who may do what is judged by the steps' own rules wherever they're read
//! (scope/src/rules.rs): what's checked here only says why sooner.

use ::custom_build::config::CLI;
use ::custom_build::allowed;
use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail, ensure};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use ::custom_build::config::YOURS;
use crate::content::{self, Content};
use crate::scopes::{self, Kind, Role, Scope, ScopeDb};
use crate::steps::Welcome;
use ::key::{Endpoint, Key, Signs, hex, unhex};
use crate::sign_in_proof::{self, Account, Proof};
use ::custom_build::protocol::{Coded, code};

pub mod recovery;

/// How long app schedules last, unless their user renews them as they use the app.
pub const APP_SCHEDULES_DAYS: u64 = 30;
const DAY: u64 = 86_400_000;
/// The invites this device made, by who they're for: their secret, name, role and until when.
const INVITES: &str = "invites.json";

/// A device signing who can do what as its user's.
pub trait Me: Signs {
    /// What `f` finds in who's who as this device last read it.
    fn who<R>(&self, f: impl FnOnce(&ScopeDb) -> Result<R>) -> Result<R>;
    /// Its user: `code::NOT_LET_IN` before who's who lists it.
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
    /// Keeps a key it sealed, as `POST /api/keys` takes it: one there already fails with `code::ALREADY`.
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
    /// Takes itself out of its user's devices, as it signs out.
    fn leave(&self) -> impl Future<Output = Result<()>>;
    /// What's kept at `name` in the bucket, sealed as it is.
    fn stored(&self, name: &str) -> impl Future<Output = Result<Option<Vec<u8>>>>;
    /// Keeps `bytes` at `name` in the bucket, only if nothing is there (`stored::created`): false if something else is.
    fn create(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<bool>>;
    /// Deletes what's at `name` in the bucket.
    fn delete(&self, name: &str) -> impl Future<Output = Result<()>>;
    /// Signs `content` with `key`, the network's recovery key, as `user`'s, as the next step of `global`'s scope
    /// database, and writes it (`recovery`).
    fn write_as(&self, key: &::key::SigningKey, user: &str, content: Content) -> impl Future<Output = Result<()>>;
    /// Opens what's sealed to this device.
    fn open(&self, sealed: &[u8]) -> impl Future<Output = Result<Vec<u8>>>;
}

/// What a device answers itself of an app runtime's routes, signing what they change.
#[derive(Clone, Copy)]
enum Route {
    LinkSignIn,
    RecordSignIn,
    Role,
    Remove,
    Settings,
    Group,
    Invite,
    Accept,
    Member,
    Handle,
    Name,
    Service,
    Retire,
    ForgetSignIn,
    InviteLink,
    WithdrawInvite,
    RunSchedules,
    StopSchedules,
    Requests,
    Approve,
    Refuse,
    Seen,
    Update,
    VaultGrant,
    RecoveryHolder,
    Code,
    Restore,
}

/// The only list of them: a route of a path and method, which `answer` runs (its `match` fails to compile for one
/// missing here or there), and `answered` tells a request that comes by of; and whether a machine with an app runtime answers
/// it with this alone (`on_a_node`), false where the app runtime serves that path itself: its own name, and services, which it serves for
/// more than its own programs, and a tool that is a route (`api::Runs`), which asks `answer_as_mine` itself.
const ROUTES: &[(&str, &str, Route, bool)] = &[
    ("POST", "/api/role", Route::Role, true),
    ("POST", "/api/devices/remove", Route::Remove, false),
    ("POST", "/api/settings", Route::Settings, true),
    ("POST", "/api/group", Route::Group, true),
    ("POST", "/api/invite", Route::Invite, true),
    ("POST", "/api/accept", Route::Accept, true),
    ("POST", "/api/member", Route::Member, true),
    ("POST", "/api/groups/handle", Route::Handle, false),
    ("POST", "/api/name", Route::Name, false),
    ("POST", "/api/service", Route::Service, false),
    ("POST", "/api/retire", Route::Retire, true),
    ("POST", "/api/devices/forget-sign-in", Route::ForgetSignIn, true),
    ("POST", "/api/devices/link-sign-in", Route::LinkSignIn, true),
    ("POST", "/api/devices/record-sign-in", Route::RecordSignIn, true),
    ("POST", "/api/devices/invite", Route::InviteLink, false),
    ("POST", "/api/devices/invite/withdraw", Route::WithdrawInvite, true),
    ("POST", "/api/devices/run_schedules", Route::RunSchedules, false),
    ("POST", "/api/devices/stop_schedules", Route::StopSchedules, false),
    ("POST", "/api/devices/requests", Route::Requests, false),
    ("POST", "/api/devices/approve", Route::Approve, false),
    ("POST", "/api/devices/refuse", Route::Refuse, true),
    ("GET", "/api/devices/seen", Route::Seen, true),
    ("POST", "/api/devices/update", Route::Update, false),
    // A machine with an app runtime that opens the Vault seals what's in it to the new key at once.
    ("POST", "/api/vault/grant", Route::VaultGrant, false),
    ("POST", "/api/devices/recovery-holder", Route::RecoveryHolder, true),
    ("GET", "/api/devices/code", Route::Code, true),
    ("POST", "/api/devices/restore", Route::Restore, true),
];

/// What the OpenAPI document says of a route: a line for whoever reads it (`Tool`'s own, for a tool's route), and the body
/// `answered_by` reads of it.
fn about(route: Route) -> (&'static str, Value) {
    use ::custom_build::protocol::schema_of as of;
    match route {
        Route::Role => ("Change a user's role", of::<crate::settings::body::Role>()),
        Route::Remove => ("", of::<crate::settings::body::Remove>()),
        Route::Settings => ("Change one of the network's settings", of::<crate::settings::body::Setting>()),
        Route::Group => ("Make a group", of::<crate::settings::body::Named>()),
        Route::Invite => ("Invite a user to a group", of::<crate::settings::body::Invite>()),
        Route::Accept => ("Accept a group's invite", of::<crate::settings::body::Group>()),
        Route::Member => ("Change a user's place in a group", of::<crate::settings::body::Member>()),
        Route::Handle => ("", of::<crate::settings::body::Handle>()),
        Route::Name => ("Rename a device", of::<crate::settings::body::Name>()),
        Route::Service => ("Switch a service on or off on a device", of::<crate::settings::body::Service>()),
        Route::Retire => ("Delete, restore or erase an app", of::<crate::settings::body::Retire>()),
        Route::ForgetSignIn => ("Forget a device's sign-in", of::<crate::settings::body::ForgetSignIn>()),
        Route::LinkSignIn => ("Where this device links a further sign-in", of::<crate::settings::body::Nothing>()),
        Route::RecordSignIn => ("What this device just linked, and with confirm, record it", of::<crate::settings::body::Confirm>()),
        Route::InviteLink => ("", of::<crate::settings::body::InviteLink>()),
        Route::WithdrawInvite => ("Withdraw an invite by email", of::<crate::settings::body::WithdrawInvite>()),
        Route::RunSchedules => ("", of::<crate::settings::body::RunSchedules>()),
        Route::StopSchedules => ("", of::<crate::settings::body::StopSchedules>()),
        Route::Requests => ("", of::<crate::settings::body::Nothing>()),
        Route::Approve => ("", of::<crate::settings::body::Approve>()),
        Route::Refuse => ("Turn down a device waiting to join", of::<crate::settings::body::Code>()),
        Route::Seen => ("Say the waiting devices were seen", of::<crate::settings::body::Nothing>()),
        Route::Update => ("", of::<crate::settings::body::Update>()),
        Route::VaultGrant => ("", of::<crate::settings::body::VaultGrant>()),
        Route::RecoveryHolder => ("Name someone as holding the recovery key, or take them off", of::<crate::settings::body::RecoveryHolder>()),
        Route::Code => ("This device's code, read from its own screen", of::<crate::settings::body::Nothing>()),
        Route::Restore => ("For someone holding the recovery key: give a device of someone who lost every device their keys", of::<crate::settings::body::Approve>()),
    }
}

/// Each route a device answers itself, as the OpenAPI document lists it: its method and path, then `about`.
pub fn answered_routes() -> impl Iterator<Item = (&'static str, &'static str, &'static str, Value)> {
    ROUTES.iter().map(|(method, path, route, _)| {
        let (summary, body) = about(*route);
        (*method, *path, summary, body)
    })
}

/// Whether a device answers the app runtime's route of this path itself, whatever the method.
pub fn answered(path: &str) -> bool {
    ROUTES.iter().any(|(_, p, _, _)| *p == path)
}

/// The routes a machine with an app runtime answers as its own device, by method and path.
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
    /// All of the code the device asking shows (`keys::join_code`).
    code: String,
}

impl Asked {
    /// A request as auth lists it: `{ key, seal, user, name, user?, proof?, device?, made?, place?, sign_in? }`.
    pub fn of(r: &Value) -> Option<Asked> {
        let endpoint = Endpoint { key: ::key::unhex(r["key"].as_str()?).ok()?, seal: ::key::unhex(r["seal"].as_str()?).ok()? };
        Some(Asked { said: r.clone(), endpoint, user: text(r, "user"), name: text(r, "name"), code: ::key::join_code(&endpoint) })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Its user, by id.
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

/// The answer to a request a device answers itself, as an app runtime's route of the same path would (`/api/role`, `/api/remove`,
/// `/api/devices/approve`, …): none for any other, which goes to an app runtime. Its body is the route's own (`body`), every
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

    use crate::scopes;

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Role {
        pub user: String,
        pub role: scopes::Role,
    }

    /// A device by its name or key, or a user by their id or name, with all their devices.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Remove {
        pub device: Option<String>,
        pub user: Option<String>,
    }

    /// Which device, by name.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Update {
        pub device: String,
    }

    /// `value` none sets it back to what it is by default.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Setting {
        pub name: String,
        pub value: Option<String>,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RecoveryHolder {
        /// By name or id.
        pub user: String,
        pub holds: bool,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Named {
        pub name: String,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Invite {
        pub group: String,
        pub user: String,
        pub role: scopes::Role,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Group {
        pub group: String,
    }

    /// `role` none takes them out of the group.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Member {
        pub group: String,
        pub user: String,
        pub role: Option<scopes::Role>,
    }

    /// A group's handle, or with no `group`, this user's.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Handle {
        pub handle: String,
        pub group: Option<String>,
    }

    /// Another device's name, or with no `key`, this one's.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Name {
        pub key: Option<String>,
        pub name: String,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Service {
        pub device: String,
        pub name: String,
        pub on: bool,
    }

    /// `how`: deleted, restored or erased.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Retire {
        pub app: String,
        pub how: String,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct ForgetSignIn {
        pub key: String,
        /// One it linked; none forgets what it was let in with.
        pub account: Option<crate::sign_in_proof::Account>,
    }

    /// `user` invites someone in the network already; `email` is the account a newcomer signs up with.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct InviteLink {
        pub name: String,
        #[serde(default)]
        pub admin: bool,
        pub user: Option<String>,
        pub email: Option<String>,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct WithdrawInvite {
        pub user: String,
    }

    /// `device` by its name or key; `scope` user (theirs, the default), global, or a group by its name or id; `days`, 30
    /// at most.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct RunSchedules {
        pub app: String,
        pub device: String,
        pub scope: Option<String>,
        pub days: Option<u64>,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct StopSchedules {
        pub app: String,
        pub device: String,
        pub scope: Option<String>,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Code {
        pub code: String,
    }

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Confirm {
        #[serde(default)]
        pub confirm: bool,
    }

    /// Without `confirm`, whose the device showing `code` is.
    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Approve {
        pub code: String,
        #[serde(default)]
        pub confirm: bool,
    }

    /// `device` added to the machines that open `scope`'s Vault (user, the default; global; or a group by its name or
    /// id), or with `on: false`, taken off them.
    #[derive(Deserialize, schemars::JsonSchema)]
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

    #[derive(Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct Nothing {}
}

async fn answered_by(m: &impl Me, route: Route, path: &str, raw: &[u8]) -> Result<Value> {
    let done = json!({});
    Ok(match route {
        Route::Role => {
            let body: crate::settings::body::Role = read(path, raw)?;
            m.write(content::role(&m.who(|db| db.find_user(&body.user))?, body.role)).await?;
            done
        }
        Route::Remove => {
            let mut removed = match read(path, raw)? {
                crate::settings::body::Remove { device: Some(device), user: None } => remove(m, &device).await?,
                crate::settings::body::Remove { device: None, user: Some(user) } => remove_user(m, &user).await?,
                _ => bail!("Give a device or a user, not both."),
            };
            removed["said"] = json!(said_removed(&removed));
            removed
        }
        Route::Settings => {
            let body: crate::settings::body::Setting = read(path, raw)?;
            m.write(content::setting(&body.name, body.value.as_deref())).await?;
            done
        }
        Route::RecoveryHolder => recovery_holder(m, read::<crate::settings::body::RecoveryHolder>(path, raw)?).await?,
        Route::Group => group(m, &read::<crate::settings::body::Named>(path, raw)?.name).await?,
        Route::Invite => {
            let body: crate::settings::body::Invite = read(path, raw)?;
            let (group, user) = m.who(|db| Ok((db.find_group(&body.group)?, db.find_user(&body.user)?)))?;
            m.write(content::invite(&group, &user, body.role)).await?;
            done
        }
        Route::Accept => {
            let body: crate::settings::body::Group = read(path, raw)?;
            m.write(content::accept(&m.who(|db| db.find_group(&body.group))?)).await?;
            done
        }
        Route::Member => {
            let body: crate::settings::body::Member = read(path, raw)?;
            let (group, user) = m.who(|db| Ok((db.find_group(&body.group)?, db.find_user(&body.user)?)))?;
            m.write(content::member(&group, &user, body.role)).await?;
            done
        }
        Route::Handle => {
            let body: crate::settings::body::Handle = read(path, raw)?;
            let (what, id) = match &body.group {
                Some(g) => ("group", m.who(|db| db.find_group(g))?),
                None => ("user", m.user()?),
            };
            m.write(content::handle(what, &id, body.handle.trim())).await?;
            done
        }
        Route::Name => {
            let body: crate::settings::body::Name = read(path, raw)?;
            let key = match &body.key {
                Some(k) => m.who(|db| db.find_machine(k))?,
                None => m.public(),
            };
            m.write(content::rename("key", &hex(&key), body.name.trim())).await?;
            done
        }
        Route::Service => {
            let body: crate::settings::body::Service = read(path, raw)?;
            let user = m.user()?;
            m.write(m.who(|db| service(db, &user, &body.device, &body.name, body.on, None))?).await?;
            done
        }
        Route::Retire => {
            let body: crate::settings::body::Retire = read(path, raw)?;
            let user = m.user()?;
            m.write(m.who(|db| retire(db, &user, &body.app, &body.how, &body.app))?).await?;
            done
        }
        Route::ForgetSignIn => {
            let body: crate::settings::body::ForgetSignIn = read(path, raw)?;
            forget_sign_in(m, &body.key, body.account.as_ref()).await?;
            done
        }
        Route::LinkSignIn => {
            let _: crate::settings::body::Nothing = read(path, raw)?;
            let address = m.address().ok_or_else(|| anyhow!("{YOURS} has no address yet, so it has no sign-ins."))?;
            json!({ "url": format!("{address}/sign-in/link?join={}{}", hex(&m.public()), hex(&m.seal())) })
        }
        Route::RecordSignIn => {
            let body: crate::settings::body::Confirm = read(path, raw)?;
            record_sign_in(m, body.confirm).await?
        }
        Route::InviteLink => {
            let body: crate::settings::body::InviteLink = read(path, raw)?;
            let mut invited = invite(m, &body.name, if body.admin { Role::Admin } else { Role::Member }, body.user.as_deref(), body.email.as_deref()).await?;
            invited["said"] = json!(said_invited(body.name.trim(), &invited));
            invited
        }
        Route::WithdrawInvite => {
            let body: crate::settings::body::WithdrawInvite = read(path, raw)?;
            m.write(content::withdraw_invite(&body.user)).await?;
            ask_auth(m, "DELETE", &format!("/api/auth/invites/{}", body.user), json!({})).await?;
            done
        }
        Route::RunSchedules => {
            let body: crate::settings::body::RunSchedules = read(path, raw)?;
            app_schedules_asked(m, body.app, &body.device, body.scope.as_deref(), body.days.unwrap_or(APP_SCHEDULES_DAYS)).await?
        }
        Route::StopSchedules => {
            let body: crate::settings::body::StopSchedules = read(path, raw)?;
            app_schedules_asked(m, body.app, &body.device, body.scope.as_deref(), 0).await?
        }
        Route::Requests => {
            read::<crate::settings::body::Nothing>(path, raw)?;
            requests(m).await?
        }
        Route::Approve => {
            let body: crate::settings::body::Approve = read(path, raw)?;
            let mut approved = approve_code(m, &body.code, body.confirm).await?;
            approved["said"] = json!(said_approved(&approved));
            approved
        }
        Route::Refuse => refuse(m, &read::<crate::settings::body::Code>(path, raw)?.code).await?,
        Route::Seen => {
            read::<crate::settings::body::Nothing>(path, raw)?;
            seen(m).await?
        }
        Route::Update => update_device(m, &read::<crate::settings::body::Update>(path, raw)?.device).await?,
        Route::VaultGrant => vault_grant(m, read::<crate::settings::body::VaultGrant>(path, raw)?).await?,
        Route::Code => {
            read::<crate::settings::body::Nothing>(path, raw)?;
            json!({ "code": ::key::spaced_code(&::key::join_code(&Endpoint { key: m.public(), seal: m.seal() })) })
        }
        Route::Restore => {
            let body: crate::settings::body::Approve = read(path, raw)?;
            restore(m, &body.code, body.confirm).await?
        }
    })
}

/// Says which machines open a Vault: `device` added to them or taken off, by its user or its admins. Always a new
/// key pair (`new_vault_key`); what's sealed to the old one is sealed to the new by a machine that opens both, at its
/// next write there. → `{ said }`.
async fn vault_grant(m: &impl Me, body: crate::settings::body::VaultGrant) -> Result<Value> {
    let user = m.user()?;
    let given = body.scope.as_deref().unwrap_or("user");
    let (scope, endpoint, name, current) = m.who(|who| {
        let scope = Scope::parse_for(given, &user).or_else(|_| who.find_group(given).map(Scope::Group))?;
        allowed!(who.may(&user, "choose-devices", &scopes::object(&scope)), "Only this Vault's user, or its admins, say which machines open it.");
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
        false if to.is_empty() => format!("{name} no longer opens this Vault, and no machine does: nothing secret can be set there until one is granted."),
        false => format!("{name} no longer opens this Vault: what's set from now on is sealed to a key it doesn't have."),
    } }))
}

/// Makes `scope`'s Vault key `n`, for the machines `to`: its `vault` step first, then its secret half sealed to each of
/// them and to the recovery key.
pub async fn new_vault_key(m: &impl Me, scope: &Scope, n: u32, to: &[[u8; 32]]) -> Result<()> {
    let secret: [u8; 32] = rand::random();
    let public = ::key::public_of(&secret);
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
    for (at, seal) in std::iter::once(("recovery".to_string(), ::key::twin_public(&recovery)?)).chain(seals) {
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": at, "vault": true, "sealed": B64U.encode(&::key::seal_to(&seal, &secret)?) })).await?;
    }
    Ok(())
}

/// The step that deletes `app` (`deleted`), restores it (`restored`) or erases it for good (`erased`), as `user`, who
/// acts for its owner, says it: whoever writes it, from any device or an app's call. `title` is what users call it.
/// A deleted app stops running and leaves every page, but keeps its data until it's erased, which deletes its keys in
/// the bucket as each app runtime reads the step (`Host::follow_erased`).
pub fn retire(who: &ScopeDb, user: &str, app: &str, how: &str, title: &str) -> Result<Content> {
    let a = who.app(app)?.ok_or_else(|| anyhow!("no app named {app}"))?;
    allowed!(who.may(user, "delete", &scopes::app(app)), "Only {title}'s owners can delete, restore or erase it.");
    ensure!(how != "erased" || a.deleted.is_some(), "Delete {title} before erasing it.");
    content::retire(app, &a.owner.to_string(), how)
}

/// The `service` entry that switches a part of a device's oned on or off, the device found by name or key prefix; for
/// the device's user, or to switch it off, for who may add apps where it ran. A service runs for the app added in
/// `scope`, which its writer may add apps to; switched on, every part but the app runtime has one.
pub fn service(db: &ScopeDb, user: &str, device: &str, name: &str, on: bool, scope: Option<&Scope>) -> Result<Content> {
    if on && name != scopes::APPS_PART {
        let Some(s) = scope else { bail!("{name} is an app's service, and that app isn't added: `{CLI} add {name}` first, which sets its service up") };
        allowed!(db.may(user, "add-apps", &scopes::object(s)), "only who may add apps there runs a service for them");
    }
    let managed = |k: &Key| -> Result<bool> { Ok(!on && db.switch(k, name)?.and_then(|s| s.scope).and_then(|s| Scope::parse(&s).ok()).is_some_and(|s| db.may(user, "add-apps", &scopes::object(&s)))) };
    // oned on Cloudflare, by its name in run priority or its key: it runs apps and no service, which its admins switch.
    if let Some(k) = db.cloudflare_node()?.filter(|k| device.eq_ignore_ascii_case(crate::rank::CLOUDFLARE) || (!device.is_empty() && hex(k).starts_with(device))) {
        ensure!(name == scopes::APPS_PART, "Cloudflare can't run {name}: it runs apps, and services run on computers");
        allowed!(db.may(user, "change-settings", scopes::NETWORK), "only an admin switches what Cloudflare runs");
        return Ok(content::service(&k, name, on, scope));
    }
    let mut found: Vec<scopes::Machine> = db.machines()?.into_iter().filter(|s| !s.removed).filter(|s| db.device_name(s).is_ok_and(|n| n.eq_ignore_ascii_case(device)) || (!device.is_empty() && hex(&s.key).starts_with(device))).collect();
    // A machine's services are named for it: by its name, it's its app runtime.
    if found.len() > 1 {
        found.retain(|s| s.is(scopes::Kind::Compute));
    }
    let key = match found.as_slice() {
        // What a device's app runtime runs is switched for the device, whose switches it reads (`ScopeDb::device_of`).
        [s] => s.runtime_of().unwrap_or(s.key),
        // An app runtime that was removed still has its records, which who added the app may switch off.
        _ => match unhex(device).ok() {
            Some(k) if managed(&k)? => k,
            _ => bail!("no device is called {device}, or more than one is"),
        },
    };
    allowed!(managed(&key)? || db.may(user, "switch", &scopes::device(&key)), "only {device}'s user switches what it runs");
    Ok(content::service(&key, name, on, scope))
}

/// An admin says `body.user` holds the recovery key, or no longer does: the whole list in one setting, written after
/// reading what's new. It hands them nothing: someone who has the key does that on a computer (`devices::cli`). → what
/// to tell the user.
async fn recovery_holder(m: &impl Me, body: crate::settings::body::RecoveryHolder) -> Result<Value> {
    let me = m.user()?;
    let (value, name) = m.who(|who| {
        allowed!(who.may(&me, "change-settings", scopes::NETWORK), "Only an admin says who holds the recovery key.");
        let user = who.find_user(&body.user)?;
        let name = who.user(&user)?.map(|u| u.name).unwrap_or_default();
        let mut holders = who.recovery_holders()?;
        match body.holds {
            true => drop(holders.insert(user)),
            false => drop(holders.remove(&user)),
        }
        Ok((serde_json::to_string(&holders)?, name))
    })?;
    m.write(content::setting(scopes::RECOVERY_HOLDERS, Some(&value))).await?;
    Ok(json!({ "said": match body.holds {
        true => format!("{name} is named as holding the recovery key. They get it once someone who has it seals it to their devices."),
        false => format!("{name} is no longer named as holding the recovery key. Anything already sealed to their devices still opens it, until the recovery key itself is replaced."),
    } }))
}

/// A group called `name`, with this user its first admin: → `{ group }`, its id.
async fn group(m: &impl Me, name: &str) -> Result<Value> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give the group a name.");
    let group = scopes::new_id();
    m.write(content::group(&group, name)).await?;
    Ok(json!({ "group": group }))
}

/// As each start of a device does: says where its user's own things run (the machines that run everyone's), if they
/// haven't; names the device it's on in its listing, if it was listed before sign-ins named it (`on_device`), and says
/// what it's built for, if it hasn't (`platform`, for a client); and makes
/// their own scope's first epoch, if it has none. What's to do is looked at again in what it reads fresh, as another
/// device of theirs may have done it.
pub async fn settle_own(m: &impl Me, on_device: Option<&str>, platform: Option<&str>) -> Result<()> {
    let me = m.public();
    let todo = || {
        m.who(|who| {
            let mine = who.machine(&me)?;
            let user = mine.as_ref().map(|d| d.user.clone()).unwrap_or_default();
            let runs = who.runs_until_chosen(&user)?;
            let unnamed = mine.filter(|d| d.passkey.is_none() && (d.device.is_none() && on_device.is_some() || d.platform.is_none() && platform.is_some())).map(|d| (d.name, d.roles.into_iter().collect::<Vec<_>>(), d.device));
            let first = who.epoch(&Scope::User(user.clone()))? == 0;
            Ok((!runs.is_empty() || unnamed.is_some() || first).then_some((user, runs, unnamed, first)))
        })
    };
    if todo()?.is_none() {
        return Ok(());
    }
    m.read().await?;
    let Some((user, runs, unnamed, first)) = todo()? else { return Ok(()) };
    if let Some((name, roles, device)) = unnamed {
        let mut listed = content::device(&Endpoint { key: me, seal: m.seal() }, &user, &name, &roles, None);
        if let Some(device) = device.as_ref().map(hex).as_deref().or(on_device) {
            listed.data["device"] = json!(device);
        }
        if let Some(platform) = platform {
            listed.data["platform"] = json!(platform);
        }
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
/// users who open it but `except` (keys being taken out), this device among them unless it's going.
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
    let hash = ::key::epoch_hash(&key);
    m.write(content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
    m.read().await?;
    ensure!(m.who(|who| who.epoch_hash(scope, n))? == Some(hash), "Another device made new keys for this just now. Try again.");
    let sealed = |to: &[u8; 32]| ::key::seal_to(to, &key).map(|s| B64U.encode(&s));
    // Not while there's none (the one there was forked `global`), until an admin names the next.
    if let Some(recovery) = recovery {
        let recovery = ::key::twin_public(&recovery)?;
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": "recovery", "sealed": sealed(&recovery)? })).await?;
    }
    for e in &to {
        m.keep_key(json!({ "scope": scope.to_string(), "n": n, "to": hex(&e.key), "sealed": sealed(&e.seal)? })).await?;
    }
    m.made_epoch(scope, n, key);
    Ok(())
}

/// A new epoch of each scope this user may start one of, sealed to none of `except`: → those it left to one of their
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
        let at = ::key::next_at(m.now());
        let signed = ::key::signed_request_by(by, ::custom_build::protocol::RELAY, fields.clone(), at).await?;
        let (status, out) = m.ask("POST", &url, None, Some(signed)).await?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
        if again && out["code"] == code::REPLAYED {
            again = false;
            m.wait(::key::again_in(at).as_millis() as u64).await;
            continue;
        }
        let said = error(&out).unwrap_or_else(|| format!("{YOURS} didn't take that ({status})."));
        return Err(match out["code"].as_str() {
            Some(c) => Coded::error(c, said),
            None => anyhow!(said),
        });
    }
}

/// Takes a key off the relay's list; one it doesn't list is out already. Its sign-in ends first, while this device
/// is still one the network lists, or auth wouldn't take its word for it.
pub async fn take_out(m: &impl Me, key: &[u8; 32]) -> Result<()> {
    if m.address().is_some() {
        let (status, out) = asked_auth(m, "POST", "/api/auth/devices/removed", json!({ "key": hex(key) })).await?;
        // A network's auth from before this route has no sign-in to end: an app runtime is sent out before the Workers.
        ensure!((200..300).contains(&status) || status == 404, "{}", error(&out).unwrap_or_else(|| format!("{YOURS}'s auth answered {status}.")));
    }
    match tell(m, m, json!({ "remove": hex(key) })).await {
        Err(err) if Coded::of(&err).as_deref() != Some(code::NOT_LISTED) => Err(err),
        _ => Ok(()),
    }
}

/// When each of the network's devices was last let onto it, and which key let it in, as the relay says: `{ seen: { key:
/// ms }, let_in: { key: key } }`.
async fn seen(m: &impl Me) -> Result<Value> {
    let Some(address) = m.address() else { return Ok(json!({ "seen": {}, "let_in": {} })) };
    let signed = ::key::signed_request_by(m, ::custom_build::protocol::RELAY, json!({ "seen": true }), ::key::next_at(m.now())).await?;
    let (status, out) = m.ask("POST", &format!("{address}/relay/seen"), None, Some(signed)).await?;
    Ok(if status == 200 { json!({ "seen": out["seen"], "let_in": out["let_in"] }) } else { json!({ "seen": {}, "let_in": {} }) })
}

/// Asks a device, through the relay, to take the newest published build. Only its user or an admin may; its own device
/// checks that too, and only ever starts `one update`. Cloudflare is asked to deploy the Workers again, from the device they were
/// last deployed from, which the relay knows: only an admin may.
async fn update_device(m: &impl Me, device: &str) -> Result<Value> {
    let user = m.user()?;
    if device.eq_ignore_ascii_case(crate::rank::CLOUDFLARE) {
        m.who(|who| {
            allowed!(who.may(&user, "change-settings", scopes::NETWORK), "Only an admin updates Cloudflare.");
            Ok(())
        })?;
        tell(m, m, json!({ "cloudflare": true })).await?;
        return Ok(json!({ "said": "Asked Cloudflare to update." }));
    }
    let (key, name) = m.who(|who| {
        let key = who.find_node(device)?;
        allowed!(who.may(&user, "remove", &scopes::device(&key)), "Only its user or an admin updates a device.");
        Ok((key, who.machine(&key)?.map(|d| d.name).unwrap_or_default()))
    })?;
    tell(m, m, json!({ "update": hex(&key) })).await?;
    Ok(json!({ "said": format!("Asked {name} to update.") }))
}

/// Takes one of this user's devices out (by its key or name, or its app runtime's), or for an admin, anyone's: off the
/// network first, then unlisted, it and its parts, each part only if who's who didn't take it out with the device; then
/// each scope its user opens that this user may move on gets a new epoch it doesn't have. Itself, it leaves
/// (`Me::leave`).
async fn remove(m: &impl Me, device: &str) -> Result<Value> {
    let key = m.who(|who| who.find_machine(device).map(|k| who.device_of(&k)))?;
    if key == m.public() {
        return m.leave().await.map(|()| json!({ "removed": "this device" }));
    }
    let user = m.user()?;
    let (name, keys, scopes) = m.who(|who| {
        let it = who.machine(&key)?.filter(|d| !d.removed).ok_or_else(|| anyhow!("That device isn’t listed."))?;
        allowed!(who.may(&user, "remove", &scopes::device(&key)), "Only its user or an admin takes a device out.");
        let keys: Vec<[u8; 32]> = std::iter::once(key).chain(who.machines()?.into_iter().filter(|d| d.device == Some(key) && d.key != key && !d.removed).map(|d| d.key)).collect();
        let scopes = who.scopes()?.into_iter().filter(|s| who.may(&it.user, "use", &scopes::object(s))).collect::<Vec<_>>();
        Ok((it.name, keys, scopes))
    })?;
    for k in &keys {
        take_out(m, k).await?;
        if m.who(|who| who.machine(k))?.is_some() {
            m.write(content::remove_device(k)).await?;
        }
    }
    let left = renew(m, &scopes, &keys).await?;
    Ok(json!({ "removed": name, "not_renewed": left }))
}

/// Takes a user out, for an admin: each of their devices off the network first, then them, then each scope they
/// could open but their own gets a new epoch they don't have.
async fn remove_user(m: &impl Me, named: &str) -> Result<Value> {
    let user = m.user()?;
    let me = m.public();
    let (gone, name, keys, scopes) = m.who(|who| {
        let gone = who.find_user(named)?;
        ensure!(gone != user, "You can’t remove yourself. Another admin can.");
        allowed!(who.may(&user, "remove", scopes::NETWORK), "Only an admin takes someone out.");
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
    let (status, out) = asked_auth(m, method, path, fields).await?;
    ensure!((200..300).contains(&status), "{}", error(&out).unwrap_or_else(|| format!("{YOURS}'s auth answered {status}.")));
    Ok(out)
}

async fn asked_auth(m: &impl Me, method: &str, path: &str, fields: Value) -> Result<(u16, Value)> {
    let address = m.address().ok_or_else(|| anyhow!("{YOURS} has no address yet, so it has no sign-ins or requests to join."))?;
    let mut fields = fields;
    fields["route"] = json!(format!("{method} {}", path.split('?').next().unwrap_or_default()));
    let signed = ::key::signed_request_by(m, ::custom_build::protocol::AUTH_DOMAIN, fields, m.now()).await?;
    let url = format!("{address}{path}");
    match method {
        "POST" => m.ask(method, &url, None, Some(signed)).await,
        _ => m.ask(method, &url, Some((::custom_build::protocol::header::REQUEST, B64U.encode(&serde_json::to_vec(&signed)?))), None).await,
    }
}

/// The invites this device made that still work, by who they're for: an error if what it keeps of them can't be read,
/// which the next invite would otherwise write over.
async fn invites(m: &impl Me) -> Result<BTreeMap<String, Invited>> {
    let Some(kept) = m.kept(INVITES).await else { return Ok(BTreeMap::new()) };
    let kept: BTreeMap<String, Invited> = serde_json::from_slice(&kept).map_err(|err| anyhow!("The invites this device made can't be read ({INVITES}: {err}), so none was changed."))?;
    let now = m.now();
    Ok(kept.into_iter().filter(|(_, i)| i.until > now).collect())
}

/// Whether an invite could let someone in with no tap now: one by email not yet used, or a link this device made, each
/// still in date. With none, asking auth who waits finds no one this device lets in by itself.
pub async fn invite_open(m: &impl Me) -> bool {
    let now = m.now();
    m.who(|who| Ok(who.email_invites()?.iter().any(|i| !i.used && i.until > now))).unwrap_or(true) || invites(m).await.map_or(true, |i| !i.is_empty())
}

async fn keep_invites(m: &impl Me, invites: &BTreeMap<String, Invited>) -> Result<()> {
    m.keep(INVITES, &serde_json::to_vec(invites)?).await
}

/// An invite link for someone called `name`, as `role`, made by this admin's device: its secret stays here, to check
/// the newcomer's device by when it asks to be let in; auth is told only its hash. A new user gets a new id; `user`
/// invites someone who's in the network already, as who they are. Where the network has providers, `email` is an
/// invite with no secret: it's written where every device reads it, and they sign in with that email at the network's
/// address and go straight in (`Joining::New`). → `{ link, until }`, with `email` beside them if it's that: `link` is what
/// to send them either way.
async fn invite(m: &impl Me, name: &str, role: Role, user: Option<&str>, email: Option<&str>) -> Result<Value> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Give their name.");
    ensure!(::custom_build::config::JOIN_DOMAIN.is_empty() || email.is_some(), "Users here sign in with an account: give the one they sign up with.");
    let secret = B64U.encode(&rand::random::<[u8; 32]>());
    let new = user.is_none();
    let user = user.map(str::to_string).unwrap_or_else(scopes::new_id);
    let until = m.now() + crate::sign_in_proof::INVITE_DAYS * DAY;
    let mut fields = json!({ "id": hex(&::key::sha256(secret.as_bytes())), "name": name, "user": user, "until": until });
    if let Some(email) = email {
        fields["email"] = json!(email);
    }
    let mut kept = invites(m).await?;
    let told = ask_auth(m, "POST", "/api/auth/invites", fields).await?;
    if let (true, Some(email), true) = (new, email, m.who(|who| Ok(who.setting(sign_in_proof::CLIENTS)?.is_some_and(|c| !c.trim().is_empty())))?) {
        let user = told["user"].as_str().unwrap_or(&user);
        m.write(content::invite_email(user, email, name, role, until, &provider_keys(m).await)).await?;
        return Ok(json!({ "email": email, "link": m.address().unwrap_or_default(), "until": until }));
    }
    if new {
        kept.insert(user, Invited { secret: secret.clone(), name: name.to_string(), role, until });
        keep_invites(m, &kept).await?;
    }
    Ok(json!({ "link": format!("{}/invite#{secret}", m.address().unwrap_or_default()), "until": until }))
}

/// The keys the network's providers sign ID tokens with now, for an invite to name: none that can't be fetched, so only
/// an admin's device lets its newcomer in.
async fn provider_keys(m: &impl Me) -> Vec<Value> {
    let (Some(address), Ok((Some(root), clients))) = (m.address(), m.who(|who| Ok((who.root()?, who.setting(sign_in_proof::CLIENTS)?)))) else { return Vec::new() };
    let network = sign_in_proof::Network::new(&address, *root.as_bytes(), clients.as_deref());
    let mut keys = Vec::new();
    for (issuer, _) in &network.clients {
        let Some(at) = sign_in_proof::discovery(issuer, &network) else { continue };
        let Some(jwks) = m.issuer_keys(&at).await.and_then(|d| d["jwks_uri"].as_str().map(str::to_string)) else { continue };
        keys.extend(m.issuer_keys(&jwks).await.and_then(|k| k["keys"].as_array().cloned()).unwrap_or_default());
    }
    keys
}

/// Who a device waiting is, and so who may say yes.
pub enum Joining {
    /// One of this user's own: `proved` if its sign-in, checked here, is one they've signed in with, which only they
    /// could have made for that key. One that isn't is checked by its whole code, which can't be found to match.
    Own { proved: bool },
    /// Someone new, from an invite link this device made: the link is the approval, unless their sign-in was finished
    /// from another link, which could be anyone's who sent it.
    Link(Invited),
    /// Someone new: named by an invite, or asking from the network's domain, whom an admin may let in. `invite` is what
    /// lets them in with no tap, when an invite named them and they signed in on the device asking: by any device of the
    /// network, when the key that signed it is one the invite named.
    New { name: String, role: Role, invite: Option<InviteProof> },
    /// Someone in the network with no device of theirs left.
    Lost,
}

/// An ID token for an invited email, and the key its issuer signed it with.
pub struct InviteProof {
    token: String,
    jwk: Value,
}

/// Devices waiting that this device may let in: its user's own; someone new from an invite made here, whose proof is
/// that invite's; someone an invite names, signed in with a key it named; and for an admin, any other an invite names,
/// someone asking from the network's domain, or someone with no device left, each as their sign-in, checked here,
/// shows. Never those taken out or turned down.
async fn waiting(m: &impl Me) -> Result<Vec<(Asked, Joining)>> {
    let user = m.user()?;
    let admin = m.who(|who| Ok(who.may(&user, "invite", scopes::NETWORK)))?;
    let asked = asked(m).await?;
    let invites = invites(m).await?;
    let accounts = m.who(|who| who.sign_ins_ever(&user))?;
    let mut out = Vec::new();
    for r in asked {
        let gone = m.who(|who| Ok(who.key_for(&r.endpoint.key, &r.user)?.is_some_and(|d| d.removed)))?;
        if r.endpoint.key == m.public() || r.said["service"].is_string() || gone {
            continue;
        }
        if r.user == user {
            let proved = vouched(m, &r, &accounts).await;
            out.push((r, Joining::Own { proved }));
        } else if let Some(invite) = invites.get(&r.user).filter(|i| r.said["proof"].as_str() == Some(&::key::invite_proof(i.secret.as_bytes(), &r.endpoint))) {
            let invite = invite.clone();
            out.push((r, Joining::Link(invite)));
        } else if let Some(joining) = newcomer(m, &r, admin).await {
            out.push((r, joining));
        }
    }
    Ok(out)
}

/// Whether `r`'s sign-in, checked here, is one of `accounts` and names its key: a provider's alone, as a passkey's public
/// key is auth's word, so a device showing only that is approved by its whole code.
async fn vouched(m: &impl Me, r: &Asked, accounts: &[Account]) -> bool {
    let Ok(proof) = serde_json::from_value::<Proof>(r.said["sign_in"].clone()) else { return false };
    if proof.id_token().is_none() {
        return false;
    }
    let Some((network, jwks)) = checked_against(m, &proof).await else { return false };
    sign_in_proof::vouches(&proof, &r.endpoint, &network, jwks.as_ref(), accounts).is_ok()
}

/// Who `r` is, someone else's device, if their sign-in, checked here, shows this device may answer them: to any device,
/// someone an invite names, signed in with a key the invite named; to an admin's, anyone who may ask at all.
async fn newcomer(m: &impl Me, r: &Asked, admin: bool) -> Option<Joining> {
    let proof: Proof = serde_json::from_value(r.said["sign_in"].clone()).ok()?;
    let (ever, invite, none_left, accounts, listed) = m
        .who(|who| {
            let none_left = !who.clients()?.iter().any(|c| c.user == r.user && c.key != r.endpoint.key && c.passkey.is_none());
            let listed = who.client(&r.endpoint.key)?.is_some_and(|c| c.user == r.user && c.seal == r.endpoint.seal);
            Ok((who.user_ever(&r.user)?.is_some(), who.email_invite(&r.user)?, who.user(&r.user)?.is_some() && none_left, who.sign_ins_ever(&r.user)?, listed))
        })
        .ok()?;
    let (network, jwks) = checked_against(m, &proof).await?;
    if ever && !listed {
        sign_in_proof::vouches(&proof, &r.endpoint, &network, jwks.as_ref(), &accounts).ok()?;
        return (admin && none_left).then_some(Joining::Lost);
    }
    // Listed already: an admin's approval that the relay didn't hear of, so it's the same answer, for approving again to
    // finish it.
    let Some(token) = proof.id_token() else { return (admin && listed).then_some(Joining::Lost) };
    let named = invite.as_ref().and_then(|i| sign_in_proof::signing_key(token, &json!({ "keys": i.keys })).ok());
    let jwk = match named {
        Some(jwk) => jwk,
        None if admin => sign_in_proof::signing_key(token, jwks.as_ref()?).ok()?,
        None => return None,
    };
    let shown = sign_in_proof::id_token(token, &r.endpoint, &network, &jwk).ok()?;
    let sign_in_proof::Account::Provider { email: Some(email), .. } = &shown.account else { return (admin && listed).then_some(Joining::Lost) };
    let email = email.to_lowercase();
    if let Some(invite) = invite.filter(|i| shown.email_verified && i.email == email && (listed || !i.used) && shown.issued <= i.until) {
        // A sign-in finished from a link could be anyone's who sent it, so it waits for a tap, an admin's.
        let here = matches!(proof, Proof::IdToken(_));
        return (admin || here).then(|| Joining::New { name: invite.name, role: invite.role, invite: here.then(|| InviteProof { token: token.to_string(), jwk }) });
    }
    if !admin {
        return None;
    }
    let domain = ::custom_build::config::JOIN_DOMAIN;
    if !domain.is_empty() && shown.email_verified && email.ends_with(&format!("@{domain}")) {
        return Some(Joining::New { name: text(&r.said, "user_name"), role: Role::Member, invite: None });
    }
    listed.then_some(Joining::Lost)
}

/// Every request to join auth lists to this device.
async fn asked(m: &impl Me) -> Result<Vec<Asked>> {
    let asked = ask_auth(m, "GET", "/api/auth/devices/requests", json!({})).await?;
    Ok(asked["requests"].as_array().into_iter().flatten().filter_map(Asked::of).collect())
}

/// A code as typed: its letters and digits.
fn typed(code: &str) -> String {
    code.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// The first of a whole code, compared where a sign-in names its device.
fn short_code(whole: &str) -> &str {
    &whole[..::key::JOIN_CODE_SHORT]
}

/// What of a waiting device's code has to match to let it in.
fn code_needed<'a>(r: &'a Asked, joining: &Joining) -> &'a str {
    match joining {
        Joining::Own { proved: false } | Joining::Lost => &r.code,
        _ => short_code(&r.code),
    }
}

/// The one of `found` whose code is `code`, as typed, or `needed`, as much of it as counts: none if two are, as a device
/// made to show another's code would be.
fn the_one<T>(found: impl IntoIterator<Item = T>, whole: impl Fn(&T) -> &str, needed: impl Fn(&T) -> &str, code: &str) -> Result<T> {
    let code = typed(code);
    let mut found: Vec<T> = found.into_iter().filter(|r| code == whole(r) || code == needed(r)).collect();
    match found.len() {
        0 => bail!("No device is waiting with that code. Check it, or start again on that device."),
        1 => Ok(found.remove(0)),
        _ => bail!("Two devices are waiting with that code, so neither was let in. Start again on yours, for a new code."),
    }
}

/// The one waiting with `code` that this device may let in.
async fn waiting_with(m: &impl Me, code: &str) -> Result<(Asked, Joining)> {
    let found = waiting(m).await?;
    ensure!(!found.iter().any(|(r, j)| code_needed(r, j) == r.code && short_code(&r.code) == typed(code)), "That device needs every letter of the code it shows.");
    the_one(found, |(r, _)| &r.code, |(r, j)| code_needed(r, j), code)
}

/// The one asking with `code`, whoever's it is: for the network's recovery key, which lets in anyone's.
pub async fn asked_with(m: &impl Me, code: &str) -> Result<Asked> {
    the_one(asked(m).await?, |r| &r.code, |r| &r.code, code)
}

/// Marks a request approved at auth, for the device waiting on it to see, with the network it's in (its root), the
/// only one that device then takes, and where this device read who's who to, which that device starts from.
pub async fn mark_approved(m: &impl Me, key: &[u8; 32]) -> Result<()> {
    let root = m.who(|who| who.root())?.ok_or_else(|| Coded::error(code::NOT_LET_IN, "This device hasn't been let in yet."))?;
    let welcome = m.welcome().map(|w| format!("&welcome={w}")).unwrap_or_default();
    ask_auth(m, "DELETE", &format!("/api/auth/devices/requests/{}?approved&root={}{welcome}", hex(key), root.to_hex()), json!({})).await.map(|_| ())
}

/// What a device waiting is called, as its card says.
fn who_joins(r: &Asked, joining: &Joining) -> Value {
    match joining {
        Joining::Link(i) => json!(i.name),
        Joining::New { name, .. } => json!(name),
        Joining::Own { .. } | Joining::Lost => r.said["user_name"].clone(),
    }
}

/// Devices waiting that this device may let in, as Settings › Devices shows them, each with its code and what
/// approving it records as what they sign in with. Someone an invite already said yes to is let in, not shown.
pub async fn requests(m: &impl Me) -> Result<Value> {
    let mut out = Vec::new();
    for (r, joining) in waiting(m).await? {
        if said_yes(&r, &joining) {
            match approve(m, &r, &joining).await {
                Ok(_) => continue,
                Err(err) => tracing::warn!("letting {} in by their invite: {err:#}", r.name),
            }
        }
        let mut shown = json!({ "device": r.name, "user": who_joins(&r, &joining), "code": ::key::spaced_code(code_needed(&r, &joining)), "made": r.said["made"], "place": r.said["place"], "platform": r.said["platform"] });
        if r.said["by_link"] == true {
            shown["by_link"] = json!(true);
        }
        let replaces = replaced(m.who(|who| who.clients())?, &r);
        if !replaces.is_empty() {
            shown["replaces"] = json!(replaces.iter().map(|c| &c.name).collect::<Vec<_>>());
        }
        shown[match joining {
            Joining::Own { .. } => "yours",
            Joining::Lost => "lost",
            Joining::Link(_) | Joining::New { .. } => "new",
        }] = json!(true);
        if let Some(account) = signed_in_with(m, &r).await {
            shown["sign_in"] = json!(account);
        }
        out.push(shown);
    }
    Ok(json!(out))
}

/// Whether an invite already said yes to `r`, so nobody is asked: only auth saying its sign-in wasn't finished from a link.
fn said_yes(r: &Asked, joining: &Joining) -> bool {
    match joining {
        Joining::New { invite, .. } => invite.is_some(),
        Joining::Link(_) => r.said["by_link"] == false,
        _ => false,
    }
}

/// The sign-ins letting `r` in takes out: its user's others on the device it says it's on. Auth passes that on unchecked,
/// so every card that can let it in names them.
fn replaced(clients: Vec<scopes::Machine>, r: &Asked) -> Vec<scopes::Machine> {
    let on = device_of(r);
    clients.into_iter().filter(|c| on.is_some() && c.user == r.user && c.device == on && c.key != r.endpoint.key && c.passkey.is_none()).collect()
}

/// The device `r` says it's on, if it says.
fn device_of(r: &Asked) -> Option<[u8; 32]> {
    r.said["device"].as_str().and_then(|d| ::key::unhex(d).ok())
}

/// What `r`'s sign-in shows it signed in with, checked here (`sign_in_proof`). None if it shows nothing that checks out.
async fn signed_in_with(m: &impl Me, r: &Asked) -> Option<Account> {
    let proof = serde_json::from_value(r.said["sign_in"].clone()).ok()?;
    proved(m, &proof, &r.endpoint).await
}

/// A further identity this device linked: what auth kept of the sign-in that named it, checked here as any is. Auth
/// could have made it, so it's shown first, and written where the user's own devices count it only once they say so
/// (`confirm`). → `{ account }` to show, `{ linked: true }` once written, `{ linked: false }` if there's nothing new.
async fn record_sign_in(m: &impl Me, confirm: bool) -> Result<Value> {
    m.read().await?;
    let user = m.user()?;
    let said = ask_auth(m, "GET", "/api/auth/devices/proof", json!({})).await?;
    let Some(shown) = said["proofs"].as_array().into_iter().flatten().find(|p| p["user"].as_str() == Some(&user)) else { return Ok(json!({ "linked": false })) };
    let proof = serde_json::from_value(shown["proof"].clone()).map_err(|_| anyhow!("That sign-in can't be read."))?;
    let me = Endpoint { key: m.public(), seal: m.seal() };
    let account = proved(m, &proof, &me).await.ok_or_else(|| anyhow!("That sign-in couldn't be checked. Link it again."))?;
    if m.who(|who| who.sign_ins(&user))?.iter().any(|a| a.same(&account)) {
        return Ok(json!({ "linked": false }));
    }
    if !confirm {
        return Ok(json!({ "account": account }));
    }
    m.write(content::link_sign_in(&me.key, &account)).await?;
    Ok(json!({ "linked": true }))
}

/// Forgets an account a device of this user's signed in with: on every client they've ever had that has it, so it counts
/// for nothing any more, and at auth, so it signs no one in. Auth keeps someone's last way to sign in.
async fn forget_sign_in(m: &impl Me, device: &str, account: Option<&Account>) -> Result<()> {
    let user = m.user()?;
    let (account, forgets) = m.who(|db| {
        let key = db.find_machine(device)?;
        let mine = db.machine(&key)?.filter(|d| d.user == user);
        let account = match account {
            Some(account) => db.linked_sign_ins(&key)?.into_iter().find(|a| a == account),
            None => mine.as_ref().and_then(|d| d.sign_in.clone()),
        };
        let account = account.filter(|_| mine.is_some()).ok_or_else(|| anyhow!("That isn't one of your devices with a sign-in to forget."))?;
        let mut forgets = Vec::new();
        for c in db.clients_ever(&user)? {
            if c.sign_in.as_ref().is_some_and(|a| a.same(&account)) {
                forgets.push((c.key, None));
            }
            forgets.extend(db.linked_sign_ins(&c.key)?.into_iter().filter(|a| a.same(&account)).map(|a| (c.key, Some(a))));
        }
        Ok((account, forgets))
    })?;
    for (key, linked) in &forgets {
        m.write(content::forget_sign_in(key, linked.as_ref())).await?;
    }
    if let Err(err) = ask_auth(m, "POST", "/api/auth/devices/forget-sign-in", json!({ "user": user, "account": account })).await {
        tracing::warn!("forgetting a sign-in at auth: {err:#}");
    }
    Ok(())
}

/// The network as this device reads it, and for an ID token, its issuer's keys: none if they can't be fetched now.
async fn checked_against(m: &impl Me, proof: &Proof) -> Option<(sign_in_proof::Network, Option<Value>)> {
    let address = m.address()?;
    let (root, clients) = m.who(|who| Ok((who.root()?, who.setting(sign_in_proof::CLIENTS)?))).ok()?;
    let network = sign_in_proof::Network::new(&address, *root?.as_bytes(), clients.as_deref());
    let jwks = match proof.issuer().and_then(|i| sign_in_proof::discovery(&i, &network)) {
        Some(at) => Some(m.issuer_keys(m.issuer_keys(&at).await?["jwks_uri"].as_str()?).await?),
        None => None,
    };
    Some((network, jwks))
}

/// What `proof` shows `device` signed in with, checked here.
async fn proved(m: &impl Me, proof: &Proof, device: &Endpoint) -> Option<Account> {
    let (network, jwks) = checked_against(m, proof).await?;
    sign_in_proof::check(proof, device, &network, jwks.as_ref()).ok()
}

/// Without `confirm`, whose the device showing `code` is, and what it signed in with; with, it's let in, its request
/// marked approved at auth.
async fn approve_code(m: &impl Me, code: &str, confirm: bool) -> Result<Value> {
    let (r, joining) = waiting_with(m, code).await?;
    if !confirm {
        let sign_in = signed_in_with(m, &r).await;
        let replaces: Vec<String> = replaced(m.who(|who| who.clients())?, &r).into_iter().map(|c| c.name).collect();
        return Ok(json!({ "device": r.name, "user": who_joins(&r, &joining), "yours": matches!(joining, Joining::Own { .. }), "new": matches!(joining, Joining::Link(_) | Joining::New { .. }), "sign_in": sign_in, "replaces": replaces, "by_link": r.said["by_link"] == true }));
    }
    approve(m, &r, &joining).await
}

/// Whether what `POST /api/devices/approve` said of a device without `confirm` lets it in from its notification, with no
/// card read: only one of this user's own, taking none out, that signed in on itself. A sign-in finished from a link may be
/// someone else's device, so it waits for the card that says so.
pub fn approves_from_a_note(about: &Value) -> bool {
    about["yours"] == true && about["by_link"] == false && about["replaces"].as_array().is_none_or(|r| r.is_empty())
}

/// Lets in a device that asked: lists it (and for a newcomer, them), with what it signed in with, the device it's
/// on and where it asked from; seals it each epoch this device opens of the scopes its user uses; takes their
/// sign-in before on the same device out; lets it onto the network; and marks its request approved with where this
/// device read who's who to, which the device waiting starts from. → `{ device, sealed, not_sealed }`.
pub async fn approve(m: &impl Me, r: &Asked, joining: &Joining) -> Result<Value> {
    m.read().await?;
    let user = m.user()?;
    let e = &r.endpoint;
    let device = device_of(r);
    let (removed, listed, may_invite) = m.who(|who| {
        let removed = who.key_for(&e.key, &r.user)?.is_some_and(|d| d.removed);
        let listed = who.client(&e.key)?.is_some_and(|c| c.user == r.user && c.seal == e.seal);
        Ok((removed, listed, who.may(&user, "invite", scopes::NETWORK)))
    })?;
    // Taken out, or turned down: never sealed anything again, whatever an app runtime says.
    ensure!(!removed, "{} was turned down or taken out, so it isn’t let in again. Start again on it, for a new code.", r.name);
    let new_user = match joining {
        Joining::Own { .. } => {
            ensure!(r.user == user, "{} is someone else's: only their own devices approve one of theirs.", r.name);
            None
        }
        Joining::Lost => {
            allowed!(may_invite, "Only an admin lets someone back in.");
            None
        }
        Joining::Link(i) => {
            allowed!(may_invite, "Only an admin lets someone new in.");
            ensure!(r.said["proof"].as_str() == Some(&::key::invite_proof(i.secret.as_bytes(), e)), "{}'s proof isn't from {}'s invite.", r.name, i.name);
            Some((i.name.clone(), i.role))
        }
        Joining::New { name, role, invite } => {
            allowed!(may_invite || invite.is_some(), "Only an admin lets someone new in.");
            Some((name.clone(), *role))
        }
    };
    if !listed {
        let mut listing = content::device(e, &r.user, &r.name, &[Kind::Client], new_user.as_ref().map(|(n, role)| (n.as_str(), *role)));
        if let Some(on) = &device {
            listing.data["device"] = json!(hex(on));
        }
        if let Joining::New { invite: Some(proof), .. } = joining {
            listing.data["invite"] = json!({ "token": proof.token, "jwk": proof.jwk });
        } else if let Some(account) = signed_in_with(m, r).await {
            listing.data["sign_in"] = json!(account);
        }
        if r.said["place"].is_string() {
            listing.data["place"] = r.said["place"].clone();
        }
        // What it said it is, kept out if it isn't a platform: a word from a device not yet let in doesn't refuse its listing.
        if r.said["platform"].as_str().is_some_and(|p| crate::rules::PLATFORMS.contains(&p)) {
            listing.data["platform"] = r.said["platform"].clone();
        }
        m.write(listing).await?;
        m.read().await?;
    }
    // Someone new's own scope is theirs: their device makes its first epoch. Someone with no device left opens theirs
    // again only with what restores them (dev/plans/plan-auth.md, step 6): an admin holds no key to it.
    let own_too = matches!(joining, Joining::Own { .. });
    let (scopes, admin, before) = m.who(|who| {
        let own = Scope::User(r.user.clone());
        let scopes = who.scopes()?.into_iter().filter(|s| who.may(&r.user, "use", &scopes::object(s)) && (own_too || *s != own)).collect::<Vec<_>>();
        let before: Vec<[u8; 32]> = replaced(who.clients()?, r).into_iter().map(|c| c.key).collect();
        // The relay takes an admin's device only from an admin's: one let in by a member's is an admin there once an
        // admin's device tells it whose are (`tell_roles`).
        Ok((scopes, may_invite && who.may(&r.user, "invite", scopes::NETWORK), before))
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
        mark_approved(m, &e.key).await?;
    }
    if matches!(joining, Joining::Link(_)) {
        let mut kept = invites(m).await?;
        kept.remove(&r.user);
        keep_invites(m, &kept).await?;
    }
    Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not_sealed }))
}

/// Restores someone who lost every device, on the device of someone holding the recovery key: opens this device's own
/// copy of it, and with that each epoch of each scope the device showing `code`'s user may use, sealed to that device.
/// Only someone holding it can, as only their own copy opens it; an admin has already let the device in. `code` is the
/// whole of its code, from its own screen. → whose it is without `confirm`; what was given it, with.
async fn restore(m: &impl Me, code: &str, confirm: bool) -> Result<Value> {
    let me = m.user()?;
    allowed!(m.who(|who| Ok(who.recovery_holders()?.contains(&me)))?, "Only someone named as holding the recovery key restores someone.");
    let listed = m.who(|who| Ok(who.clients()?.into_iter().filter(|c| c.passkey.is_none() && c.seal != c.key).map(|c| {
        let whole = ::key::join_code(&Endpoint { key: c.key, seal: c.seal });
        (c, whole)
    }).collect::<Vec<_>>()))?;
    let (target, _) = the_one(listed, |(_, whole)| whole, |(_, whole)| whole, code)?;
    let whose = m.who(|who| Ok(who.user(&target.user)?.map(|u| u.name).unwrap_or_default()))?;
    if !confirm {
        return Ok(json!({ "device": target.name, "user": whose, "said": format!("{} is {whose}'s device. Restore it only if they say so: it opens everything of theirs.", target.name) }));
    }
    let recovery = m.who(|who| who.recovery())?.ok_or_else(|| anyhow!("{YOURS} has no recovery key."))?;
    let to = hex(&m.public());
    let copy = m.stored(&database::keys::recovery_held_name(&hex(&recovery), &to)).await?.ok_or_else(|| anyhow!("This device doesn't have the recovery key: someone with its file seals it here first."))?;
    let sealed = ::key::held_sealed(&recovery, &to, &copy).ok_or_else(|| anyhow!("What's kept for this device isn't a copy the recovery key signed."))?;
    let secret: [u8; 32] = m.open(sealed).await?.try_into().map_err(|_| anyhow!("What's sealed to this device isn't the recovery key."))?;
    let signing = ed25519_dalek::SigningKey::from_bytes(&secret);
    ensure!(signing.verifying_key().to_bytes() == recovery, "What's sealed to this device isn't {YOURS}'s recovery key.");
    let (seal_secret, twin) = (::key::seal_secret(&signing), ::key::twin_public(&recovery)?);
    let scopes = m.who(|who| who.may_scopes(&target.user, "use"))?;
    let (mut given, mut not_given) = (Vec::new(), Vec::new());
    for scope in scopes {
        let name = scope.to_string();
        let mut opened = 0;
        let mut failed: Option<anyhow::Error> = None;
        for (n, hash) in m.who(|who| who.epoch_hashes(&scope))? {
            let mut kept = Vec::new();
            for at in database::keys::recovery_epoch_names(&name, n, &twin) {
                kept.extend(m.stored(&at).await?);
            }
            let Some(key) = ::key::opened_recovery_epoch(&seal_secret, &hash, &kept) else { continue };
            opened += 1;
            match m.keep_key(json!({ "scope": name, "n": n, "to": hex(&target.key), "sealed": B64U.encode(&::key::seal_to(&target.seal, &key)?) })).await {
                Err(err) if Coded::of(&err).as_deref() != Some(code::ALREADY) => {
                    failed = Some(err);
                    break;
                }
                _ => {}
            }
        }
        match failed {
            Some(err) => not_given.push(json!({ "scope": name, "why": format!("{err:#}") })),
            None if opened > 0 => given.push(name),
            None => {}
        }
    }
    let mut said = format!("{} has {whose}'s keys now.", target.name);
    for n in &not_given {
        said.push_str(&format!(" It wasn't given {}'s: {}.", n["scope"].as_str().unwrap_or(""), n["why"].as_str().unwrap_or("")));
    }
    Ok(json!({ "device": target.name, "user": whose, "given": given, "not_given": not_given, "said": said }))
}

/// Seals each epoch of `scope` sealed to this device to `to`, but those it has.
async fn seal_held(m: &impl Me, scope: &Scope, to: &Endpoint) -> Result<()> {
    for (n, key) in m.held(scope).await? {
        let body = json!({ "scope": scope.to_string(), "n": n, "to": hex(&to.key), "sealed": B64U.encode(&::key::seal_to(&to.seal, &key)?) });
        match m.keep_key(body).await {
            Err(err) if Coded::of(&err).as_deref() != Some(code::ALREADY) => return Err(err),
            _ => {}
        }
    }
    Ok(())
}

/// Turns down the device waiting with `code`: one of this user's own by a step, so none of their devices lets it in
/// after; either way its request is forgotten at auth. → which it was.
async fn refuse(m: &impl Me, code: &str) -> Result<Value> {
    // By its short code too: a refusal lets no one in.
    let (r, joining) = the_one(waiting(m).await?, |(r, _)| &r.code, |(r, _)| short_code(&r.code), code)?;
    if matches!(joining, Joining::Own { .. }) {
        m.write(content::refuse(&r.endpoint, &r.name)).await?;
    }
    ask_auth(m, "DELETE", &format!("/api/auth/devices/requests/{}", hex(&r.endpoint.key)), json!({})).await?;
    Ok(json!({ "device": r.name }))
}

/// Runs an app's schedules on a device for `days`, or with 0, stops them.
async fn app_schedules_asked(m: &impl Me, app: String, device: &str, scope: Option<&str>, days: u64) -> Result<Value> {
    let user = m.user()?;
    let given = scope.unwrap_or("user");
    let (endpoint, scope, name) = m.who(|who| {
        let endpoint = who.find_node(device)?;
        let scope = Scope::parse_for(given, &user).or_else(|_| who.find_group(given).map(Scope::Group))?;
        ensure!(who.app(&app)?.is_some(), "no app called {app}");
        Ok((endpoint, scope, who.machine(&endpoint)?.map(|d| d.name).unwrap_or_default()))
    })?;
    let until = set_app_schedules(m, &endpoint, &app, &scope, days).await?;
    Ok(match until {
        0 => json!({ "said": format!("{name} no longer runs {app}'s schedules, and what's written from now on is under new keys it doesn't have.") }),
        until => json!({ "said": format!("{name} runs {app}'s schedules for you, for {} days, and longer as you use it.", days.min(APP_SCHEDULES_DAYS)), "until": until }),
    })
}

/// Runs `app`'s schedules in `scope` on one of its user's devices (`endpoint`) for `days`, 30 at most (0 stops
/// them): the Vault's database's key in each epoch of the scope kept under the app's, and the app's sealed to it
/// first, as a call of the app is handed both (`keys::may_hand`), then the app schedules naming the hash of each app's
/// key. Stopping them starts a new epoch of the scope, which the device isn't sealed, so the keys it held open
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
            let vault = ::key::database_key(&m.app_key(scope, ::key::APP, n).await?, ::key::APP, ::key::CoreDb::Vault.name());
            let name = database::keys::app_schedules_vault_name(&scope.to_string(), app, n);
            m.keep_key(json!({ "scope": scope.to_string(), "n": n, "app": app, "vault": true, "sealed": B64U.encode(&::key::seal(n, &key, &name, &vault)) })).await?;
            m.keep_key(json!({ "scope": scope.to_string(), "n": n, "app": app, "to": hex(endpoint), "sealed": B64U.encode(&::key::seal_to(&seal, &key)?) })).await?;
            hashes.insert(n, ::key::epoch_hash(&key));
        }
    }
    m.write(content::app_schedules(endpoint, scope, app, until, &hashes)).await?;
    m.read().await?;
    if until == 0 {
        renew(m, std::slice::from_ref(scope), &[]).await?;
    }
    Ok(until)
}

/// Renews this user's app schedules of `app` as they use it: once a day at most, and at once when the scope has
/// an epoch their keys don't reach; one of theirs that ran out is stopped, as Stop does. Only those the network's
/// steps hold, never those an app runtime lists: a hacked one could list itself.
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

/// Whose a scope is, as a user reads it: everyone's, a group's or someone's, by name.
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

/// What letting a device in, or asking whose it is, says to a user (an approval's answer).
pub fn said_approved(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    if let Some(service) = r["service"].as_str() {
        return match r.get("approved") {
            Some(_) => format!("{service} on {} is in: it answers your devices' calls.", s("device")),
            None => format!("{service} on {} wants to be yours. Approve it only if that's the code it shows.", s("device")),
        };
    }
    if r.get("sealed").is_none() && r["recovery"] == true {
        return format!("{} is {}'s device. Approve it with the recovery key only if they lost all their devices and say it's theirs.", s("device"), s("user"));
    }
    if r.get("sealed").is_none() {
        let new = if r["new"] == true { ", who you invited" } else { "" };
        let with = serde_json::from_value::<Account>(r["sign_in"].clone()).map_or(String::new(), |a| match a {
            Account::Provider { email: Some(email), .. } => format!(", signed in as {email}"),
            Account::Provider { subject, .. } => format!(", signed in as {subject}"),
            Account::Passkey { .. } => ", signed in with a passkey".to_string(),
        });
        let replaces = r["replaces"].as_array().filter(|l| !l.is_empty()).map_or(String::new(), |l| format!(" It replaces {}.", l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
        return format!("{} is {}'s device{new}{with}. Approve it only if they say it's theirs.{replaces}", s("device"), s("user"));
    }
    let mut out = format!("{} is in.", s("device"));
    if r["recovery"] == true {
        out = format!("{} is in, let in with the recovery key: {} can open what they could before.", s("device"), s("user"));
    }
    if let Some(relay) = r["relay"].as_str() {
        out.push_str(&format!(" The network didn't take it yet: {relay}."));
    }
    for n in r["not_sealed"].as_array().into_iter().flatten() {
        out.push_str(&format!(" It wasn't given {}'s keys: {}.", n["scope"].as_str().unwrap_or(""), n["why"].as_str().unwrap_or("")));
    }
    out
}

fn said_removed(r: &Value) -> String {
    let mut out = format!("Removed {}.", r["removed"].as_str().unwrap_or("it"));
    let left: Vec<&str> = r["not_renewed"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !left.is_empty() {
        out.push_str(&format!(" New keys for {} come from one of their own devices, or their admins'.", left.join(", ")));
    }
    out
}

fn said_invited(name: &str, r: &Value) -> String {
    let days = crate::sign_in_proof::INVITE_DAYS;
    if let Some(email) = r["email"].as_str() {
        return format!("Send {name} {}: they sign in with {email}, for {days} days, and go straight in.", r["link"].as_str().unwrap_or(""));
    }
    let how = if ::custom_build::config::JOIN_DOMAIN.is_empty() { "they join" } else { "they sign in and join" };
    format!("Send {name} this link. It works once, for {days} days: opening it, {how}.\n\n  {}", r["link"].as_str().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use crate::settings::{Asked, Joining, code_needed, replaced, said_yes, short_code, the_one};
    use crate::scopes::{Kind, Machine};
    use ::key::hex;

    fn asked(key: u8, user: &str, device: Option<[u8; 32]>) -> Asked {
        let mut r = json!({ "key": hex(&[key; 32]), "seal": hex(&[key + 1; 32]), "user": user, "name": "MacBook Pro" });
        if let Some(on) = device {
            r["device"] = json!(hex(&on));
        }
        Asked::of(&r).unwrap()
    }

    fn client(key: u8, user: &str, device: Option<[u8; 32]>, passkey: Option<&str>) -> Machine {
        let roles = BTreeSet::from([Kind::Client]);
        Machine { key: [key; 32], seal: [key; 32], user: user.into(), name: format!("device {key}"), roles, passkey: passkey.map(str::to_string), platform: None, service: None, shared: false, device, by: user.into(), since: 0, sign_in: None, place: None, removed: false }
    }

    /// 40 bits can be found to match offline, so a device whose sign-in shows nothing is let in only by every letter of its
    /// code, and one whose sign-in names it by the short code; a note's code, cut to the short one, can't let in the first.
    #[test]
    fn a_device_that_cant_show_its_yours_needs_its_whole_code() {
        let r = asked(1, "a", None);
        let (whole, short) = (::key::join_code(r.endpoint()), short_code(&::key::join_code(r.endpoint())).to_string());
        assert_eq!(whole.len(), 16, "80 bits");
        let one = |joining: Joining, code: &str| the_one([(asked(1, "a", None), joining)], |(r, _)| r.code.as_str(), |(r, j)| code_needed(r, j), code).is_ok();
        assert!(one(Joining::Own { proved: true }, &short));
        assert!(one(Joining::Own { proved: true }, &whole));
        assert!(!one(Joining::Own { proved: false }, &short), "a short code is no check on one that shows nothing");
        assert!(one(Joining::Own { proved: false }, &whole));
        assert!(!one(Joining::Lost, &short), "someone with no device left shows no device of theirs to compare");
        assert!(one(Joining::Lost, &whole));
    }

    /// A notification's Approve lets in only one of the user's own that signed in on itself: what a link signed in, or what
    /// says nothing of how it did, waits for its card.
    #[test]
    fn a_sign_in_from_a_link_is_never_approved_from_a_notification() {
        let about = |yours: bool, by_link: serde_json::Value, replaces: serde_json::Value| json!({ "yours": yours, "by_link": by_link, "replaces": replaces });
        assert!(crate::settings::approves_from_a_note(&about(true, json!(false), json!([]))));
        assert!(!crate::settings::approves_from_a_note(&about(true, json!(true), json!([]))), "signed in from a link");
        assert!(!crate::settings::approves_from_a_note(&about(true, serde_json::Value::Null, json!([]))), "says nothing of how");
        assert!(!crate::settings::approves_from_a_note(&about(false, json!(false), json!([]))), "someone else's");
        assert!(!crate::settings::approves_from_a_note(&about(true, json!(false), json!(["old"]))), "takes one out");
    }

    /// An invite link lets a device in with no tap only when auth says its sign-in wasn't finished from a link: not when it
    /// says nothing.
    #[test]
    fn an_invite_link_goes_straight_in_only_if_auth_says_it_wasnt_by_link() {
        let invite = || Joining::Link(serde_json::from_value(json!({ "secret": "s", "name": "Sam", "role": "member", "until": u64::MAX })).unwrap());
        let told = |by_link: serde_json::Value| Asked::of(&json!({ "key": hex(&[1; 32]), "seal": hex(&[2; 32]), "user": "a", "name": "n", "by_link": by_link })).unwrap();
        assert!(said_yes(&told(json!(false)), &invite()));
        assert!(!said_yes(&told(json!(true)), &invite()));
        assert!(!said_yes(&asked(1, "a", None), &invite()), "a request that doesn't say goes through a tap");
    }

    /// Letting a device in takes out only its user's other sign-ins on the device it says it's on, each named on its card.
    #[test]
    fn only_that_users_sign_ins_on_that_device_are_replaced() {
        let (on, other) = ([7; 32], [8; 32]);
        let r = asked(1, "a", Some(on));
        let clients = vec![client(2, "a", Some(on), None), client(3, "a", Some(other), None), client(4, "b", Some(on), None), client(5, "a", Some(on), Some("cred")), client(6, "a", None, None), client(1, "a", Some(on), None)];
        assert_eq!(replaced(clients.clone(), &r).iter().map(|c| c.key[0]).collect::<Vec<_>>(), vec![2]);
        assert!(replaced(clients, &asked(1, "a", None)).is_empty(), "one that says nothing replaces nothing");
    }
}
