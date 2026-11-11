//! Each kind of step's rule: who may write it, as `may` questions of `global`'s scope database (and rows where a
//! question is of what exists), and what it changes, as SQL on the database it's applied to. What follows from those
//! rows (who owns which device and app, which devices run which scope's calls) is worked out once after
//! (`scope::derive`), never written by a step.
//!
//! The signer is checked first, for every kind (`signed`): one of its user's own devices still listed; or, for a
//! scope's own state, a device that runs the scope's calls for them; or, for letting a device in or handing over to
//! the next recovery key, the network's recovery key. So the user a step is written as is always someone in the
//! network, and each rule asks only what that user may do.

use ::custom_build::allowed;
use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::steps::Step;

use ::key::{Hash, Key, hex, unhex};

use crate::scopes::{Scope, is_id};
use crate::content::Change;
use crate::scopes::{AGENTS, EVERYONE, Kind, Machine, Principal, Role, handle_from, source_link, valid_app_name, valid_handle, valid_host, valid_page, valid_secret_name};
use ::key::APP;

use crate::scope::{group, int, owner, user_principal, role};
use crate::scopes::{NETWORK, ScopeDb, object, owner_object};

/// Whether a step is one of `global`'s own: who's who, written only by its user's own device (or the recovery key).
fn of_global(step: &Step, scope: &Scope) -> bool {
    *scope == Scope::Global && step.content.app == APP && !step.content.change().is_some_and(Change::own)
}

/// Whether `step`'s signer may write it as its user, in `scope`, by `who`, `global`'s database before it.
pub fn signed(who: &ScopeDb, scope: &Scope, step: &Step) -> Result<()> {
    let (w, author) = (step.user(), step.author());
    if who.recovery()? == Some(author) {
        // The network's recovery key, kept offline by an admin, only lets a device of someone's in, as an admin's would,
        // hands over to the next recovery key, and starts the next keys of each scope, which only it can do for a user
        // whose devices are all gone, so what the old key could open stops growing.
        ensure!(of_global(step, scope) && matches!(step.content.change(), Some(Change::Device | Change::Recovery | Change::Epoch | Change::Vault)) && who.user(w)?.is_some(), "the recovery key only lets a device in, or replaces itself");
        return Ok(());
    }
    allowed!(who.may(w, "use", &object(scope)), "{w} can't write in {scope}");
    let theirs = who.client(&author)?.is_some_and(|c| c.user == w);
    ensure!(theirs || (!of_global(step, scope) && who.runs_for(&author, scope)), "{} can't write in {scope} for that user", ::key::short(&author));
    Ok(())
}

/// Applies `step`, whose signer `signed` let write it, to `out`, `scope`'s database before it, by its kind's rule, `who`
/// being `global`'s database before it: → why it doesn't count, if it doesn't. Each kind asks once whether its user may
/// make the change, and a step the network's recovery key signed only what `signed` lets it.
pub fn apply(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Step) -> Result<()> {
    if step.content.app != APP {
        return Ok(());
    }
    let change = step.content.change();
    if *scope != Scope::Global || change.is_some_and(Change::own) {
        return own(who, out, scope, step);
    }
    let c = Rule { who, out, step, w: step.user(), data: &step.content.data };
    match change {
        Some(Change::Device) if who.recovery()? == Some(step.author()) => c.recovered(),
        Some(Change::Device) => c.device(),
        Some(Change::Refuse) => c.refuse(),
        Some(Change::ForgetSignIn) => c.forget_sign_in(),
        Some(Change::LinkSignIn) => c.link_sign_in(),
        Some(Change::Role) => c.role(),
        Some(Change::Remove) => c.remove(),
        Some(Change::Epoch) => c.epoch(),
        Some(Change::Runs) => c.runs(),
        Some(Change::AppSchedules) => c.app_schedules(),
        Some(Change::Vault) => c.vault(),
        Some(Change::Group) => c.group(),
        Some(Change::Invite) => c.invite(),
        Some(Change::InviteEmail) => c.invite_email(),
        Some(Change::WithdrawInvite) => c.withdraw_invite(),
        Some(Change::Accept) => c.accept(),
        Some(Change::Member) => c.member(),
        Some(Change::Rename) => c.rename(),
        Some(Change::Setting) => c.setting(),
        Some(Change::Recovery) => c.recovery(),
        Some(Change::App) => c.app(),
        Some(Change::Deploy) => c.deploy(),
        Some(Change::Service) => c.service(),
        Some(Change::AddApp | Change::RemoveApp) => own(who, out, scope, step),
        Some(Change::NodeRelease | Change::ServiceRelease) => Ok(()),
        Some(Change::Freeze | Change::Start | Change::Defaults | Change::Schedule | Change::VaultRequest | Change::Keep | Change::Forget | Change::Arrange) | None => {
            bail!("unknown kind {}", step.content.kind)
        }
    }
}

/// The network `root`'s first user, device, epoch and recovery key, as `data` says them (`content::first`), into
/// `out`, empty, at `now`.
pub fn first(out: &Connection, root: Hash, data: &Value, now: u64) -> Result<()> {
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let user = text("user")?;
    ensure!(is_id(&user), "a user's id is 16 hex characters");
    let machine = listed(&data["device"], &user, now)?;
    ensure!(machine.is(Kind::Client), "a network's first device is a client");
    let hash = epoch_hash_of(&data["epoch"])?;
    let time = int(now);
    out.execute("UPDATE network SET root = ?, name = ?, devices_at = ?", params![root.to_hex(), text("name")?, time])?;
    out.execute("INSERT INTO recovery VALUES (0, ?, ?)", params![hex(&unhex(&text("recovery")?)?), time])?;
    add_user(out, &user, &text("user_name")?, Role::Admin)?;
    put(out, &machine)?;
    out.execute("INSERT INTO epochs VALUES ('global', 1, ?)", [&hash])?;
    out.execute("INSERT INTO sealed VALUES ('global', 1, ?)", [hex(&machine.key)])?;
    Ok(())
}

struct Rule<'a> {
    who: &'a ScopeDb,
    out: &'a Connection,
    step: &'a Step,
    w: &'a str,
    data: &'a Value,
}

impl Rule<'_> {
    fn text(&self, k: &str) -> Result<String> {
        self.data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"))
    }

    fn key(&self, k: &str) -> Result<Key> {
        unhex(&self.text(k)?)
    }

    fn id(&self, k: &str) -> Result<String> {
        let id = self.text(k)?;
        ensure!(is_id(&id), "{k} isn't an id");
        Ok(id)
    }

    fn role_of(&self, k: &str) -> Result<Role> {
        Ok(serde_json::from_value(self.data[k].clone())?)
    }

    fn keys(&self, k: &str) -> Result<Vec<Key>> {
        self.data[k].as_array().with_context(|| format!("no {k}"))?.iter().map(|k| unhex(k.as_str().unwrap_or(""))).collect()
    }

    fn time(&self) -> i64 {
        int(self.step.time())
    }

    fn may(&self, verb: &str, object: &str) -> bool {
        self.who.may(self.w, verb, object)
    }

    fn may_make(&self, verb: &str, container: &str, subject: &str, relation: &str, object: &str) -> bool {
        crate::permissions::may_make(self.who.conn(), &user_principal(self.w), verb, container, subject, relation, object, self.who.now()).unwrap_or(false)
    }

    /// Whether `k` is one of the network's machines that run things, or its oned on Cloudflare.
    fn by_recovery(&self) -> Result<bool> {
        Ok(self.who.recovery()? == Some(self.step.author()))
    }

    fn runs_things(&self, k: &Key) -> Result<bool> {
        Ok(self.who.compute(k)?.is_some() || self.who.has_runtime(k)? || self.who.oned_on_cloudflare()? == Some(*k))
    }

    fn devices_changed(&self) -> Result<()> {
        self.out.execute("UPDATE network SET devices_at = max(devices_at, ?)", [self.time()])?;
        Ok(())
    }

    fn owe(&self, scope: &Scope) -> Result<()> {
        if self.who.epoch(scope)? > 0 {
            self.out.execute("INSERT OR IGNORE INTO owed VALUES (?)", [scope.to_string()])?;
        }
        Ok(())
    }

    /// Owes a new epoch of each scope whose keys `keys` held, which their removal takes the right to: each a client of
    /// them opens, but `except` (a user's own, which removing them keeps as it is), and each one of them runs app
    /// schedules in.
    fn owe_held_by(&self, keys: &[Key], except: Option<&Scope>) -> Result<()> {
        let mut clients = Vec::new();
        for k in keys {
            clients.extend(self.who.client(k)?);
        }
        let opens = |s: &Scope| clients.iter().any(|d| self.who.may(&d.user, "use", &object(s)));
        let scheduled = self.who.app_schedules()?.into_keys().filter(|(k, _, _)| keys.contains(k)).map(|(_, s, _)| s);
        for s in self.who.scopes()?.into_iter().filter(opens).chain(scheduled).filter(|s| Some(s) != except) {
            self.owe(&s)?;
        }
        Ok(())
    }

    /// A user who lost every device let back in with the recovery key: a new client of someone already in.
    fn recovered(&self) -> Result<()> {
        let user = self.id("user")?;
        let machine = listed(self.data, &user, self.step.time())?;
        ensure!(user == self.w && self.who.user(&user)?.is_some(), "the recovery key lets in a device of someone already in");
        ensure!(machine.roles == BTreeSet::from([Kind::Client]) && machine.passkey.is_none(), "the recovery key lets in a device users use");
        ensure!(self.who.key_for(&machine.key, &user)?.is_none(), "that key is listed already");
        put(self.out, &machine)?;
        self.devices_changed()?;
        self.out.execute("UPDATE network SET recovery_used_at = ?, recovery_used_by = ?", params![self.time(), user_principal(&user)])?;
        Ok(())
    }

    fn device(&self) -> Result<()> {
        let (w, user) = (self.w, self.id("user")?);
        let mut machine = listed(self.data, &user, self.step.time())?;
        machine.by = w.to_string();
        let theirs = self.may("add-devices", &user_principal(&user));
        // A service answers its user's calls as them: only their own device lists one, never an admin's.
        ensure!(!machine.is(Kind::Service) || theirs, "only a user's own device lists their services");
        // An app runtime carries its device's rights to run things: only that device says what's its runtime.
        if let Some(of) = machine.runtime_of() {
            ensure!(self.step.author() == of, "only a device lists its own app runtime");
            let device = self.who.client(&of)?.filter(|d| d.user == user && d.passkey.is_none() && d.device.is_none());
            ensure!(device.is_some(), "an app runtime is of one of its user's own devices");
            ensure!(self.who.key_for(&machine.key, &user)?.is_none() && !self.who.has_runtime(&of)?, "a device has one app runtime, listed once");
        }
        // A machine an admin listed for someone is listed again by that user's own device to be theirs: then it runs
        // their own scope too.
        if let Some(was) = self.who.key_for(&machine.key, &user)? {
            // Or a service of theirs listed again by their own device, to name the device it's part of; or a sign-in that
            // named no device, naming the one it's on.
            let part = was.is(Kind::Service) && was.roles == machine.roles && was.service == machine.service;
            let part = part || (was.is(Kind::Client) && was.roles == machine.roles && was.device.is_none() && machine.device.is_some() && was.passkey == machine.passkey);
            // Or a sign-in saying what it is, once, which changes nothing else about it.
            let says = was.is(Kind::Client) && was.roles == machine.roles && was.passkey == machine.passkey && was.platform.is_none() && machine.platform.is_some() && (was.device.is_none() || was.device == machine.device);
            let part = part || says;
            // Or a device with an app runtime of its own, no longer running things itself.
            let drops = was.roles == BTreeSet::from([Kind::Client, Kind::Compute]) && machine.roles == BTreeSet::from([Kind::Client]) && self.step.author() == machine.key && self.who.has_runtime(&machine.key)?;
            ensure!(!was.removed && was.user == user && theirs && (was.by != user || part || drops) && was.seal == machine.seal, "that key is listed already");
            if drops {
                machine = Machine { roles: machine.roles, ..was };
            } else if says {
                (machine.since, machine.place, machine.sign_in) = (was.since, was.place, was.sign_in);
            }
        } else if self.who.user_ever(&user)?.is_some() {
            let verb = if machine.is(Kind::Client) || machine.is(Kind::Service) { "add-devices" } else { "add-machines" };
            let lists = self.who.user(&user)?.is_some() && self.may_make(verb, &user_principal(&user), &user_principal(&user), "owner", &crate::scopes::device(&machine.key));
            ensure!(lists || self.lost(&user, &machine)?, "only a user's own device lists their devices; an admin's lists compute and services");
        } else {
            let r = self.role_of("role")?;
            let name = if self.data.get("invite").is_some() {
                self.by_invite(&user, r, &mut machine)?
            } else {
                allowed!(self.who.may_give(w, &user, role(r), NETWORK), "only admins add users");
                self.text("user_name")?
            };
            add_user(self.out, &user, &name, r)?;
        }
        put(self.out, &machine)?;
        self.devices_changed()
    }

    /// Whether an admin may let in this next device of `user`, who has no device of theirs left to approve it: a device
    /// people use, nothing else.
    fn lost(&self, user: &str, machine: &Machine) -> Result<bool> {
        let none_left = !self.who.clients()?.iter().any(|c| c.user == user && c.passkey.is_none());
        Ok(none_left && machine.roles == BTreeSet::from([Kind::Client]) && machine.passkey.is_none() && self.who.user(user)?.is_some() && self.may("invite", NETWORK))
    }

    /// A newcomer's first device, joining by the invite that names them: only their device people use, only with that
    /// invite, once, by an ID token for its email that names this device, checked with the key saved beside it, as its
    /// issuer's keys change. The invite is the approval, so any device of the network may write it, when that key is one
    /// the invite named; any other only an admin's, whose device checked it was the issuer's, as a member's could make
    /// up a key and a token to match. → their name.
    fn by_invite(&self, user: &str, r: Role, machine: &mut Machine) -> Result<String> {
        use crate::sign_in_proof::{self, Network};
        let invite = self.who.email_invite(user)?.context("they weren't invited by their email")?;
        ensure!(!invite.used, "that invite has been used");
        ensure!(invite.role == r, "they were invited as something else");
        ensure!(machine.roles == BTreeSet::from([Kind::Client]) && machine.passkey.is_none(), "an invite lets in a device people use");
        let root = self.who.root()?.context("no network")?;
        let address = self.who.setting("address")?.unwrap_or_default();
        let network = Network::new(&address, *root.as_bytes(), self.who.setting(sign_in_proof::CLIENTS)?.as_deref());
        let token = self.data["invite"]["token"].as_str().context("no token")?;
        let endpoint = ::key::Endpoint { key: machine.key, seal: machine.seal };
        let jwk = &self.data["invite"]["jwk"];
        allowed!(invite.keys.contains(jwk) || self.who.may_give(self.w, user, role(r), NETWORK), "only an admin's device lets them in by a key their invite didn't name");
        let shown = sign_in_proof::id_token(token, &endpoint, &network, jwk)?;
        let sign_in_proof::Account::Provider { email: Some(email), .. } = &shown.account else { bail!("that sign-in names no email") };
        ensure!(shown.email_verified && email.to_lowercase() == invite.email, "that sign-in isn't the invited email's");
        ensure!(shown.issued <= invite.until, "that invite had expired");
        self.out.execute("UPDATE email_invites SET used = ? WHERE user = ?", params![hex(&machine.key), user])?;
        machine.by = user.to_string();
        machine.sign_in = Some(shown.account);
        Ok(invite.name)
    }

    /// A device its user said isn't theirs ("Not Me"), before it was ever listed: its key is taken out now, so no device
    /// of theirs lets it in, and nothing it writes counts. It counts only against who said it (`ScopeDb::key_for`).
    fn refuse(&self) -> Result<()> {
        let key = self.key("key")?;
        ensure!(self.who.key_for(&key, self.w)?.is_none(), "that key is listed already: take it out instead");
        let data = json!({ "key": self.data["key"], "seal": self.data["seal"], "name": self.data["name"], "roles": [Kind::Client] });
        let mut machine = listed(&data, self.w, self.step.time())?;
        machine.removed = true;
        put(self.out, &machine)?;
        self.devices_changed()
    }

    /// What one of their clients, taken out or not, signed in with, forgotten by its user's own device: a new device's
    /// sign-in is no longer checked against it.
    fn forget_sign_in(&self) -> Result<()> {
        let key = self.key("key")?;
        ensure!(self.who.machine_ever(&key)?.context("no such device")?.user == self.w, "only its user's own device forgets what it signed in with");
        match self.data.get("account") {
            Some(account) => self.out.execute("DELETE FROM linked_sign_ins WHERE device = ? AND account = ?", [hex(&key), account_text(account)?])?,
            None => self.out.execute("UPDATE devices SET sign_in = NULL WHERE key = ?", [hex(&key)])?,
        };
        self.devices_changed()
    }

    /// One more account their device `key` checked is theirs, which their other devices then let a new one in by.
    fn link_sign_in(&self) -> Result<()> {
        let key = self.key("key")?;
        let device = self.who.machine(&key)?.context("no such device")?;
        ensure!(device.user == self.w && device.is(Kind::Client), "only its user's own client links a sign-in");
        self.out.execute("INSERT OR IGNORE INTO linked_sign_ins VALUES (?, ?)", [hex(&key), account_text(&self.data["account"])?])?;
        self.devices_changed()
    }

    /// A user's role in the network, given by who may give it, in place of the one they had, which who changes it
    /// takes from them unless they keep it.
    fn role(&self) -> Result<()> {
        let (user, to) = (self.id("user")?, self.role_of("role")?);
        let was = self.who.user(&user)?.context("no such user")?.role;
        let gives = self.who.may_give(self.w, &user, role(to), NETWORK);
        allowed!(gives && (to == was || self.who.may_take(self.w, &user, role(was), NETWORK)), "only admins change roles, and the last admin stays");
        set_role(self.out, &user_principal(&user), NETWORK, Some(to))
    }

    fn remove(&self) -> Result<()> {
        if let Ok(user) = self.id("user") {
            let was = self.who.user(&user)?.context("no such user")?.role;
            allowed!(self.who.may_take(self.w, &user, role(was), NETWORK), "only admins take someone out, and the last admin stays");
            let theirs: Vec<Key> = self.who.machines()?.into_iter().filter(|d| d.user == user).map(|d| d.key).collect();
            self.owe_held_by(&theirs, Some(&Scope::User(user.clone())))?;
            // Everything they held goes with them; what they made stays.
            self.out.execute("UPDATE principals SET removed = 1 WHERE id = ?", [user_principal(&user)])?;
            self.out.execute("DELETE FROM relations WHERE subject = ?", [user_principal(&user)])?;
            self.out.execute("DELETE FROM invites WHERE principal = ?", [user_principal(&user)])?;
        } else {
            let key = self.key("key")?;
            let gone = self.who.machine(&key)?.context("no such key")?;
            allowed!(self.may("remove", &crate::scopes::device(&key)), "only its user or an admin removes a device");
            // A device signing itself out deletes its own keys: signed by it, or, in steps written before devices
            // signed their own, by an app runtime of its user's it asked (`itself`). So does a sign-in before on a device
            // signed in again (`settings::approve`). One removed otherwise may still hold them.
            let author = self.step.author();
            let itself = author == key || (self.data["itself"] == true && self.who.machine_ever(&author)?.is_some_and(|a| a.user == gone.user));
            let again = gone.device.is_some() && gone.passkey.is_none() && self.who.clients()?.iter().any(|c| c.key != key && c.user == gone.user && c.device == gone.device && c.passkey.is_none());
            if !(itself || again) {
                self.owe_held_by(&[key], None)?;
            }
            self.out.execute("UPDATE devices SET removed = 1 WHERE key = ?", [hex(&key)])?;
        }
        self.devices_changed()
    }

    fn epoch(&self) -> Result<()> {
        let scope = Scope::parse(&self.text("scope")?)?;
        allowed!(self.by_recovery()? || self.may("remove", &object(&scope)), "only the scope's admins start its new epoch");
        if let Scope::Group(g) = &scope {
            self.who.group(g)?.context("no such group")?;
        }
        let n = self.data["n"].as_u64().context("no n")?;
        ensure!(n == u64::from(self.who.epoch(&scope)?) + 1, "an epoch follows the one before it");
        let to = self.keys("to")?;
        for k in &to {
            ensure!(self.who.machine(k)?.is_some(), "an epoch is sealed only to the network's endpoints");
        }
        let hash = epoch_hash_of(&self.data["hash"])?;
        let s = scope.to_string();
        self.out.execute("INSERT INTO epochs VALUES (?, ?, ?)", params![s, n as i64, hash])?;
        self.out.execute("DELETE FROM owed WHERE scope = ?", [&s])?;
        self.out.execute("DELETE FROM sealed WHERE scope = ?", [&s])?;
        for k in &to {
            self.out.execute("INSERT OR IGNORE INTO sealed VALUES (?, ?, ?)", params![s, n as i64, hex(k)])?;
        }
        Ok(())
    }

    /// Where a user's own things run besides their own machines: only from their own device, for their own scope,
    /// and only the network's machines that run things, and its oned on Cloudflare.
    fn runs(&self) -> Result<()> {
        let on: BTreeSet<Key> = self.keys("on").context("on is a list of endpoints' keys")?.into_iter().collect();
        for k in &on {
            ensure!(self.runs_things(k)?, "only the network's computers, and Cloudflare, run someone's own");
        }
        let me = user_principal(self.w);
        self.out.execute("INSERT OR IGNORE INTO runs_chosen VALUES (?)", [&me])?;
        self.out.execute("DELETE FROM runs WHERE principal = ?", [&me])?;
        for k in &on {
            self.out.execute("INSERT INTO runs VALUES (?, ?)", params![me, hex(&self.who.device_of(k))])?;
        }
        Ok(())
    }

    fn app_schedules(&self) -> Result<()> {
        let endpoint = self.key("endpoint")?;
        let scope = Scope::parse(&self.text("scope")?)?;
        let app = self.text("app")?;
        // The app runtime's own databases of a scope (its log, its Vault) are never an app's to schedule.
        ensure!(valid_app_name(&app), "app schedules are an app's");
        let until = self.data["until"].as_u64().context("no until")?;
        let mut hashes = Vec::new();
        for (n, h) in self.data["keys"].as_object().into_iter().flatten() {
            hashes.push((n.parse::<u32>().context("an epoch is a number")?, epoch_hash_of(h)?));
        }
        allowed!(self.may("choose-devices", &object(&scope)), "only the scope's user, or its group's admins, set app schedules in it");
        ensure!(until == 0 || self.who.machine(&endpoint)?.is_some(), "no such endpoint");
        let row = params![hex(&endpoint), scope.to_string(), app];
        let had = self.out.execute("DELETE FROM app_schedules WHERE device = ? AND scope = ? AND app = ?", row)? > 0;
        self.out.execute("DELETE FROM app_schedule_keys WHERE device = ? AND scope = ? AND app = ?", row)?;
        if until == 0 {
            if had {
                self.owe(&scope)?;
            }
            return Ok(());
        }
        self.out.execute("INSERT INTO app_schedules VALUES (?, ?, ?, ?, ?)", params![hex(&endpoint), scope.to_string(), app, user_principal(self.w), int(until)])?;
        for (n, hash) in hashes {
            self.out.execute("INSERT OR REPLACE INTO app_schedule_keys VALUES (?, ?, ?, ?, ?)", params![hex(&endpoint), scope.to_string(), app, n, hash])?;
        }
        Ok(())
    }

    /// Who opens a scope's Vault is its admins' to say, never a member's who can only add to it.
    fn vault(&self) -> Result<()> {
        let scope = Scope::parse(&self.text("scope")?)?;
        allowed!(self.by_recovery()? || self.may("choose-devices", &object(&scope)), "only the scope's user, or its admins, say which machines open its Vault");
        let n = self.data["n"].as_u64().context("no n")?;
        ensure!(n == u64::from(self.who.vault(&scope)?.map_or(0, |v| v.n)) + 1, "a Vault key follows the one before it");
        let public = self.key("public")?;
        let to = self.keys("to")?;
        for k in &to {
            ensure!(self.runs_things(k)?, "a Vault opens only on the network's machines that run things");
        }
        let s = scope.to_string();
        self.out.execute("INSERT INTO vault VALUES (?, ?, ?)", params![s, n as i64, hex(&public)])?;
        self.out.execute("DELETE FROM vault_opens WHERE scope = ?", [&s])?;
        for k in &to {
            self.out.execute("INSERT OR IGNORE INTO vault_opens VALUES (?, ?)", params![s, hex(k)])?;
        }
        Ok(())
    }

    /// A new group, its maker its first admin.
    fn group(&self) -> Result<()> {
        allowed!(self.may("make-groups", NETWORK), "only the network's users make groups");
        let g = self.id("group")?;
        ensure!(self.who.group(&g)?.is_none(), "that group exists");
        let name = self.text("name")?;
        self.out.execute("INSERT INTO principals VALUES (?, 'group', ?, ?, 0)", params![group(&g), name, pick_handle(self.out, &name, "group")?])?;
        set_role(self.out, &user_principal(self.w), &group(&g), Some(Role::Admin))
    }

    fn invite(&self) -> Result<()> {
        let (g, user, to) = (self.id("group")?, self.id("user")?, self.role_of("role")?);
        self.who.group(&g)?.context("no such group")?;
        allowed!(self.who.may_give(self.w, &user, role(to), &group(&g)), "only the group's admins invite");
        self.who.user(&user)?.context("no such user")?;
        self.out.execute("INSERT OR REPLACE INTO invites VALUES (?, ?, ?)", params![user_principal(&user), group(&g), role(to)])?;
        Ok(())
    }

    /// Someone new, named by the email they'll sign in with: only by who may add them, for the days an invite lasts.
    fn invite_email(&self) -> Result<()> {
        let (user, to) = (self.id("user")?, self.role_of("role")?);
        allowed!(self.who.may_give(self.w, &user, role(to), NETWORK), "only admins invite someone new");
        ensure!(self.who.user_ever(&user)?.is_none(), "they're in the network already");
        let (email, name) = (self.text("email")?.to_lowercase(), self.text("name")?);
        ensure!(email.contains('@') && !name.trim().is_empty(), "an invite names an email and a name");
        let until = self.data["until"].as_u64().context("no until")?;
        ensure!(until <= self.step.time() + crate::sign_in_proof::INVITE_DAYS * 86_400_000, "an invite lasts {} days at most", crate::sign_in_proof::INVITE_DAYS);
        let keys = self.data["keys"].as_array().cloned().unwrap_or_default();
        ensure!(keys.len() <= 16 && keys.iter().all(Value::is_object), "an invite's keys are its provider's, a few at most");
        self.out.execute("INSERT OR REPLACE INTO email_invites VALUES (?, ?, ?, ?, ?, NULL, ?, ?)", params![user, email, name.trim(), role(to), int(until), self.w, serde_json::to_string(&keys)?])?;
        Ok(())
    }

    /// An invite by email not taken up yet, withdrawn by an admin: whoever it named can't join by it.
    fn withdraw_invite(&self) -> Result<()> {
        let user = self.id("user")?;
        allowed!(self.may("invite", NETWORK), "only an admin withdraws an invite");
        ensure!(self.out.execute("DELETE FROM email_invites WHERE user = ? AND used IS NULL", [&user])? == 1, "no such invite");
        Ok(())
    }

    fn accept(&self) -> Result<()> {
        let g = self.id("group")?;
        let invited = self.who.group(&g)?.context("no such group")?.invited.get(self.w).copied().context("not invited")?;
        self.out.execute("DELETE FROM invites WHERE principal = ? AND invited_to = ?", params![user_principal(self.w), group(&g)])?;
        set_role(self.out, &user_principal(self.w), &group(&g), Some(invited))
    }

    /// A member's role in a group changed, or them taken out of it (or their invite), by its admins; or leaving it. Who
    /// changes it gives the new role and takes the one they had, unless they keep it.
    fn member(&self) -> Result<()> {
        let (g, user) = (self.id("group")?, self.id("user")?);
        let to: Option<Role> = serde_json::from_value(self.data["role"].clone())?;
        let grp = self.who.group(&g)?.context("no such group")?;
        let was = self.who.group_role(&user, &g)?.or_else(|| grp.invited.get(&user).copied());
        let took = was.is_none_or(|r| Some(r) == to || self.who.may_take(self.w, &user, role(r), &group(&g)));
        let gives = to.is_none_or(|r| self.who.may_give(self.w, &user, role(r), &group(&g)));
        allowed!(took && gives, "only the group's admins change its members");
        let member = grp.members.contains_key(&user) && self.who.user(&user)?.is_some();
        ensure!(member || grp.invited.contains_key(&user), "not in the group");
        match to {
            Some(_) => ensure!(member, "not in the group"),
            None => {
                self.out.execute("DELETE FROM invites WHERE principal = ? AND invited_to = ?", params![user_principal(&user), group(&g)])?;
                if member {
                    self.owe(&Scope::Group(g.clone()))?;
                }
            }
        }
        set_role(self.out, &user_principal(&user), &group(&g), to)
    }

    fn rename(&self) -> Result<()> {
        let (it, row) = if let Ok(key) = self.key("key") {
            self.who.machine(&key)?.context("no such key")?;
            (crate::scopes::device(&key), None)
        } else if let Ok(user) = self.id("user") {
            self.who.user(&user)?.context("no such user")?;
            (user_principal(&user), Some(Principal::User(user)))
        } else {
            let g = self.id("group")?;
            self.who.group(&g)?.context("no such group")?;
            (group(&g), Some(Principal::Group(g)))
        };
        ensure!(self.may("rename", &it), ::custom_build::protocol::Coded::refused("not allowed to rename that"));
        if let Some(handle) = self.data.get("handle") {
            let handle = handle.as_str().context("a handle is text")?;
            ensure!(valid_handle(handle), "a handle is lowercase letters, digits and single hyphens, up to 39, like {}", handle_from(handle, "name"));
            let mine = row.context("a handle is a user's or a group's")?;
            ensure!(handle != EVERYONE && self.who.by_handle(handle)?.is_none_or(|p| p == mine), "{handle} is taken");
            self.out.execute("UPDATE principals SET handle = ? WHERE id = ?", params![handle, owner(&mine)])?;
            return Ok(());
        }
        let name = self.text("name")?;
        ensure!(!name.trim().is_empty(), "no name");
        match row {
            None => self.out.execute("UPDATE devices SET name = ? WHERE key = ?", params![name, hex(&self.key("key")?)])?,
            Some(_) => self.out.execute("UPDATE principals SET name = ? WHERE id = ?", params![name, it])?,
        };
        Ok(())
    }

    fn setting(&self) -> Result<()> {
        allowed!(self.may("change-settings", NETWORK), "only admins change the network's settings");
        let name = self.text("name")?;
        ensure!(crate::rules::valid_setting(&name), "setting names are lowercase words joined by _, or a lab's (lab.…)");
        match &self.data["value"] {
            Value::Null => self.out.execute("DELETE FROM settings WHERE name = ?", [&name])?,
            value => self.out.execute("INSERT OR REPLACE INTO settings VALUES (?, ?)", params![name, value.as_str().context("a setting is text")?])?,
        };
        Ok(())
    }

    /// Only the recovery key itself hands over to a new one: no admin's device can, since every epoch is sealed to it,
    /// own scopes too. Unless there's none: then an admin's own device names the next.
    fn recovery(&self) -> Result<()> {
        let now = self.who.recovery()?;
        let named = now.is_none() && self.may("change-settings", NETWORK);
        ensure!(now == Some(self.step.author()) || named, "only the network's recovery key changes it");
        let key = self.key("key")?;
        ensure!(Some(key) != now && !self.who.recovery_was()?.contains(&key) && self.who.machine_ever(&key)?.is_none(), "a new recovery key is a key of its own");
        self.out.execute("UPDATE recovery SET set_at = NULL", [])?;
        self.out.execute("INSERT INTO recovery VALUES ((SELECT count(*) FROM recovery), ?, ?)", params![hex(&key), self.time()])?;
        Ok(())
    }

    /// An app made; or deleted (to Recently Deleted), restored, erased for good, given away, or its code's place said.
    fn app(&self) -> Result<()> {
        let name = self.text("name")?;
        ensure!(valid_app_name(&name), "app names are lowercase words joined by -");
        let to = Principal::parse(&self.text("owner")?)?;
        let acts_for = |p: &Principal| self.may("add-apps", &owner_object(p));
        let Some(app) = self.who.app(&name)? else {
            let holders = match &to {
                Principal::User(u) => user_principal(u),
                Principal::Group(g) => format!("{}#admin", group(g)),
                Principal::Admins => format!("{NETWORK}#admin"),
            };
            allowed!(self.may_make("add-apps", &owner_object(&to), &holders, "owner", &crate::scopes::app(&name)), "only someone acting for an app's owner makes it");
            let from = self.data["from"].as_str();
            self.out.execute("INSERT INTO apps VALUES (?, ?, ?, NULL, 0, NULL, NULL, 0, NULL)", params![name, owner(&to), from])?;
            return Ok(());
        };
        ensure!(!app.erased, "that app was erased");
        ensure!(acts_for(&app.owner) && acts_for(&to), "only someone acting for the app's owner, and its new one, changes it");
        self.out.execute("UPDATE apps SET owner = ? WHERE name = ?", params![owner(&to), name])?;
        if self.data["erased"] == true {
            ensure!(app.deleted.is_some(), "delete it first");
            self.out.execute("UPDATE apps SET erased = 1 WHERE name = ?", [&name])?;
        } else if let Some(deleted) = self.data["deleted"].as_bool() {
            let by = deleted.then(|| (self.time(), user_principal(self.w)));
            self.out.execute("UPDATE apps SET deleted_at = ?, deleted_by = ? WHERE name = ?", params![by.as_ref().map(|b| b.0), by.as_ref().map(|b| &b.1), name])?;
        }
        if let Some(source) = self.data.get("source") {
            self.out.execute("UPDATE apps SET source = ? WHERE name = ?", params![source_link(source)?, name])?;
        }
        Ok(())
    }

    fn deploy(&self) -> Result<()> {
        let (name, bundle) = (self.text("app")?, self.text("bundle")?);
        let app = self.who.app(&name)?.context("no such app")?;
        allowed!(self.may("deploy", &crate::scopes::app(&name)), "only someone acting for the app's owner deploys it");
        ensure!(app.deleted.is_none(), "it was deleted: restore it first");
        ensure!(self.step.content.blobs.contains(&bundle), "a deploy names its bundle among its blobs");
        self.out.execute("UPDATE apps SET bundle = ?, deploys = deploys + 1 WHERE name = ?", params![bundle, name])?;
        self.out.execute(
            "INSERT INTO deploys VALUES (?1, (SELECT count(*) FROM deploys WHERE app = ?1) + 1, ?2, ?3, ?4)",
            params![name, bundle, self.time(), user_principal(self.w)],
        )?;
        Ok(())
    }

    /// A part of a device's oned switched on or off: by the device's user (oned on Cloudflare's are its admins'),
    /// and a service run for a scope's app only by who may add apps there, who may switch it off there too.
    fn service(&self) -> Result<()> {
        let device = self.key("device")?;
        let name = self.text("name")?;
        ensure!(valid_app_name(&name), "service names are lowercase words joined by -");
        let on = self.data["on"] == true;
        let scope = self.data["scope"].as_str().map(Scope::parse).transpose()?;
        let adds = |s: &Scope| self.may("add-apps", &object(s));
        let was = self.who.switch(&device, &name)?.and_then(|s| s.scope.as_deref().map(Scope::parse)).and_then(Result::ok);
        let managed = !on && was.as_ref().is_some_and(adds);
        if self.who.oned_on_cloudflare()? == Some(device) {
            allowed!(managed || self.may("change-settings", NETWORK), "only an admin switches what Cloudflare runs");
        } else {
            self.who.machine(&device)?.context("no such device")?;
            allowed!(managed || self.may("switch", &crate::scopes::device(&device)), "only a device's user switches what it runs");
        }
        ensure!(!on || scope.as_ref().is_none_or(adds), "only who may add apps there runs its service for it");
        let scope = scope.or(was.filter(|_| !on)).map(|s| s.to_string());
        self.out.execute("INSERT OR REPLACE INTO services VALUES (?, ?, ?, ?, ?, ?)", params![hex(&device), name, on, user_principal(self.w), self.time(), scope])?;
        Ok(())
    }
}

/// A step of a scope's own state, in `out`: apps added, schedules, Vault requests, pages and the blobs it no longer needs.
fn own(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Step) -> Result<()> {
    let w = step.user();
    let data = &step.content.data;
    let time = int(step.time());
    let manages = || who.may(w, "add-apps", &object(scope));
    match step.content.change() {
        Some(change @ (Change::AddApp | Change::RemoveApp)) => {
            ensure!(manages(), ::custom_build::protocol::Coded::refused("not allowed to add apps there"));
            let app = data["app"].as_str().context("no app")?;
            if change == Change::AddApp {
                who.app(app)?.context("no such app")?;
                out.execute("INSERT OR REPLACE INTO added VALUES (?, 1, ?, ?)", params![app, user_principal(w), time])?;
            } else {
                out.execute("UPDATE added SET added = 0 WHERE app = ?", [app])?;
            }
        }
        Some(Change::Schedule) => {
            ensure!(manages(), ::custom_build::protocol::Coded::refused("not allowed to switch schedules there"));
            let app = data["app"].as_str().context("no app")?;
            let name = data["schedule"].as_str().context("no schedule")?;
            let on = data["on"].as_bool().context("on or off?")?;
            out.execute("INSERT OR REPLACE INTO schedules VALUES (?, ?, ?, ?, ?)", params![app, name, on, user_principal(w), time])?;
        }
        // A blob the scope keeps, which its `blobs` names.
        Some(Change::Keep) => {}
        // Only someone who may write there says a blob is no longer needed: its data goes from the bucket.
        Some(Change::Forget) => {
            let blob = data["blob"].as_str().filter(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())).context("no blob")?;
            out.execute("INSERT OR IGNORE INTO forgotten VALUES (?)", [blob])?;
        }
        Some(Change::VaultRequest) => {
            ensure!(manages(), ::custom_build::protocol::Coded::refused("not allowed to change the Vault there"));
            let id = data["id"].as_str().filter(|id| is_id(id)).context("no id")?;
            let answered: Option<Option<String>> = out.query_row("SELECT answer FROM vault_requests WHERE id = ?", [id], |r| r.get(0)).optional()?;
            // Answered: declined, or set once the item it asked for is in the scope's Vault (`app-runtime/src/vault.rs`).
            if let Some(how) = ["declined", "set"].into_iter().find(|how| data[*how] == true) {
                ensure!(answered.context("no such request")?.is_none(), "already answered");
                out.execute("UPDATE vault_requests SET answer = ?, answered_by = ?, answered_at = ? WHERE id = ?", params![how, user_principal(w), time, id])?;
                return Ok(());
            }
            ensure!(answered.is_none(), "that request is already made");
            let app = data["app"].as_str().context("no app")?;
            // An item the app runtime itself uses (a backup place's) is asked for as the app runtime's, and one agents use
            // (vault_fetch) as theirs.
            ensure!(who.app(app)?.is_some() || app == APP || app == AGENTS, "no such app");
            let fields: Vec<(String, bool)> = data["fields"].as_object().context("no fields: give the item's fields and hosts, or need")?.iter().map(|(n, f)| (n.clone(), f["secret"] == true)).collect();
            let title = data["title"].as_str().unwrap_or("").trim();
            ensure!(!title.is_empty(), "a request has a title");
            ensure!(!fields.is_empty(), "a request asks for one field or more");
            if let Some((name, _)) = fields.iter().find(|(n, _)| !valid_secret_name(n)) {
                bail!("“{name}” can't be a field's name: use letters, digits, _, - and . with no spaces, like {}", name.trim().replace(|c: char| !c.is_ascii_alphanumeric() && !"_-.".contains(c), "_"));
            }
            let hosts: Vec<&str> = data["hosts"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            ensure!(hosts.iter().all(|h| valid_host(h)), "a host is a host name, like api.example.com");
            ensure!(!hosts.is_empty() || !fields.iter().any(|(_, secret)| *secret), "a secret field goes to one host or more");
            let fields = json!(fields.iter().map(|(name, secret)| json!({ "name": name, "secret": secret })).collect::<Vec<_>>());
            out.execute(
                "INSERT INTO vault_requests VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL)",
                params![id, app, title, fields.to_string(), json!(hosts).to_string(), data["why"].as_str().unwrap_or(""), data["need"].as_str(), user_principal(w), data["agent"].as_str(), time],
            )?;
        }
        Some(Change::Arrange) => {
            ensure!(*scope == Scope::User(w.to_string()), "each user arranges pages in their own scope");
            let page = data["page"].as_str().context("no page")?;
            ensure!(valid_page(page), "a page is user, group:<id> or global");
            let items = data["items"].as_array().context("no items")?;
            ensure!(items.len() <= 500, "too many items on one page");
            for item in items {
                ensure!(item["app"].as_str().is_some_and(valid_app_name), "each item names an app");
                if !item["widget"].is_null() {
                    ensure!(item["widget"].is_string() && item["id"].as_str().is_some_and(|id| !id.is_empty() && id.len() <= 64), "a widget on a page has an id");
                    ensure!(item["options"].is_null() || item["options"].is_object(), "a widget's options are an object");
                }
            }
            let hidden = data["hidden"].as_array().cloned().unwrap_or_default();
            ensure!(hidden.iter().all(|a| a.as_str().is_some_and(valid_app_name)), "hidden lists apps");
            let mut kept = json!({ "items": items, "hidden": hidden, "time": step.time() });
            // With your own page: the order of your groups' pages, and those you hid (Edit Pages…).
            if page == "user" && !data["pages"].is_null() {
                let groups = |k: &str| data["pages"][k].as_array().is_some_and(|l| l.len() <= 500 && l.iter().all(|g| g.as_str().is_some_and(|g| g.starts_with("group:") && valid_page(g))));
                ensure!(groups("order") && groups("hidden"), "pages orders and hides groups' pages");
                kept["pages"] = json!({ "order": data["pages"]["order"], "hidden": data["pages"]["hidden"] });
            }
            out.execute("INSERT OR REPLACE INTO pages VALUES (?, ?)", params![page, kept.to_string()])?;
        }
        _ => bail!("{} isn't a step of a user's or group's own", step.content.kind),
    }
    Ok(())
}

fn add_user(out: &Connection, user: &str, name: &str, r: Role) -> Result<()> {
    out.execute("INSERT INTO principals VALUES (?, 'user', ?, ?, 0)", params![user_principal(user), name, pick_handle(out, name, "user")?])?;
    set_role(out, &user_principal(user), NETWORK, Some(r))
}

/// `subject`'s role on `object` (the network or a group), in place of any they held; none takes it.
fn set_role(out: &Connection, subject: &str, object: &str, to: Option<Role>) -> Result<()> {
    out.execute("DELETE FROM relations WHERE subject = ? AND object = ? AND relation IN ('admin', 'member')", params![subject, object])?;
    if let Some(r) = to {
        out.execute("INSERT INTO relations VALUES (?, ?, ?, NULL)", params![subject, role(r), object])?;
    }
    Ok(())
}

/// A user's or group's handle, picked from the name they joined or were made with: `handle_from`, with -2, -3… after
/// it while that's taken (by anyone, removed users too).
fn pick_handle(out: &Connection, name: &str, fallback: &str) -> Result<String> {
    let base = handle_from(name, fallback);
    let numbered = |n: u32| {
        let tail = format!("-{n}");
        format!("{}{tail}", base[..base.len().min(39 - tail.len())].trim_end_matches('-'))
    };
    for h in (1..).map(|n| if n == 1 { base.clone() } else { numbered(n) }) {
        if h != EVERYONE && out.query_row("SELECT 1 FROM principals WHERE handle = ?", [&h], |_| Ok(())).optional()?.is_none() {
            return Ok(h);
        }
    }
    unreachable!()
}

fn put(out: &Connection, d: &Machine) -> Result<()> {
    out.execute(
        "INSERT OR REPLACE INTO devices VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            hex(&d.key),
            hex(&d.seal),
            user_principal(&d.user),
            d.name,
            d.is(Kind::Client),
            d.is(Kind::Compute),
            d.service,
            d.shared,
            d.passkey,
            d.device.as_ref().map(hex),
            user_principal(&d.by),
            int(d.since),
            d.sign_in.as_ref().map(|a| json!(a).to_string()),
            d.place,
            d.removed,
            d.platform
        ],
    )?;
    Ok(())
}

/// An account as `linked_sign_ins` keeps it: this release's shape, or it doesn't count.
fn account_text(account: &Value) -> Result<String> {
    let account: crate::sign_in_proof::Account = serde_json::from_value(account.clone()).context("that isn't a sign-in")?;
    Ok(json!(account).to_string())
}

fn epoch_hash_of(hash: &Value) -> Result<String> {
    let hash = hash.as_str().context("an epoch names its key's hash")?;
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "an epoch's hash is 64 hex characters");
    Ok(hash.to_string())
}

fn listed(data: &Value, user: &str, since: u64) -> Result<Machine> {
    let text = |key: &str| data[key].as_str().with_context(|| format!("no {key}"));
    let roles: BTreeSet<Kind> = serde_json::from_value(data["roles"].clone()).context("roles are client, compute or service")?;
    ensure!(!roles.is_empty(), "an endpoint does something: client, compute or service");
    let name = text("name")?.trim().to_string();
    ensure!(!name.is_empty(), "an endpoint has a name");
    let service = data["service"].as_str().map(str::to_string);
    ensure!(service.as_deref().is_none_or(valid_app_name), "a service's name is lowercase words joined by -");
    ensure!(service.is_some() == roles.contains(&Kind::Service), "a service names the service it offers, and only it does");
    let platform = data["platform"].as_str().map(str::to_string);
    ensure!(platform.as_deref().is_none_or(|p| PLATFORMS.contains(&p)), "a client's platform is one of {PLATFORMS:?}");
    Ok(Machine {
        key: unhex(text("key")?)?,
        seal: unhex(text("seal")?)?,
        user: user.to_string(),
        name,
        roles,
        passkey: data["passkey"].as_str().map(str::to_string),
        platform,
        shared: service.is_some() && data["shared"] == true,
        service,
        device: data["device"].as_str().map(unhex).transpose().context("device is the key of the device it's part of")?,
        by: user.to_string(),
        since,
        sign_in: serde_json::from_value(data["sign_in"].clone()).ok(),
        place: data["place"].as_str().map(|p| p.chars().take(64).collect()),
        removed: false,
    })
}

/// What a client can say it's built for.
pub const PLATFORMS: [&str; 7] = ["mac", "windows", "linux", "iphone", "android", "browser", "cloudflare"];

/// Whether `name` may be one of the network's settings: lowercase words joined by _, or a lab's, whose parts are an
/// app's or service's name and a lab's.
pub fn valid_setting(name: &str) -> bool {
    let word = |w: &str| !w.is_empty() && w.len() <= 40 && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    match name.strip_prefix("lab.") {
        Some(rest) => {
            let parts: Vec<&str> = rest.split('.').collect();
            match parts.as_slice() {
                [_] => parts.iter().all(|p| word(p)),
                ["app" | "service", _, _] => parts.iter().all(|p| word(p)),
                _ => false,
            }
        }
        None => !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use ed25519_dalek::SigningKey;
    use serde_json::{Value, json};

    use crate::scopes::Scope;
    use ::key::{Endpoint, Hash, Key, hex};
    use crate::content::*;
    use crate::scopes::{APPS_PART, Kind, Role};

    use crate::scopes::ScopeDb;
    use crate::steps;

    /// A step of the core's of any kind, one this release doesn't know too, as an older or newer release may write it.
    fn core_step(kind: &str, data: Value) -> Content {
        Content::new(crate::rules::APP, kind, data)
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const BOTH: &[Kind] = &[Kind::Client, Kind::Compute];
    const H: &str = "e";

    struct Dev(SigningKey);

    impl Dev {
        fn new(seed: u8) -> Self {
            Dev(SigningKey::from_bytes(&[seed; 32]))
        }
        fn id(&self) -> Key {
            self.0.verifying_key().to_bytes()
        }
        fn endpoint(&self) -> Endpoint {
            Endpoint::of(&self.0)
        }
    }

    /// `global`'s scope database, each step applied as its readers apply it: those that don't count are passed over.
    struct Net {
        db: ScopeDb,
        at: u64,
    }

    impl Net {
        /// A network whose first device is `mac`, listing itself as Andre's `kinds`, its recovery key `recovery`.
        fn new(mac: &Dev, kinds: &[Kind], recovery: &Key) -> Net {
            let start = first("Home", ANDRE, "Andre", &mac.endpoint(), "mac", kinds, recovery, &H.repeat(64));
            Net { db: steps::first(Hash::from_bytes([5; 32]), &start, 0).unwrap(), at: 1 }
        }

        /// Writes `content` as `user` from `dev`: whether it counted.
        fn step(&mut self, dev: &Dev, user: &str, content: Content) -> bool {
            self.at += 1;
            let s = steps::sign(&dev.0, &Scope::Global, self.at, user, 1, self.at, content).unwrap();
            match steps::apply(&self.db, None, &Scope::Global, &s) {
                Ok(a) => {
                    self.db = ScopeDb::open(a.db, 0).unwrap();
                    true
                }
                Err(_) => false,
            }
        }

        fn owed(&self) -> Vec<Scope> {
            let mut owed = self.db.owed().unwrap();
            owed.sort();
            owed
        }
    }

    fn setup() -> (Dev, Net) {
        let mac = Dev::new(1);
        let net = Net::new(&mac, BOTH, &[7; 32]);
        (mac, net)
    }

    /// A device's app runtime: only that device says it has one, and it then runs what the device runs, grants and
    /// others' choices included, until the device is taken out.
    #[test]
    fn a_devices_runtime_is_its_own_to_list_and_runs_what_it_runs() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let (linux, rt, bobs) = (Dev::new(4), Dev::new(5), Dev::new(3));
        let (andre, bob) = (Scope::User(ANDRE.into()), Scope::User(BOB.into()));
        let runtime = |d: &Dev, of: &Dev, user: &str| {
            let mut c = device(&d.endpoint(), user, "runtime", &[Kind::Compute], None);
            c.data["device"] = json!(hex(&of.id()));
            c
        };
        assert!(net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member)))));
        let mut listing = device(&linux.endpoint(), ANDRE, "linux", BOTH, None);
        listing.data["platform"] = json!("linux");
        assert!(net.step(&mac, ANDRE, listing));
        // Bob chose linux to run his things, and it opens Andre's Vault.
        assert!(net.step(&bobs, BOB, runs(&[linux.id()])));
        assert!(net.step(&mac, ANDRE, vault(&andre, 1, &[9; 32], &[linux.id()])));
        // A device with no runtime of its own keeps its Compute; its user's other device doesn't list one for it, nor
        // anyone else's.
        assert!(!net.step(&linux, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Client], None)), "no runtime yet");
        assert!(!net.step(&mac, ANDRE, runtime(&rt, &linux, ANDRE)), "another of its user's devices");
        assert!(!net.step(&bobs, BOB, runtime(&rt, &linux, BOB)), "someone else's");
        assert!(net.db.machine(&rt.id())?.is_none());

        assert!(net.step(&linux, ANDRE, runtime(&rt, &linux, ANDRE)));
        assert_eq!(net.db.device_of(&rt.id()), linux.id());
        assert!(net.db.runs_for(&rt.id(), &andre) && net.db.runs_for(&rt.id(), &Scope::Global));
        assert!(net.db.runs_for(&rt.id(), &bob) && net.db.serves(&rt.id(), BOB)?, "what Bob chose for the device");
        assert!(net.db.opens_vault(&rt.id(), &andre)?, "a Vault granted to the device");
        assert!(net.db.runners(BOB, std::slice::from_ref(&bob))?.iter().any(|m| m.key == rt.id()));
        assert!(!net.step(&linux, ANDRE, runtime(&Dev::new(6), &linux, ANDRE)), "a device has one");
        // The device drops its own Compute, and nothing else moves.
        assert!(!net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Client], None)), "only itself drops it");
        assert!(net.step(&linux, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Client], None)));
        let was = net.db.machine(&linux.id())?.unwrap();
        assert_eq!(was.roles, BTreeSet::from([Kind::Client]));
        assert_eq!(was.by, ANDRE);
        assert!(net.db.compute(&linux.id())?.is_none());
        assert!(net.db.runs_for(&rt.id(), &bob) && net.db.runs_for(&linux.id(), &bob) && net.db.opens_vault(&rt.id(), &andre)?);
        assert!(net.step(&bobs, BOB, runs(&[linux.id()])), "the device stays what they name");
        assert!(net.step(&mac, ANDRE, vault(&andre, 2, &[9; 32], &[linux.id()])), "and what's granted");
        assert_eq!(net.db.find_node("linux")?, linux.id());
        assert_eq!(net.db.find_runner("linux")?, rt.id());
        // A device with a runtime of its own isn't just a client: the recovery key isn't sealed to it.
        assert!(net.db.recovery_devices(ANDRE)?.iter().all(|m| m.key != linux.id()));
        // Choosing the runtime itself is choosing the device.
        assert!(net.step(&bobs, BOB, runs(&[rt.id()])));
        assert!(net.db.runs_for(&rt.id(), &bob));
        assert_eq!(net.db.runs(BOB)?, Some(BTreeSet::from([linux.id()])));
        // Its device taken out, it's gone with it, and runs nothing.
        assert!(net.step(&mac, ANDRE, remove_device(&linux.id())));
        assert!(net.db.machine(&rt.id())?.is_none() && !net.db.runs_for(&rt.id(), &andre) && !net.db.opens_vault(&rt.id(), &andre)?);
        assert!(net.db.computes()?.iter().all(|m| m.key != rt.id()));
        Ok(())
    }

    /// Only an admin names who holds the recovery key; a value that doesn't read, or someone since taken out, names no
    /// one; and it's sealed only to a holder's own client devices that say what they are, never an app runtime, a browser, a
    /// remote MCP client, a passkey or a service.
    #[test]
    fn only_an_admin_names_a_recovery_holder_and_only_some_devices_may_hold_it() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let holders = |list: &str| setting(crate::scopes::RECOVERY_HOLDERS, Some(list));
        let with = |d: &Dev, user: &str, name: &str, platform: Option<&str>, kinds: &[Kind], new: Option<(&str, Role)>| {
            let mut c = device(&d.endpoint(), user, name, kinds, new);
            if let Some(p) = platform {
                c.data["platform"] = json!(p);
            }
            c
        };
        let (phone, browser, mcp, laptop, unsaid, node, bobs, bobs_phone) = (Dev::new(3), Dev::new(4), Dev::new(5), Dev::new(6), Dev::new(8), Dev::new(9), Dev::new(10), Dev::new(11));
        for (d, name, platform, kinds) in [(&phone, "iphone", Some("iphone"), &[Kind::Client][..]), (&browser, "chrome", Some("browser"), &[Kind::Client]), (&mcp, "claude", Some("cloudflare"), &[Kind::Client]), (&laptop, "cli", Some("linux"), &[Kind::Client]), (&unsaid, "old", None, &[Kind::Client]), (&node, "box", Some("linux"), BOTH)] {
            assert!(net.step(&mac, ANDRE, with(d, ANDRE, name, platform, kinds, None)));
        }
        // A passkey listed before they stopped being listed, still read from the steps that hold it.
        let mut passkey = device(&Dev::new(12).endpoint(), ANDRE, "Passkey", &[Kind::Client], None);
        passkey.data["passkey"] = json!("cred");
        passkey.data["platform"] = json!("iphone");
        assert!(net.step(&mac, ANDRE, passkey));
        assert!(net.step(&mac, ANDRE, with(&bobs, BOB, "bob", Some("android"), &[Kind::Client], Some(("Bob", Role::Member)))));
        assert!(net.step(&bobs, BOB, with(&bobs_phone, BOB, "bob's", Some("iphone"), &[Kind::Client], None)));

        let named = json!([ANDRE, BOB]).to_string();
        assert!(!net.step(&bobs, BOB, holders(&named)), "only an admin names one");
        assert!(net.db.recovery_holders()?.is_empty());
        assert!(net.step(&mac, ANDRE, holders(&named)));
        assert_eq!(net.db.recovery_holders()?, [ANDRE.to_string(), BOB.to_string()].into());
        let keys = |user: &str, net: &Net| net.db.recovery_devices(user).unwrap().iter().map(|d| d.key).collect::<BTreeSet<_>>();
        assert_eq!(keys(ANDRE, &net), [phone.id(), laptop.id()].into(), "not the computer, browser, remote MCP, passkey or one that hasn't said what it is");
        assert_eq!(keys(BOB, &net), [bobs.id(), bobs_phone.id()].into());

        assert!(net.step(&mac, ANDRE, remove_device(&phone.id())));
        assert_eq!(keys(ANDRE, &net), [laptop.id()].into(), "a device taken out holds nothing more");
        assert!(net.step(&mac, ANDRE, holders(&json!([ANDRE]).to_string())));
        assert_eq!(net.db.recovery_holders()?, [ANDRE.to_string()].into(), "taken off the list");
        assert!(net.step(&mac, ANDRE, holders(&json!([ANDRE, BOB]).to_string())));
        assert!(net.step(&mac, ANDRE, remove_user(BOB)));
        assert_eq!(net.db.recovery_holders()?, [ANDRE.to_string()].into(), "someone taken out of the network holds nothing");
        assert!(net.step(&mac, ANDRE, holders("not json")));
        assert!(net.db.recovery_holders()?.is_empty(), "a value that doesn't read names no one");
        Ok(())
    }

    /// What a device signed in with is recorded with its listing, for its user's next device to be checked against;
    /// a device taken out takes its record with it, and a shape this release doesn't know records nothing.
    #[test]
    fn a_devices_sign_in_is_its_users_while_it_is_listed() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let google = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
        let listed = |d: &Dev, sign_in: Value| {
            let mut c = device(&d.endpoint(), ANDRE, "phone", &[Kind::Client], None);
            c.data["sign_in"] = sign_in;
            c
        };
        let (phone, laptop) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, listed(&phone, google.clone()));
        net.step(&mac, ANDRE, listed(&laptop, json!({ "password": "hunter2" })));
        assert_eq!(net.db.machine(&phone.id())?.unwrap().sign_in, serde_json::from_value(google.clone()).ok());
        assert_eq!(net.db.machine(&laptop.id())?.unwrap().sign_in, None);
        assert_eq!(net.db.sign_ins(ANDRE)?, vec![serde_json::from_value(google).unwrap()]);
        net.step(&mac, ANDRE, remove_device(&phone.id()));
        assert!(net.db.machine(&phone.id())?.is_none() && net.db.sign_ins(ANDRE)?.is_empty(), "taken out, with what it signed in with");
        Ok(())
    }

    /// What a device signed in with is forgotten only by its user's own device; and a device they said isn't theirs
    /// ("Not Me") is never listed after, by them or anyone, and nothing it writes counts.
    #[test]
    fn a_linked_sign_in_counts_from_its_users_own_device_until_forgotten() -> anyhow::Result<()> {
        use crate::sign_in_proof::Account;
        let (mac, mut net) = setup();
        let (phone, bobs) = (Dev::new(3), Dev::new(5));
        net.step(&mac, ANDRE, device(&phone.endpoint(), ANDRE, "phone", &[Kind::Client], None));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        let google = Account::Provider { issuer: "https://accounts.google.com".into(), audience: "ours".into(), subject: "1".into(), email: None };
        assert!(!net.step(&bobs, BOB, link_sign_in(&phone.id(), &google)), "only its own user links from a device");
        assert!(net.step(&phone, ANDRE, link_sign_in(&phone.id(), &google)));
        assert_eq!(net.db.sign_ins(ANDRE)?, vec![google.clone()]);
        assert!(net.db.sign_ins(BOB)?.is_empty());
        assert!(!net.step(&bobs, BOB, forget_sign_in(&phone.id(), Some(&google))), "only its user forgets it");
        assert!(net.step(&phone, ANDRE, forget_sign_in(&phone.id(), Some(&google))));
        assert!(net.db.sign_ins(ANDRE)?.is_empty());
        Ok(())
    }

    #[test]
    fn a_sign_in_is_forgotten_and_a_device_turned_down_stays_out() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let google = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
        let (phone, bobs) = (Dev::new(3), Dev::new(5));
        let mut c = device(&phone.endpoint(), ANDRE, "phone", &[Kind::Client], None);
        c.data["sign_in"] = google;
        net.step(&mac, ANDRE, c);
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        assert!(!net.step(&bobs, BOB, forget_sign_in(&phone.id(), None)), "only its user forgets it");
        assert!(net.step(&phone, ANDRE, forget_sign_in(&phone.id(), None)));
        assert_eq!(net.db.machine(&phone.id())?.unwrap().sign_in, None);

        let stranger = Dev::new(9);
        assert!(net.step(&mac, ANDRE, refuse(&stranger.endpoint(), "MacBook Pro")));
        assert!(!net.step(&phone, ANDRE, device(&stranger.endpoint(), ANDRE, "MacBook Pro", &[Kind::Client], None)), "turned down, it's never listed");
        assert!(!net.step(&stranger, ANDRE, setting("sneaky", Some("1"))), "and what it writes doesn't count");
        assert!(net.db.machine(&stranger.id())?.is_none() && net.db.machine_ever(&stranger.id())?.unwrap().removed);
        // One listed already is taken out instead, as Remove does.
        assert!(!net.step(&mac, ANDRE, refuse(&phone.endpoint(), "phone")));
        Ok(())
    }

    /// A Not Me counts only against who said it: anyone in the network can write one, for any key they read at auth, and
    /// it must never stop another user's device.
    #[test]
    fn someone_elses_not_me_doesnt_stop_your_device() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let (bobs, waiting) = (Dev::new(5), Dev::new(9));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        assert!(net.step(&bobs, BOB, refuse(&waiting.endpoint(), "MacBook Pro")), "anyone may say it");
        assert!(net.db.key_for(&waiting.id(), BOB)?.is_some_and(|d| d.removed), "it stops it for them");
        assert!(net.db.key_for(&waiting.id(), ANDRE)?.is_none(), "and not for anyone else");
        assert!(net.step(&mac, ANDRE, device(&waiting.endpoint(), ANDRE, "MacBook Pro", &[Kind::Client], None)), "so its user's own device lets it in");
        assert!(!net.step(&bobs, BOB, refuse(&waiting.endpoint(), "MacBook Pro")), "one listed now isn't refused, by anyone");
        Ok(())
    }

    /// A sign-in forgotten stops counting everywhere it was recorded, on clients taken out too; and one an admin's device
    /// recorded as it let someone in counts for them, or an admin could never let them back in.
    #[test]
    fn a_forgotten_sign_in_counts_nowhere_and_an_admin_recorded_one_counts() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let account = |s: &str| json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": s } });
        let (phone, laptop, bobs) = (Dev::new(3), Dev::new(4), Dev::new(5));
        let mut listed = device(&laptop.endpoint(), ANDRE, "laptop", &[Kind::Client], None);
        listed.data["sign_in"] = account("1");
        net.step(&mac, ANDRE, listed);
        net.step(&mac, ANDRE, device(&phone.endpoint(), ANDRE, "phone", &[Kind::Client], None));
        net.step(&mac, ANDRE, remove_device(&laptop.id()));
        assert_eq!(net.db.sign_ins_ever(ANDRE)?.len(), 1, "a client taken out still says what they sign in with");
        assert!(net.step(&phone, ANDRE, forget_sign_in(&laptop.id(), None)), "forgotten on a client taken out");
        assert!(net.db.sign_ins_ever(ANDRE)?.is_empty(), "so it counts for nothing");

        let mut bob = device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member)));
        bob.data["sign_in"] = account("2");
        net.step(&mac, ANDRE, bob);
        assert_eq!(net.db.sign_ins_ever(BOB)?.len(), 1, "their only device was listed by the admin who let them in");
        net.step(&mac, ANDRE, remove_device(&bobs.id()));
        assert_eq!(net.db.sign_ins_ever(BOB)?.len(), 1, "so an admin can check them once it's gone");
        Ok(())
    }

    /// An invite by email counts only while whoever made it may still invite; an admin can withdraw it; and it lasts only
    /// as long as an invite does.
    #[test]
    fn an_email_invite_ends_with_its_admin_or_when_withdrawn() -> anyhow::Result<()> {
        const SAM: &str = "c000000000000003";
        let (mac, mut net) = setup();
        let (second, bobs) = (Dev::new(3), Dev::new(5));
        net.step(&mac, ANDRE, device(&second.endpoint(), BOB, "bob's mac", &[Kind::Client], Some(("Bob", Role::Admin))));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), "d000000000000004", "dee's phone", &[Kind::Client], Some(("Dee", Role::Member))));
        let invite = |until| invite_email(SAM, "sam@example.com", "Sam", Role::Member, until, &[]);
        assert!(!net.step(&second, BOB, invite(9 * 86_400_000)), "it can't outlast what an invite lasts");
        assert!(net.step(&second, BOB, invite(2_000_000)));
        assert!(net.db.email_invite(SAM)?.is_some());
        assert!(!net.step(&bobs, "d000000000000004", withdraw_invite(SAM)), "only an admin withdraws one");
        net.step(&mac, ANDRE, role(BOB, Role::Member));
        assert!(net.db.email_invite(SAM)?.is_none(), "whoever made it isn't an admin now");
        net.step(&mac, ANDRE, role(BOB, Role::Admin));
        assert!(net.db.email_invite(SAM)?.is_some());
        assert!(net.step(&mac, ANDRE, withdraw_invite(SAM)), "any admin withdraws it");
        assert!(net.db.email_invite(SAM)?.is_none());
        assert!(!net.step(&mac, ANDRE, withdraw_invite(SAM)), "there's none to withdraw now");
        assert!(net.step(&second, BOB, invite(3_000_000)), "and it can be made again");
        net.step(&mac, ANDRE, remove_user(BOB));
        assert!(net.db.email_invite(SAM)?.is_none(), "nor once they're out of the network");
        Ok(())
    }

    /// A new device's sign-in is checked only against what a user's own devices recorded of their own clients: not a
    /// machine an admin listed for them, nor their first device, which the inviting admin let in.
    #[test]
    fn only_a_users_own_clients_say_what_they_sign_in_with() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let account = |s: &str| json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": s } });
        let with = |mut c: Content, s: &str| {
            c.data["sign_in"] = account(s);
            c
        };
        let (bobs, box_, laptop) = (Dev::new(5), Dev::new(6), Dev::new(7));
        net.step(&mac, ANDRE, with(device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))), "by-admin"));
        assert!(net.step(&mac, ANDRE, with(device(&box_.endpoint(), BOB, "box", &[Kind::Compute], None), "admins-machine")), "the admin lists it");
        net.step(&bobs, BOB, with(device(&laptop.endpoint(), BOB, "laptop", &[Kind::Client], None), "bobs-own"));
        assert_eq!(net.db.sign_ins(BOB)?, vec![serde_json::from_value(account("bobs-own")).unwrap()]);
        assert_eq!(net.db.seal_key(&mac.id())?, Some(mac.endpoint().seal), "the seal key it was listed with");
        Ok(())
    }

    mod invites {
        use crate::rules::tests::*;
        use crate::scopes::Role;
        use crate::sign_in_proof::{CLIENTS, nonce};
        use data_encoding::BASE64URL_NOPAD as B64U;
        use ed25519_dalek::Signer;

        const SAM: &str = "c000000000000003";
        const ISSUER: &str = "https://login.example";
        const EMAIL: &str = "sam@example.com";

        /// The provider's key, as an admin's device saves it with a proof.
        fn provider() -> (SigningKey, Value) {
            let key = SigningKey::from_bytes(&[8; 32]);
            let jwk = json!({ "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "kid": "k", "x": B64U.encode(key.verifying_key().as_bytes()) });
            (key, jwk)
        }

        fn token(key: &SigningKey, device: &Dev, root: [u8; 32], email: &str, iat: u64) -> String {
            let head = B64U.encode(json!({ "alg": "EdDSA", "kid": "k" }).to_string().as_bytes());
            let body = B64U.encode(json!({ "iss": ISSUER, "aud": "ours", "sub": "s1", "iat": iat, "email": email, "email_verified": true, "nonce": nonce(&root, &device.endpoint()) }).to_string().as_bytes());
            let sig = key.sign(format!("{head}.{body}").as_bytes());
            format!("{head}.{body}.{}", B64U.encode(&sig.to_bytes()))
        }

        fn joins(device: &Dev, user: &str, token: &str, jwk: &Value) -> Content {
            joins_as(device, user, token, jwk, Role::Member)
        }

        fn joins_as(device: &Dev, user: &str, token: &str, jwk: &Value, role: Role) -> Content {
            let mut c = crate::content::device(&device.endpoint(), user, "Sam's phone", &[Kind::Client], Some(("Sam", role)));
            c.data["invite"] = json!({ "token": token, "jwk": jwk });
            c
        }

        /// A network whose users sign in at `ISSUER`, where an admin has named Sam by email, with the provider's key then.
        fn invited() -> (Dev, Net, [u8; 32]) {
            let (mac, mut net) = setup();
            assert!(net.step(&mac, ANDRE, setting(CLIENTS, Some(&format!("{ISSUER} ours")))));
            assert!(net.step(&mac, ANDRE, invite_email(SAM, EMAIL, "Sam", Role::Member, 2_000_000, &[provider().1])));
            let root = *net.db.root().unwrap().unwrap().as_bytes();
            (mac, net, root)
        }

        #[test]
        fn only_an_admin_names_someone_by_email() {
            let (mac, mut net, _) = invited();
            let bobs = Dev::new(3);
            net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
            assert!(!net.step(&bobs, BOB, invite_email("d000000000000004", "x@example.com", "X", Role::Member, 2_000_000, &[])), "a member names no one");
            assert!(!net.step(&mac, ANDRE, invite_email(BOB, "bob@example.com", "Bob", Role::Member, 2_000_000, &[])), "someone in already");
        }

        /// An invite lets in the device its ID token names, once, for its email, as the role it says, written by any
        /// device of the network when a key the invite named signed it, and by an admin's for any other; every other try
        /// counts for nothing.
        #[test]
        fn an_invite_lets_in_only_its_own_user_once() -> anyhow::Result<()> {
            let (mac, mut net, root) = invited();
            let (key, jwk) = provider();
            let (phone, other, bobs) = (Dev::new(3), Dev::new(4), Dev::new(5));
            let good = token(&key, &phone, root, EMAIL, 1000);

            assert!(!net.step(&mac, ANDRE, joins(&phone, SAM, &token(&key, &phone, root, "someone-else@example.com", 1000), &jwk)), "the wrong person's email");
            assert!(!net.step(&mac, ANDRE, joins(&other, SAM, &good, &jwk)), "a token naming another device");
            assert!(!net.step(&mac, ANDRE, joins(&phone, SAM, &token(&key, &phone, [9; 32], EMAIL, 1000), &jwk)), "a token for another network");
            assert!(!net.step(&mac, ANDRE, joins(&phone, SAM, &token(&key, &phone, root, EMAIL, 3000), &jwk)), "made after the invite ended");
            let (forger, _) = (SigningKey::from_bytes(&[9; 32]), ());
            assert!(!net.step(&mac, ANDRE, joins(&phone, SAM, &token(&forger, &phone, root, EMAIL, 1000), &jwk)), "signed by a key that isn't the one saved");
            assert!(!net.step(&mac, ANDRE, joins(&phone, "d000000000000004", &good, &jwk)), "no invite for that user");
            assert!(net.db.user(SAM)?.is_none());

            net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
            let made_up = SigningKey::from_bytes(&[10; 32]);
            let made_up_jwk = json!({ "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "kid": "k", "x": B64U.encode(made_up.verifying_key().as_bytes()) });
            assert!(!net.step(&bobs, BOB, joins(&phone, SAM, &token(&made_up, &phone, root, EMAIL, 1000), &made_up_jwk)), "a member's device, by a key it made up");
            assert!(!net.step(&bobs, BOB, joins_as(&phone, SAM, &good, &jwk, Role::Admin)), "as a role the invite didn't say");
            let mut more = joins(&phone, SAM, &good, &jwk);
            more.data["roles"] = json!([Kind::Client, Kind::Compute]);
            assert!(!net.step(&mac, ANDRE, more), "only a device people use");

            assert!(net.step(&bobs, BOB, joins(&phone, SAM, &good, &jwk)), "any device of the network carries the invite in");
            let sam = net.db.user(SAM)?.unwrap();
            assert_eq!((sam.name.as_str(), sam.role), ("Sam", Role::Member));
            assert_eq!(net.db.sign_ins(SAM)?.len(), 1, "what it signed in with is theirs");
            assert!(!net.step(&mac, ANDRE, joins(&Dev::new(6), SAM, &token(&key, &Dev::new(6), root, EMAIL, 1000), &jwk)), "used twice");
            assert!(net.db.machine(&Dev::new(6).id())?.is_none());
            Ok(())
        }

        /// An admin lets in someone's next device only when they have no device of their own left.
        #[test]
        fn an_admin_lets_in_someone_with_no_device_left() -> anyhow::Result<()> {
            let (mac, mut net) = setup();
            let (bobs, next, later, carol) = (Dev::new(3), Dev::new(4), Dev::new(5), Dev::new(6));
            net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
            net.step(&mac, ANDRE, device(&carol.endpoint(), "e000000000000005", "carol's phone", &[Kind::Client], Some(("Carol", Role::Member))));
            assert!(!net.step(&mac, ANDRE, device(&next.endpoint(), BOB, "new", &[Kind::Client], None)), "not while they have one");
            net.step(&mac, ANDRE, remove_device(&bobs.id()));
            assert!(!net.step(&carol, "e000000000000005", device(&next.endpoint(), BOB, "new", &[Kind::Client], None)), "only an admin");
            assert!(!net.step(&mac, ANDRE, device(&next.endpoint(), BOB, "new", BOTH, None)), "only a device people use");
            assert!(net.step(&mac, ANDRE, device(&next.endpoint(), BOB, "new", &[Kind::Client], None)));
            assert!(!net.step(&mac, ANDRE, device(&later.endpoint(), BOB, "later", &[Kind::Client], None)), "and they have one again");
            Ok(())
        }
    }

    #[test]
    fn a_service_runs_for_a_scope_its_adder_may_add_to_and_its_scopes_admins_may_stop_it() {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's computer", BOTH, Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
        let (linux_id, bobs_id) = (linux.id(), bobs.id());
        let whatsapp = |db: &ScopeDb, k: &Key| db.switch(k, "whatsapp").unwrap().map(|s| (s.on, s.scope));
        // Bob runs it on his own app runtime for himself, not for everyone; Andre runs it on his for everyone.
        assert!(!net.step(&bobs, BOB, service(&bobs_id, "whatsapp", true, Some(&Scope::Global))));
        net.step(&bobs, BOB, service(&bobs_id, "whatsapp", true, Some(&Scope::User(BOB.into()))));
        net.step(&linux, ANDRE, service(&linux_id, "whatsapp", true, Some(&Scope::Global)));
        // Bob may not stop everyone's on Andre's app runtime, nor Andre Bob's own on Bob's.
        assert!(!net.step(&bobs, BOB, service(&linux_id, "whatsapp", false, None)));
        assert!(!net.step(&mac, ANDRE, service(&bobs_id, "whatsapp", false, None)));
        assert_eq!(whatsapp(&net.db, &linux_id), Some((true, Some("global".into()))));
        assert_eq!(whatsapp(&net.db, &bobs_id), Some((true, Some(format!("user:{BOB}")))));
        // Once Bob is an admin, he stops everyone's on Andre's app runtime (moving it, or removing the app), and it stays named.
        net.step(&mac, ANDRE, role(BOB, Role::Admin));
        assert!(net.step(&bobs, BOB, service(&linux_id, "whatsapp", false, None)));
        assert_eq!(whatsapp(&net.db, &linux_id), Some((false, Some("global".into()))));
    }

    #[test]
    fn only_a_devices_user_switches_its_parts_and_lists_them_as_its_own() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
        // Bob, even with Andre's listing, switches nothing on Andre's linux; Andre's own linux does.
        let (linux_id, bobs_id, mac_id) = (linux.id(), bobs.id(), mac.id());
        assert!(!net.step(&bobs, BOB, service(&linux_id, "whatsapp", true, None)));
        assert!(net.db.switches()?.is_empty());
        net.step(&linux, ANDRE, service(&linux_id, "whatsapp", true, None));
        net.step(&mac, ANDRE, service(&linux_id, APPS_PART, false, None));
        assert!(net.db.switch(&linux_id, "whatsapp")?.unwrap().on && !net.db.switch(&linux_id, APPS_PART)?.unwrap().on);
        // linux lists its part; it lists it again naming itself, which an old listing lacked; Bob's device can't.
        let part = Dev::new(8);
        let listed = |device: &Key| listed_service(&part.endpoint(), ANDRE, "linux", "whatsapp", device);
        net.step(&linux, ANDRE, listed(&mac_id));
        assert!(net.step(&linux, ANDRE, listed(&linux_id)));
        assert!(!net.step(&bobs, BOB, listed_service(&part.endpoint(), BOB, "linux", "whatsapp", &bobs_id)));
        let p = net.db.machine(&part.id())?.unwrap();
        assert_eq!((p.user.as_str(), p.device), (ANDRE, Some(linux_id)));
        // Nor is anything but a service listed again by its user.
        assert!(!net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None)));
        assert_eq!(net.db.machine(&linux_id)?.unwrap().roles.len(), 2);
        Ok(())
    }

    #[test]
    fn a_client_says_what_it_is_once_and_keeps_what_it_was_let_in_with() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let phone = Dev::new(3);
        let mut listing = device(&phone.endpoint(), ANDRE, "iPhone", &[Kind::Client], None);
        listing.data["place"] = json!("Melbourne");
        net.step(&mac, ANDRE, listing);
        let says = |platform: &str| {
            let mut c = device(&phone.endpoint(), ANDRE, "iPhone", &[Kind::Client], None);
            c.data["platform"] = json!(platform);
            c
        };
        assert!(!net.step(&phone, ANDRE, says("toaster")));
        assert!(net.step(&phone, ANDRE, says("iphone")));
        let listed = net.db.machine(&phone.id())?.unwrap();
        assert_eq!((listed.platform.as_deref(), listed.place.as_deref()), (Some("iphone"), Some("Melbourne")));
        assert!(!net.step(&phone, ANDRE, says("android")), "it says once");
        Ok(())
    }

    #[test]
    fn a_step_counts_only_when_signed_by_a_device_of_its_user() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        // Andre's device can't sign as Bob, Bob's can't as Andre, and an app runtime signs no change to who's who.
        assert!(!net.step(&mac, BOB, core_step("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(!net.step(&bobs, ANDRE, core_step("setting", json!({ "name": "a", "value": "1" }))));
        assert!(!net.step(&linux, ANDRE, core_step("setting", json!({ "name": "b", "value": "1" }))));
        assert_eq!(net.db.user(BOB)?.unwrap().name, "Bob");
        assert!(net.db.settings()?.is_empty());
        // Bob's own device renames him; it can't list a device for Andre, nor Andre's list a client for Bob.
        assert!(net.step(&bobs, BOB, core_step("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(!net.step(&bobs, BOB, device(&Dev::new(5).endpoint(), ANDRE, "x", &[Kind::Client], None)));
        assert!(!net.step(&mac, ANDRE, device(&Dev::new(6).endpoint(), BOB, "y", &[Kind::Client], None)));
        assert_eq!(net.db.user(BOB)?.unwrap().name, "Bobby");
        // The app runtime runs Andre's scope and the network's, not Bob's.
        assert!(net.db.runs_for(&linux.id(), &Scope::User(ANDRE.into())) && net.db.runs_for(&linux.id(), &Scope::Global));
        assert!(!net.db.runs_for(&linux.id(), &Scope::User(BOB.into())) && !net.db.runs_for(&bobs.id(), &Scope::User(BOB.into())));
        // A machine an admin lists as Bob's runs what Bob may run elsewhere, never Bob's own scope, until Bob's own
        // device lists it.
        let server = Dev::new(7);
        net.step(&mac, ANDRE, device(&server.endpoint(), BOB, "bob's server", &[Kind::Compute], None));
        assert!(!net.db.runs_for(&server.id(), &Scope::User(BOB.into())), "not by an admin's say");
        net.step(&bobs, BOB, device(&server.endpoint(), BOB, "bob's server", &[Kind::Compute], None));
        assert!(net.db.runs_for(&server.id(), &Scope::User(BOB.into())));
        assert!(!net.step(&mac, ANDRE, device(&server.endpoint(), ANDRE, "mine now", &[Kind::Compute], None)), "an admin can't take it back");
        Ok(())
    }

    #[test]
    fn a_new_epoch_counts_only_from_the_scopes_admins_and_names_its_key() -> anyhow::Result<()> {
        let (mac, mut net) = setup();
        let bobs = Dev::new(3);
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        assert_eq!((net.db.epoch(&Scope::Global)?, net.db.sealed_to(&Scope::Global)?), (1, vec![mac.id()]), "global's first is its first device's");
        let to = json!([hex(&mac.id())]);
        let epoch = |scope: &str, n: u32, hash: Option<String>| core_step("epoch", json!({ "scope": scope, "n": n, "to": to, "hash": hash }));
        assert!(!net.step(&bobs, BOB, epoch("global", 2, Some(H.repeat(64)))), "not Bob's to start");
        assert!(!net.step(&bobs, BOB, epoch(&format!("user:{ANDRE}"), 1, Some(H.repeat(64)))));
        assert!(!net.step(&mac, ANDRE, epoch("global", 3, Some(H.repeat(64)))), "epochs go one at a time");
        assert!(!net.step(&mac, ANDRE, epoch("global", 2, None)), "each naming its key");
        assert!(net.step(&mac, ANDRE, epoch("global", 2, Some("f".repeat(64)))));
        let bob = hex(&bobs.id());
        assert!(net.step(&bobs, BOB, core_step("epoch", json!({ "scope": format!("user:{BOB}"), "n": 1, "to": [bob], "hash": H.repeat(64) }))), "a user's own scope is theirs");
        assert_eq!(net.db.epoch_hash(&Scope::Global, 2)?, Some("f".repeat(64)));
        assert_eq!(net.db.epoch_hash(&Scope::Global, 1)?, Some(H.repeat(64)), "each epoch's hash is kept");
        assert_eq!((net.db.epoch(&Scope::User(BOB.into()))?, net.db.epoch(&Scope::User(ANDRE.into()))?), (1, 0));
        Ok(())
    }

    /// A network whose recovery key is `recovery`'s, with Andre's mac and linux (an app runtime), and Bob in with his phone.
    fn with_bob(recovery: &Dev) -> (Dev, Dev, Dev, Net) {
        let (mac, linux, bobs) = (Dev::new(1), Dev::new(4), Dev::new(3));
        let mut net = Net::new(&mac, BOTH, &recovery.id());
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        (mac, linux, bobs, net)
    }

    #[test]
    fn only_a_users_own_device_makes_their_scopes_keys_and_says_where_it_runs() -> anyhow::Result<()> {
        let (mac, linux, bobs, mut net) = with_bob(&Dev::new(9));
        let bob = Scope::User(BOB.into());
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        // Nothing runs Bob's own things until his device says: then the network's machine he chose does.
        assert!(!net.db.runs_for(&linux.id(), &bob));
        net.step(&mac, ANDRE, runs(&[linux.id()]));
        assert!(!net.step(&mac, BOB, runs(&[linux.id()])), "an admin's device can't say where Bob's things run");
        assert!(net.step(&bobs, BOB, runs(&[linux.id()])));
        assert!(net.db.runs_for(&linux.id(), &bob) && net.db.serves(&linux.id(), BOB)?);
        assert!(net.db.runs_for(&linux.id(), &Scope::User(ANDRE.into())), "Andre's says nothing of his own: his machines run it");
        // Only machines that run things: his phone isn't one.
        assert!(!net.step(&bobs, BOB, runs(&[linux.id(), bob_id])), "a list naming what can't run things doesn't count");
        assert!(net.step(&bobs, BOB, runs(&[])));
        assert!(!net.db.runs_for(&linux.id(), &bob), "nothing runs them now");
        // His scope's keys: an admin's device can't make them, his own can, sealed only to his devices.
        assert!(!net.step(&mac, ANDRE, epoch(&bob, 1, &[mac_id], &H.repeat(64))));
        assert!(!net.step(&mac, BOB, epoch(&bob, 1, &[mac_id], &H.repeat(64))));
        assert!(net.step(&bobs, BOB, epoch(&bob, 1, &[bob_id], &"b".repeat(64))));
        assert_eq!(net.db.epoch_hash(&bob, 1)?, Some("b".repeat(64)));
        assert_eq!(net.db.openers(&bob)?.iter().map(|d| d.key).collect::<Vec<_>>(), vec![bobs.id()], "only Bob's own devices open it");
        Ok(())
    }

    #[test]
    fn a_removed_users_devices_get_no_new_epoch_and_their_steps_stop() -> anyhow::Result<()> {
        let (mac, _, bobs, mut net) = with_bob(&Dev::new(9));
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        assert!(net.step(&mac, ANDRE, remove_user(BOB)));
        // A new epoch of everyone's can't be sealed to Bob's phone, and what his phone writes after doesn't count.
        assert!(!net.step(&mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id, bob_id], &"c".repeat(64))), "an epoch sealed to a removed user's device doesn't count");
        assert!(!net.step(&bobs, BOB, core_step("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(net.db.client(&bobs.id())?.is_none() && net.db.clients()?.iter().all(|c| c.user != BOB));
        assert_eq!(net.db.user_ever(BOB)?.unwrap().name, "Bob");
        assert!(net.step(&mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id], &"c".repeat(64))), "sealed to those left, it does");
        assert!(net.db.scopes()?.contains(&Scope::User(BOB.into())), "their own scope is kept");
        Ok(())
    }

    /// The network's last admin can't be taken out or step down, nor be made a member by another; once there's
    /// another, they can.
    #[test]
    fn the_last_admin_stays() -> anyhow::Result<()> {
        let (mac, _, bobs, mut net) = with_bob(&Dev::new(9));
        assert!(!net.step(&mac, ANDRE, role(ANDRE, Role::Member)), "the last admin doesn't step down");
        assert!(!net.step(&mac, ANDRE, remove_user(ANDRE)), "nor take themselves out");
        assert!(!net.step(&bobs, BOB, remove_user(ANDRE)) && !net.step(&bobs, BOB, role(ANDRE, Role::Member)), "nor does a member");
        assert!(!net.step(&bobs, BOB, role(BOB, Role::Admin)), "and no one gives themselves more");
        assert!(net.step(&mac, ANDRE, role(ANDRE, Role::Admin)), "though they may say again they're admin");
        assert!(net.step(&mac, ANDRE, role(BOB, Role::Admin)));
        assert!(net.step(&bobs, BOB, role(ANDRE, Role::Member)), "once there's another, they can");
        assert!(!net.step(&bobs, BOB, remove_user(BOB)), "and the last again stays");
        assert_eq!(net.db.user(ANDRE)?.unwrap().role, Role::Member);
        Ok(())
    }

    #[test]
    fn a_scope_owes_a_new_epoch_once_someone_holding_its_keys_loses_the_right_until_one_counts() -> anyhow::Result<()> {
        let (mac, linux, bobs, mut net) = with_bob(&Dev::new(9));
        let (mac_id, bob_id, linux_id) = (mac.id(), bobs.id(), linux.id());
        let (bob, work) = (Scope::User(BOB.into()), Scope::Group("c000000000000003".into()));
        net.step(&bobs, BOB, epoch(&bob, 1, &[bob_id], &H.repeat(64)));
        net.step(&mac, ANDRE, group("c000000000000003", "Work"));
        net.step(&mac, ANDRE, epoch(&work, 1, &[mac_id], &H.repeat(64)));
        net.step(&mac, ANDRE, invite("c000000000000003", BOB, Role::Member));
        net.step(&bobs, BOB, accept("c000000000000003"));
        // Apps of the group's, the admins' and Bob's, and oned on Cloudflare, which change nothing owed.
        net.step(&mac, ANDRE, core_step("app", json!({ "name": "notes", "owner": "group:c000000000000003" })));
        net.step(&mac, ANDRE, core_step("app", json!({ "name": "tally", "owner": "admins" })));
        net.step(&bobs, BOB, core_step("app", json!({ "name": "bobs", "owner": format!("user:{BOB}") })));
        net.step(&mac, ANDRE, setting(crate::scopes::ONED_ON_CLOUDFLARE, Some(&hex(&Dev::new(14).id()))));
        let (laptop, tablet) = (Dev::new(5), Dev::new(6));
        net.step(&bobs, BOB, device(&laptop.endpoint(), BOB, "laptop", &[Kind::Client], None));
        net.step(&bobs, BOB, device(&tablet.endpoint(), BOB, "tablet", &[Kind::Client], None));
        assert!(net.owed().is_empty());
        // Signing itself out, a device deletes its own keys: nothing's owed.
        assert!(net.step(&laptop, BOB, remove_device(&laptop.id())));
        assert!(net.owed().is_empty() && net.db.machine(&laptop.id())?.is_none(), "signing out owes nothing");
        // Nor one an app runtime of its user's wrote for it, saying it signed itself out; an admin's saying so for another's
        // device is taken as removing it.
        let (asked, said) = (Dev::new(12), Dev::new(13));
        net.step(&bobs, BOB, device(&asked.endpoint(), BOB, "app", &[Kind::Client], None));
        net.step(&bobs, BOB, device(&said.endpoint(), BOB, "app", &[Kind::Client], None));
        let gone = |k: &Dev| core_step("remove", json!({ "key": hex(&k.id()), "itself": true }));
        assert!(net.step(&bobs, BOB, gone(&asked)));
        assert!(net.owed().is_empty() && net.db.machine(&asked.id())?.is_none(), "its own app runtime signing it out owes nothing");
        assert!(net.step(&mac, ANDRE, gone(&said)));
        assert_eq!(net.owed().len(), 3, "an admin's saying it signed itself out is a removal");
        net.step(&bobs, BOB, epoch(&bob, 2, &[bob_id], &H.repeat(64)));
        net.step(&mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id, bob_id], &H.repeat(64)));
        net.step(&mac, ANDRE, epoch(&work, 2, &[mac_id, bob_id], &H.repeat(64)));
        // Nor does a sign-in before on a device signed in again, which goes as signing out would have taken it.
        let on = |d: &Dev| {
            let mut c = device(&d.endpoint(), BOB, "phone app", &[Kind::Client], None);
            c.data["device"] = json!(hex(&[8; 32]));
            c
        };
        let (before, again) = (Dev::new(10), Dev::new(11));
        net.step(&bobs, BOB, on(&before));
        net.step(&bobs, BOB, on(&again));
        assert!(net.step(&bobs, BOB, remove_device(&before.id())));
        assert!(net.owed().is_empty(), "signing in again owes nothing");
        // Taken out by another, it may still hold every key it opened, until each scope's next epoch counts.
        net.step(&mac, ANDRE, remove_device(&tablet.id()));
        assert_eq!(net.owed(), vec![Scope::Global, work.clone(), bob.clone()]);
        net.step(&bobs, BOB, epoch(&bob, 3, &[bob_id], &H.repeat(64)));
        assert!(!net.step(&mac, ANDRE, epoch(&Scope::Global, 3, &[tablet.id()], &H.repeat(64))), "one sealed to the device taken out doesn't count");
        assert_eq!(net.owed(), vec![Scope::Global, work.clone()]);
        net.step(&mac, ANDRE, epoch(&Scope::Global, 3, &[mac_id, bob_id], &H.repeat(64)));
        net.step(&mac, ANDRE, epoch(&work, 3, &[mac_id, bob_id], &H.repeat(64)));
        assert!(net.owed().is_empty());
        // App schedules stopped, and a machine running them taken out.
        net.step(&bobs, BOB, app_schedules(&linux_id, &bob, "notes", 1, &BTreeMap::new()));
        net.step(&bobs, BOB, app_schedules(&linux_id, &bob, "notes", 0, &BTreeMap::new()));
        assert_eq!(net.owed(), vec![bob.clone()]);
        net.step(&bobs, BOB, epoch(&bob, 4, &[bob_id], &H.repeat(64)));
        net.step(&bobs, BOB, app_schedules(&linux_id, &bob, "notes", 1, &BTreeMap::new()));
        net.step(&mac, ANDRE, remove_device(&linux_id));
        assert_eq!(net.owed(), vec![bob.clone()], "a machine holds no epoch, only the keys of the app schedules it runs");
        net.step(&bobs, BOB, epoch(&bob, 5, &[bob_id], &H.repeat(64)));
        // Taken out of a group, then out of the network: their own scope is kept as it is.
        net.step(&mac, ANDRE, member("c000000000000003", BOB, None));
        assert_eq!(net.owed(), vec![work.clone()]);
        net.step(&mac, ANDRE, epoch(&work, 4, &[mac_id], &H.repeat(64)));
        net.step(&mac, ANDRE, remove_user(BOB));
        assert_eq!(net.owed(), vec![Scope::Global]);
        Ok(())
    }

    /// Who may write each kind of step, as the rules say it: each step below, alone on one network, signed by each of
    /// its users' devices and its app runtimes in turn, counts (Y) or doesn't (.). The network: Andre and Carol its admins,
    /// Bob and Dan members, Fay removed; Bob made group G and invited Dan, who's in, and Carol, who hasn't said; Andre's
    /// linux runs everyone's and Bob's own; a box Andre listed for Bob, which Bob listed again; Bob's tablet removed.
    #[test]
    fn who_may_write_each_kind_of_step() {
        const G: &str = "c000000000000003";
        const CAROL: &str = "c000000000000004";
        const DAN: &str = "d000000000000005";
        const FAY: &str = "f000000000000006";
        const GUS: &str = "a000000000000007";
        let recovery = Dev::new(9);
        let (mac, linux, phone) = (Dev::new(1), Dev::new(4), Dev::new(3));
        let (laptop, box_, tablet, dans, fays, cloudflare) = (Dev::new(20), Dev::new(21), Dev::new(22), Dev::new(23), Dev::new(24), Dev::new(14));
        let mut net = Net::new(&mac, BOTH, &recovery.id());
        let (bob, andre, g) = (Scope::User(BOB.into()), Scope::User(ANDRE.into()), Scope::Group(G.into()));
        let base = [
            (&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None)),
            (&mac, ANDRE, device(&phone.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member)))),
            (&mac, ANDRE, device(&laptop.endpoint(), CAROL, "carol's laptop", &[Kind::Client], Some(("Carol", Role::Admin)))),
            (&mac, ANDRE, device(&dans.endpoint(), DAN, "dan's phone", &[Kind::Client], Some(("Dan", Role::Member)))),
            (&mac, ANDRE, device(&fays.endpoint(), FAY, "fay's phone", &[Kind::Client], Some(("Fay", Role::Member)))),
            (&mac, ANDRE, remove_user(FAY)),
            (&mac, ANDRE, device(&box_.endpoint(), BOB, "bob's box", &[Kind::Compute], None)),
            (&phone, BOB, device(&box_.endpoint(), BOB, "bob's box", &[Kind::Compute], None)),
            (&phone, BOB, device(&tablet.endpoint(), BOB, "bob's tablet", &[Kind::Client], None)),
            (&phone, BOB, remove_device(&tablet.id())),
            (&phone, BOB, runs(&[linux.id()])),
            (&mac, ANDRE, setting(crate::scopes::ONED_ON_CLOUDFLARE, Some(&hex(&cloudflare.id())))),
            (&phone, BOB, group(G, "G")),
            (&phone, BOB, invite(G, DAN, Role::Member)),
            (&dans, DAN, accept(G)),
            (&phone, BOB, invite(G, CAROL, Role::Member)),
            (&phone, BOB, epoch(&bob, 1, &[phone.id()], &H.repeat(64))),
            (&mac, ANDRE, epoch(&andre, 1, &[mac.id()], &H.repeat(64))),
            (&phone, BOB, epoch(&g, 1, &[phone.id()], &H.repeat(64))),
            (&phone, BOB, core_step("app", json!({ "name": "notes", "owner": format!("user:{BOB}") }))),
            (&mac, ANDRE, core_step("app", json!({ "name": "tally", "owner": "admins" }))),
            (&mac, ANDRE, core_step("app", json!({ "name": "old", "owner": "admins" }))),
            (&mac, ANDRE, retire("old", "admins", "deleted").unwrap()),
            (&phone, BOB, core_step("app", json!({ "name": "band", "owner": format!("group:{G}") }))),
            (&mac, ANDRE, service(&linux.id(), "whatsapp", true, Some(&Scope::Global))),
            (&phone, BOB, service(&box_.id(), "whatsapp", true, Some(&bob))),
        ];
        for (dev, user, content) in base {
            let label = format!("{} {}", content.kind, content.data);
            assert!(net.step(dev, user, content), "{label}");
        }
        let signers: [(&str, &Dev, &str); 12] = [
            ("andre", &mac, ANDRE),
            ("carol", &laptop, CAROL),
            ("bob", &phone, BOB),
            ("dan", &dans, DAN),
            ("linux as andre", &linux, ANDRE),
            ("linux as bob", &linux, BOB),
            ("linux as dan", &linux, DAN),
            ("mac as bob", &mac, BOB),
            ("bob's removed tablet", &tablet, BOB),
            ("removed fay", &fays, FAY),
            ("recovery as andre", &recovery, ANDRE),
            ("recovery as bob", &recovery, BOB),
        ];
        let (gus, stranger, part) = (Dev::new(30), Dev::new(32), Dev::new(33));
        let k = |d: &Dev| hex(&d.id());
        let bundle = "b".repeat(64);
        let deploy = |app: &str| Content { blobs: vec![bundle.clone()], ..core_step("deploy", json!({ "app": app, "bundle": bundle })) };
        let request = core_step("vault-request", json!({ "id": "e000000000000008", "app": "notes", "title": "Token", "fields": { "token": { "secret": true } }, "hosts": ["api.example.com"] }));
        let global = [
            ("someone new, a member", device(&gus.endpoint(), GUS, "gus's phone", &[Kind::Client], Some(("Gus", Role::Member)))),
            ("someone new, an admin", device(&gus.endpoint(), GUS, "gus's phone", &[Kind::Client], Some(("Gus", Role::Admin)))),
            ("bob's new client", device(&gus.endpoint(), BOB, "bob's laptop", &[Kind::Client], None)),
            ("bob's new compute", device(&gus.endpoint(), BOB, "bob's server", &[Kind::Compute], None)),
            ("bob's new service", listed_service(&part.endpoint(), BOB, "box", "whatsapp", &box_.id())),
            ("andre's new client", device(&gus.endpoint(), ANDRE, "andre's laptop", &[Kind::Client], None)),
            ("fay's new client", device(&gus.endpoint(), FAY, "fay's laptop", &[Kind::Client], None)),
            ("bob's box listed again", device(&box_.endpoint(), BOB, "bob's box", &[Kind::Compute], None)),
            ("bob's phone listed again", device(&phone.endpoint(), BOB, "bob's phone", &[Kind::Client], None)),
            ("refuse a stranger", refuse(&stranger.endpoint(), "a stranger")),
            ("forget bob's phone's sign-in", forget_sign_in(&phone.id(), None)),
            ("bob made admin", role(BOB, Role::Admin)),
            ("bob made member (no change)", role(BOB, Role::Member)),
            ("andre made admin (no change)", role(ANDRE, Role::Admin)),
            ("carol made member", role(CAROL, Role::Member)),
            ("andre made member", role(ANDRE, Role::Member)),
            ("fay made member", role(FAY, Role::Member)),
            ("remove bob", remove_user(BOB)),
            ("remove carol", remove_user(CAROL)),
            ("remove fay (removed)", remove_user(FAY)),
            ("remove bob's phone", remove_device(&phone.id())),
            ("remove linux", remove_device(&linux.id())),
            ("remove bob's box", remove_device(&box_.id())),
            ("remove bob's tablet (removed)", remove_device(&tablet.id())),
            ("global's epoch 2", epoch(&Scope::Global, 2, &[mac.id()], &H.repeat(64))),
            ("bob's epoch 2", epoch(&bob, 2, &[phone.id()], &H.repeat(64))),
            ("andre's epoch 2", epoch(&andre, 2, &[mac.id()], &H.repeat(64))),
            ("g's epoch 2", epoch(&g, 2, &[phone.id()], &H.repeat(64))),
            ("runs on linux", runs(&[linux.id()])),
            ("app schedules, bob's", app_schedules(&linux.id(), &bob, "notes", 1, &BTreeMap::new())),
            ("app schedules, global's", app_schedules(&linux.id(), &Scope::Global, "tally", 1, &BTreeMap::new())),
            ("app schedules, g's", app_schedules(&linux.id(), &g, "band", 1, &BTreeMap::new())),
            ("vault, global's", vault(&Scope::Global, 1, &[8; 32], &[linux.id()])),
            ("vault, bob's", vault(&bob, 1, &[8; 32], &[linux.id()])),
            ("vault, g's", vault(&g, 1, &[8; 32], &[linux.id()])),
            ("a new group", group("c000000000000009", "H")),
            ("invite andre to g, member", invite(G, ANDRE, Role::Member)),
            ("invite andre to g, admin", invite(G, ANDRE, Role::Admin)),
            ("accept g", accept(G)),
            ("dan made g's admin", member(G, DAN, Some(Role::Admin))),
            ("dan g's member (no change)", member(G, DAN, Some(Role::Member))),
            ("bob g's admin (no change)", member(G, BOB, Some(Role::Admin))),
            ("dan out of g", member(G, DAN, None)),
            ("bob out of g", member(G, BOB, None)),
            ("carol's invite to g taken back", member(G, CAROL, None)),
            ("rename bob's phone", rename("key", &k(&phone), "phone")),
            ("rename linux", rename("key", &k(&linux), "server")),
            ("rename bob's box", rename("key", &k(&box_), "box")),
            ("rename bob's tablet (removed)", rename("key", &k(&tablet), "tablet")),
            ("rename cloudflare", rename("key", &k(&cloudflare), "edge")),
            ("rename bob", rename("user", BOB, "Robert")),
            ("rename andre", rename("user", ANDRE, "Andrew")),
            ("rename g", rename("group", G, "Band")),
            ("bob's handle", handle("user", BOB, "robert")),
            ("andre's handle", handle("user", ANDRE, "andrew")),
            ("g's handle", handle("group", G, "band")),
            ("a setting", setting("name", Some("Ours"))),
            ("a new recovery key", core_step("recovery", json!({ "key": k(&stranger) }))),
            ("an app of bob's", core_step("app", json!({ "name": "bobs", "owner": format!("user:{BOB}") }))),
            ("an app of andre's", core_step("app", json!({ "name": "andres", "owner": format!("user:{ANDRE}") }))),
            ("an app of the admins'", core_step("app", json!({ "name": "admins", "owner": "admins" }))),
            ("an app of g's", core_step("app", json!({ "name": "g", "owner": format!("group:{G}") }))),
            ("delete bob's notes", retire("notes", &format!("user:{BOB}"), "deleted").unwrap()),
            ("delete the admins' tally", retire("tally", "admins", "deleted").unwrap()),
            ("delete g's band", retire("band", &format!("group:{G}"), "deleted").unwrap()),
            ("restore the admins' old", retire("old", "admins", "restored").unwrap()),
            ("erase the admins' old", retire("old", "admins", "erased").unwrap()),
            ("erase the admins' tally", retire("tally", "admins", "erased").unwrap()),
            ("give tally to bob", core_step("app", json!({ "name": "tally", "owner": format!("user:{BOB}") }))),
            ("give notes to the admins", core_step("app", json!({ "name": "notes", "owner": "admins" }))),
            ("tally's source", core_step("app", json!({ "name": "tally", "owner": "admins", "source": "https://example.com/tally.git" }))),
            ("deploy notes", deploy("notes")),
            ("deploy tally", deploy("tally")),
            ("deploy band", deploy("band")),
            ("deploy old (deleted)", deploy("old")),
            ("linux's whatsapp off", service(&linux.id(), "whatsapp", false, None)),
            ("linux's whatsapp on, global", service(&linux.id(), "whatsapp", true, Some(&Scope::Global))),
            ("linux's whatsapp on, bob's", service(&linux.id(), "whatsapp", true, Some(&bob))),
            ("linux's apps off", service(&linux.id(), crate::scopes::APPS_PART, false, None)),
            ("bob's box's whatsapp off", service(&box_.id(), "whatsapp", false, None)),
            ("bob's box's whatsapp on, global", service(&box_.id(), "whatsapp", true, Some(&Scope::Global))),
            ("bob's box's notes on, bob's", service(&box_.id(), "notes", true, Some(&bob))),
            ("cloudflare's notes on", service(&cloudflare.id(), "notes", true, Some(&Scope::Global))),
            ("cloudflare's notes off", service(&cloudflare.id(), "notes", false, None)),
            ("a node release", core_step("node-release", json!({ "version": "9.9.9" }))),
            ("an unknown kind", core_step("frobnicate", json!({}))),
            ("an app's own", Content::new("notes", "note", json!({ "text": "hi" }))),
        ];
        let own = |page: &str| {
            [
                ("add-app notes", added("notes", true)),
                ("remove-app notes", added("notes", false)),
                ("schedule", core_step("schedule", json!({ "app": "notes", "schedule": "daily", "on": true }))),
                ("vault request", request.clone()),
                ("vault request declined", core_step("vault-request", json!({ "id": "e000000000000008", "declined": true }))),
                ("keep", core_step("keep", json!({}))),
                ("forget", core_step("forget", json!({ "blob": "c".repeat(64) }))),
                ("arrange", core_step("arrange", json!({ "page": page, "items": [{ "app": "notes" }] }))),
                ("an app's own", Content::new("notes", "note", json!({ "text": "hi" }))),
                ("a setting", setting("name", Some("Ours"))),
            ]
        };
        let mut rows = vec![(Scope::Global, global.to_vec())];
        for (s, page) in [(Scope::Global, "global".to_string()), (andre.clone(), "user".into()), (bob.clone(), "user".into()), (g.clone(), format!("group:{G}"))] {
            rows.push((s, own(&page).to_vec()));
        }
        let mut table = String::new();
        for (scope, steps) in rows {
            let empty = ScopeDb::empty(&scope, 0).unwrap();
            for (what, content) in steps {
                let marks: String = signers
                    .iter()
                    .map(|(_, dev, user)| {
                        let s = steps::sign(&dev.0, &scope, net.at + 1, user, 1, net.at + 1, content.clone()).unwrap();
                        let counts = match scope {
                            Scope::Global => steps::apply(&net.db, None, &scope, &s),
                            _ => steps::apply(&empty, Some(&net.db), &scope, &s),
                        };
                        if counts.is_ok() { 'Y' } else { '.' }
                    })
                    .collect();
                let scope = match &scope {
                    Scope::Global => "global",
                    Scope::User(u) if u == ANDRE => "andre's",
                    Scope::User(_) => "bob's",
                    Scope::Group(_) => "g's",
                };
                table += &format!("{scope:<8} {what:<34} {marks}\n");
            }
        }
        let header: Vec<&str> = signers.iter().map(|s| s.0).collect();
        assert!(table.lines().eq(WHO_MAY.lines().map(str::trim).filter(|l| !l.is_empty())), "who may write what changed (columns: {}):\n{table}", header.join(", "));
    }

    #[test]
    fn the_recovery_key_lets_someones_device_in_as_an_admins_approval_and_is_listed() -> anyhow::Result<()> {
        let recovery = Dev::new(9);
        let (mac, _, _, mut net) = with_bob(&recovery);
        let laptop = Dev::new(5);
        assert!(net.db.recovery_used()?.is_none());
        assert!(net.step(&recovery, BOB, device(&laptop.endpoint(), BOB, "bob's laptop", &[Kind::Client], None)));
        assert_eq!(net.db.client(&laptop.id())?.map(|c| c.user), Some(BOB.to_string()), "it counts as an admin's approval");
        assert_eq!(net.db.recovery_used()?, Some((net.at, BOB.to_string())), "and it's listed");
        // Only a device users use, of someone already in; and nothing else.
        assert!(!net.step(&recovery, BOB, device(&Dev::new(6).endpoint(), BOB, "box", &[Kind::Compute], None)));
        assert!(!net.step(&recovery, "c000000000000003", device(&Dev::new(7).endpoint(), "c000000000000003", "new", &[Kind::Client], Some(("Cy", Role::Admin)))));
        assert!(!net.step(&recovery, ANDRE, core_step("setting", json!({ "name": "a", "value": "1" }))));
        assert!(!net.step(&recovery, ANDRE, role(BOB, Role::Admin)));
        assert!(net.db.user("c000000000000003")?.is_none() && net.db.user(BOB)?.unwrap().role == Role::Member);
        // A key that isn't the network's recovery key lets nobody in.
        let other = Dev::new(8);
        assert!(!net.step(&other, BOB, device(&Dev::new(10).endpoint(), BOB, "x", &[Kind::Client], None)));
        // Only the recovery key hands over to a new one: an admin's device can't, and the old key counts no more.
        assert!(!net.step(&mac, ANDRE, core_step("recovery", json!({ "key": hex(&other.id()) }))), "not by an admin's device");
        assert!(net.step(&recovery, ANDRE, core_step("recovery", json!({ "key": hex(&other.id()) }))));
        assert_eq!((net.db.recovery()?, net.db.recovery_set()?, net.db.recovery_was()?), (Some(other.id()), net.at, vec![recovery.id()]));
        assert!(!net.step(&recovery, BOB, device(&Dev::new(11).endpoint(), BOB, "y", &[Kind::Client], None)), "the old recovery key lets nobody in");
        // Handed over, the new key starts a scope's next keys, for a user with no device left to do it, and nothing else.
        let new_epoch = |n: u32| epoch(&Scope::User(BOB.to_string()), n, &[], &H.repeat(64));
        assert!(!net.step(&recovery, BOB, new_epoch(1)), "not the old key");
        assert!(net.step(&other, ANDRE, new_epoch(1)), "and a scope's next keys, whoever's it is");
        assert!(!net.step(&other, ANDRE, role(BOB, Role::Admin)), "and nothing else");
        Ok(())
    }

    /// Columns: andre, carol, bob, dan, linux as andre, linux as bob, linux as dan, mac as bob, bob's removed tablet,
    /// removed fay, recovery as andre, recovery as bob.
    const WHO_MAY: &str = "
        global   someone new, a member              YY..........
        global   someone new, an admin              YY..........
        global   bob's new client                   ..Y........Y
        global   bob's new compute                  YYY.........
        global   bob's new service                  ..Y.........
        global   andre's new client                 Y.........Y.
        global   fay's new client                   ............
        global   bob's box listed again             ............
        global   bob's phone listed again           ..Y.........
        global   refuse a stranger                  YYYY........
        global   forget bob's phone's sign-in       ..Y.........
        global   bob made admin                     YY..........
        global   bob made member (no change)        YY..........
        global   andre made admin (no change)       YY..........
        global   carol made member                  YY..........
        global   andre made member                  YY..........
        global   fay made member                    ............
        global   remove bob                         YY..........
        global   remove carol                       YY..........
        global   remove fay (removed)               ............
        global   remove bob's phone                 YYY.........
        global   remove linux                       YY..........
        global   remove bob's box                   YYY.........
        global   remove bob's tablet (removed)      ............
        global   global's epoch 2                   YY........YY
        global   bob's epoch 2                      ..Y.......YY
        global   andre's epoch 2                    Y.........YY
        global   g's epoch 2                        YYY.......YY
        global   runs on linux                      YYYY........
        global   app schedules, bob's               ..Y.........
        global   app schedules, global's            YY..........
        global   app schedules, g's                 ..Y.........
        global   vault, global's                    YY........YY
        global   vault, bob's                       ..Y.......YY
        global   vault, g's                         ..Y.......YY
        global   a new group                        YYYY........
        global   invite andre to g, member          YYY.........
        global   invite andre to g, admin           YYY.........
        global   accept g                           .Y..........
        global   dan made g's admin                 YYY.........
        global   dan g's member (no change)         YYY.........
        global   bob g's admin (no change)          YYY.........
        global   dan out of g                       YYYY........
        global   bob out of g                       YYY.........
        global   carol's invite to g taken back     YYY.........
        global   rename bob's phone                 YYY.........
        global   rename linux                       YY..........
        global   rename bob's box                   YYY.........
        global   rename bob's tablet (removed)      ............
        global   rename cloudflare                  ............
        global   rename bob                         YYY.........
        global   rename andre                       YY..........
        global   rename g                           YYY.........
        global   bob's handle                       YYY.........
        global   andre's handle                     YY..........
        global   g's handle                         YYY.........
        global   a setting                          YY..........
        global   a new recovery key                 ..........YY
        global   an app of bob's                    ..Y.........
        global   an app of andre's                  Y...........
        global   an app of the admins'              YY..........
        global   an app of g's                      ..Y.........
        global   delete bob's notes                 ..Y.........
        global   delete the admins' tally           YY..........
        global   delete g's band                    ..Y.........
        global   restore the admins' old            YY..........
        global   erase the admins' old              YY..........
        global   erase the admins' tally            ............
        global   give tally to bob                  ............
        global   give notes to the admins           ............
        global   tally's source                     YY..........
        global   deploy notes                       ..Y.........
        global   deploy tally                       YY..........
        global   deploy band                        ..Y.........
        global   deploy old (deleted)               ............
        global   linux's whatsapp off               YY..........
        global   linux's whatsapp on, global        Y...........
        global   linux's whatsapp on, bob's         ............
        global   linux's apps off                   Y...........
        global   bob's box's whatsapp off           ..Y.........
        global   bob's box's whatsapp on, global    ............
        global   bob's box's notes on, bob's        ..Y.........
        global   cloudflare's notes on              YY..........
        global   cloudflare's notes off             YY..........
        global   a node release                     YYYY........
        global   an unknown kind                    ............
        global   an app's own                       YYYYYYYY....
        global   add-app notes                      YY..........
        global   remove-app notes                   YY..........
        global   schedule                           YY..Y.......
        global   vault request                      YY..Y.......
        global   vault request declined             ............
        global   keep                               YYYYYYYY....
        global   forget                             YYYYYYYY....
        global   arrange                            ............
        global   an app's own                       YYYYYYYY....
        global   a setting                          YY..........
        andre's  add-app notes                      Y...Y.......
        andre's  remove-app notes                   Y...Y.......
        andre's  schedule                           Y...Y.......
        andre's  vault request                      Y...Y.......
        andre's  vault request declined             ............
        andre's  keep                               Y...Y.......
        andre's  forget                             Y...Y.......
        andre's  arrange                            Y...Y.......
        andre's  an app's own                       Y...Y.......
        andre's  a setting                          ............
        bob's    add-app notes                      ..Y..Y......
        bob's    remove-app notes                   ..Y..Y......
        bob's    schedule                           ..Y..Y......
        bob's    vault request                      ..Y..Y......
        bob's    vault request declined             ............
        bob's    keep                               ..Y..Y......
        bob's    forget                             ..Y..Y......
        bob's    arrange                            ..Y..Y......
        bob's    an app's own                       ..Y..Y......
        bob's    a setting                          ............
        g's      add-app notes                      ..Y..Y.Y....
        g's      remove-app notes                   ..Y..Y.Y....
        g's      schedule                           ..Y..Y.Y....
        g's      vault request                      ..Y..Y.Y....
        g's      vault request declined             ............
        g's      keep                               ..YY.YYY....
        g's      forget                             ..YY.YYY....
        g's      arrange                            ............
        g's      an app's own                       ..YY.YYY....
        g's      a setting                          ............
    ";
}
