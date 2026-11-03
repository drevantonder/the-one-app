//! Devices joining and leaving: the keys sealed to each (`GET`/`POST /v1/keys`, which any of the install's devices may
//! read, sealed as they are), approving a device's request to join (its own person's, or a newcomer's by invite), invites,
//! and removing a device.

use super::*;
use crate::bucket::Bucket;
use data_encoding::BASE64URL_NOPAD as B64U;

/// The most a sealed key may be: an epoch's key sealed to an endpoint, or an app's under an epoch, is far less.
const SEALED_MAX: usize = 256;

/// Whom an epoch's key is sealed to, in `keys::epoch_name`: an endpoint's key in hex, or `recovery`.
fn sealed_to(to: &str) -> Result<()> {
    ensure!(to == "recovery" || crate::keys::unhex(to).is_ok(), "to is an endpoint's key (64 hex characters) or recovery");
    Ok(())
}

/// Whether `user` may start a scope's new epoch, as `members` counts an `epoch` entry: an admin of the install for
/// `global`, of the group for a group's, and the person for their own.
fn starts_epochs(m: &Members, user: &str, scope: &Scope) -> bool {
    match scope {
        Scope::Global => m.is_admin(user),
        Scope::Group(g) => m.manages(user, g),
        Scope::User(u) => u == user,
    }
}

impl Node {
    /// What's sealed of a scope's keys, for any person of the install, since it opens only with keys they may hold:
    /// with `to`, each of its epochs sealed to that endpoint, `{ epoch, sealed: { n: base64url } }`, `epoch` the newest
    /// made; with `app`, the app's key in each epoch, sealed under that epoch's, `{ sealed: { n: base64url } }`.
    pub async fn sealed_keys(&self, caller: &Caller, scope: &Scope, to: Option<&str>, app: Option<&str>) -> Result<Value> {
        ensure!(self.is_user(&caller.user), "only the install's people read its keys");
        let bucket = self.bucket()?;
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        let mut sealed = serde_json::Map::new();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                for n in 1..=newest {
                    if let Some(o) = Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, to)).await? {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "epoch": newest, "sealed": sealed }))
            }
            (None, Some(app)) => {
                ensure!(members::valid_app_name(app), "{app} isn't an app's name");
                for n in 1..=newest {
                    if let Some(o) = Bucket::get(&bucket, &crate::keys::app_key_name(&name, app, n)).await? {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "sealed": sealed }))
            }
            _ => bail!("give to (an endpoint's key, or recovery) or app"),
        }
    }

    /// Keeps a key sealed by a device of `caller`'s, who may use the scope: epoch `n` sealed to `to`, an endpoint listed
    /// as someone's who may use the scope (or the recovery key, or a new epoch, only by one who may start it), only if
    /// it isn't there (→ `{}`); or an app's key in epoch `n`, if it isn't there (→ `{ sealed }`, whichever is there
    /// now, which every device then uses).
    pub async fn seal_key(&self, caller: &Caller, scope: &Scope, n: u32, to: Option<&str>, app: Option<&str>, sealed: &str) -> Result<Value> {
        let sealed = B64U.decode(sealed.trim().as_bytes()).map_err(|_| anyhow!("sealed is base64url"))?;
        ensure!(!sealed.is_empty() && sealed.len() <= SEALED_MAX, "a sealed key is at most {SEALED_MAX} bytes");
        let bucket = self.bucket()?;
        let newest = self.newest_epoch(scope).await?;
        let name = scope.to_string();
        match (to, app) {
            (Some(to), None) => {
                sealed_to(to)?;
                self.with(|i| {
                    let m = &i.r.members;
                    ensure!(m.may_use(&caller.user, scope), "you don't use {scope}");
                    ensure!(n >= 1 && n <= newest + 1, "{scope} has no epoch {n}: its newest is {newest}");
                    ensure!(n <= newest || starts_epochs(m, &caller.user, scope), "only {scope}'s admins start its new epoch");
                    match to {
                        "recovery" => ensure!(starts_epochs(m, &caller.user, scope), "only {scope}'s admins seal its keys to the recovery key"),
                        to => {
                            let key = crate::keys::unhex(to)?;
                            let listed = m.machine(&key).context("that endpoint isn't one of the install's")?;
                            ensure!(m.may_use(&listed.user, scope), "that endpoint's person doesn't use {scope}");
                        }
                    }
                    Ok(())
                })?;
                ensure!(crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, to), &sealed).await?, "{scope}'s epoch {n} is sealed to {to} already");
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(members::valid_app_name(app), "{app} isn't an app's name");
                ensure!(self.with(|i| Ok(i.r.members.may_use(&caller.user, scope)))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = crate::keys::app_key_name(&name, app, n);
                let there = match crate::db::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            _ => bail!("give to (an endpoint's key, or recovery) or app"),
        }
    }
}

/// A device's request to join, as auth keeps it (`/api/auth/devices/requests`): its endpoint's keys and name, its
/// person, and for a newcomer's first device, the proof that their invite's secret was theirs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceRequest {
    pub key: String,
    pub seal: String,
    pub name: String,
    pub user: String,
    /// Their person's name, as auth knows it.
    #[serde(default)]
    pub person: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
}

/// An invite this device's person made, kept in their own scope so any of their devices approves its newcomer.
#[derive(Clone, Debug)]
pub struct Invited {
    /// The link's secret, as its text.
    pub secret: String,
    pub name: String,
    pub user: String,
    pub role: Role,
    pub until: u64,
}

/// Where a person's invites are kept: a database of the node's own, in their scope.
const INVITES: &str = "invites";
const INVITES_SCHEMA: &str = "create table invites (user text primary key, secret text not null, name text not null, role text not null, until integer not null)";

/// A join code as someone types it: its letters, upper case, without spaces or dashes.
fn code_of(typed: &str) -> String {
    typed.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase()
}

impl Node {
    /// The install's address, where its kernel is.
    fn address(&self) -> Result<String> {
        self.with(|i| Ok(i.r.members.settings.get("address").map(|a| a.trim_end_matches('/').to_string())))?.context("your One App has no address yet, so it has no sign-ins or requests to join")
    }

    /// Asks the install's auth service something as this device, signed with its key (`one/auth`): a POST's body is
    /// the signed request, a GET's or DELETE's is in `x-one-request`. → its answer.
    async fn ask_auth(&self, method: &'static str, path: &str, fields: Value) -> Result<Value> {
        let (address, key, path) = (self.address()?, self.signing_key(), path.to_string());
        tokio::task::spawn_blocking(move || {
            let mut fields = fields;
            fields["route"] = json!(format!("{method} {path}"));
            let signed = crate::keys::signed_request(&key, "one/auth", fields, now_ms());
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
            let url = format!("{address}{path}");
            let header = B64U.encode(&serde_json::to_vec(&signed)?);
            let mut resp = match method {
                "POST" => agent.post(&url).send_json(&signed)?,
                "DELETE" => agent.delete(&url).header("x-one-request", &header).call()?,
                _ => agent.get(&url).header("x-one-request", &header).call()?,
            };
            let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            ensure!(resp.status().is_success(), "{}", out["error"].as_str().unwrap_or("the sign-in service didn't answer"));
            Ok(out)
        })
        .await?
    }

    /// Tells the relay something now, signed by this device (`one/relay`), as `tell_relay` does in the background.
    async fn tell_relay_now(&self, fields: Value) -> Result<()> {
        let (address, key) = (self.address()?, self.signing_key());
        tokio::task::spawn_blocking(move || {
            let body = crate::keys::signed_request(&key, "one/relay", fields, now_ms());
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(15))).build().into();
            let mut resp = agent.post(format!("{address}/relay/tell")).send_json(&body)?;
            let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
            ensure!(resp.status().is_success(), "the relay said: {}", out["error"].as_str().unwrap_or("no"));
            Ok(())
        })
        .await?
    }

    /// The invites this device's person made that still work, by who they're for.
    async fn invites(&self) -> Result<BTreeMap<String, Invited>> {
        let scope = Scope::User(self.user()?);
        let db = self.database(&scope, members::APP, INVITES, Some(vec![INVITES_SCHEMA.into()]), None).await?;
        let mut db = db.lock().await;
        db.catch_up(&self.bucket()?).await?;
        let rows = db.query("select * from invites where until > ?", &[json!(now_ms())])?;
        let text = |r: &serde_json::Map<String, Value>, k: &str| r[k].as_str().unwrap_or("").to_string();
        Ok(rows
            .iter()
            .filter_map(|r| {
                let role = serde_json::from_value(json!(text(r, "role"))).ok()?;
                Some((text(r, "user"), Invited { secret: text(r, "secret"), name: text(r, "name"), user: text(r, "user"), role, until: r["until"].as_u64().unwrap_or(0) }))
            })
            .collect())
    }

    /// Keeps or forgets (`None`) an invite, in this device's person's own scope.
    async fn keep_invite(&self, user: &str, invite: Option<&Invited>) -> Result<()> {
        let scope = Scope::User(self.user()?);
        let db = self.database(&scope, members::APP, INVITES, Some(vec![INVITES_SCHEMA.into()]), None).await?;
        let mut db = db.lock().await;
        db.write(&self.bucket()?, |c| {
            match invite {
                Some(i) => crate::db::execute(c, "insert or replace into invites values (?, ?, ?, ?, ?)", &[json!(i.user), json!(i.secret), json!(i.name), json!(i.role), json!(i.until)])?,
                None => crate::db::execute(c, "delete from invites where user = ?", &[json!(user)])?,
            };
            Ok(())
        })
        .await
    }

    /// Invites someone new to join, called `name`, as `role`, for this device's person, an admin: its secret is made
    /// here, auth is told only its hash (signed by this device), and it's kept in their own scope, for any of their
    /// devices to approve the newcomer's first device. → (the link, until when).
    pub async fn invite_someone(&self, caller: &Caller, name: &str, role: Role) -> Result<(String, u64)> {
        ensure!(caller.user == self.user()?, "invite from one of your own devices: {} keeps your invites with you", self.device_name());
        ensure!(self.with(|i| Ok(i.r.members.is_admin(&caller.user)))?, "only admins invite someone");
        let (address, key, name_) = (self.address()?, self.signing_key(), name.to_string());
        let user = new_id();
        let made = {
            let user = user.clone();
            tokio::task::spawn_blocking(move || crate::kernel::invite(&address, &key, &name_, Some(&user), None)).await??
        };
        let secret = made.link.rsplit_once('#').map(|(_, s)| s.to_string()).context("an invite link carries its secret")?;
        self.keep_invite(&user, Some(&Invited { secret, name: name.trim().to_string(), user: user.clone(), role, until: made.until })).await?;
        Ok((made.link, made.until))
    }

    /// The requests to join that `caller` may approve, as auth lists them to this device: their own devices', and
    /// those of the people they invited whose proof is right. A request of their own is shown without its code, which
    /// only the device asking shows; a newcomer's with it, as their proof says it's theirs.
    pub async fn device_requests(&self, caller: &Caller) -> Result<Vec<Value>> {
        let asked: Value = self.ask_auth("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("auth's requests aren't requests")?;
        let invites = if caller.user == self.user()? { self.invites().await? } else { BTreeMap::new() };
        let mut out = Vec::new();
        for r in asked {
            let Ok(key) = crate::keys::unhex(&r.key) else { continue };
            match (&r.proof, invites.get(&r.user)) {
                (None, _) if r.user == caller.user => out.push(json!({ "device": r.name, "person": r.person, "yours": true })),
                (Some(proof), Some(i)) if *proof == crate::keys::invite_proof(i.secret.as_bytes(), &key) => {
                    out.push(json!({ "device": r.name, "person": i.name, "code": crate::keys::join_code(&key), "role": i.role }))
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Approves the device whose join code is `code`, for `caller`: with `confirm`, lets it in
    /// (`approve_request`) and forgets its request; without, says what it is. → what it is, or what was done.
    pub async fn approve(&self, caller: &Caller, code: &str, confirm: bool) -> Result<Value> {
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_auth("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("auth's requests aren't requests")?;
        let r = asked.into_iter().find(|r| crate::keys::unhex(&r.key).is_ok_and(|k| crate::keys::join_code(&k) == code)).context("no device is waiting with that code: it waits 15 minutes after it asks")?;
        let invite = match &r.proof {
            Some(_) if caller.user == self.user()? => self.invites().await?.remove(&r.user),
            _ => None,
        };
        let who = invite.as_ref().map(|i| i.name.clone()).unwrap_or(r.person.clone());
        if !confirm {
            ensure!(r.user == caller.user || invite.is_some(), "that device is {who}'s: one of their own devices approves it");
            return Ok(json!({ "device": r.name, "person": who, "new": invite.is_some() }));
        }
        let approved = self.approve_request(caller, &r, invite.as_ref()).await?;
        if let Err(err) = self.ask_auth("DELETE", &format!("/api/auth/devices/requests/{}", r.key), json!({})).await {
            tracing::warn!("forgetting an approved request at auth: {err:#}");
        }
        if invite.is_some() {
            self.keep_invite(&r.user, None).await?;
        }
        Ok(approved)
    }

    /// Lets a device in: lists it (`device`, a client of its person; for a newcomer, writing them in too, as an admin's
    /// device may), seals it every epoch this device holds of each scope its person uses (a newcomer's own scope is
    /// theirs: their device makes its first epoch), and tells the relay to let it in. A newcomer's request counts only with
    /// the proof their invite's secret makes with its key. → what it was sealed, and what it wasn't.
    pub async fn approve_request(&self, caller: &Caller, r: &DeviceRequest, invite: Option<&Invited>) -> Result<Value> {
        let endpoint = crate::keys::Endpoint { key: crate::keys::unhex(&r.key)?, seal: crate::keys::unhex(&r.seal)? };
        let listed = match invite {
            Some(i) => {
                let proof = r.proof.as_deref().context("a newcomer's device brings their invite's proof")?;
                ensure!(proof == crate::keys::invite_proof(i.secret.as_bytes(), &endpoint.key), "that device's proof isn't from {}'s invite", i.name);
                ensure!(r.user == i.user, "that device isn't {}'s", i.name);
                members::device(&endpoint, &r.user, &r.name, &[members::Kind::Client], Some((&i.name, i.role)))
            }
            None => {
                ensure!(r.user == caller.user, "that device is {}'s: one of their own devices approves it", if r.person.is_empty() { &r.user } else { &r.person });
                members::device(&endpoint, &r.user, &r.name, &[members::Kind::Client], None)
            }
        };
        self.member_content(caller, listed).await?;
        let admin = self.with(|i| Ok(i.r.members.is_admin(&r.user)))?;
        self.tell_relay(json!({ "add": r.key, "admin": admin }));
        if invite.is_some_and(|i| i.role == Role::Admin)
            && let Err(err) = self.ask_auth("POST", "/api/auth/people/admin", json!({ "user": r.user, "admin": true })).await
        {
            tracing::warn!("telling auth {} is an admin: {err:#}", r.user);
        }
        let scopes: Vec<Scope> = self.with(|i| {
            let m = &i.r.members;
            Ok(m.scopes().into_iter().filter(|s| m.may_use(&r.user, s) && (invite.is_none() || *s != Scope::User(r.user.clone()))).collect())
        })?;
        let (mut sealed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            match self.seal_held(&scope, &endpoint).await {
                Ok(()) => sealed.push(scope.to_string()),
                Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
            }
        }
        Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not }))
    }

    /// Seals each epoch of `scope` this device holds to `to`, where it isn't yet.
    async fn seal_held(&self, scope: &Scope, to: &crate::keys::Endpoint) -> Result<()> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        for (n, key) in self.epoch_keys(scope).await? {
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::db::created(&bucket, &at, &crate::kernel::seal_to(&to.seal, &key)?).await?;
            }
        }
        Ok(())
    }

    /// Removes a device (by name or key), for its person or an admin: the relay told to let it in no more first, so it
    /// reaches the bucket no more, then its `remove` entry, and a new epoch of each scope it could open (as a client:
    /// compute holds no epochs) that `caller` starts epochs of, sealed to every client left that opens it and the
    /// recovery key; databases move to it at their next routine copy. A scope `caller` can't start a new epoch of is
    /// said: one of its own admins' devices does. → what was done.
    pub async fn remove_device(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let (gone, scopes) = self.with(|i| {
            let m = &i.r.members;
            let d = m.machine(&key).context("no such device")?;
            ensure!(m.looks_after(&caller.user, &key), "only its person or an admin removes a device");
            let opens = |s: &Scope| d.is(members::Kind::Client) && m.may_use(&d.user, s);
            Ok((d.name.clone(), m.scopes().into_iter().filter(opens).collect::<Vec<_>>()))
        })?;
        ensure!(key != self.0.id, "{gone} is this device: remove it from another of your devices");
        let relay = self.tell_relay_now(json!({ "remove": hex(&key) })).await.err().map(|err| format!("{err:#}"));
        self.member_as(caller, "remove", json!({ "key": hex(&key) })).await?;
        let (mut renewed, mut left) = (Vec::new(), Vec::new());
        for scope in scopes {
            let starts = self.with(|i| Ok(starts_epochs(&i.r.members, &caller.user, &scope)))?;
            if !starts {
                left.push(scope.to_string());
                continue;
            }
            match self.new_epoch(caller, &scope).await {
                Ok(n) => renewed.push(json!({ "scope": scope.to_string(), "epoch": n })),
                Err(err) => left.push(format!("{scope} ({err:#})")),
            }
        }
        Ok(json!({ "removed": gone, "new_epochs": renewed, "not_renewed": left, "relay": relay }))
    }

    /// A scope's next epoch: a random key sealed to the recovery key (first, which says it's made) and to every client
    /// device of someone who uses the scope, then its `epoch` entry naming its hash, before anything uses it. → its
    /// number.
    pub(super) async fn new_epoch(&self, caller: &Caller, scope: &Scope) -> Result<u32> {
        let newest = self.newest_epoch(scope).await?;
        ensure!(newest >= 1, "{scope} has no keys yet");
        let n = newest + 1;
        let (recovery, to) = self.with(|i| {
            let m = &i.r.members;
            let to: Vec<(Key, [u8; 32])> = m.clients().filter(|d| m.may_use(&d.user, scope)).map(|d| (d.key, d.seal)).collect();
            Ok((m.recovery, to))
        })?;
        let recovery = recovery.context("the install has no recovery key to seal its keys to")?;
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let key: [u8; 32] = rand::random();
        ensure!(crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, "recovery"), &crate::kernel::seal_to(&crate::kernel::twin_public(&recovery)?, &key)?).await?, "another device started {scope}'s epoch {n} just now");
        for (device, seal) in &to {
            crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, &hex(device)), &crate::kernel::seal_to(seal, &key)?).await?;
        }
        self.member_as(caller, "epoch", json!({ "scope": name, "n": n, "to": to.iter().map(|(k, _)| hex(k)).collect::<Vec<_>>(), "hash": crate::keys::epoch_hash(&key) })).await?;
        self.forget_epochs(scope);
        Ok(n)
    }
}
