//! What a device changes of who can do what, signed by itself as its person's device, never by a node: Settings'
//! changes (roles, groups, handles, names, settings, removing a device or a person, the new epochs that go with it),
//! letting devices in or turning them down, inviting someone new, app schedules, passkeys, and leaving as it signs out.
//! Every client asks these as a node's routes of the same paths (`Keyring::answers`), and its keyring answers them.
//!
//! What it signs, it signs with what each client has (`keys::Signs`): a native app's key, or a browser's WebCrypto key,
//! which can't be read. Who may do what is judged by the steps' own rules wherever they're read (src/database/rules.rs):
//! what's checked here only says why sooner.

use std::collections::BTreeMap;

use data_encoding::BASE64URL_NOPAD as B64U;
use one::config::YOURS;
use one::database::content::{self, Content};
use one::database::scopes::{self, Kind, Role, Scope, ScopeDb};
use one::keys::{self, Endpoint, Opens, Signs, hex};
use one::protocol::code;
use serde_json::{Value, json};

use crate::keyring::{Address, Keep, Keyring, error, now, query, refused};
use crate::{Client, Error, unsent};

/// How long app schedules last, unless their person renews them as they use the app (src/node/app_schedules.rs).
const APP_SCHEDULES_DAYS: u64 = 30;
const DAY: u64 = 86_400_000;
const HOUR: u64 = 3_600_000;
/// The invites this device made, by who they're for: their secret, name, role and until when.
const INVITES: &str = "invites.json";

/// The paths of a node's routes a device answers itself (`Keyring::answer`), signing what they change: a node signs
/// what changes who can do what only for its own machine's programs.
pub const ANSWERED: &[&str] = &[
    "/v1/role", "/v1/remove", "/v1/settings", "/v1/group", "/v1/invite", "/v1/accept", "/v1/member", "/v1/handle", "/v1/name", "/v1/runs", "/v1/service", "/v1/retire", "/v1/add",
    "/v1/devices/forget-sign-in", "/v1/invite-link", "/v1/app-schedules", "/v1/devices/requests", "/v1/devices/approve", "/v1/devices/refuse", "/v1/devices/passkey", "/v1/devices/seen",
];

fn failed(message: impl std::fmt::Display) -> Error {
    unsent(message)
}

/// `body[k]` as text, "" if it isn't.
fn text(body: &Value, k: &str) -> String {
    body[k].as_str().unwrap_or("").to_string()
}

/// A request to join, as auth lists it.
struct Asked {
    said: Value,
    endpoint: Endpoint,
    user: String,
    name: String,
    /// "ABCD EFGH", as the device asking shows it.
    code: String,
}

impl Asked {
    fn of(r: &Value) -> Option<Asked> {
        let endpoint = Endpoint { key: keys::unhex(r["key"].as_str()?).ok()?, seal: keys::unhex(r["seal"].as_str()?).ok()? };
        let code = keys::join_code(&endpoint);
        Some(Asked { said: r.clone(), endpoint, user: text(r, "user"), name: text(r, "name"), code: format!("{} {}", &code[..4], &code[4..]) })
    }
}

/// An invite this device made: the newcomer's first device brings the proof its secret makes with its key.
#[derive(Clone)]
struct Invited {
    secret: String,
    name: String,
    role: Role,
    until: u64,
}

impl Invited {
    fn of(v: &Value) -> Option<Invited> {
        Some(Invited { secret: v["secret"].as_str()?.into(), name: text(v, "name"), role: serde_json::from_value(v["role"].clone()).unwrap_or(Role::Member), until: v["until"].as_u64()? })
    }

    fn kept(&self) -> Value {
        json!({ "secret": self.secret, "name": self.name, "role": self.role, "until": self.until })
    }
}

impl<O: Opens> Keyring<O> {
    /// The answer to a request this device answers itself, as a node's route of the same path would (`/v1/role`,
    /// `/v1/remove`, `/v1/devices/approve`, …): none for any other, which goes to a node.
    pub(crate) async fn answer(&self, client: &Client, on: &(impl Address + Keep + Signs), method: &str, path: &str, body: &[u8]) -> Option<Result<Value, Error>> {
        let path = path.split('?').next().unwrap_or(path);
        if !ANSWERED.contains(&path) {
            return None;
        }
        let body: Value = if body.is_empty() { json!({}) } else { serde_json::from_slice(body).unwrap_or(Value::Null) };
        // The network's recovery key is used only on the machine it's kept on, by its node.
        if body.get("recovery").is_some() {
            return None;
        }
        // What a request names (a group, a device) may be newer than this device last read: it reads what's new first.
        if let Err(err) = self.read(on).await {
            return Some(Err(err));
        }
        let w = |c: Content| self.write(client, on, c);
        let done = |r: Result<(), Error>| r.map(|()| json!({}));
        Some(match (method, path) {
            ("POST", "/v1/role") => match (self.find(|db| db.find_user(&text(&body, "user"))), serde_json::from_value::<Role>(body["role"].clone())) {
                (Ok(user), Ok(role)) => done(w(content::role(&user, role)).await),
                (Err(err), _) => Err(err),
                (_, Err(_)) => Err(failed("A role is admin or member.")),
            },
            ("POST", "/v1/remove") if body["user"].is_string() => self.remove_person(client, on, &text(&body, "user")).await,
            ("POST", "/v1/remove") => self.remove(client, on, &text(&body, "key")).await,
            ("POST", "/v1/settings") => done(w(content::setting(&text(&body, "name"), body["value"].as_str())).await),
            ("POST", "/v1/group") => self.group(client, on, &text(&body, "name")).await,
            ("POST", "/v1/invite") => match self.find(|db| Ok((db.find_group(&text(&body, "group"))?, db.find_user(&text(&body, "user"))?))) {
                Ok((group, user)) => done(w(content::invite(&group, &user, serde_json::from_value(body["role"].clone()).unwrap_or(Role::Member))).await),
                Err(err) => Err(err),
            },
            ("POST", "/v1/accept") => match self.find(|db| db.find_group(&text(&body, "group"))) {
                Ok(group) => done(w(content::accept(&group)).await),
                Err(err) => Err(err),
            },
            ("POST", "/v1/member") => match self.find(|db| Ok((db.find_group(&text(&body, "group"))?, db.find_user(&text(&body, "user"))?))) {
                Ok((group, user)) => done(w(content::member(&group, &user, serde_json::from_value(body["role"].clone()).ok())).await),
                Err(err) => Err(err),
            },
            ("POST", "/v1/handle") => {
                let handle = text(&body, "handle").trim().to_string();
                let of = match body["group"].as_str() {
                    Some(g) => self.find(|db| db.find_group(g)).map(|g| ("group", g)),
                    None => self.user().ok_or_else(refused).map(|u| ("user", u)),
                };
                match of {
                    Ok((what, id)) => done(w(content::handle(what, &id, &handle)).await),
                    Err(err) => Err(err),
                }
            }
            ("POST", "/v1/name") => {
                let key = match body["key"].as_str() {
                    Some(k) => self.find(|db| db.find_machine(k)),
                    None => Ok(on.public()),
                };
                match key {
                    Ok(key) => done(w(content::rename("key", &hex(&key), text(&body, "name").trim())).await),
                    Err(err) => Err(err),
                }
            }
            ("POST", "/v1/runs") => {
                let on_: Result<Vec<_>, _> = body["on"].as_array().into_iter().flatten().map(|d| self.find(|db| db.find_machine(d.as_str().unwrap_or("")))).collect();
                match on_ {
                    Ok(machines) => done(w(content::runs(&machines)).await),
                    Err(err) => Err(err),
                }
            }
            ("POST", "/v1/service") => match self.find(|db| db.find_machine(&text(&body, "device"))) {
                Ok(device) => done(w(content::service(&device, &text(&body, "name"), body["on"] == true, None)).await),
                Err(err) => Err(err),
            },
            ("POST", "/v1/retire") => {
                let app = text(&body, "app");
                let owner = self.find(|db| db.app(&app)?.map(|a| a.owner.to_string()).ok_or_else(|| anyhow::anyhow!("no app named {app}")));
                match owner.and_then(|o| content::retire(&app, &o, &text(&body, "how")).map_err(failed)) {
                    Ok(c) => done(w(c).await),
                    Err(err) => Err(err),
                }
            }
            // An app on everyone's page is a change to who can do what; on yours or a group's, a node writes it.
            ("POST", "/v1/add") if body["scope"] == "global" => done(w(content::added(&text(&body, "app"), body["added"] != false)).await),
            ("POST", "/v1/devices/forget-sign-in") => match self.find(|db| db.find_machine(&text(&body, "key"))) {
                Ok(key) => done(w(content::forget_sign_in(&key)).await),
                Err(err) => Err(err),
            },
            ("POST", "/v1/invite-link") => self.invite(on, &text(&body, "name"), if body["admin"] == true { Role::Admin } else { Role::Member }).await,
            ("POST", "/v1/app-schedules") => self.app_schedules_asked(client, on, &body).await,
            ("GET", "/v1/devices/requests") => self.requests(client, on).await,
            ("POST", "/v1/devices/approve") => self.approve_code(client, on, &text(&body, "code"), body["confirm"] == true).await,
            ("POST", "/v1/devices/refuse") => self.refuse(client, on, &text(&body, "code")).await,
            ("POST", "/v1/devices/passkey") => match B64U.decode(text(&body, "prf").trim_end_matches('=').as_bytes()) {
                Ok(prf) => done(self.list_passkey(client, on, &prf, &text(&body, "credential")).await),
                Err(_) => Err(failed("Sign in with your passkey first.")),
            },
            ("GET", "/v1/devices/seen") => self.seen(on).await,
            _ => return None,
        })
    }

    /// Something found in who's who as it last read it, by `f`.
    fn find<R>(&self, f: impl FnOnce(&ScopeDb) -> anyhow::Result<R>) -> Result<R, Error> {
        self.who(f).ok_or_else(refused)?.map_err(failed)
    }

    /// Signs a step of `content` in `global`'s scope database for the next position (`one::database::steps`), and
    /// writes it, signing it again for the one after while another device's goes first.
    pub(crate) async fn write(&self, client: &Client, on: &(impl Address + Keep + Signs), content: Content) -> Result<(), Error> {
        let user = self.user().ok_or_else(refused)?;
        let newest = format!("/v1/steps/global?from={}", one::database::steps::NUMBER_MAX);
        let mut steps = self.asked(client, on, "GET", &newest, None).await?;
        for _ in 0..5 {
            let (at, epoch) = (steps["at"].as_u64().unwrap_or(0), steps["epoch"].as_u64().unwrap_or(0) as u32);
            let step = one::database::steps::sign_by(on, &Scope::Global, at + 1, &user, epoch, now(), content.clone()).await.map_err(failed)?;
            match self.asked(client, on, "POST", "/v1/steps", Some(json!([step.to_json()]))).await {
                Ok(_) => return Ok(()),
                Err(err) if err.code.as_deref() == Some(code::TAKEN) => steps = self.asked(client, on, "GET", &newest, None).await?,
                Err(err) => return Err(err),
            }
        }
        Err(failed("Who's who changed too often to write to: try again."))
    }

    /// A group called `name`, with this person its first admin: → `{ group }`, its id.
    async fn group(&self, client: &Client, on: &(impl Address + Keep + Signs), name: &str) -> Result<Value, Error> {
        let name = name.trim();
        if name.is_empty() {
            return Err(failed("Give the group a name."));
        }
        let group = scopes::new_id();
        self.write(client, on, content::group(&group, name)).await?;
        Ok(json!({ "group": group }))
    }

    /// As each start of a client does: says where its person's own things run (the machines that run everyone's), if
    /// they haven't; names the device it's on in its listing, if it was listed before sign-ins named it (`on_device`);
    /// and makes their own scope's first epoch, if it has none. What's to do is looked at again in what it reads fresh,
    /// as another device of theirs may have done it.
    pub async fn settle_own(&self, client: &Client, on: &(impl Address + Keep + Signs), on_device: Option<&str>) -> Result<(), Error> {
        let me = on.public();
        let todo = || {
            self.find(|who| {
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
        self.read(on).await?;
        let Some((user, runs, unnamed, first)) = todo()? else { return Ok(()) };
        if let (Some(device), Some((name, roles))) = (on_device, unnamed) {
            let mut listed = content::device(&Endpoint { key: me, seal: self.opens().public() }, &user, &name, &roles, None);
            listed.data["device"] = json!(device);
            self.write(client, on, listed).await?;
        }
        if !runs.is_empty() {
            self.write(client, on, content::runs(&runs)).await?;
        }
        if first {
            self.new_epoch(client, on, &Scope::User(user), 1, &[]).await?;
        }
        Ok(())
    }

    /// Starts epoch `n` of a scope: a random key, its `epoch` step naming its hash written first (which, first in causal
    /// order, makes it), then, once who's who says it did, sealed to the recovery key and to each client device of the
    /// people who open it but `except` (keys being taken out), this device among them unless it's going.
    async fn new_epoch(&self, client: &Client, on: &(impl Address + Keep + Signs), scope: &Scope, n: u32, except: &[[u8; 32]]) -> Result<(), Error> {
        let me = Endpoint { key: on.public(), seal: self.opens().public() };
        let (recovery, to) = self.find(|who| {
            let mut to: Vec<Endpoint> = who.openers(scope)?.into_iter().filter(|c| c.seal != c.key && !except.contains(&c.key)).map(|c| Endpoint { key: c.key, seal: c.seal }).collect();
            if !to.iter().any(|e| e.key == me.key) && !except.contains(&me.key) {
                to.push(me);
            }
            Ok((who.recovery()?, to))
        })?;
        let key: [u8; 32] = rand::random();
        let hash = keys::epoch_hash(&key);
        self.write(client, on, content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
        self.read(on).await?;
        if self.find(|who| who.epoch_hash(scope, n))? != Some(hash) {
            return Err(failed("Another device made new keys for this just now. Try again."));
        }
        let sealed = |to: &[u8; 32]| keys::seal_to(to, &key).map(|s| B64U.encode(&s)).map_err(failed);
        // Not while there's none (the one there was forked `global`), until an admin names the next.
        if let Some(recovery) = recovery {
            let recovery = keys::twin_public(&recovery).map_err(failed)?;
            self.asked(client, on, "POST", "/v1/keys", Some(json!({ "scope": scope.to_string(), "n": n, "to": "recovery", "sealed": sealed(&recovery)? }))).await?;
        }
        for e in &to {
            self.asked(client, on, "POST", "/v1/keys", Some(json!({ "scope": scope.to_string(), "n": n, "to": hex(&e.key), "sealed": sealed(&e.seal)? }))).await?;
        }
        self.made_epoch(scope, n, key);
        Ok(())
    }

    /// A new epoch of each scope this person may start one of (an admin of it), sealed to none of `except`: → those it
    /// left to one of their admins, by name.
    async fn renew(&self, client: &Client, on: &(impl Address + Keep + Signs), scopes: &[Scope], except: &[[u8; 32]]) -> Result<Vec<String>, Error> {
        let user = self.user().ok_or_else(refused)?;
        let mut left = Vec::new();
        for scope in scopes {
            let (starts, newest) = self.find(|who| Ok((who.role_in(&user, scope)? == Some(Role::Admin), who.epoch(scope)?)))?;
            if !starts {
                left.push(self.find(|who| whose(who, scope))?);
            } else if newest > 0 {
                self.new_epoch(client, on, scope, newest + 1, except).await?;
            }
        }
        Ok(left)
    }

    /// Tells the relay something (`{ add, admin? }` or `{ remove }`), signed by this device.
    async fn tell(&self, on: &(impl Address + Signs), fields: Value) -> Result<(), Error> {
        let url = format!("{}/relay/tell", on.address());
        let mut again = true;
        loop {
            let at = keys::next_at(now());
            let signed = keys::signed_request_by(on, one::protocol::RELAY, fields.clone(), at).await.map_err(failed)?;
            let (status, out) = on.ask("POST", &url, None, Some(signed)).await?;
            if (200..300).contains(&status) {
                return Ok(());
            }
            // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
            if again && out["code"] == code::REPLAYED {
                again = false;
                n0_future::time::sleep(keys::again_in(at)).await;
                continue;
            }
            return Err(Error { code: out["code"].as_str().map(str::to_string), ..failed(error(&out).unwrap_or_else(|| format!("{YOURS} didn't take that ({status})."))) });
        }
    }

    /// Takes a key off the relay's list; one it doesn't list is out already.
    async fn take_out(&self, on: &(impl Address + Signs), key: &[u8; 32]) -> Result<(), Error> {
        match self.tell(on, json!({ "remove": hex(key) })).await {
            Err(err) if err.code.as_deref() != Some(code::NOT_LISTED) => Err(err),
            _ => Ok(()),
        }
    }

    /// When each of the network's devices was last let onto it, as the relay says: `{ key: ms }`.
    async fn seen(&self, on: &(impl Address + Signs)) -> Result<Value, Error> {
        let signed = keys::signed_request_by(on, one::protocol::RELAY, json!({ "seen": true }), keys::next_at(now())).await.map_err(failed)?;
        let (status, out) = on.ask("POST", &format!("{}/relay/seen", on.address()), None, Some(signed)).await?;
        Ok(if status == 200 { out["seen"].clone() } else { json!({}) })
    }

    /// Takes one of this person's devices out (by its key or name), or for an admin, anyone's: off the network first,
    /// then unlisted, it and its parts; then each scope its person opens that this person may move on gets a new epoch
    /// it doesn't have. Itself, it leaves (`leave`).
    async fn remove(&self, client: &Client, on: &(impl Address + Keep + Signs), device: &str) -> Result<Value, Error> {
        let key = self.find(|who| who.find_machine(device))?;
        if key == on.public() {
            return self.leave(client, on).await.map(|()| json!({ "removed": "this device" }));
        }
        let user = self.user().ok_or_else(refused)?;
        let (name, keys, scopes) = self.find(|who| {
            let it = who.machine(&key)?.filter(|d| !d.removed).ok_or_else(|| anyhow::anyhow!("That device isn’t listed."))?;
            anyhow::ensure!(who.may(&user, "remove", &scopes::device(&key)), "Only its person or an admin takes a device out.");
            let keys: Vec<[u8; 32]> = std::iter::once(key).chain(who.machines()?.into_iter().filter(|d| d.device == Some(key) && d.key != key && !d.removed).map(|d| d.key)).collect();
            let scopes = who.scopes()?.into_iter().filter(|s| who.may(&it.user, "use", &scopes::object(s))).collect::<Vec<_>>();
            Ok((it.name, keys, scopes))
        })?;
        for k in &keys {
            self.take_out(on, k).await?;
            self.write(client, on, content::remove_device(k)).await?;
        }
        let left = self.renew(client, on, &scopes, &keys).await?;
        Ok(json!({ "removed": name, "not_renewed": left }))
    }

    /// Takes a person out, for an admin: each of their devices off the network first, then them, then each scope they
    /// could open but their own gets a new epoch they don't have.
    async fn remove_person(&self, client: &Client, on: &(impl Address + Keep + Signs), person: &str) -> Result<Value, Error> {
        let user = self.user().ok_or_else(refused)?;
        let (gone, name, keys, scopes) = self.find(|who| {
            let gone = who.find_user(person)?;
            anyhow::ensure!(gone != user, "You can’t remove yourself. Another admin can.");
            anyhow::ensure!(who.may(&user, "remove", scopes::NETWORK), "Only an admin takes someone out.");
            let name = who.user(&gone)?.map(|u| u.name).unwrap_or_default();
            let keys: Vec<[u8; 32]> = who.machines()?.into_iter().filter(|d| d.user == gone && !d.removed).map(|d| d.key).collect();
            let own = Scope::User(gone.clone());
            let scopes = who.scopes()?.into_iter().filter(|s| *s != own && who.may(&gone, "use", &scopes::object(s))).collect::<Vec<_>>();
            Ok((gone, name, keys, scopes))
        })?;
        for k in &keys {
            self.take_out(on, k).await?;
        }
        self.write(client, on, content::remove_user(&gone)).await?;
        let left = self.renew(client, on, &scopes, &keys).await?;
        Ok(json!({ "removed": name, "not_renewed": left }))
    }

    /// Takes this device out of its person's devices, as it signs out, so signing in again makes a new one that
    /// replaces it, never a second. Nothing it writes after counts, so it goes in this order: its removal, then off the
    /// network. Its scopes keep their keys, as it forgets its own (ARCHITECTURE.md).
    pub async fn leave(&self, client: &Client, on: &(impl Address + Keep + Signs)) -> Result<(), Error> {
        self.read(on).await?;
        // A node closes a removed device's connection as it applies its removal, so its answer may never come: off
        // the network all the same.
        let wrote = self.write(client, on, content::remove_device(&on.public())).await;
        self.take_out(on, &on.public()).await?;
        // One sent whose answer was lost is that removal applied: only one never sent failed.
        match wrote {
            Err(err) if !err.sent => Err(err),
            _ => Ok(()),
        }
    }

    /// Asks the network's auth something as this device, signed with its key: a GET's or DELETE's in
    /// `x-one-request`, signed for `path` without its query; a POST's as its body, with `fields`.
    async fn ask_auth(&self, on: &(impl Address + Signs), method: &str, path: &str, fields: Value) -> Result<Value, Error> {
        let mut fields = fields;
        fields["route"] = json!(format!("{method} {}", path.split('?').next().unwrap_or_default()));
        let signed = keys::signed_request_by(on, one::protocol::AUTH_DOMAIN, fields, now()).await.map_err(failed)?;
        let url = format!("{}{path}", on.address());
        let (status, out) = match method {
            "POST" => on.ask(method, &url, None, Some(signed)).await?,
            _ => on.ask(method, &url, Some(("x-one-request", B64U.encode(&serde_json::to_vec(&signed).map_err(failed)?))), None).await?,
        };
        if !(200..300).contains(&status) {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS}'s auth answered {status}."))));
        }
        Ok(out)
    }

    /// The invites this device made that still work, by who they're for.
    async fn invites(&self, on: &impl Keep) -> BTreeMap<String, Invited> {
        let kept: Value = on.get(INVITES).await.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        kept.as_object().into_iter().flatten().filter_map(|(user, v)| Some((user.clone(), Invited::of(v)?))).filter(|(_, i)| i.until > now()).collect()
    }

    async fn keep_invites(&self, on: &impl Keep, invites: &BTreeMap<String, Invited>) -> Result<(), Error> {
        let kept: serde_json::Map<String, Value> = invites.iter().map(|(user, i)| (user.clone(), i.kept())).collect();
        on.put(INVITES, Value::Object(kept).to_string().as_bytes()).await.map_err(failed)
    }

    /// An invite link for someone new called `name`, as `role`, made by this admin's device: its secret stays here, to
    /// check the newcomer's device by when it asks to be let in; auth is told only its hash. → `{ link, until }`.
    async fn invite(&self, on: &(impl Address + Keep + Signs), name: &str, role: Role) -> Result<Value, Error> {
        let name = name.trim();
        if name.is_empty() {
            return Err(failed("Give their name."));
        }
        let secret = B64U.encode(&rand::random::<[u8; 32]>());
        let user = scopes::new_id();
        let until = now() + one::protocol::INVITE_DAYS * DAY;
        let fields = json!({ "id": hex(&keys::sha256(secret.as_bytes())), "name": name, "user": user, "until": until });
        self.ask_auth(on, "POST", "/api/auth/invites", fields).await?;
        let mut invites = self.invites(on).await;
        invites.insert(user, Invited { secret: secret.clone(), name: name.to_string(), role, until });
        self.keep_invites(on, &invites).await?;
        Ok(json!({ "link": format!("{}/invite#{secret}", on.address()), "until": until }))
    }

    /// Devices waiting that this device may let in: its person's own, and someone new from an invite made here, whose
    /// proof is that invite's; but those taken out or turned down. Each with the invite it came from, if it did.
    async fn waiting(&self, on: &(impl Address + Keep + Signs)) -> Result<Vec<(Asked, Option<Invited>)>, Error> {
        let user = self.user().ok_or_else(refused)?;
        let asked = self.ask_auth(on, "GET", "/api/auth/devices/requests", json!({})).await?;
        let invites = self.invites(on).await;
        let mut out = Vec::new();
        for r in asked["requests"].as_array().into_iter().flatten() {
            let Some(r) = Asked::of(r) else { continue };
            let gone = self.find(|who| Ok(who.machine_ever(&r.endpoint.key)?.is_some_and(|m| m.removed)))?;
            if r.endpoint.key == on.public() || r.said["service"].is_string() || gone {
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

    /// The one waiting with `code`, as typed.
    async fn waiting_with(&self, on: &(impl Address + Keep + Signs), code: &str) -> Result<(Asked, Option<Invited>), Error> {
        let code: String = code.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let mut found: Vec<_> = self.waiting(on).await?.into_iter().filter(|(r, _)| r.code.replace(' ', "") == code).collect();
        match found.len() {
            0 => Err(failed("No device is waiting with that code. Check it, or start again on that device.")),
            1 => Ok(found.remove(0)),
            _ => Err(failed("Two devices are waiting with that code, so neither was let in. Start again on yours, for a new code.")),
        }
    }

    /// Devices waiting that this device may let in, as Settings › Devices shows them, each with its code and what
    /// approving it records as what they sign in with. One of its person's own whose sign-in shows it's theirs (one
    /// of the accounts their devices record) is let in by it, not shown.
    async fn requests(&self, client: &Client, on: &(impl Address + Keep + Signs)) -> Result<Value, Error> {
        let user = self.user().ok_or_else(refused)?;
        let accounts = self.find(|who| who.sign_ins(&user))?;
        let mut out = Vec::new();
        for (r, invite) in self.waiting(on).await? {
            if invite.is_none() && !accounts.is_empty() && self.signed_in_with(on, &r, Some(&accounts)).await.is_some() {
                match self.approve(client, on, &r, None).await {
                    Ok(_) => continue,
                    Err(err) => eprintln!("letting {} in by its sign-in: {err}", r.name),
                }
            }
            let mut shown = json!({ "device": r.name, "person": invite.as_ref().map_or(r.said["person"].clone(), |i| json!(i.name)), "code": r.code, "made": r.said["made"], "place": r.said["place"] });
            shown[if invite.is_some() { "new" } else { "yours" }] = json!(true);
            if let Some(account) = self.signed_in_with(on, &r, None).await {
                shown["sign_in"] = json!(account);
            }
            out.push(shown);
        }
        Ok(json!(out))
    }

    /// What `r`'s sign-in shows it signed in with, checked here (`one::sign_in_proof`): against `accounts`, its
    /// person's as their devices record them, for one let in by it, or with none, for one approved by its code. None if
    /// it shows nothing that checks out.
    async fn signed_in_with(&self, on: &impl Address, r: &Asked, accounts: Option<&[one::sign_in_proof::Account]>) -> Option<one::sign_in_proof::Account> {
        let proof: one::sign_in_proof::Proof = serde_json::from_value(r.said["sign_in"].clone()).ok()?;
        let (root, clients) = self.who(|who| -> anyhow::Result<_> { Ok((who.root()?, who.setting(one::sign_in_proof::CLIENTS)?)) })?.ok()?;
        let network = one::sign_in_proof::Network::new(on.address(), *root?.as_bytes(), clients.as_deref());
        let jwks = match proof.issuer().and_then(|i| one::sign_in_proof::keys_at(&i, on.address())) {
            Some(at) => Some(on.ask("GET", &at, None, None).await.ok().filter(|(s, _)| *s == 200)?.1),
            None => None,
        };
        match accounts {
            Some(theirs) => one::sign_in_proof::vouches(&proof, &r.endpoint, &network, jwks.as_ref(), theirs, now()).ok(),
            None => one::sign_in_proof::check(&proof, &r.endpoint, &network, jwks.as_ref(), now()).ok(),
        }
    }

    /// Without `confirm`, whose the device showing `code` is; with, it's let in.
    async fn approve_code(&self, client: &Client, on: &(impl Address + Keep + Signs), code: &str, confirm: bool) -> Result<Value, Error> {
        let (r, invite) = self.waiting_with(on, code).await?;
        if !confirm {
            let person = invite.as_ref().map_or(r.said["person"].clone(), |i| json!(i.name));
            return Ok(json!({ "device": r.name, "person": person, "new": invite.is_some() }));
        }
        self.approve(client, on, &r, invite.as_ref()).await
    }

    /// Lets in a device that asked: lists it (and for a newcomer, them), with what it signed in with, the device it's
    /// on and where it asked from; seals it each epoch this device opens of the scopes its person uses; takes their
    /// sign-in before on the same device out; lets it onto the network; and marks its request approved with where this
    /// device read who's who to, which the device waiting starts from. → `{ device, sealed, not_sealed }`.
    async fn approve(&self, client: &Client, on: &(impl Address + Keep + Signs), r: &Asked, invite: Option<&Invited>) -> Result<Value, Error> {
        self.read(on).await?;
        let user = self.user().ok_or_else(refused)?;
        let e = &r.endpoint;
        let device = r.said["device"].as_str().map(keys::unhex).transpose().map_err(|_| failed("The device it's on is 64 hex characters."))?;
        let (removed, listed, may_invite) = self.find(|who| {
            let removed = who.machine_ever(&e.key)?.is_some_and(|m| m.removed);
            let listed = who.client(&e.key)?.is_some_and(|c| c.user == r.user && c.seal == e.seal);
            Ok((removed, listed, who.may(&user, "invite", scopes::NETWORK)))
        })?;
        // Taken out, or turned down: never sealed anything again, whatever a node says.
        if removed {
            return Err(failed(format!("{} was turned down or taken out, so it isn’t let in again. Start again on it, for a new code.", r.name)));
        }
        if invite.is_some() && !may_invite {
            return Err(failed("Only an admin lets someone new in."));
        }
        if invite.is_none() && r.user != user {
            return Err(failed("Only their own devices approve one of theirs."));
        }
        if !listed {
            let mut listing = content::device(e, &r.user, &r.name, &[Kind::Client], invite.map(|i| (i.name.as_str(), i.role)));
            if let Some(on) = &device {
                listing.data["device"] = json!(hex(on));
            }
            if let Some(account) = self.signed_in_with(on, r, None).await {
                listing.data["sign_in"] = json!(account);
            }
            if r.said["place"].is_string() {
                listing.data["place"] = r.said["place"].clone();
            }
            self.write(client, on, listing).await?;
            self.read(on).await?;
        }
        // A newcomer's own scope is theirs: their device makes its first epoch.
        let (scopes, admin, before) = self.find(|who| {
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
            match self.seal_held(client, on, scope, e).await {
                Ok(()) => sealed.push(scope.to_string()),
                Err(err) => not_sealed.push(json!({ "scope": scope.to_string(), "why": err.to_string() })),
            }
        }
        match self.tell(on, json!({ "add": hex(&e.key), "role": "client", "admin": admin })).await {
            Err(err) if !(listed && err.code.as_deref() == Some(code::ALREADY)) => return Err(err),
            _ => {}
        }
        // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
        // keep their keys. One the relay wouldn't take out stays listed, for the next sign-in there to try again.
        for old in before {
            if let Err(err) = self.take_out(on, &old).await {
                eprintln!("{}'s earlier sign-in is still let in: {err}", r.name);
                continue;
            }
            self.write(client, on, content::remove_device(&old)).await?;
        }
        if invite.is_some_and(|i| i.role == Role::Admin)
            && let Err(err) = self.ask_auth(on, "POST", "/api/auth/people/admin", json!({ "user": r.user, "admin": true })).await
        {
            eprintln!("telling sign-in {} is an admin: {err}", r.name);
        }
        let root = self.find(|who| who.root())?.ok_or_else(refused)?;
        let welcome = self.welcome().map(|w| format!("&welcome={w}")).unwrap_or_default();
        self.ask_auth(on, "DELETE", &format!("/api/auth/devices/requests/{}?approved&root={}{welcome}", hex(&e.key), root.to_hex()), json!({})).await?;
        if invite.is_some() {
            let mut invites = self.invites(on).await;
            invites.remove(&r.user);
            self.keep_invites(on, &invites).await?;
        }
        Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not_sealed }))
    }

    /// Seals each epoch of `scope` sealed to this device to `to`, but those it has, each checked against the hash
    /// who's who names.
    async fn seal_held(&self, client: &Client, on: &(impl Address + Keep + Signs), scope: &Scope, to: &Endpoint) -> Result<(), Error> {
        let mine = self.asked(client, on, "GET", &format!("/v1/keys?scope={}&to={}", query(&scope.to_string()), hex(&on.public())), None).await?;
        for (n, sealed) in mine["sealed"].as_object().into_iter().flatten() {
            let n: u32 = n.parse().map_err(failed)?;
            let key = keys::open_sealed_by(self.opens(), &B64U.decode(sealed.as_str().unwrap_or("").as_bytes()).map_err(failed)?).await.map_err(failed)?;
            let key: [u8; 32] = key.try_into().map_err(|_| failed("A key for this didn't check out, so it wasn't used."))?;
            if self.find(|who| who.epoch_hash(scope, n))? != Some(keys::epoch_hash(&key)) {
                return Err(failed("A key for this didn't check out, so it wasn't used."));
            }
            let body = json!({ "scope": scope.to_string(), "n": n, "to": hex(&to.key), "sealed": B64U.encode(&keys::seal_to(&to.seal, &key).map_err(failed)?) });
            match self.asked(client, on, "POST", "/v1/keys", Some(body)).await {
                Err(err) if err.code.as_deref() != Some(code::ALREADY) => return Err(err),
                _ => {}
            }
        }
        Ok(())
    }

    /// Turns down the device waiting with `code`: one of its person's own by a step, so none of their devices lets it
    /// in after; either way its request is forgotten at auth. → which it was.
    async fn refuse(&self, client: &Client, on: &(impl Address + Keep + Signs), code: &str) -> Result<Value, Error> {
        let (r, invite) = self.waiting_with(on, code).await?;
        if invite.is_none() {
            self.write(client, on, content::refuse(&r.endpoint, &r.name)).await?;
        }
        self.ask_auth(on, "DELETE", &format!("/api/auth/devices/requests/{}", hex(&r.endpoint.key)), json!({})).await?;
        Ok(json!({ "device": r.name }))
    }

    /// Lists a passkey (its PRF output, `keys::passkey_keys`) as a way back in: it's sealed what this device opens, and
    /// for an admin's, is an admin's device at the relay too, so the devices it lets in are let in as theirs.
    async fn list_passkey(&self, client: &Client, on: &(impl Address + Keep + Signs), prf: &[u8], credential: &str) -> Result<(), Error> {
        let (sign, seal) = keys::passkey_keys(prf);
        let pk = Endpoint { key: sign.public(), seal: keys::public_of(&seal) };
        let user = self.user().ok_or_else(refused)?;
        self.write(client, on, content::passkey(&pk, &user, "Passkey", credential)).await?;
        self.read(on).await?;
        let (scopes, admin) = self.find(|who| Ok((who.scopes()?.into_iter().filter(|s| who.may(&user, "use", &scopes::object(s))).collect::<Vec<_>>(), who.may(&user, "invite", scopes::NETWORK))))?;
        for scope in &scopes {
            self.seal_held(client, on, scope, &pk).await?;
        }
        self.tell(on, json!({ "add": hex(&pk.key), "role": "client", "admin": admin })).await
    }

    /// `/v1/app-schedules`, as a node's route reads it: `device` by its name or key; `scope` user (theirs, the
    /// default), global, or a group by its name or id; `days`, 30 at most, 0 to stop.
    async fn app_schedules_asked(&self, client: &Client, on: &(impl Address + Keep + Signs), body: &Value) -> Result<Value, Error> {
        let user = self.user().ok_or_else(refused)?;
        let (app, given) = (text(body, "app"), body["scope"].as_str().unwrap_or("user"));
        let (endpoint, scope, name) = self.find(|who| {
            let endpoint = who.find_node(&text(body, "device"))?;
            let scope = Scope::parse_for(given, &user).or_else(|_| who.find_group(given).map(Scope::Group))?;
            anyhow::ensure!(who.app(&app)?.is_some(), "no app called {app}");
            Ok((endpoint, scope, who.machine(&endpoint)?.map(|m| m.name).unwrap_or_default()))
        })?;
        let days = body["days"].as_u64().unwrap_or(APP_SCHEDULES_DAYS);
        let until = self.set_app_schedules(client, on, &endpoint, &app, &scope, days).await?;
        Ok(match until {
            0 => json!({ "said": format!("{name} no longer runs {app}'s schedules, and what's written from now on is under new keys it doesn't have.") }),
            until => json!({ "said": format!("{name} runs {app}'s schedules for you, for {} days, and longer as you use it.", days.min(APP_SCHEDULES_DAYS)), "until": until }),
        })
    }

    /// Runs `app`'s schedules in `scope` on one of its person's devices (`endpoint`) for `days`, 30 at most (0 stops
    /// them): the app's key in each epoch of the scope sealed to it first, then the app schedules naming each one's
    /// hash. Stopping them starts a new epoch of the scope, which the device isn't sealed, so the keys it held open
    /// nothing written after. → until when.
    async fn set_app_schedules(&self, client: &Client, on: &(impl Address + Keep + Signs), endpoint: &[u8; 32], app: &str, scope: &Scope, days: u64) -> Result<u64, Error> {
        let (seal, made) = self.find(|who| {
            let seal = who.machine(endpoint)?.filter(|d| !d.user.is_empty()).map(|d| d.seal).ok_or_else(|| anyhow::anyhow!("That device isn’t one of yours."))?;
            Ok((seal, who.epoch_hashes(scope)?.into_keys().collect::<Vec<_>>()))
        })?;
        let until = if days == 0 { 0 } else { now() + days.min(APP_SCHEDULES_DAYS) * DAY };
        let mut hashes = BTreeMap::new();
        if until > 0 {
            for n in if made.is_empty() { vec![1] } else { made } {
                let key = self.app_key(client, on, &scope.to_string(), app, n).await?;
                self.asked(client, on, "POST", "/v1/keys", Some(json!({ "scope": scope.to_string(), "n": n, "app": app, "to": hex(endpoint), "sealed": B64U.encode(&keys::seal_to(&seal, &key).map_err(failed)?) }))).await?;
                hashes.insert(n, keys::epoch_hash(&key));
            }
        }
        self.write(client, on, content::app_schedules(endpoint, scope, app, until, &hashes)).await?;
        self.read(on).await?;
        if until == 0 {
            self.renew(client, on, std::slice::from_ref(scope), &[]).await?;
        }
        Ok(until)
    }

    /// Renews this person's app schedules of `app` as they use it: once a day at most, and at once when the scope has
    /// an epoch their keys don't reach; one of theirs that ran out is stopped, as Stop does. Only those the network's
    /// steps hold, never those a node lists: a hacked one could list itself. Who's who is read again first at most once
    /// an hour, so it renews what its person's other devices set too.
    pub async fn renew_app_schedules(&self, client: &Client, on: &(impl Address + Keep + Signs), app: &str) {
        if self.renew_checked(HOUR) {
            let _ = self.read(on).await;
        }
        let Some(user) = self.user() else { return };
        let due = self.find(|who| {
            let now = now();
            let mut due = Vec::new();
            for ((endpoint, scope, of), s) in who.app_schedules()? {
                if s.by != user || s.until == 0 {
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
            if let Err(err) = self.set_app_schedules(client, on, &endpoint, &app, &scope, days).await {
                eprintln!("renewing {app}'s schedules: {err}");
            }
        }
    }

    /// Registers where a push service reaches this device (`one::push::Address`) with the network's push Worker, signed
    /// with its key and naming its seal key, which each note pushed to it is sealed to, and its person, so sign-in
    /// pushes it when another device of theirs asks to join. Once it's out of the network, the Worker forgets it.
    pub async fn register_push(&self, on: &(impl Address + Signs), address: &one::push::Address) -> Result<(), Error> {
        let fields = one::push::register_fields(&self.opens().public(), address, self.user().as_deref()).map_err(failed)?;
        let body = keys::signed_request_by(on, one::push::DOMAIN, fields, now()).await.map_err(failed)?;
        let (status, out) = on.ask("POST", &format!("{}/push/register", on.address()), None, Some(body)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS}'s push service didn't take this device ({status})."))));
        }
        Ok(())
    }
}

/// Whose a scope is, as a person reads it: everyone's, a group's or someone's, by name.
fn whose(who: &ScopeDb, scope: &Scope) -> anyhow::Result<String> {
    Ok(match scope {
        Scope::Global => "everyone".into(),
        Scope::Group(g) => who.group(g)?.map(|g| g.name).unwrap_or_else(|| g.clone()),
        Scope::User(u) => who.user_ever(u)?.map(|u| u.name).unwrap_or_else(|| u.clone()),
    })
}
