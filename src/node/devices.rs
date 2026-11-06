//! Devices joining and leaving: the keys sealed to each (`GET`/`POST /v1/keys`, which any of the network's devices may
//! read, sealed as they are), approving a device's request to join (its own person's, or a newcomer's by invite), invites,
//! and removing a device.

use crate::config::YOURS;
use super::*;

pub mod app;
pub(crate) mod routes;
use crate::bucket::Bucket;
use data_encoding::BASE64URL_NOPAD as B64U;

/// The most a sealed key may be: an epoch's key sealed to an endpoint, or an app's under an epoch, is far less.
const SEALED_MAX: usize = 256;

/// Whom an epoch's key is sealed to, in `keys::epoch_name`: an endpoint's key in hex, or `recovery`.
fn sealed_to(to: &str) -> Result<()> {
    ensure!(to == "recovery" || crate::keys::unhex(to).is_ok(), "to is an endpoint's key (64 hex characters) or recovery");
    Ok(())
}

/// A scope's keys that may need making again: its epoch, or its Vault's key pair.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owing {
    Epoch(Scope),
    Vault(Scope),
}

/// What `Node::start_owed_keys` knows between its looks.
#[derive(Default)]
pub(super) struct Owed {
    /// What owed new keys at the last look.
    seen: BTreeSet<Owing>,
    /// What it's making new keys of now.
    starting: BTreeSet<Owing>,
    /// Keys whose entry lists this device that are sealed to it, by number, known so they're asked of the bucket once.
    sealed: BTreeSet<(Owing, u32)>,
    /// Those that weren't, at the last look.
    unsealed: BTreeSet<(Owing, u32)>,
}

/// Whether `who` may list a device of `user`'s that does `kinds`, as `rules` counts a `device` step: only of someone in
/// the network now (one removed, or not in it yet, is added with their first device, by an invite's approval); a
/// client or a service only from their own devices (`add-devices`), a machine that runs calls also by an admin
/// (`add-machines`), which runs only what that admin's listing may, never their own scope. The rows give the verbs;
/// which kind of device takes which is its rule's.
pub(super) fn may_list(db: &ScopeDb, who: &str, user: &str, kinds: &[scopes::Kind]) -> bool {
    let theirs = kinds.iter().any(|k| matches!(k, scopes::Kind::Client | scopes::Kind::Service));
    db.user(user).is_some() && db.may(who, if theirs { "add-devices" } else { "add-machines" }, &scopes::person(user))
}

/// How often a client of its person's looks for their new devices whose sign-in lets them in.
const SIGN_INS_EVERY: Duration = Duration::from_secs(4);

/// How long an issuer's keys are kept before they're fetched again: a key it no longer signs with goes within the hour.
const ISSUER_KEYS_FOR: Duration = Duration::from_secs(3600);

/// What `Node::sign_in_loop` knows between its looks.
#[derive(Default)]
pub(super) struct SignIns {
    /// Each issuer's keys, by where they're published, and when they were fetched.
    keys: HashMap<String, (Value, Instant)>,
    /// Requests whose sign-in was checked and didn't let them in, by key and the accounts they were checked against:
    /// left for the person, by code, unless their accounts change.
    refused: std::collections::HashSet<String>,
}

/// Whether `user` may start a scope's new epoch: they may `remove` from it, and it's a group that's there.
fn starts_epochs(db: &ScopeDb, user: &str, scope: &Scope) -> bool {
    !matches!(scope, Scope::Group(g) if db.group(g).is_none()) && db.may(user, "remove", &scopes::object(scope))
}

impl Node {
    /// What's sealed of a scope's keys, for any person of the network, since it opens only with keys they may hold:
    /// with `to`, each of its epochs sealed to that endpoint, `{ epoch, sealed: { n: base64url } }`, `epoch` the newest
    /// made; with `app`, the app's key in each epoch, sealed under that epoch's, `{ sealed: { n: base64url } }`.
    pub async fn sealed_keys(&self, caller: &Caller, scope: &Scope, to: Option<&str>, app: Option<&str>) -> Result<Value> {
        ensure!(self.is_user(&caller.user), "only the network's people read its keys");
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
                ensure!(app == scopes::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
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
    /// network that will run the app's schedules there (`keys::app_schedules_key_name`), by one who may set them (→ `{}`,
    /// there already or not: the app schedules name the hash of the one they mean).
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
                    let db = i.r.db();
                    ensure!(db.may(&caller.user, "use", &scopes::object(scope)), "you don't use {scope}");
                    ensure!(n >= 1 && n <= newest + 1, "{scope} has no epoch {n}: its newest is {newest}");
                    ensure!(n <= newest || starts_epochs(db, &caller.user, scope), "only {scope}'s admins start its new epoch");
                    match to {
                        "recovery" => ensure!(starts_epochs(db, &caller.user, scope), "only {scope}'s admins seal its keys to the recovery key"),
                        to => {
                            let key = crate::keys::unhex(to)?;
                            // A scope's epochs are its people's client devices' alone: a node is handed a call's keys,
                            // and a service holds none, reading nothing and asking its person's node to write.
                            let listed = db.client(&key).context("a scope's keys are sealed only to client devices and the recovery key")?;
                            ensure!(db.may(&listed.user, "use", &scopes::object(scope)), "that endpoint's person doesn't use {scope}");
                            ensure!(db.service(&key).is_none(), "a service is sealed no keys");
                        }
                    }
                    Ok(())
                })?;
                ensure!(crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, to), &sealed).await?, "{scope}'s epoch {n} is sealed to {to} already");
                Ok(json!({}))
            }
            (None, Some(app)) => {
                ensure!(app == scopes::APP || scopes::valid_app_name(app), "{app} isn't an app's name");
                ensure!(self.with(|i| Ok(i.r.db().may(&caller.user, "use", &scopes::object(scope))))?, "you don't use {scope}");
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                let key = crate::keys::app_key_name(&name, app, n);
                let there = match crate::database::stored::created(&bucket, &key, &sealed).await? {
                    true => sealed,
                    false => Bucket::get(&bucket, &key).await?.context("an app's key went as it was made")?.bytes,
                };
                Ok(json!({ "sealed": B64U.encode(&there) }))
            }
            (Some(to), Some(app)) => {
                ensure!(scopes::valid_app_name(app), "{app} isn't an app's name");
                let key = crate::keys::unhex(to)?;
                self.with(|i| {
                    let db = i.r.db();
                    ensure!(db.may(&caller.user, "choose-devices", &scopes::object(scope)), "only {scope}'s person, or its group's admins, choose where its apps' schedules run");
                    ensure!(db.machine(&key).is_some(), "that endpoint isn't one of the network's");
                    Ok(())
                })?;
                ensure!(n >= 1 && n <= newest, "{scope} has no epoch {n}: its newest is {newest}");
                crate::database::stored::created(&bucket, &crate::keys::app_schedules_key_name(&name, app, to, n), &sealed).await?;
                Ok(json!({}))
            }
            _ => bail!("give to (an endpoint's key, or recovery), app, or both"),
        }
    }
}

/// A device's request to join, as sign-in keeps it (`/api/auth/devices/requests`): its endpoint's keys and name, its
/// person, and for a newcomer's first device, the proof that their invite's secret was theirs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceRequest {
    pub key: String,
    pub seal: String,
    pub name: String,
    pub user: String,
    /// Their person's name, as sign-in knows it.
    #[serde(default)]
    pub person: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    /// For a native app, the device it's on (hex): its oned's key, or an id of the device's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// When it asked (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<u64>,
    /// The city it asked from, as Cloudflare placed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// What its sign-in showed (`sign_in_proof::Proof`), as sign-in passed it on: unchecked, and read only as a proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<Value>,
}

impl DeviceRequest {
    /// What its sign-in showed, if it's a proof this release reads.
    pub fn proof(&self) -> Option<crate::sign_in_proof::Proof> {
        serde_json::from_value(self.sign_in.clone()?).ok()
    }

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

/// Whether `caller` may answer request `r`: `Some(None)` for one of their own devices, `Some(invite)` for the first
/// device of someone they invited, whose proof is that invite's.
fn mine_or_invited<'a>(r: &DeviceRequest, caller: &Caller, invites: &'a BTreeMap<String, Invited>, device: &crate::keys::Endpoint) -> Option<Option<&'a Invited>> {
    match (&r.proof, invites.get(&r.user)) {
        (None, _) if r.user == caller.user => Some(None),
        (Some(proof), Some(i)) if *proof == crate::keys::invite_proof(i.secret.as_bytes(), device) => Some(Some(i)),
        _ => None,
    }
}

/// A join code as someone types it: its letters, upper case, without spaces or dashes.
fn code_of(typed: &str) -> String {
    typed.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase()
}

impl Node {
    /// The network's address, where people sign in.
    pub fn address(&self) -> Result<String> {
        self.with(|i| Ok(super::calls::address(i.r.db())))?.with_context(|| format!("{YOURS} has no address yet, so it has no sign-ins or requests to join"))
    }

    /// Asks the network's sign-in something as this device, signed with its key (`one/auth`): a POST's body is
    /// the signed request, a GET's or DELETE's is in `x-one-request`; it's signed for `path` without its query. → its
    /// answer.
    async fn ask_sign_in(&self, method: &'static str, path: &str, fields: Value) -> Result<Value> {
        let (address, key, path) = (self.address()?, self.signing_key(), path.to_string());
        tokio::task::spawn_blocking(move || {
            let mut fields = fields;
            fields["route"] = json!(format!("{method} {}", path.split('?').next().unwrap_or_default()));
            let signed = crate::keys::signed_request(&key, crate::protocol::SIGN_IN_DOMAIN, fields, now_ms());
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

    /// Once the network's recovery key changed, an admin's device tells the relay to let the old one in no more and the
    /// new one in as a client: once each since this device started.
    pub(super) fn follow_recovery(&self) {
        let told = self.with(|i| {
            let db = i.r.db();
            let admins = db.machine(&self.0.id).is_some_and(|d| db.may(&d.user, "invite", scopes::NETWORK));
            let was = db.recovery_was();
            Ok(db.recovery().filter(|_| admins && !was.is_empty()).map(|now| (now, was)))
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

    /// Tells the relay something now, signed by `key`: this device's, or the network's recovery key.
    async fn tell_relay_as(&self, key: ed25519_dalek::SigningKey, fields: Value) -> Result<()> {
        let address = self.address()?;
        tokio::task::spawn_blocking(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(15))).build().into();
            // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
            let mut again = true;
            loop {
                let at = crate::keys::next_at(now_ms());
                let body = crate::keys::signed_request(&key, crate::protocol::RELAY, fields.clone(), at);
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
        let db = self.database(&scope, scopes::APP, INVITES, Some(vec![INVITES_SCHEMA.into()]), None).await?;
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
        let db = self.database(&scope, scopes::APP, INVITES, Some(vec![INVITES_SCHEMA.into()]), None).await?;
        let mut db = db.lock().await;
        self.write_in(&scope, &mut db, &self.bucket()?, |c| {
            match invite {
                Some(i) => crate::database::stored::execute(c, "insert or replace into invites values (?, ?, ?, ?, ?)", &[json!(i.user), json!(i.secret), json!(i.name), json!(i.role), json!(i.until)])?,
                None => crate::database::stored::execute(c, "delete from invites where user = ?", &[json!(user)])?,
            };
            Ok(())
        })
        .await
    }

    /// Invites someone new to join, called `name`, as `role`, for this device's person, an admin: its secret is made
    /// here, sign-in is told only its hash (signed by this device), and it's kept in their own scope, for any of their
    /// devices to approve the newcomer's first device. → (the link, until when).
    pub async fn invite_someone(&self, caller: &Caller, name: &str, role: Role) -> Result<(String, u64)> {
        ensure!(caller.user == self.user()?, "invite from one of your own devices: {} keeps your invites with you", self.device_name());
        ensure!(self.with(|i| Ok(i.r.db().may(&caller.user, "invite", scopes::NETWORK)))?, "only admins invite someone");
        let (address, key, name_) = (self.address()?, self.signing_key(), name.to_string());
        let user = new_id();
        let made = {
            let user = user.clone();
            tokio::task::spawn_blocking(move || crate::cloudflare::invite(&address, &key, &name_, Some(&user), None)).await??
        };
        let secret = made.link.rsplit_once('#').map(|(_, s)| s.to_string()).context("an invite link carries its secret")?;
        self.keep_invite(&user, Some(&Invited { secret, name: name.trim().to_string(), user: user.clone(), role, until: made.until })).await?;
        Ok((made.link, made.until))
    }

    /// The requests to join that `caller` may approve, as sign-in lists them to this device: their own devices', and
    /// those of the people they invited whose proof is right, but those they said weren't theirs. Each with its code,
    /// which the device asking shows too, to compare, and when and where it asked from.
    pub async fn device_requests(&self, caller: &Caller) -> Result<Vec<Value>> {
        let asked: Value = self.ask_sign_in("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("sign-in's requests aren't requests")?;
        let invites = if caller.user == self.user()? { self.invites().await? } else { BTreeMap::new() };
        let refused = self.0.refused.lock().unwrap().clone();
        let mut out = Vec::new();
        for r in asked {
            let Ok(device) = r.endpoint() else { continue };
            if refused.contains(&r.key) || self.taken_out(&device.key)? {
                continue;
            }
            let mut shown = match mine_or_invited(&r, caller, &invites, &device) {
                Some(None) => json!({ "device": r.name, "person": r.person, "yours": true }),
                Some(Some(i)) => json!({ "device": r.name, "person": i.name, "role": i.role, "new": true }),
                None => continue,
            };
            // As the device asking shows it: "ABCD EFGH".
            let code = crate::keys::join_code(&device);
            shown["code"] = json!(format!("{} {}", &code[..4], &code[4..]));
            if let Some(made) = r.made {
                shown["made"] = json!(made);
            }
            if let Some(place) = &r.place {
                shown["place"] = json!(place);
            }
            // What approving it records as what they sign in with, for the card to say.
            if let Some(account) = self.signed_in_with(&r, None).await {
                shown["sign_in"] = json!(account);
            }
            out.push(shown);
        }
        Ok(out)
    }

    /// Turns down the device whose join code is `code`, one `caller` may approve: its request (and what its sign-in
    /// showed) is forgotten at sign-in, and it isn't shown here again when it asks again. One of their own ("Not Me") is
    /// turned down by an entry (`content::refuse`), so no device of theirs lets it in after. → which it was.
    pub async fn refuse_device(&self, caller: &Caller, code: &str) -> Result<Value> {
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_sign_in("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("sign-in's requests aren't requests")?;
        let r = waiting(asked, &code)?;
        let invites = if caller.user == self.user()? { self.invites().await? } else { BTreeMap::new() };
        let mine = mine_or_invited(&r, caller, &invites, &r.endpoint()?).context("that device isn't yours, or someone's you invited, to turn down")?;
        if mine.is_none() && !self.taken_out(&r.endpoint()?.key)? {
            ensure!(caller.user == self.user()?, "{} isn't one of your devices, so turn it down on one that is", self.device_name());
            self.member_content(caller, content::refuse(&r.endpoint()?, &r.name)).await?;
        }
        {
            let mut refused = self.0.refused.lock().unwrap();
            if refused.len() >= 50 {
                refused.clear();
            }
            refused.insert(r.key.clone());
        }
        self.ask_sign_in("DELETE", &format!("/api/auth/devices/requests/{}", r.key), json!({})).await?;
        Ok(json!({ "device": r.name }))
    }

    /// Lets back in someone who lost every device, with the network's recovery key, on this machine (never one asked
    /// over the network): the device whose join code is `code`, theirs, is listed by an entry the recovery key signs
    /// (which counts as an admin's approval, and is listed as its use), sealed each epoch of each scope they may open
    /// (opened through the recovery key's copy), and let onto the network by the recovery key. Without `confirm`, says
    /// whose it is. → what it is, or what was done.
    pub async fn approve_with_recovery(&self, caller: &Caller, code: &str, recovery: &ed25519_dalek::SigningKey, confirm: bool) -> Result<Value> {
        ensure!(self.this_machine(caller), "the recovery key is used only on the machine it's kept on");
        let at = recovery.verifying_key().to_bytes();
        ensure!(self.with(|i| Ok(i.r.db().recovery() == Some(at)))?, "that isn't {YOURS}'s recovery key");
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_sign_in("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("sign-in's requests aren't requests")?;
        let r = waiting(asked, &code)?;
        ensure!(r.proof.is_none(), "the recovery key lets in a device of someone already in {YOURS}");
        let person = self.with(|i| Ok(i.r.db().user(&r.user).map(|u| u.name)))?.with_context(|| format!("the recovery key lets in a device of someone already in {YOURS}"))?;
        if !confirm {
            return Ok(json!({ "device": r.name, "person": person, "recovery": true }));
        }
        let endpoint = r.endpoint()?;
        self.read_scope_now(&Scope::Global).await?;
        // Listed already by an approval that didn't finish: the rest is done again, each step once.
        if !self.with(|i| Ok(i.r.db().client(&endpoint.key).is_some_and(|c| c.user == r.user && c.seal == endpoint.seal)))? {
            self.write_step_as(recovery, &r.user, content::device(&endpoint, &r.user, &r.name, &[scopes::Kind::Client], None)).await?;
        }
        let relay = self.tell_relay_as(recovery.clone(), json!({ "add": r.key, "role": "client" })).await.err().map(|err| format!("{err:#}"));
        let scopes: Vec<Scope> = self.with(|i| Ok(i.r.db().may_scopes(&r.user, "use")))?;
        let (mut sealed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            match self.seal_from_recovery(&scope, recovery, &endpoint).await {
                Ok(0) => {}
                Ok(_) => sealed.push(scope.to_string()),
                Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
            }
        }
        // Marked approved, for the device waiting on it to see, with the network it's in.
        if let Err(err) = self.ask_sign_in("DELETE", &self.approved_path(&r.key)?, json!({})).await {
            tracing::warn!("marking a request approved at sign-in: {err:#}");
        }
        Ok(json!({ "device": r.name, "person": person, "recovery": true, "sealed": sealed, "not_sealed": not, "relay": relay }))
    }

    /// Where a device's request is marked approved at sign-in, saying the network it was let into (its root),
    /// the only one that device then takes, and once `global`'s log is frozen, where this device read its scope database
    /// to, which that device starts from (`steps::Welcome`).
    fn approved_path(&self, key: &str) -> Result<String> {
        let (root, welcome) = self.with(|i| Ok((i.r.root.map(|r| r.to_hex()), i.r.welcome()?)))?;
        let root = root.with_context(|| format!("this device isn't in {YOURS} yet"))?;
        let welcome = welcome.map(|w| format!("&welcome={w}")).unwrap_or_default();
        Ok(format!("/api/auth/devices/requests/{key}?approved&root={root}{welcome}"))
    }

    /// Seals each epoch of `scope` its entries made, opened through the recovery key's copy, to `to` where it isn't yet.
    /// → how many it opened.
    async fn seal_from_recovery(&self, scope: &Scope, recovery: &ed25519_dalek::SigningKey, to: &crate::keys::Endpoint) -> Result<usize> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let hashes: BTreeMap<u32, String> = self.with(|i| Ok(i.r.db().epoch_hashes(scope)))?;
        let secret = crate::keys::seal_secret(recovery);
        let mut opened = 0;
        for (n, hash) in hashes {
            let Some(o) = Bucket::get(&bucket, &crate::keys::epoch_name(&name, n, "recovery")).await? else {
                tracing::warn!("{scope}'s epoch {n} isn't sealed to the recovery key");
                continue;
            };
            let key = crate::keys::open_sealed(&secret, &o.bytes).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).filter(|k| crate::keys::epoch_hash(k) == hash);
            let Some(key) = key else {
                tracing::warn!("{scope}'s epoch {n} sealed to the recovery key isn't the key its entry made: passed over");
                continue;
            };
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::database::stored::created(&bucket, &at, &crate::keys::seal_to(&to.seal, &key)?).await?;
            }
            opened += 1;
        }
        Ok(opened)
    }

    /// Approves the device whose join code is `code`, for `caller`, whose device this is: with `confirm`, lets it in
    /// (`approve_request`) and forgets its request; without, says what it is. → what it is, or what was done.
    pub async fn approve(&self, caller: &Caller, code: &str, confirm: bool) -> Result<Value> {
        // Its entries count, and the keys it seals are its person's, only as one of their own devices.
        ensure!(caller.user == self.user()?, "{} isn't one of your devices, so approve it on one that is", self.device_name());
        let code = code_of(code);
        ensure!(code.len() == 8, "a code is the 8 letters and digits the new device shows");
        let asked: Value = self.ask_sign_in("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("sign-in's requests aren't requests")?;
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
        // Marked approved, for the device waiting on it to see, with the network it's in.
        if let Err(err) = self.ask_sign_in("DELETE", &self.approved_path(&r.key)?, json!({})).await {
            tracing::warn!("marking a request approved at sign-in: {err:#}");
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
        ensure!(!self.taken_out(&endpoint.key)?, "{} was turned down or taken out, so it isn't let in again: start again on it, for a new code", r.name);
        let on = r.device.as_deref().map(crate::keys::unhex).transpose().context("the device it's on is 64 hex characters")?;
        let mut listed = match invite {
            Some(i) => {
                let proof = r.proof.as_deref().context("a newcomer's device brings their invite's proof")?;
                ensure!(proof == crate::keys::invite_proof(i.secret.as_bytes(), &endpoint), "that device's proof isn't from {}'s invite", i.name);
                ensure!(r.user == i.user, "that device isn't {}'s", i.name);
                content::device(&endpoint, &r.user, &r.name, &[scopes::Kind::Client], Some((&i.name, i.role)))
            }
            None => {
                ensure!(r.user == caller.user, "that device is {}'s: one of their own devices approves it", if r.person.is_empty() { &r.user } else { &r.person });
                content::device(&endpoint, &r.user, &r.name, &[scopes::Kind::Client], None)
            }
        };
        if let Some(on) = &on {
            listed.data["device"] = json!(hex(on));
        }
        // What it signed in with, checked here, recorded with it: their next device's sign-in is checked against it.
        if let Some(account) = self.signed_in_with(r, None).await {
            listed.data["sign_in"] = json!(account);
        }
        if let Some(place) = &r.place {
            listed.data["place"] = json!(place);
        }
        // Approved before, but its request wasn't marked so (the device still waits on it): what's left is done again.
        let already = self.with(|i| Ok(i.r.db().client(&endpoint.key).is_some_and(|c| c.user == r.user && c.seal == endpoint.seal)))?;
        if !already {
            self.member_content(caller, listed).await?;
        }
        // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
        // keep their keys.
        if let Some(on) = on {
            let before: Vec<Key> = self.with(|i| {
                Ok(i.r.db().clients().into_iter().filter(|c| c.user == r.user && c.device == Some(on) && c.key != endpoint.key && c.passkey.is_none()).map(|c| c.key).collect())
            })?;
            for old in before {
                if let Err(err) = self.tell_relay_now(json!({ "remove": hex(&old) })).await {
                    tracing::warn!("taking {}'s sign-in before off the network: {err:#}", r.name);
                }
                self.member_as(caller, "remove", content::remove_device(&old).data).await?;
            }
        }
        let admin = self.with(|i| Ok(i.r.db().may(&r.user, "invite", scopes::NETWORK)))?;
        if invite.is_some_and(|i| i.role == Role::Admin)
            && let Err(err) = self.ask_sign_in("POST", "/api/auth/people/admin", json!({ "user": r.user, "admin": true })).await
        {
            tracing::warn!("telling sign-in {} is an admin: {err:#}", r.user);
        }
        let scopes: Vec<Scope> = self.with(|i| {
            let db = i.r.db();
            Ok(db.may_scopes(&r.user, "use").into_iter().filter(|s| invite.is_none() || *s != Scope::User(r.user.clone())).collect())
        })?;
        let (mut sealed, mut not) = (Vec::new(), Vec::new());
        for scope in scopes {
            match self.seal_held(&scope, &endpoint).await {
                Ok(()) => sealed.push(scope.to_string()),
                Err(err) => not.push(json!({ "scope": scope.to_string(), "why": format!("{err:#}") })),
            }
        }
        // Last, and waited for where there's a relay (at the network's address): until it lets the device in, the device
        // reaches nothing and keeps waiting. Approving it again finishes this, as it's listed already.
        if self.address().is_ok() {
            let told = self.tell_relay_now(json!({ "add": r.key, "admin": admin, "role": "client" })).await;
            told.with_context(|| format!("{} is let in, but {YOURS}'s network didn't take it yet: approve it again", r.name))?;
        }
        Ok(json!({ "device": r.name, "sealed": sealed, "not_sealed": not }))
    }

    /// What `r`'s sign-in shows it signed in with, checked here: against `accounts` (the person's, as their devices
    /// record them) for one let in by it, else for one approved by its code. None if it shows nothing that checks out.
    async fn signed_in_with(&self, r: &DeviceRequest, accounts: Option<&[crate::sign_in_proof::Account]>) -> Option<crate::sign_in_proof::Account> {
        let proof = r.proof()?;
        let (endpoint, network) = (r.endpoint().ok()?, self.sign_in_network().ok()?);
        let jwks = match proof.issuer().and_then(|i| crate::sign_in_proof::keys_at(&i, &network.address)) {
            Some(at) => match self.issuer_keys(&at).await {
                Ok(keys) => Some(keys),
                Err(err) => {
                    tracing::warn!("fetching {at}, to check {}'s sign-in: {err:#}", r.name);
                    return None;
                }
            },
            None => None,
        };
        let checked = match accounts {
            Some(theirs) => crate::sign_in_proof::vouches(&proof, &endpoint, &network, jwks.as_ref(), theirs, now_ms()),
            None => crate::sign_in_proof::check(&proof, &endpoint, &network, jwks.as_ref(), now_ms()),
        };
        checked.map_err(|err| tracing::info!("{}'s sign-in doesn't let it in by itself: {err:#}", r.name)).ok()
    }

    /// The network a new device's sign-in must be for, as this device's signed entries say.
    fn sign_in_network(&self) -> Result<crate::sign_in_proof::Network> {
        let address = self.address()?;
        self.with(|i| {
            let root = i.r.root.with_context(|| format!("this device isn't in {YOURS} yet"))?;
            Ok(crate::sign_in_proof::Network::new(&address, *root.as_bytes(), i.r.db().setting(crate::sign_in_proof::CLIENTS).as_deref()))
        })
    }

    /// Whether `key` was listed and taken out, or turned down before it was: never let in again.
    fn taken_out(&self, key: &Key) -> Result<bool> {
        self.with(|i| Ok(i.r.db().machine_ever(key).is_some_and(|m| m.removed)))
    }

    /// Forgets what one of `caller`'s devices (`key`, hex) signed in with: a new device's sign-in is no longer checked
    /// against it.
    pub async fn forget_sign_in(&self, caller: &Caller, key: &str) -> Result<()> {
        ensure!(caller.user == self.user()?, "{} isn't one of your devices, so do this on one that is", self.device_name());
        let key = crate::keys::unhex(key).context("a device's key is 64 hex characters")?;
        let theirs = self.with(|i| Ok(i.r.db().machine(&key).is_some_and(|m| m.user == caller.user && m.sign_in.is_some())))?;
        ensure!(theirs, "that isn't one of your devices with a sign-in to forget");
        self.member_content(caller, content::forget_sign_in(&key)).await
    }

    /// An issuer's keys, published at `at`: kept for an hour.
    async fn issuer_keys(&self, at: &str) -> Result<Value> {
        if let Some((keys, when)) = self.0.sign_ins.lock().unwrap().keys.get(at)
            && when.elapsed() < ISSUER_KEYS_FOR
        {
            return Ok(keys.clone());
        }
        let url = at.to_string();
        let keys: Value = tokio::task::spawn_blocking(move || -> Result<Value> {
            let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
            Ok(agent.get(&url).call()?.body_mut().read_json()?)
        })
        .await??;
        self.0.sign_ins.lock().unwrap().keys.insert(at.to_string(), (keys.clone(), Instant::now()));
        Ok(keys)
    }

    /// Every few seconds, on a client of its person's, lets in each of their new devices whose sign-in shows it's
    /// theirs (`sign_in_proof::vouches`), as their approving it would, with no one tapping; any other waits for them to
    /// approve it by its code. Nodes that only run calls let in no one.
    pub(super) async fn sign_in_loop(self) {
        loop {
            tokio::time::sleep(SIGN_INS_EVERY).await;
            if let Err(err) = self.approve_by_sign_in().await {
                tracing::debug!("looking for devices let in by their sign-in: {err:#}");
            }
        }
    }

    async fn approve_by_sign_in(&self) -> Result<()> {
        let me = self.with(|i| Ok(i.r.db().client(&self.0.id).map(|c| c.user)))?;
        let Some(user) = me else { return Ok(()) };
        if self.address().is_err() {
            return Ok(());
        }
        let asked: Value = self.ask_sign_in("GET", "/api/auth/devices/requests", json!({})).await?;
        let asked: Vec<DeviceRequest> = serde_json::from_value(asked["requests"].clone()).context("sign-in's requests aren't requests")?;
        let theirs = asked.into_iter().filter(|r| r.user == user && r.proof.is_none() && r.sign_in.is_some());
        for r in theirs {
            let accounts = self.with(|i| Ok(i.r.db().sign_ins(&user)))?;
            // Checked once against the accounts they have now, and again if those change.
            let tried = format!("{} {}", r.key, json!(accounts));
            let done = self.0.refused.lock().unwrap().contains(&r.key) || self.0.sign_ins.lock().unwrap().refused.contains(&tried) || r.endpoint().ok().is_none_or(|d| self.taken_out(&d.key).unwrap_or(true));
            if done || accounts.is_empty() {
                continue;
            }
            if self.signed_in_with(&r, Some(&accounts)).await.is_none() {
                let mut kept = self.0.sign_ins.lock().unwrap();
                if kept.refused.len() >= 100 {
                    kept.refused.clear();
                }
                kept.refused.insert(tried);
                continue;
            }
            // Another device of theirs may have just let it in.
            self.read_scope_now(&Scope::Global).await?;
            let caller = Caller::own(self.0.id, user.clone());
            let approved = self.approve_request(&caller, &r, None).await;
            match approved {
                Ok(_) => {
                    tracing::info!("let {} in by its sign-in", r.name);
                    if let Err(err) = self.ask_sign_in("DELETE", &self.approved_path(&r.key)?, json!({})).await {
                        tracing::warn!("marking a request approved at sign-in: {err:#}");
                    }
                }
                Err(err) => tracing::warn!("letting {} in by its sign-in: {err:#}", r.name),
            }
        }
        Ok(())
    }

    /// Lists one of this device's parts, the service `service`, as its person's, a part of this device (oned turned it
    /// on, and it asked with its endpoint and whether it serves everyone), and tells the relay to let it in. It holds no
    /// keys: the nodes that call it bring it the network's entries, which say who may. A part listed before it named its
    /// device is listed again, naming it.
    pub async fn list_part(&self, endpoint: &crate::keys::Endpoint, service: &str, shared: bool) -> Result<()> {
        let user = self.user()?;
        let caller = Caller::own(self.0.id, user.clone());
        let listed = self.with(|i| {
            let db = i.r.db();
            ensure!(db.service(&endpoint.key).is_some() || db.machine_ever(&endpoint.key).is_none(), "that key is listed already, and not as a service");
            Ok(db.service(&endpoint.key).map(|s| s.device == Some(self.0.id)))
        })?;
        if listed.is_none() {
            // Told first: a service the relay won't let in is never reached.
            if let Err(err) = self.tell_relay_now(json!({ "add": hex(&endpoint.key), "admin": false, "role": "service" })).await {
                ensure!(format!("{err:#}").contains("in already"), "{YOURS}'s relay won't let {service} in: {err:#}");
            }
        }
        if listed != Some(true) {
            let name = self.device_name();
            self.member_content(&caller, content::service_endpoint(endpoint, &user, &name, service, shared, &self.0.id)).await?;
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
            tracing::warn!("service {} was listed but this device hasn't reached it", crate::keys::short(&key));
        });
        Ok(())
    }

    /// Seals each epoch of `scope` this device holds to `to`, where it isn't yet.
    async fn seal_held(&self, scope: &Scope, to: &crate::keys::Endpoint) -> Result<()> {
        let (bucket, name) = (self.bucket()?, scope.to_string());
        for (n, key) in self.epoch_keys(scope, false).await? {
            let at = crate::keys::epoch_name(&name, n, &hex(&to.key));
            if Bucket::get(&bucket, &at).await?.is_none() {
                crate::database::stored::created(&bucket, &at, &crate::keys::seal_to(&to.seal, &key)?).await?;
            }
        }
        Ok(())
    }

    /// Erases what this device keeps (by name or key), for its person or an admin asking from one of their own devices,
    /// directly: asked of another device, it says to ask that one, if it's on. Once it has answered, its oned stops its
    /// parts and deletes its home (its copies of the network's data, the pages it kept, and each service's folder and
    /// data, WhatsApp's link among them), then stops. Removing it next takes it out. → what's erasing.
    pub async fn erase(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let name = self.with(|i| Ok(i.r.db().machine(&key).map(|d| d.name).unwrap_or_default()))?;
        if key != self.0.id {
            ensure!(self.with(|i| Ok(i.online(&key)))?, "{name} isn't on, so it can't erase what it keeps");
            return Err(super::calls::Elsewhere { why: format!("only {name} erases what it keeps"), nodes: vec![hex(&key)] }.into());
        }
        self.with(|i| {
            let db = i.r.db();
            ensure!(caller.key != self.0.id && db.client(&caller.key).is_some() && db.may(&caller.user, "remove", &scopes::device(&key)), "only {name}'s person or an admin erases it, from another of their devices");
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
    /// said: one of its own admins' devices does. A device removing itself (as it signs out) forgets its own keys, so
    /// its scopes keep theirs (ARCHITECTURE.md), its `remove` entry saying so (`itself`); it's told off the relay last,
    /// since it hears no answer after. → what was done.
    pub async fn remove_device(&self, caller: &Caller, device: &str) -> Result<Value> {
        let key = self.find_machine(device)?;
        let (gone, keys, scopes) = self.with(|i| {
            let db = i.r.db();
            let d = db.machine(&key).context("no such device")?;
            ensure!(db.may(&caller.user, "remove", &scopes::device(&key)), "only its person or an admin removes a device");
            let keys: Vec<Key> = std::iter::once(key).chain(db.machines().into_iter().filter(|e| e.device == Some(key) && e.key != key && !e.removed).map(|e| e.key)).collect();
            let opens = |s: &Scope| keys.iter().filter_map(|k| db.machine(k)).any(|d| d.is(scopes::Kind::Client) && db.may(&d.user, "use", &scopes::object(s)));
            Ok((d.name, keys.clone(), db.scopes().into_iter().filter(opens).collect::<Vec<_>>()))
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
            let mut said = content::remove_device(k).data;
            if itself {
                said["itself"] = json!(true);
            }
            self.member_as(caller, "remove", said).await?;
        }
        let (renewed, left) = if itself { Default::default() } else { self.renew(caller, scopes).await };
        if itself {
            relay = off_relay().await;
        }
        Ok(json!({ "removed": gone, "new_epochs": renewed, "not_renewed": left, "relay": relay }))
    }

    /// Removes a person, for an admin: the relay told to let each of their devices in no more first, then their removal,
    /// and a new epoch of each scope they could open but their own (`global`, and each group they were in),
    /// sealed to every client left that opens it and the recovery key. Their own scope is kept as it is: removing deletes
    /// nothing. → what was done.
    pub async fn remove_person(&self, caller: &Caller, user: &str) -> Result<Value> {
        let user = self.find_user(user)?;
        let (name, keys, scopes) = self.with(|i| {
            let db = i.r.db();
            ensure!(db.may(&caller.user, "remove", scopes::NETWORK), "only admins remove someone");
            ensure!(user != caller.user, "you can't remove yourself: another admin can");
            let name = db.user(&user).context("no such person")?.name;
            let keys: Vec<Key> = db.machines().into_iter().filter(|d| d.user == user && !d.removed).map(|d| d.key).collect();
            let mut scopes = vec![Scope::Global];
            scopes.extend(db.groups_of(&user).into_keys().map(Scope::Group));
            Ok((name, keys, scopes))
        })?;
        ensure!(!keys.contains(&self.0.id), "this device is {name}'s: remove them from another");
        let mut relay = Vec::new();
        for key in &keys {
            if let Err(err) = self.tell_relay_now(json!({ "remove": hex(key) })).await {
                relay.push(format!("{err:#}"));
            }
        }
        self.member_content(caller, content::remove_user(&user)).await?;
        let (renewed, left) = self.renew(caller, scopes).await;
        Ok(json!({ "removed": name, "new_epochs": renewed, "not_renewed": left, "relay": (!relay.is_empty()).then(|| relay.join("; ")) }))
    }

    /// A new epoch of each scope, where `caller` may start one: → those made, and those left to one of their admins.
    async fn renew(&self, caller: &Caller, scopes: Vec<Scope>) -> (Vec<Value>, Vec<String>) {
        let (mut renewed, mut left) = (Vec::new(), Vec::new());
        for scope in scopes {
            if !self.with(|i| Ok(starts_epochs(i.r.db(), &caller.user, &scope))).unwrap_or(false) {
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

    /// Makes, in the background, each scope's keys that are owed, of what this device's person makes them of: a new
    /// epoch a removal or stopped app schedules asked for (`ScopeDb::owed`) that the device writing it didn't make (it failed,
    /// or went off first); or a new epoch or Vault key pair where the newest's entry lists this device but it was never
    /// sealed to it, as when the device making it stopped between its entry and its seals, so nobody holds it. Only
    /// once one has been owed at two looks in a row, so a removal's own new epoch, and every seal, goes first.
    pub(super) fn start_owed_keys(&self) {
        let Ok(me) = self.user() else { return };
        let (mut owed, listed, vaults) = self
            .with(|i| {
                let db = i.r.db();
                if db.client(&self.0.id).is_none_or(|c| c.user != me) {
                    return Ok(Default::default());
                }
                let owed: BTreeSet<Owing> = db.owed().into_iter().filter(|s| starts_epochs(db, &me, s)).map(Owing::Epoch).collect();
                let epochs = db.epochs().into_iter().filter(|(s, _)| db.sealed_to(s).contains(&self.0.id) && starts_epochs(db, &me, s)).map(|(s, n)| (Owing::Epoch(s), n));
                let vaults: BTreeMap<Scope, (u32, Vec<Key>)> = db.vaults().into_iter().filter(|(s, v)| v.to.contains(&self.0.id) && db.may(&me, "choose-devices", &scopes::object(s))).map(|(s, v)| (s, (v.n, v.to))).collect();
                let listed: Vec<(Owing, u32)> = epochs.chain(vaults.iter().map(|(s, (n, _))| (Owing::Vault(s.clone()), *n))).collect();
                Ok((owed, listed, (vaults, db.recovery())))
            })
            .unwrap_or_default();
        let (due, unsure) = {
            let mut held = self.0.owed.lock().unwrap();
            owed.extend(held.unsealed.iter().filter(|e| listed.contains(e)).map(|(o, _)| o.clone()));
            let due = owed.intersection(&held.seen).filter(|o| !held.starting.contains(*o)).cloned().collect::<Vec<_>>();
            held.starting.extend(due.iter().cloned());
            held.seen = owed;
            let unsure: Vec<(Owing, u32)> = listed.into_iter().filter(|e| !held.sealed.contains(e)).collect();
            (due, unsure)
        };
        if !unsure.is_empty() {
            let node = self.clone();
            tokio::spawn(async move {
                let Ok(bucket) = node.bucket() else { return };
                let mut unsealed = BTreeSet::new();
                for (owing, n) in unsure {
                    let name = match &owing {
                        Owing::Epoch(s) => crate::keys::epoch_name(&s.to_string(), n, &hex(&node.0.id)),
                        Owing::Vault(s) => crate::keys::vault_key_name(&s.to_string(), n, &hex(&node.0.id)),
                    };
                    match Bucket::get(&bucket, &name).await {
                        Ok(Some(_)) => drop(node.0.owed.lock().unwrap().sealed.insert((owing, n))),
                        Ok(None) => drop(unsealed.insert((owing, n))),
                        Err(_) => {}
                    }
                }
                node.0.owed.lock().unwrap().unsealed = unsealed;
            });
        }
        for owing in due {
            let (node, me, (vaults, recovery)) = (self.clone(), Caller::own(self.0.id, me.clone()), vaults.clone());
            tokio::spawn(async move {
                let made = match &owing {
                    Owing::Epoch(scope) => node.new_epoch(&me, scope).await.map(|_| ()),
                    Owing::Vault(scope) => match (vaults.get(scope), recovery) {
                        (Some((n, to)), Some(recovery)) => node.new_vault_key(scope, n + 1, to, &recovery).await,
                        _ => Ok(()),
                    },
                };
                match made {
                    Ok(()) => tracing::info!("made {owing:?}'s new keys, which it owed"),
                    Err(err) => tracing::warn!("making {owing:?}'s new keys, which it owed: {err:#}"),
                }
                node.0.owed.lock().unwrap().starting.remove(&owing);
            });
        }
    }

    /// What a person's first device does once it's in, here for a machine (cloudflare/web/device.js for a browser): where
    /// their own things run besides their own machines, if their devices never said (the network's machines that run
    /// everyone's, chosen without asking), and their own scope's first epoch, if it isn't made.
    pub(super) fn settle_own(&self) {
        let node = self.clone();
        tokio::spawn(async move {
            let todo = node.with(|i| {
                let db = i.r.db();
                let me = i.user.clone();
                if db.client(&node.0.id).is_none_or(|c| c.user != me) || db.epoch(&Scope::User(me.clone())) > 0 {
                    return Ok(None);
                }
                Ok(Some((me.clone(), db.runs_until_chosen(&me))))
            });
            let Ok(Some((me, on))) = todo else { return };
            if !on.is_empty()
                && let Err(err) = node.member_content(&Caller::own(node.0.id, me.clone()), content::runs(&on)).await
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
            Ok(match scope {
                Scope::Global => "everyone's".to_string(),
                Scope::User(u) => format!("{}'s own", Some(super::whose(i.r.db(), scope)).filter(|n| !n.is_empty()).unwrap_or(u.clone())),
                Scope::Group(g) => format!("{}'s", Some(super::whose(i.r.db(), scope)).filter(|n| !n.is_empty()).unwrap_or(g.clone())),
            })
        })
        .unwrap_or_default()
    }

    /// A scope's next epoch: its `epoch` entry first, naming its key's hash (which, first in causal order, makes it),
    /// then the key sealed to the recovery key, if there is one, and to every client device of someone who uses the scope, before
    /// anything uses it. → its number.
    pub(super) async fn new_epoch(&self, caller: &Caller, scope: &Scope) -> Result<u32> {
        let newest = self.newest_epoch(scope).await?;
        ensure!(newest >= 1, "{scope} has no keys yet");
        let n = newest + 1;
        let (recovery, to) = self.with(|i| Ok((i.r.db().recovery(), i.r.db().openers(scope))))?;
        let (bucket, name) = (self.bucket()?, scope.to_string());
        let key: [u8; 32] = rand::random();
        let hash = crate::keys::epoch_hash(&key);
        self.member_content(caller, content::epoch(scope, n, &to.iter().map(|c| c.key).collect::<Vec<_>>(), &hash)).await?;
        ensure!(self.with(|i| Ok(i.r.db().epoch_hash(scope, n).as_deref() == Some(hash.as_str())))?, "another device started {scope}'s epoch {n} just now");
        // Not while there's none (the one there was forked `global`), until an admin names the next: a removal still
        // takes keys away from whoever it removes.
        if let Some(recovery) = recovery
            && !crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, "recovery"), &crate::keys::seal_to(&crate::keys::twin_public(&recovery)?, &key)?).await?
        {
            tracing::warn!("{scope}'s epoch {n} was sealed to the recovery key already, not with its key: the recovery key won't open it");
        }
        for c in &to {
            crate::database::stored::created(&bucket, &crate::keys::epoch_name(&name, n, &hex(&c.key)), &crate::keys::seal_to(&c.seal, &key)?).await?;
        }
        self.forget_epochs(scope);
        Ok(n)
    }
}
