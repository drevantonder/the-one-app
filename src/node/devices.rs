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
                // Every epoch at once: one wait on the bucket, not one each.
                let got = futures_util::future::try_join_all((1..=newest).map(|n| {
                    let (bucket, at) = (&bucket, crate::keys::epoch_name(&name, n, to));
                    async move { Bucket::get(bucket, &at).await }
                }))
                .await?;
                for (n, o) in (1..=newest).zip(got) {
                    if let Some(o) = o {
                        sealed.insert(n.to_string(), json!(B64U.encode(&o.bytes)));
                    }
                }
                Ok(json!({ "epoch": newest, "sealed": sealed }))
            }
            (None, Some(app)) => {
                ensure!(app == members::APP || members::valid_app_name(app), "{app} isn't an app's name");
                let got = futures_util::future::try_join_all((1..=newest).map(|n| {
                    let (bucket, at) = (&bucket, crate::keys::app_key_name(&name, app, n));
                    async move { Bucket::get(bucket, &at).await }
                }))
                .await?;
                for (n, o) in (1..=newest).zip(got) {
                    if let Some(o) = o {
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
    /// it isn't there (→ `{}`); an app's key in epoch `n`, if it isn't there (→ `{ sealed }`, whichever is there
    /// now, which every device then uses); or with both, an app's key in epoch `n` sealed to `to`, an endpoint of the
    /// install that will keep the app running there (`keys::grant_name`), by one who may grant that (→ `{}`, there
    /// already or not: its `grant` entry names the hash of the one it means).
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
                            // A scope's epochs are its people's client devices' alone: a node is handed a call's keys,
                            // and a service holds none, reading nothing and asking its person's node to write.
                            let listed = m.client(&key).context("a scope's keys are sealed only to client devices and the recovery key")?;
                            ensure!(m.may_use(&listed.user, scope), "that endpoint's person doesn't use {scope}");
                            ensure!(m.service(&key).is_none(), "a service is sealed no keys");
                        }
                    }
                    Ok(())
                })?;
                ensure!(crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, to), &sealed).await?, "{scope}'s epoch {n} is sealed to {to} already");
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(app == members::APP || members::valid_app_name(app), "{app} isn't an app's name");
                ensure!(self.with(|i| Ok(i.r.members.may_use(&caller.user, scope)))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = crate::keys::app_key_name(&name, app, n);
                let there = match crate::db::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            (Some(to), Some(app)) => {
                ensure!(members::valid_app_name(app), "{app} isn't an app's name");
                let key = crate::keys::unhex(to)?;
                self.with(|i| {
                    let m = &i.r.members;
                    ensure!(m.grants_in(&caller.user, scope), "only {scope}'s person, or its group's admins, keep its apps running");
                    ensure!(m.machine(&key).is_some(), "that endpoint isn't one of the install's");
                    Ok(())
                })?;
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                crate::db::created(&bucket, &crate::keys::grant_name(&name, app, to, n), &sealed).await?;
                Ok(json!({}))
            }
            _ => bail!("give to (an endpoint's key, or recovery), app, or both"),
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
    /// For a native app, the device it's on (hex): its oned's key, or an id of the device's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl DeviceRequest {
    /// The device that asks: its key and seal key, which its join code and proof cover both of.
    pub fn endpoint(&self) -> Result<crate::keys::Endpoint> {
        Ok(crate::keys::Endpoint { key: crate::keys::unhex(&self.key)?, seal: crate::keys::unhex(&self.seal)? })
    }
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

#[cfg(test)]
mod waiting_tests {

}

/// The one device waiting with join `code`: none if two are, as a device made to show another's code would be.
fn waiting(asked: Vec<DeviceRequest>, code: &str) -> Result<DeviceRequest> {
    let mut found = asked.into_iter().filter(|r| r.endpoint().is_ok_and(|d| crate::keys::join_code(&d) == code));
    let r = found.next().context("no device is waiting with that code: it waits 15 minutes after it asks")?;
    ensure!(found.next().is_none(), "two devices are waiting with that code, so neither was let in: start again on yours, for a new code");
    Ok(r)
}

/// A join code as someone types it: its letters, upper case, without spaces or dashes.
fn code_of(typed: &str) -> String {
    typed.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase()
}

impl Node {
    /// The install's address, where its kernel is.
    pub fn address(&self) -> Result<String> {
        self.with(|i| Ok(super::calls::address(&i.r.members)))?.context("your One App has no address yet, so it has no sign-ins or requests to join")
    }

    /// Asks the install's auth service something as this device, signed with its key (`one/auth`): a POST's body is
    /// the signed request, a GET's or DELETE's is in `x-one-request`; it's signed for `path` without its query. → its
    /// answer.
    async fn ask_auth(&self, method: &'static str, path: &str, fields: Value) -> Result<Value> {
        let (address, key, path) = (self.address()?, self.signing_key(), path.to_string());
        tokio::task::spawn_blocking(move || {
            let mut fields = fields;
            fields["route"] = json!(format!("{method} {}", path.split('?').next().unwrap_or_default()));
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

    /// Once the install's recovery key changed, an admin's device tells the relay to let the old one in no more and the
    /// new one in as a client: once each since this device started.
    pub(super) fn follow_recovery(&self) {
        let told = self.with(|i| {
            let m = &i.r.members;
            let admins = m.machine(&self.0.id).is_some_and(|d| m.is_admin(&d.user));
            Ok(m.recovery.filter(|_| admins && !m.recovery_was.is_empty()).map(|now| (now, m.recovery_was.clone())))
        });
        let Ok(Some((now, was))) = told else { return };
        let mut done = self.0.recovery_told.lock().unwrap();
        for old in was.into_iter().filter(|k| done.insert(*k)) {
            self.tell_relay(json!({ "remove": hex(&old) }));
        }
        if done.insert(now) {
            self.tell_relay(json!({ "add": hex(&now), "role": "client" }));
        }
    }

    /// Tells the relay something now, signed by this device (`one/relay`), as `tell_relay` does in the background.
    pub(super) async fn tell_relay_now(&self, fields: Value) -> Result<()> {
        self.tell_relay_as(self.signing_key(), fields).await
    }

    /// Tells the relay something now, signed by `key`: this device's, or the install's recovery key.
    async fn tell_relay_as(&self, key: ed25519_dalek::SigningKey, fields: Value) -> Result<()> {
        let address = self.address()?;
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(15))).build().into();
            // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
            let mut again = true;
            loop {
                let at = crate::keys::next_at(now_ms());
                let body = crate::keys::signed_request(&key, "one/relay", fields.clone(), at);
                let mut resp = agent.post(format!("{address}/relay/tell")).send_json(&body)?;
                let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
                let said = out["error"].as_str().unwrap_or("no");
                if !resp.status().is_success() && again && crate::keys::taken_already(said) {
                    again = false;
                    std::thread::sleep(crate::keys::again_in(at));
                    continue;
                }
                ensure!(resp.status().is_success(), "the relay said: {said}");
                return Ok(());
            }
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
            let Ok(device) = r.endpoint() else { continue };
            match (&r.proof, invites.get(&r.user)) {
                (None, _) if r.user == caller.user => out.push(json!({ "device": r.name, "person": r.person, "yours": true })),
                (Some(proof), Some(i)) if *proof == crate::keys::invite_proof(i.secret.as_bytes(), &device) => {
                    out.push(json!({ "device": r.name, "person": i.name, "code": crate::keys::join_code(&device), "role": i.role }))
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Lets back in someone who lost every device, with the install's recovery key, on this machine (never one asked
    /// over the network): the device whose join code is `code`, theirs, is listed by an entry the recovery key signs
    /// (which counts as an admin's approval, and is listed as its use), sealed each epoch of each scope they may open
    /// (opened through the recovery key's copy), and let onto the network by the recovery key. Without `confirm`, says
    /// whose it is. → what it is, or what was done.
    pub async fn approve_with_recovery(&self, caller: &Caller, code: &str, recovery: &ed25519_dalek::SigningKey, confirm: bool) -> Result<Value> {
        ensure!(caller.token.is_none() && caller.key == self.0.id, "the recovery key is used only on the machine it's kept on");
        let at = recovery.verifying_key().to_bytes();
        ensure!(self.with(|i| Ok(i.r.members.recovery == Some(at)))?, "that isn't your One App's recovery key");
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_auth("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("auth's requests aren't requests")?;
        let r = waiting(asked, &code)?;
        ensure!(r.proof.is_none(), "the recovery key lets in a device of someone already in your One App");
        let person = self.with(|i| Ok(i.r.members.user(&r.user).map(|u| u.name.clone())))?.context("the recovery key lets in a device of someone already in your One App")?;
        if !confirm {
            return Ok(json!({ "device": r.name, "person": person, "recovery": true }));
        }
        let endpoint = r.endpoint()?;
        self.read_log_now(&Scope::Global).await?;
        let entry = self.with(|i| {
            let head = i.r.store.last(&at, "global")?.map(|e| e.head());
            let ext = crate::entry::Ext { log: "global".into(), time: now_ms(), user: r.user.clone(), epoch: i.r.members.epoch(&Scope::Global), previous: i.r.store.heads("global", Some(&at))? };
            Ok(Entry::create(recovery, ext, head, members::device(&endpoint, &r.user, &r.name, &[members::Kind::Client], None)))
        })?;
        self.put_checked(vec![entry]).await?;
        let relay = self.tell_relay_as(recovery.clone(), json!({ "add": r.key, "role": "client" })).await.err().map(|err| format!("{err:#}"));
        let scopes: Vec<Scope> = self.with(|i| Ok(i.r.members.scopes().into_iter().filter(|s| i.r.members.may_use(&r.user, s)).collect()))?;
        let (mut sealed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            match self.seal_from_recovery(&scope, recovery, &endpoint).await {
                Ok(0) => {}
                Ok(_) => sealed.push(scope.to_string()),
                Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
            }
        }
        // Marked approved, for the device waiting on it to see, with the install it's in.
        if let Err(err) = self.ask_auth("DELETE", &self.approved_path(&r.key)?, json!({})).await {
            tracing::warn!("marking a request approved at auth: {err:#}");
        }
        Ok(json!({ "device": r.name, "person": person, "recovery": true, "sealed": sealed, "not_sealed": not, "relay": relay }))
    }

    /// Where a device's request is marked approved at auth, saying the install it was let into (its first entry's hash),
    /// the only one that device then takes.
    fn approved_path(&self, key: &str) -> Result<String> {
        let root = self.with(|i| Ok(i.r.root.map(|r| r.to_hex())))?.context("this device isn't in your One App yet")?;
        Ok(format!("/api/auth/devices/requests/{key}?approved&genesis={root}"))
    }

    /// Seals each epoch of `scope` its entries made, opened through the recovery key's copy, to `to` where it isn't yet.
    /// → how many it opened.
    async fn seal_from_recovery(&self, scope: &Scope, recovery: &ed25519_dalek::SigningKey, to: &crate::keys::Endpoint) -> Result<usize> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let hashes: BTreeMap<u32, String> = self.with(|i| Ok(i.r.members.epoch_hashes.range((scope.clone(), 0)..=(scope.clone(), u32::MAX)).map(|((_, n), h)| (*n, h.clone())).collect()))?;
        let secret = crate::keys::seal_secret(recovery);
        let mut opened = 0;
        for (n, hash) in hashes {
            let Some(o) = Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, "recovery")).await? else {
                tracing::warn!("{scope}'s epoch {n} isn't sealed to the recovery key");
                continue;
            };
            let key = crate::kernel::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == hash);
            let Some(key) = key else {
                tracing::warn!("{scope}'s epoch {n} sealed to the recovery key isn't the key its entry made: passed over");
                continue;
            };
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::db::created(&bucket, &at, &crate::kernel::seal_to(&to.seal, &key)?).await?;
            }
            opened += 1;
        }
        Ok(opened)
    }

    /// Approves the device whose join code is `code`, for `caller`: with `confirm`, lets it in
    /// (`approve_request`) and forgets its request; without, says what it is. → what it is, or what was done.
    pub async fn approve(&self, caller: &Caller, code: &str, confirm: bool) -> Result<Value> {
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_auth("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("auth's requests aren't requests")?;
        let r = waiting(asked, &code)?;
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
        // Marked approved, for the device waiting on it to see, with the install it's in.
        if let Err(err) = self.ask_auth("DELETE", &self.approved_path(&r.key)?, json!({})).await {
            tracing::warn!("marking a request approved at auth: {err:#}");
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
        let endpoint = r.endpoint()?;
        let on = r.device.as_deref().map(crate::keys::unhex).transpose().context("the device it's on is 64 hex characters")?;
        let mut listed = match invite {
            Some(i) => {
                let proof = r.proof.as_deref().context("a newcomer's device brings their invite's proof")?;
                ensure!(proof == crate::keys::invite_proof(i.secret.as_bytes(), &endpoint), "that device's proof isn't from {}'s invite", i.name);
                ensure!(r.user == i.user, "that device isn't {}'s", i.name);
                members::device(&endpoint, &r.user, &r.name, &[members::Kind::Client], Some((&i.name, i.role)))
            }
            None => {
                ensure!(r.user == caller.user, "that device is {}'s: one of their own devices approves it", if r.person.is_empty() { &r.user } else { &r.person });
                members::device(&endpoint, &r.user, &r.name, &[members::Kind::Client], None)
            }
        };
        if let Some(on) = &on {
            listed.data["device"] = json!(hex(on));
        }
        // Approved before, but its request wasn't marked so (the device still waits on it): what's left is done again.
        let already = self.with(|i| Ok(i.r.members.client(&endpoint.key).is_some_and(|c| c.user == r.user && c.seal == endpoint.seal)))?;
        if !already {
            self.member_content(caller, listed).await?;
        }
        // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
        // keep their keys.
        if let Some(on) = on {
            let before: Vec<Key> = self.with(|i| {
                let m = &i.r.members;
                Ok(m.clients().filter(|c| c.user == r.user && c.device == Some(on) && c.key != endpoint.key && c.passkey.is_none()).map(|c| c.key).collect())
            })?;
            for old in before {
                if let Err(err) = self.tell_relay_now(json!({ "remove": hex(&old) })).await {
                    tracing::warn!("taking {}'s sign-in before off the network: {err:#}", r.name);
                }
                let seq = self.with(|i| Ok(i.r.store.last(&old, "global")?.map(|e| e.seq())))?;
                self.member_as(caller, "remove", json!({ "key": hex(&old), "seq": seq })).await?;
            }
        }
        let admin = self.with(|i| Ok(i.r.members.is_admin(&r.user)))?;
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
        // Last, and waited for where there's a relay (at the install's address): until it lets the device in, the device
        // reaches nothing and keeps waiting. Approving it again finishes this, as it's listed already.
        if self.address().is_ok() {
            let told = self.tell_relay_now(json!({ "add": r.key, "admin": admin, "role": "client" })).await;
            told.with_context(|| format!("{} is let in, but your One App's network didn't take it yet: approve it again", r.name))?;
        }
        Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not }))
    }

    /// Lists one of this device's parts, the service `service`, as its person's, a part of this device (oned turned it
    /// on, and it asked with its endpoint and whether it serves everyone), and tells the relay to let it in. It holds no
    /// keys: the nodes that call it bring it the install's entries, which say who may. A part listed before it named its
    /// device is listed again, naming it.
    pub async fn list_part(&self, endpoint: &crate::keys::Endpoint, service: &str, shared: bool) -> Result<()> {
        let user = self.user()?;
        let caller = Caller { key: self.0.id, user: user.clone(), agent: None, token: None };
        let listed = self.with(|i| {
            let m = &i.r.members;
            ensure!(m.service(&endpoint.key).is_some() || m.machines.get(&endpoint.key).is_none(), "that key is listed already, and not as a service");
            Ok(m.service(&endpoint.key).map(|s| s.device == Some(self.0.id)))
        })?;
        if listed.is_none() {
            // Told first: a service the relay won't let in is never reached.
            if let Err(err) = self.tell_relay_now(json!({ "add": hex(&endpoint.key), "admin": false, "role": "service" })).await {
                ensure!(format!("{err:#}").contains("in already"), "your One App's relay won't let {service} in: {err:#}");
            }
        }
        if listed != Some(true) {
            let name = self.device_name();
            self.member_content(&caller, members::service_endpoint(endpoint, &user, &name, service, shared, &self.0.id)).await?;
        }
        // Its first hello brings it the entries that say it's theirs: as soon as the relay lets it in.
        let (node, key) = (self.clone(), endpoint.key);
        tokio::spawn(async move {
            for _ in 0..20 {
                if node.hello_service_now(&key).await {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            tracing::warn!("service {} was listed but this device hasn't reached it", crate::entry::short(&key));
        });
        Ok(())
    }

    /// Seals each epoch of `scope` this device holds to `to`, where it isn't yet.
    async fn seal_held(&self, scope: &Scope, to: &crate::keys::Endpoint) -> Result<()> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        for (n, key) in self.epoch_keys(scope, false).await? {
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::db::created(&bucket, &at, &crate::kernel::seal_to(&to.seal, &key)?).await?;
            }
        }
        Ok(())
    }

    /// Erases what this device keeps (by name or key), for its person or an admin asking from one of their own devices,
    /// directly: asked of another device, it says to ask that one, if it's on. Once it has answered, its oned stops its
    /// parts and deletes its home (its copies of the install's data, the pages it kept, and each service's folder and
    /// data, WhatsApp's link among them), then stops. Removing it next takes it out. → what's erasing.
    pub async fn erase(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let name = self.with(|i| Ok(i.r.members.machine(&key).map(|d| d.name.clone()).unwrap_or_default()))?;
        if key != self.0.id {
            ensure!(self.with(|i| Ok(i.online(&key)))?, "{name} isn't on, so it can't erase what it keeps");
            return Err(super::calls::Elsewhere { why: format!("only {name} erases what it keeps"), nodes: vec![hex(&key)] }.into());
        }
        self.with(|i| {
            let m = &i.r.members;
            ensure!(caller.key != self.0.id && m.client(&caller.key).is_some() && m.looks_after(&caller.user, &key), "only {name}'s person or an admin erases it, from another of their devices");
            Ok(())
        })?;
        tracing::warn!("asked to erase what {name} keeps: its oned does, then stops");
        crate::oned::erase(&self.0.dir);
        Ok(json!({ "erasing": name }))
    }

    /// Removes a device (by name or key), and what's part of it (its services, and its apps' sign-ins), for its person or
    /// an admin: the relay told to let each in no more first, so it reaches the bucket no more, then their `remove`
    /// entries, and a new epoch of each scope it could open (as a client:
    /// compute holds no epochs) that `caller` starts epochs of, sealed to every client left that opens it and the
    /// recovery key; databases move to it at their next routine copy. A scope `caller` can't start a new epoch of is
    /// said: one of its own admins' devices does. A device removing itself (as it signs out) is told off the relay last,
    /// since it hears no answer after. → what was done.
    pub async fn remove_device(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let (gone, keys, scopes) = self.with(|i| {
            let m = &i.r.members;
            let d = m.machine(&key).context("no such device")?;
            ensure!(m.looks_after(&caller.user, &key), "only its person or an admin removes a device");
            let keys: Vec<Key> = std::iter::once(key).chain(m.machines.values().filter(|e| e.device == Some(key) && e.key != key && m.machine(&e.key).is_some()).map(|e| e.key)).collect();
            let opens = |s: &Scope| keys.iter().filter_map(|k| m.machine(k)).any(|d| d.is(members::Kind::Client) && m.may_use(&d.user, s));
            Ok((d.name.clone(), keys.clone(), m.scopes().into_iter().filter(opens).collect::<Vec<_>>()))
        })?;
        ensure!(!keys.contains(&self.0.id), "{gone} is this device: remove it from another of your devices");
        // A device taking itself out (signing out) is told last: off the relay, it hears no answer, so everything else is done.
        let itself = keys.contains(&caller.key);
        let off_relay = || async {
            let mut said = Vec::new();
            for k in &keys {
                if let Err(err) = self.tell_relay_now(json!({ "remove": hex(k) })).await {
                    said.push(format!("{err:#}"));
                }
            }
            (!said.is_empty()).then(|| said.join("; "))
        };
        let mut relay = if itself { None } else { off_relay().await };
        for k in &keys {
            // The last of its entries in `global` this device had seen: those after never count, wherever they sort.
            let seq = self.with(|i| Ok(i.r.store.last(k, "global")?.map(|e| e.seq())))?;
            self.member_as(caller, "remove", json!({ "key": hex(k), "seq": seq })).await?;
        }
        let (renewed, left) = self.renew(caller, scopes).await;
        if itself {
            relay = off_relay().await;
        }
        Ok(json!({ "removed": gone, "new_epochs": renewed, "not_renewed": left, "relay": relay }))
    }

    /// Removes a person, for an admin: the relay told to let each of their devices in no more first, then their `remove`
    /// entry, saying the last entry of each of their devices it had seen (none after counts), and a new epoch of each scope they could open but their own (`global`, and each group they were in),
    /// sealed to every client left that opens it and the recovery key. Their own scope is kept as it is: removing deletes
    /// nothing. → what was done.
    pub async fn remove_person(&self, caller: &Caller, user: &str) -> Result<Value> {
        let user = self.find_user(user)?;
        let (name, keys, scopes) = self.with(|i| {
            let m = &i.r.members;
            ensure!(m.is_admin(&caller.user), "only admins remove someone");
            ensure!(user != caller.user, "you can't remove yourself: another admin can");
            let name = m.user(&user).context("no such person")?.name.clone();
            let keys: Vec<Key> = m.machines.values().filter(|d| d.user == user && m.machine(&d.key).is_some()).map(|d| d.key).collect();
            let mut scopes = vec![Scope::Global];
            scopes.extend(m.groups_of(&user).into_keys().map(Scope::Group));
            Ok((name, keys, scopes))
        })?;
        ensure!(!keys.contains(&self.0.id), "this device is {name}'s: remove them from another");
        let mut relay = Vec::new();
        for key in &keys {
            if let Err(err) = self.tell_relay_now(json!({ "remove": hex(key) })).await {
                relay.push(format!("{err:#}"));
            }
        }
        // The last of each of their devices' entries in `global` this device had seen: those after never count.
        let seqs = self.with(|i| keys.iter().map(|k| Ok((*k, i.r.store.last(k, "global")?.map(|e| e.seq())))).collect::<Result<BTreeMap<_, _>>>())?;
        self.member_content(caller, members::remove_user(&user, &seqs)).await?;
        let (renewed, left) = self.renew(caller, scopes).await;
        Ok(json!({ "removed": name, "new_epochs": renewed, "not_renewed": left, "relay": (!relay.is_empty()).then(|| relay.join("; ")) }))
    }

    /// A new epoch of each scope, where `caller` may start one: → those made, and those left to one of their admins.
    async fn renew(&self, caller: &Caller, scopes: Vec<Scope>) -> (Vec<Value>, Vec<String>) {
        let (mut renewed, mut left) = (Vec::new(), Vec::new());
        for scope in scopes {
            if !self.with(|i| Ok(starts_epochs(&i.r.members, &caller.user, &scope))).unwrap_or(false) {
                left.push(self.whose(&scope));
                continue;
            }
            match self.new_epoch(caller, &scope).await {
                Ok(n) => renewed.push(json!({ "scope": scope.to_string(), "epoch": n })),
                Err(err) => left.push(format!("{} ({err:#})", self.whose(&scope))),
            }
        }
        (renewed, left)
    }

    /// Starts, in the background, each new epoch a scope owes (`Members::owed`) that this device's person starts epochs
    /// of: one a removal or a stopped grant asked for that the device writing it didn't make (it failed, or went off
    /// first). Only once a scope has owed one at two looks in a row, so a removal's own new epoch goes first.
    pub(super) fn start_owed_epochs(&self) {
        let Ok(me) = self.user() else { return };
        let owed: BTreeSet<Scope> = self
            .with(|i| {
                let m = &i.r.members;
                if m.client(&self.0.id).is_none_or(|c| c.user != me) {
                    return Ok(BTreeSet::new());
                }
                Ok(m.owed.iter().filter(|s| starts_epochs(m, &me, s)).cloned().collect())
            })
            .unwrap_or_default();
        let due: Vec<Scope> = {
            let mut held = self.0.owed.lock().unwrap();
            let (seen, starting) = &mut *held;
            let due = owed.intersection(seen).filter(|s| !starting.contains(*s)).cloned().collect::<Vec<_>>();
            starting.extend(due.iter().cloned());
            *seen = owed;
            due
        };
        for scope in due {
            let (node, me) = (self.clone(), Caller { key: self.0.id, user: me.clone(), agent: None, token: None });
            tokio::spawn(async move {
                match node.new_epoch(&me, &scope).await {
                    Ok(n) => tracing::info!("started {scope}'s epoch {n}, which a removal asked for"),
                    Err(err) => tracing::warn!("starting {scope}'s new keys, which a removal asked for: {err:#}"),
                }
                node.0.owed.lock().unwrap().1.remove(&scope);
            });
        }
    }

    /// What a person's first device does once it's in, here for a machine (kernel/web/device.js for a browser): where
    /// their own things run besides their own machines, if their devices never said (the install's machines that run
    /// everyone's, chosen without asking), and their own scope's first epoch, if it isn't made.
    pub(super) fn settle_own(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let todo = node.with(|i| {
                let m = &i.r.members;
                let me = i.user.clone();
                if m.client(&node.0.id).is_none_or(|c| c.user != me) || m.epoch(&Scope::User(me.clone())) > 0 {
                    return Ok(None);
                }
                let on: Vec<Key> = match m.runs.contains_key(&me) {
                    true => Vec::new(),
                    false => m.computes().filter(|c| c.user != me && m.runs_for(&c.key, &Scope::Global)).map(|c| c.key).collect(),
                };
                Ok(Some((me, on)))
            });
            let Ok(Some((me, on))) = todo else { return };
            if !on.is_empty()
                && let Err(err) = node.member_content(&Caller { key: node.0.id, user: me.clone(), agent: None, token: None }, members::runs(&on)).await
            {
                tracing::warn!("saying where {me}'s things run: {err:#}");
            }
            if let Err(err) = node.epoch_keys(&Scope::User(me.clone()), true).await {
                tracing::warn!("making {me}'s own keys: {err:#}");
            }
        });
    }

    /// A scope as a person reads it: "Sam's own", "Work's", or "everyone's".
    fn whose(&self, scope: &Scope) -> String {
        self.with(|i| {
            let m = &i.r.members;
            Ok(match scope {
                Scope::Global => "everyone's".to_string(),
                Scope::User(u) => format!("{}'s own", m.users.get(u).map_or(u.as_str(), |u| u.name.as_str())),
                Scope::Group(g) => format!("{}'s", m.groups.get(g).map_or(g.as_str(), |g| g.name.as_str())),
            })
        })
        .unwrap_or_default()
    }

    /// A scope's next epoch: its `epoch` entry first, naming its key's hash (which, first in causal order, makes it),
    /// then the key sealed to the recovery key and to every client device of someone who uses the scope, before
    /// anything uses it. → its number.
    pub(super) async fn new_epoch(&self, caller: &Caller, scope: &Scope) -> Result<u32> {
        let newest = self.newest_epoch(scope).await?;
        ensure!(newest >= 1, "{scope} has no keys yet");
        let n = newest + 1;
        let (recovery, to) = self.with(|i| Ok((i.r.members.recovery, super::databases::clients_of(&i.r.members, scope))))?;
        let recovery = recovery.context("the install has no recovery key to seal its keys to")?;
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let key: [u8; 32] = rand::random();
        let hash = crate::keys::epoch_hash(&key);
        self.member_content(caller, members::epoch(scope, n, &to.iter().map(|(k, _)| *k).collect::<Vec<_>>(), &hash)).await?;
        ensure!(self.with(|i| Ok(i.r.members.epoch_hash(scope, n) == Some(hash.as_str())))?, "another device started {scope}'s epoch {n} just now");
        if !crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, "recovery"), &crate::kernel::seal_to(&crate::kernel::twin_public(&recovery)?, &key)?).await? {
            tracing::warn!("{scope}'s epoch {n} was sealed to the recovery key already, not with its key: the recovery key won't open it");
        }
        for (device, seal) in &to {
            crate::db::created(&bucket, &crate::keys::epoch_name(&name, n, &hex(device)), &crate::kernel::seal_to(seal, &key)?).await?;
        }
        self.forget_epochs(scope);
        Ok(n)
    }
}
