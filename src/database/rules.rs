//! Each kind of step's rule: who may write it, as `may` questions of `global`'s scope database (and rows where a
//! question is of what exists), and what it changes, as SQL on the database it's applied to. What follows from those
//! rows (who owns which device and app, which devices run which scope's calls) is worked out once after
//! (`scope::derive`), never written by a step.
//!
//! The signer is checked first, for every kind (`signed`): one of its person's own devices still listed; or, for a
//! scope's own state, a device that runs the scope's calls for them; or, for letting a device in or handing over to
//! the next recovery key, the network's recovery key. So the person a step is written as is always someone in the
//! network, and each rule asks only what that person may do.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::database::steps::Step;

use crate::keys::{Hash, Key, hex, unhex};

use crate::database::scopes::{Scope, is_id};
use crate::database::scopes;
use crate::database::scopes::{AGENTS, APP, APP_SCHEDULES, Kind, Machine, Principal, Role, handle_from, source_link, valid_app_name, valid_handle, valid_host, valid_page, valid_secret_name};

use super::scope::{group, int, owner, person, role};
use super::scopes::{NETWORK, ScopeDb, object, owner_object};

/// Kinds of step that change only their own scope's state, wherever they're written.
const OWN: &[&str] = &["schedule", "vault-request", "keep", "forget", "arrange"];

/// Whether a step is one of `global`'s own: who's who, written only by its person's own device (or the recovery key).
fn of_global(step: &Step, scope: &Scope) -> bool {
    *scope == Scope::Global && step.content.app == APP && !OWN.contains(&step.content.kind.as_str())
}

/// Whether `step`'s signer may write it as its person, in `scope`, by `who`, `global`'s database before it.
pub fn signed(who: &ScopeDb, scope: &Scope, step: &Step) -> Result<()> {
    let (w, author) = (step.user(), step.author());
    if who.recovery() == Some(author) {
        // The network's recovery key, kept offline by an admin, only lets a device of someone's in, as an admin's would,
        // and hands over to the next recovery key.
        ensure!(of_global(step, scope) && matches!(step.content.kind.as_str(), "device" | "recovery") && who.user(w).is_some(), "the recovery key only lets a device in");
        return Ok(());
    }
    ensure!(who.may(w, "use", &object(scope)), "{w} can't write in {scope}");
    let theirs = who.client(&author).is_some_and(|c| c.user == w);
    ensure!(theirs || (!of_global(step, scope) && who.runs_for(&author, scope)), "{} can't write in {scope} for that person", crate::keys::short(&author));
    Ok(())
}

/// Applies `step`, which counts by `signed` and `steps::allowed`, to `out`, `scope`'s database before it, by its kind's
/// rule, `who` being `global`'s database before it: → why it doesn't count, if it doesn't.
pub fn apply(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Step) -> Result<()> {
    if step.content.app != APP {
        return Ok(());
    }
    let kind = step.content.kind.as_str();
    if *scope != Scope::Global || OWN.contains(&kind) {
        return own(who, out, scope, step);
    }
    let c = Change { who, out, step, w: step.user(), data: &step.content.data };
    match kind {
        "device" if who.recovery() == Some(step.author()) => c.recovered(),
        "device" => c.device(),
        "refuse" => c.refuse(),
        "forget-sign-in" => c.forget_sign_in(),
        "role" => c.role(),
        "remove" => c.remove(),
        "epoch" => c.epoch(),
        "runs" => c.runs(),
        APP_SCHEDULES => c.app_schedules(),
        "vault" => c.vault(),
        "group" => c.group(),
        "invite" => c.invite(),
        "accept" => c.accept(),
        "member" => c.member(),
        "rename" => c.rename(),
        "setting" => c.setting(),
        "recovery" => c.recovery(),
        "app" => c.app(),
        "deploy" => c.deploy(),
        "service" => c.service(),
        "install" | "uninstall" => own(who, out, scope, step),
        k if scopes::RELEASES.contains(&k) => Ok(()),
        other => bail!("unknown kind {other}"),
    }
}

/// The network `root`'s first person, device, epoch and recovery key, as `data` says them (`content::first`), into
/// `out`, empty, at `now`.
pub fn first(out: &Connection, root: Hash, data: &Value, now: u64) -> Result<()> {
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let user = text("user")?;
    ensure!(is_id(&user), "a person's id is 16 hex characters");
    let machine = listed(&data["device"], &user, now)?;
    ensure!(machine.is(Kind::Client), "a network's first device is a client");
    let hash = epoch_hash_of(&data["epoch"])?;
    let time = int(now);
    out.execute("UPDATE network SET root = ?, name = ?, devices_at = ?", params![root.to_hex(), text("name")?, time])?;
    out.execute("INSERT INTO recovery VALUES (0, ?, ?)", params![hex(&unhex(&text("recovery")?)?), time])?;
    add_person(out, &user, &text("user_name")?, Role::Admin)?;
    put(out, &machine)?;
    // `global`'s first epoch, made with the network, before anything is sealed under it.
    out.execute("INSERT INTO epochs VALUES ('global', 1, ?)", [&hash])?;
    out.execute("INSERT INTO sealed VALUES ('global', 1, ?)", [hex(&machine.key)])?;
    Ok(())
}

/// One of `global`'s own steps, being applied.
struct Change<'a> {
    who: &'a ScopeDb,
    out: &'a Connection,
    step: &'a Step,
    w: &'a str,
    data: &'a Value,
}

impl Change<'_> {
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

    fn admin(&self) -> bool {
        self.who.may(self.w, "change-settings", NETWORK)
    }

    /// Whether the writer manages group `g`: its admins, and the network's (so a group whose last admin left isn't stuck).
    fn manages(&self, g: &str) -> bool {
        self.who.group(g).is_some() && self.who.may(self.w, "remove", &group(g))
    }

    /// Whether the writer is a scope's admin, who sets its app schedules and Vault.
    fn scope_admin(&self, scope: &Scope) -> bool {
        self.who.may(self.w, "choose-devices", &object(scope))
    }

    /// Whether `k` is one of the network's machines that run things, or its Cloudflare node.
    fn runs_things(&self, k: &Key) -> bool {
        self.who.compute(k).is_some() || self.who.cloudflare_node() == Some(*k)
    }

    fn devices_changed(&self) -> Result<()> {
        self.out.execute("UPDATE network SET devices_at = max(devices_at, ?)", [self.time()])?;
        Ok(())
    }

    /// That `scope` owes a new epoch, if it has one to follow.
    fn owe(&self, scope: &Scope) -> Result<()> {
        if self.who.epoch(scope) > 0 {
            self.out.execute("INSERT OR IGNORE INTO owed VALUES (?)", [scope.to_string()])?;
        }
        Ok(())
    }

    /// Owes a new epoch of each scope whose keys `keys` held, which their removal takes the right to: each a client of
    /// them opens, but `except` (a person's own, which removing them keeps as it is), and each one of them runs app
    /// schedules in.
    fn owe_held_by(&self, keys: &[Key], except: Option<&Scope>) -> Result<()> {
        let opens = |s: &Scope| keys.iter().filter_map(|k| self.who.client(k)).any(|d| self.who.may(&d.user, "use", &object(s)));
        let scheduled = self.who.app_schedules().into_keys().filter(|(k, _, _)| keys.contains(k)).map(|(_, s, _)| s);
        for s in self.who.scopes().into_iter().filter(opens).chain(scheduled).filter(|s| Some(s) != except) {
            self.owe(&s)?;
        }
        Ok(())
    }

    /// A person who lost every device let back in with the recovery key: a new client of someone already in.
    fn recovered(&self) -> Result<()> {
        let user = self.id("user")?;
        let machine = listed(self.data, &user, self.step.time())?;
        ensure!(user == self.w && self.who.user(&user).is_some(), "the recovery key lets in a device of someone already in");
        ensure!(machine.roles == BTreeSet::from([Kind::Client]) && machine.passkey.is_none(), "the recovery key lets in a device people use");
        ensure!(self.who.machine_ever(&machine.key).is_none(), "that key is listed already");
        put(self.out, &machine)?;
        self.devices_changed()?;
        self.out.execute("UPDATE network SET recovery_used_at = ?, recovery_used_by = ?", params![self.time(), person(&user)])?;
        Ok(())
    }

    fn device(&self) -> Result<()> {
        let (w, user) = (self.w, self.id("user")?);
        let mut machine = listed(self.data, &user, self.step.time())?;
        machine.by = w.to_string();
        // A service answers its person's calls as them: only their own device lists one, never an admin's.
        ensure!(!machine.is(Kind::Service) || w == user, "only a person's own device lists their services");
        // A machine an admin listed for someone is listed again by that person's own device to be theirs: then it runs
        // their own scope too.
        if let Some(was) = self.who.machine_ever(&machine.key) {
            // Or a service of theirs listed again by their own device, to name the device it's part of; or a sign-in that
            // named no device, naming the one it's on.
            let part = was.is(Kind::Service) && was.roles == machine.roles && was.service == machine.service;
            let part = part || (was.is(Kind::Client) && was.roles == machine.roles && was.device.is_none() && machine.device.is_some() && was.passkey == machine.passkey);
            ensure!(!was.removed && was.user == user && w == user && (was.by != user || part) && was.seal == machine.seal, "that key is listed already");
        } else {
            match self.who.user_ever(&user) {
                Some(u) => ensure!(!u.removed && (w == user || (self.admin() && !machine.is(Kind::Client))), "only a person's own device lists their devices; an admin's lists compute and services"),
                None => {
                    ensure!(self.admin(), "only admins add people");
                    add_person(self.out, &user, &self.text("user_name")?, self.role_of("role")?)?;
                }
            }
        }
        put(self.out, &machine)?;
        self.devices_changed()
    }

    /// A device its person said isn't theirs ("Not Me"), before it was ever listed: its key is taken out now, so no
    /// device of theirs lets it in, by its sign-in or its code, and nothing it writes counts.
    fn refuse(&self) -> Result<()> {
        let key = self.key("key")?;
        ensure!(self.who.machine_ever(&key).is_none(), "that key is listed already: take it out instead");
        let data = json!({ "key": self.data["key"], "seal": self.data["seal"], "name": self.data["name"], "roles": [Kind::Client] });
        let mut machine = listed(&data, self.w, self.step.time())?;
        machine.removed = true;
        put(self.out, &machine)?;
        self.devices_changed()
    }

    /// What one of their devices signed in with, forgotten by its person's own device: a new device's sign-in is no
    /// longer checked against it.
    fn forget_sign_in(&self) -> Result<()> {
        let key = self.key("key")?;
        ensure!(self.who.machine(&key).context("no such device")?.user == self.w, "only its person's own device forgets what it signed in with");
        self.out.execute("UPDATE devices SET sign_in = NULL WHERE key = ?", [hex(&key)])?;
        self.devices_changed()
    }

    fn role(&self) -> Result<()> {
        ensure!(self.admin(), "admins only");
        let (user, to) = (self.id("user")?, self.role_of("role")?);
        self.who.user(&user).context("no such user")?;
        set_role(self.out, &person(&user), NETWORK, Some(to))
    }

    fn remove(&self) -> Result<()> {
        if let Ok(user) = self.id("user") {
            ensure!(self.admin(), "admins only");
            self.who.user_ever(&user).context("no such user")?;
            let theirs: Vec<Key> = self.who.machines().into_iter().filter(|d| d.user == user).map(|d| d.key).collect();
            self.owe_held_by(&theirs, Some(&Scope::User(user.clone())))?;
            // Everything they held goes with them; what they made stays.
            self.out.execute("UPDATE principals SET removed = 1 WHERE id = ?", [person(&user)])?;
            self.out.execute("DELETE FROM relations WHERE subject = ?", [person(&user)])?;
            self.out.execute("DELETE FROM invites WHERE principal = ?", [person(&user)])?;
        } else {
            let key = self.key("key")?;
            let gone = self.who.machine(&key).context("no such key")?;
            ensure!(self.who.may(self.w, "remove", &super::scopes::device(&key)), "only its person or an admin removes a device");
            // A device signing itself out deletes its own keys: signed by it, or by a node of its person's it asked
            // (`itself`). So does a sign-in before on a device signed in again (`Node::approve_request`). One removed
            // otherwise may still hold them.
            let author = self.step.author();
            let itself = author == key || (self.data["itself"] == true && self.who.machine_ever(&author).is_some_and(|a| a.user == gone.user));
            let again = gone.device.is_some() && gone.passkey.is_none() && self.who.clients().iter().any(|c| c.key != key && c.user == gone.user && c.device == gone.device && c.passkey.is_none());
            if !(itself || again) {
                self.owe_held_by(&[key], None)?;
            }
            self.out.execute("UPDATE devices SET removed = 1 WHERE key = ?", [hex(&key)])?;
        }
        self.devices_changed()
    }

    fn epoch(&self) -> Result<()> {
        let scope = Scope::parse(&self.text("scope")?)?;
        // Its admins': a person's own scope is theirs alone, and a group's admins' or the network's.
        let allowed = match &scope {
            Scope::Global => self.admin(),
            Scope::Group(g) => self.manages(g),
            Scope::User(u) => u == self.w,
        };
        ensure!(allowed, "only the scope's admins start its new epoch");
        let n = self.data["n"].as_u64().context("no n")?;
        ensure!(n == u64::from(self.who.epoch(&scope)) + 1, "an epoch follows the one before it");
        let to = self.keys("to")?;
        ensure!(to.iter().all(|k| self.who.machine(k).is_some()), "an epoch is sealed only to the network's endpoints");
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

    /// Where a person's own things run besides their own machines: only from their own device, for their own scope,
    /// and only the network's machines that run things, and its Cloudflare node.
    fn runs(&self) -> Result<()> {
        let on: BTreeSet<Key> = self.keys("on").context("on is a list of endpoints' keys")?.into_iter().collect();
        ensure!(on.iter().all(|k| self.runs_things(k)), "only the network's machines that run things, and its Cloudflare node, run someone's own");
        let me = person(self.w);
        self.out.execute("INSERT OR IGNORE INTO runs_chosen VALUES (?)", [&me])?;
        self.out.execute("DELETE FROM runs WHERE principal = ?", [&me])?;
        for k in &on {
            self.out.execute("INSERT INTO runs VALUES (?, ?)", params![me, hex(k)])?;
        }
        Ok(())
    }

    fn app_schedules(&self) -> Result<()> {
        let endpoint = self.key("endpoint")?;
        let scope = Scope::parse(&self.text("scope")?)?;
        let app = self.text("app")?;
        // The node's own databases of a scope (its log, its Vault) are never an app's to schedule.
        ensure!(valid_app_name(&app), "app schedules are an app's");
        let until = self.data["until"].as_u64().context("no until")?;
        let mut hashes = Vec::new();
        for (n, h) in self.data["keys"].as_object().into_iter().flatten() {
            hashes.push((n.parse::<u32>().context("an epoch is a number")?, epoch_hash_of(h)?));
        }
        ensure!(self.scope_admin(&scope), "only the scope's person, or its group's admins, set app schedules in it");
        ensure!(until == 0 || self.who.machine(&endpoint).is_some(), "no such endpoint");
        let row = params![hex(&endpoint), scope.to_string(), app];
        let had = self.out.execute("DELETE FROM app_schedules WHERE device = ? AND scope = ? AND app = ?", row)? > 0;
        self.out.execute("DELETE FROM app_schedule_keys WHERE device = ? AND scope = ? AND app = ?", row)?;
        if until == 0 {
            if had {
                self.owe(&scope)?;
            }
            return Ok(());
        }
        self.out.execute("INSERT INTO app_schedules VALUES (?, ?, ?, ?, ?)", params![hex(&endpoint), scope.to_string(), app, person(self.w), int(until)])?;
        for (n, hash) in hashes {
            self.out.execute("INSERT OR REPLACE INTO app_schedule_keys VALUES (?, ?, ?, ?, ?)", params![hex(&endpoint), scope.to_string(), app, n, hash])?;
        }
        Ok(())
    }

    /// Who opens a scope's Vault is its admins' to say, never a member's who can only add to it.
    fn vault(&self) -> Result<()> {
        let scope = Scope::parse(&self.text("scope")?)?;
        ensure!(self.scope_admin(&scope), "only the scope's person, or its admins, say which machines open its Vault");
        let n = self.data["n"].as_u64().context("no n")?;
        ensure!(n == u64::from(self.who.vault(&scope).map_or(0, |v| v.n)) + 1, "a Vault key follows the one before it");
        let public = self.key("public")?;
        let to = self.keys("to")?;
        ensure!(to.iter().all(|k| self.runs_things(k)), "a Vault opens only on the network's machines that run things");
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
        let g = self.id("group")?;
        ensure!(self.who.group(&g).is_none(), "that group exists");
        let name = self.text("name")?;
        self.out.execute("INSERT INTO principals VALUES (?, 'group', ?, ?, 0)", params![group(&g), name, pick_handle(self.out, &name, "group")?])?;
        set_role(self.out, &person(self.w), &group(&g), Some(Role::Admin))
    }

    fn invite(&self) -> Result<()> {
        let (g, user, to) = (self.id("group")?, self.id("user")?, self.role_of("role")?);
        ensure!(self.manages(&g), "only the group's admins invite");
        self.who.user(&user).context("no such user")?;
        self.out.execute("INSERT OR REPLACE INTO invites VALUES (?, ?, ?)", params![person(&user), group(&g), role(to)])?;
        Ok(())
    }

    /// Taking up an invite: the role it gave.
    fn accept(&self) -> Result<()> {
        let g = self.id("group")?;
        let invited = self.who.group(&g).context("no such group")?.invited.get(self.w).copied().context("not invited")?;
        self.out.execute("DELETE FROM invites WHERE principal = ? AND invited_to = ?", params![person(self.w), group(&g)])?;
        set_role(self.out, &person(self.w), &group(&g), Some(invited))
    }

    /// A member's role in a group changed, or them taken out of it (or their invite), by its admins; or leaving it.
    fn member(&self) -> Result<()> {
        let (g, user) = (self.id("group")?, self.id("user")?);
        let to: Option<Role> = serde_json::from_value(self.data["role"].clone())?;
        ensure!(self.manages(&g) || (to.is_none() && user == self.w), "only the group's admins change its members");
        let grp = self.who.group(&g).context("no such group")?;
        let member = grp.members.contains_key(&user) && self.who.user(&user).is_some();
        ensure!(member || grp.invited.contains_key(&user), "not in the group");
        match to {
            Some(_) => ensure!(member, "not in the group"),
            None => {
                self.out.execute("DELETE FROM invites WHERE principal = ? AND invited_to = ?", params![person(&user), group(&g)])?;
                if member {
                    self.owe(&Scope::Group(g.clone()))?;
                }
            }
        }
        set_role(self.out, &person(&user), &group(&g), to)
    }

    fn rename(&self) -> Result<()> {
        // A person's or group's handle: one that's taken (wherever it sorts) or malformed doesn't count.
        if let Some(handle) = self.data.get("handle") {
            let handle = handle.as_str().context("a handle is text")?;
            ensure!(valid_handle(handle), "a handle is lowercase letters, digits and single hyphens, up to 39, like {}", handle_from(handle, "name"));
            let mine = match (self.id("user"), self.id("group")) {
                (Ok(user), _) => {
                    ensure!(user == self.w || self.admin(), crate::protocol::Coded::refused("not allowed to change that person's handle"));
                    self.who.user(&user).context("no such user")?;
                    Principal::User(user)
                }
                (_, Ok(g)) => {
                    ensure!(self.manages(&g), crate::protocol::Coded::refused("not allowed to change that group's handle"));
                    Principal::Group(g)
                }
                _ => bail!("a handle is a person's or a group's"),
            };
            ensure!(self.who.by_handle(handle).is_none_or(|p| p == mine), "{handle} is taken");
            self.out.execute("UPDATE principals SET handle = ? WHERE id = ?", params![handle, owner(&mine)])?;
            return Ok(());
        }
        let name = self.text("name")?;
        ensure!(!name.trim().is_empty(), "no name");
        if let Ok(key) = self.key("key") {
            self.who.machine(&key).context("no such key")?;
            ensure!(self.who.may(self.w, "remove", &super::scopes::device(&key)), crate::protocol::Coded::refused("not allowed to rename that"));
            self.out.execute("UPDATE devices SET name = ? WHERE key = ?", params![name, hex(&key)])?;
        } else if let Ok(user) = self.id("user") {
            ensure!(user == self.w || self.admin(), crate::protocol::Coded::refused("not allowed to rename that user"));
            self.who.user(&user).context("no such user")?;
            self.out.execute("UPDATE principals SET name = ? WHERE id = ?", params![name, person(&user)])?;
        } else {
            let g = self.id("group")?;
            ensure!(self.manages(&g), crate::protocol::Coded::refused("not allowed to rename that group"));
            self.out.execute("UPDATE principals SET name = ? WHERE id = ?", params![name, group(&g)])?;
        }
        Ok(())
    }

    fn setting(&self) -> Result<()> {
        ensure!(self.admin(), "admins only");
        let name = self.text("name")?;
        ensure!(crate::labs::valid_setting(&name), "setting names are lowercase words joined by _, or a lab's (lab.…)");
        match &self.data["value"] {
            Value::Null => self.out.execute("DELETE FROM settings WHERE name = ?", [&name])?,
            value => self.out.execute("INSERT OR REPLACE INTO settings VALUES (?, ?)", params![name, value.as_str().context("a setting is text")?])?,
        };
        Ok(())
    }

    /// Only the recovery key itself hands over to a new one: no admin's device can, since every epoch is sealed to it,
    /// own scopes too. Unless there's none: then an admin's own device names the next.
    fn recovery(&self) -> Result<()> {
        let now = self.who.recovery();
        let named = now.is_none() && self.admin() && self.who.client(&self.step.author()).is_some_and(|c| c.user == self.w);
        ensure!(now == Some(self.step.author()) || named, "only the network's recovery key changes it");
        let key = self.key("key")?;
        ensure!(Some(key) != now && !self.who.recovery_was().contains(&key) && self.who.machine_ever(&key).is_none(), "a new recovery key is a key of its own");
        self.out.execute("UPDATE recovery SET set_at = NULL", [])?;
        self.out.execute("INSERT INTO recovery VALUES ((SELECT count(*) FROM recovery), ?, ?)", params![hex(&key), self.time()])?;
        Ok(())
    }

    /// An app made; or deleted (to Recently Deleted), restored, erased for good, given away, or its code's place said.
    fn app(&self) -> Result<()> {
        let name = self.text("name")?;
        ensure!(valid_app_name(&name), "app names are lowercase words joined by -");
        let to = Principal::parse(&self.text("owner")?)?;
        let acts_for = |p: &Principal| self.who.may(self.w, "add-apps", &owner_object(p));
        ensure!(acts_for(&to), "only someone acting for an app's owner makes it");
        let Some(app) = self.who.app(&name) else {
            let from = self.data["from"].as_str();
            self.out.execute("INSERT INTO apps VALUES (?, ?, ?, NULL, 0, NULL, NULL, 0, NULL)", params![name, owner(&to), from])?;
            return Ok(());
        };
        ensure!(!app.erased, "that app was erased");
        ensure!(acts_for(&app.owner), "only someone acting for the app's owner gives it away");
        self.out.execute("UPDATE apps SET owner = ? WHERE name = ?", params![owner(&to), name])?;
        if self.data["erased"] == true {
            ensure!(app.deleted.is_some(), "delete it first");
            self.out.execute("UPDATE apps SET erased = 1 WHERE name = ?", [&name])?;
        } else if let Some(deleted) = self.data["deleted"].as_bool() {
            let by = deleted.then(|| (self.time(), person(self.w)));
            self.out.execute("UPDATE apps SET deleted_at = ?, deleted_by = ? WHERE name = ?", params![by.as_ref().map(|b| b.0), by.as_ref().map(|b| &b.1), name])?;
        }
        if let Some(source) = self.data.get("source") {
            self.out.execute("UPDATE apps SET source = ? WHERE name = ?", params![source_link(source)?, name])?;
        }
        Ok(())
    }

    fn deploy(&self) -> Result<()> {
        let (name, bundle) = (self.text("app")?, self.text("bundle")?);
        let app = self.who.app(&name).context("no such app")?;
        ensure!(self.who.may(self.w, "add-apps", &owner_object(&app.owner)), "only someone acting for the app's owner deploys it");
        ensure!(app.deleted.is_none(), "it was deleted: restore it first");
        ensure!(self.step.content.blobs.contains(&bundle), "a deploy names its bundle among its blobs");
        self.out.execute("UPDATE apps SET bundle = ?, deploys = deploys + 1 WHERE name = ?", params![bundle, name])?;
        self.out.execute(
            "INSERT INTO deploys VALUES (?1, (SELECT count(*) FROM deploys WHERE app = ?1) + 1, ?2, ?3, ?4)",
            params![name, bundle, self.time(), person(self.w)],
        )?;
        Ok(())
    }

    /// A part of a device's oned switched on or off: by the device's person (the Cloudflare node's are its admins'),
    /// and a service run for a scope's app only by who may add apps there, who may switch it off there too.
    fn service(&self) -> Result<()> {
        let device = self.key("device")?;
        let name = self.text("name")?;
        ensure!(valid_app_name(&name), "service names are lowercase words joined by -");
        let on = self.data["on"] == true;
        let scope = self.data["scope"].as_str().map(Scope::parse).transpose()?;
        let adds = |s: &Scope| self.who.may(self.w, "add-apps", &object(s));
        let was = self.who.switch(&device, &name).and_then(|s| s.scope.as_deref().map(Scope::parse)).and_then(Result::ok);
        let managed = !on && was.as_ref().is_some_and(adds);
        if self.who.cloudflare_node() == Some(device) {
            ensure!(managed || self.admin(), "only an admin switches what the Cloudflare node runs");
        } else {
            let theirs = self.who.machine(&device).context("no such device")?.user == self.w;
            ensure!(managed || theirs, "only a device's person switches what it runs");
        }
        ensure!(!on || scope.as_ref().is_none_or(adds), "only who may add apps there runs its service for it");
        let scope = scope.or(was.filter(|_| !on)).map(|s| s.to_string());
        self.out.execute("INSERT OR REPLACE INTO services VALUES (?, ?, ?, ?, ?, ?)", params![hex(&device), name, on, person(self.w), self.time(), scope])?;
        Ok(())
    }
}

/// A step of a scope's own state, in `out`: installs, schedules, Vault requests, pages and the blobs it no longer needs.
fn own(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Step) -> Result<()> {
    let w = step.user();
    let data = &step.content.data;
    let time = int(step.time());
    let manages = || who.may(w, "add-apps", &object(scope));
    match step.content.kind.as_str() {
        "install" | "uninstall" => {
            ensure!(manages(), crate::protocol::Coded::refused("not allowed to install there"));
            let app = data["app"].as_str().context("no app")?;
            if step.content.kind == "install" {
                who.app(app).context("no such app")?;
                out.execute("INSERT OR REPLACE INTO installs VALUES (?, 1, ?, ?)", params![app, person(w), time])?;
            } else {
                out.execute("UPDATE installs SET installed = 0 WHERE app = ?", [app])?;
            }
        }
        "schedule" => {
            ensure!(manages(), crate::protocol::Coded::refused("not allowed to switch schedules there"));
            let app = data["app"].as_str().context("no app")?;
            let name = data["schedule"].as_str().context("no schedule")?;
            let on = data["on"].as_bool().context("on or off?")?;
            out.execute("INSERT OR REPLACE INTO schedules VALUES (?, ?, ?, ?, ?)", params![app, name, on, person(w), time])?;
        }
        // A blob the scope keeps, which its `blobs` names.
        "keep" => {}
        // Only someone who may write there says a blob is no longer needed: its data goes from the bucket.
        "forget" => {
            let blob = data["blob"].as_str().filter(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())).context("no blob")?;
            out.execute("INSERT OR IGNORE INTO forgotten VALUES (?)", [blob])?;
        }
        "vault-request" => {
            ensure!(manages(), crate::protocol::Coded::refused("not allowed to change the Vault there"));
            let id = data["id"].as_str().filter(|id| is_id(id)).context("no id")?;
            let answered: Option<Option<String>> = out.query_row("SELECT answer FROM vault_requests WHERE id = ?", [id], |r| r.get(0)).optional()?;
            // Answered: declined, or set once the item it asked for is in the scope's Vault (`src/node/vault.rs`).
            if let Some(how) = ["declined", "set"].into_iter().find(|how| data[*how] == true) {
                ensure!(answered.context("no such request")?.is_none(), "already answered");
                out.execute("UPDATE vault_requests SET answer = ?, answered_by = ?, answered_at = ? WHERE id = ?", params![how, person(w), time, id])?;
                return Ok(());
            }
            ensure!(answered.is_none(), "that request is already made");
            let app = data["app"].as_str().context("no app")?;
            // An item the node itself uses (a backup place's) is asked for as the node's, and one agents use
            // (vault_fetch) as theirs.
            ensure!(who.app(app).is_some() || app == APP || app == AGENTS, "no such app");
            let fields: Vec<(String, bool)> = data["fields"].as_object().context("no fields")?.iter().map(|(n, f)| (n.clone(), f["secret"] == true)).collect();
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
                params![id, app, title, fields.to_string(), json!(hosts).to_string(), data["why"].as_str().unwrap_or(""), data["need"].as_str(), person(w), data["agent"].as_str(), time],
            )?;
        }
        "arrange" => {
            ensure!(*scope == Scope::User(w.to_string()), "each person arranges pages in their own scope");
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
        _ if *scope == Scope::Global => {}
        other => bail!("{other} isn't a step of a person's or group's own"),
    }
    Ok(())
}

/// Someone new in the network, with their role in it.
fn add_person(out: &Connection, user: &str, name: &str, r: Role) -> Result<()> {
    out.execute("INSERT INTO principals VALUES (?, 'person', ?, ?, 0)", params![person(user), name, pick_handle(out, name, "person")?])?;
    set_role(out, &person(user), NETWORK, Some(r))
}

/// `subject`'s role on `object` (the network or a group), in place of any they held; none takes it.
fn set_role(out: &Connection, subject: &str, object: &str, to: Option<Role>) -> Result<()> {
    out.execute("DELETE FROM relations WHERE subject = ? AND object = ? AND relation IN ('admin', 'member')", params![subject, object])?;
    if let Some(r) = to {
        out.execute("INSERT INTO relations VALUES (?, ?, ?, NULL)", params![subject, role(r), object])?;
    }
    Ok(())
}

/// A person's or group's handle, picked from the name they joined or were made with: `handle_from`, with -2, -3… after
/// it while that's taken (by anyone, removed people too).
fn pick_handle(out: &Connection, name: &str, fallback: &str) -> Result<String> {
    let base = handle_from(name, fallback);
    let numbered = |n: u32| {
        let tail = format!("-{n}");
        format!("{}{tail}", base[..base.len().min(39 - tail.len())].trim_end_matches('-'))
    };
    for h in (1..).map(|n| if n == 1 { base.clone() } else { numbered(n) }) {
        if out.query_row("SELECT 1 FROM principals WHERE handle = ?", [&h], |_| Ok(())).optional()?.is_none() {
            return Ok(h);
        }
    }
    unreachable!()
}

/// A device as it's listed, in place of how it was.
fn put(out: &Connection, d: &Machine) -> Result<()> {
    out.execute(
        "INSERT OR REPLACE INTO devices VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            hex(&d.key),
            hex(&d.seal),
            person(&d.user),
            d.name,
            d.is(Kind::Client),
            d.is(Kind::Compute),
            d.service,
            d.shared,
            d.passkey,
            d.device.as_ref().map(hex),
            person(&d.by),
            int(d.since),
            d.sign_in.as_ref().map(|a| json!(a).to_string()),
            d.place,
            d.removed
        ],
    )?;
    Ok(())
}

/// The `hash` an `epoch` step (or `epoch`, a new network's first, `content::first`) names its key by: `keys::epoch_hash`, 64 hex characters.
fn epoch_hash_of(hash: &Value) -> Result<String> {
    let hash = hash.as_str().context("an epoch names its key's hash")?;
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "an epoch's hash is 64 hex characters");
    Ok(hash.to_string())
}

/// A device as a `device` step (or a new network's first, `content::first`) lists it, for `user`, by them.
fn listed(data: &Value, user: &str, since: u64) -> Result<Machine> {
    let text = |key: &str| data[key].as_str().with_context(|| format!("no {key}"));
    let roles: BTreeSet<Kind> = serde_json::from_value(data["roles"].clone()).context("roles are client, compute or service")?;
    ensure!(!roles.is_empty(), "an endpoint does something: client, compute or service");
    let name = text("name")?.trim().to_string();
    ensure!(!name.is_empty(), "an endpoint has a name");
    let service = data["service"].as_str().map(str::to_string);
    ensure!(service.as_deref().is_none_or(valid_app_name), "a service's name is lowercase words joined by -");
    ensure!(service.is_some() == roles.contains(&Kind::Service), "a service endpoint names the service it offers, and only it does");
    Ok(Machine {
        key: unhex(text("key")?)?,
        seal: unhex(text("seal")?)?,
        user: user.to_string(),
        name,
        roles,
        passkey: data["passkey"].as_str().map(str::to_string),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::SigningKey;
    use serde_json::{Value, json};

    use crate::database::scopes::Scope;
    use crate::keys::{Endpoint, Hash, Key, hex};
    use crate::database::content::*;
    use crate::database::scopes::{APPS_PART, Kind, Role};

    use super::super::scopes::ScopeDb;
    use super::super::steps;

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const BOTH: &[Kind] = &[Kind::Client, Kind::Compute];
    const H: &str = "e";

    /// A device, writing steps.
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
            let mut owed = self.db.owed();
            owed.sort();
            owed
        }
    }

    /// Andre's network, his mac a client and node.
    fn setup() -> (Dev, Net) {
        let mac = Dev::new(1);
        let net = Net::new(&mac, BOTH, &[7; 32]);
        (mac, net)
    }

    /// What a device signed in with is recorded with its listing, for its person's next device to be checked against;
    /// a device taken out takes its record with it, and a shape this release doesn't know records nothing.
    #[test]
    fn a_devices_sign_in_is_its_persons_while_it_is_listed() {
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
        assert_eq!(net.db.machine(&phone.id()).unwrap().sign_in, serde_json::from_value(google.clone()).ok());
        assert_eq!(net.db.machine(&laptop.id()).unwrap().sign_in, None);
        assert_eq!(net.db.sign_ins(ANDRE), vec![serde_json::from_value(google).unwrap()]);
        net.step(&mac, ANDRE, remove_device(&phone.id()));
        assert!(net.db.machine(&phone.id()).is_none() && net.db.sign_ins(ANDRE).is_empty(), "taken out, with what it signed in with");
    }

    /// What a device signed in with is forgotten only by its person's own device; and a device they said isn't theirs
    /// ("Not Me") is never listed after, by them or anyone, and nothing it writes counts.
    #[test]
    fn a_sign_in_is_forgotten_and_a_device_turned_down_stays_out() {
        let (mac, mut net) = setup();
        let google = json!({ "provider": { "issuer": "https://accounts.google.com", "audience": "ours", "subject": "1" } });
        let (phone, bobs) = (Dev::new(3), Dev::new(5));
        let mut c = device(&phone.endpoint(), ANDRE, "phone", &[Kind::Client], None);
        c.data["sign_in"] = google;
        net.step(&mac, ANDRE, c);
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        assert!(!net.step(&bobs, BOB, forget_sign_in(&phone.id())), "only its person forgets it");
        assert!(net.step(&phone, ANDRE, forget_sign_in(&phone.id())));
        assert_eq!(net.db.machine(&phone.id()).unwrap().sign_in, None);

        let stranger = Dev::new(9);
        assert!(net.step(&mac, ANDRE, refuse(&stranger.endpoint(), "MacBook Pro")));
        assert!(!net.step(&phone, ANDRE, device(&stranger.endpoint(), ANDRE, "MacBook Pro", &[Kind::Client], None)), "turned down, it's never listed");
        assert!(!net.step(&stranger, ANDRE, setting("sneaky", Some("1"))), "and what it writes doesn't count");
        assert!(net.db.machine(&stranger.id()).is_none() && net.db.machine_ever(&stranger.id()).unwrap().removed);
        // One listed already is taken out instead, as Remove does.
        assert!(!net.step(&mac, ANDRE, refuse(&phone.endpoint(), "phone")));
    }

    /// A new device's sign-in is checked only against what a person's own devices recorded of their own clients: not a
    /// machine an admin listed for them, nor their first device, which the inviting admin let in.
    #[test]
    fn only_a_persons_own_clients_say_what_they_sign_in_with() {
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
        assert_eq!(net.db.sign_ins(BOB), vec![serde_json::from_value(account("bobs-own")).unwrap()]);
        assert_eq!(net.db.seal_key(&mac.id()), Some(mac.endpoint().seal), "the seal key it was listed with");
    }

    #[test]
    fn a_service_runs_for_a_scope_its_adder_may_add_to_and_its_scopes_admins_may_stop_it() {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's computer", BOTH, Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
        let (linux_id, bobs_id) = (linux.id(), bobs.id());
        let whatsapp = |db: &ScopeDb, k: &Key| db.switch(k, "whatsapp").map(|s| (s.on, s.scope));
        // Bob runs it on his own node for himself, not for everyone; Andre runs it on his for everyone.
        assert!(!net.step(&bobs, BOB, service(&bobs_id, "whatsapp", true, Some(&Scope::Global))));
        net.step(&bobs, BOB, service(&bobs_id, "whatsapp", true, Some(&Scope::User(BOB.into()))));
        net.step(&linux, ANDRE, service(&linux_id, "whatsapp", true, Some(&Scope::Global)));
        // Bob may not stop everyone's on Andre's node, nor Andre Bob's own on Bob's.
        assert!(!net.step(&bobs, BOB, service(&linux_id, "whatsapp", false, None)));
        assert!(!net.step(&mac, ANDRE, service(&bobs_id, "whatsapp", false, None)));
        assert_eq!(whatsapp(&net.db, &linux_id), Some((true, Some("global".into()))));
        assert_eq!(whatsapp(&net.db, &bobs_id), Some((true, Some(format!("user:{BOB}")))));
        // Once Bob is an admin, he stops everyone's on Andre's node (moving it, or removing the app), and it stays named.
        net.step(&mac, ANDRE, role(BOB, Role::Admin));
        assert!(net.step(&bobs, BOB, service(&linux_id, "whatsapp", false, None)));
        assert_eq!(whatsapp(&net.db, &linux_id), Some((false, Some("global".into()))));
    }

    #[test]
    fn only_a_devices_person_switches_its_parts_and_lists_them_as_its_own() {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", BOTH, None));
        // Bob, even with Andre's listing, switches nothing on Andre's linux; Andre's own linux does.
        let (linux_id, bobs_id, mac_id) = (linux.id(), bobs.id(), mac.id());
        assert!(!net.step(&bobs, BOB, service(&linux_id, "whatsapp", true, None)));
        assert!(net.db.switches().is_empty());
        net.step(&linux, ANDRE, service(&linux_id, "whatsapp", true, None));
        net.step(&mac, ANDRE, service(&linux_id, APPS_PART, false, None));
        assert!(net.db.switch(&linux_id, "whatsapp").unwrap().on && !net.db.switch(&linux_id, APPS_PART).unwrap().on);
        // linux lists its part; it lists it again naming itself, which an old listing lacked; Bob's device can't.
        let part = Dev::new(8);
        let listed = |device: &Key| service_endpoint(&part.endpoint(), ANDRE, "linux", "whatsapp", false, device);
        net.step(&linux, ANDRE, listed(&mac_id));
        assert!(net.step(&linux, ANDRE, listed(&linux_id)));
        assert!(!net.step(&bobs, BOB, service_endpoint(&part.endpoint(), BOB, "linux", "whatsapp", false, &bobs_id)));
        let p = net.db.machine(&part.id()).unwrap();
        assert_eq!((p.user.as_str(), p.device), (ANDRE, Some(linux_id)));
        // Nor is anything but a service listed again by its person.
        assert!(!net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None)));
        assert_eq!(net.db.machine(&linux_id).unwrap().roles.len(), 2);
    }

    #[test]
    fn a_step_counts_only_when_signed_by_a_device_of_its_person() {
        let (mac, mut net) = setup();
        let (bobs, linux) = (Dev::new(3), Dev::new(4));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        // Andre's device can't sign as Bob, Bob's can't as Andre, and a node signs no change to who's who.
        assert!(!net.step(&mac, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(!net.step(&bobs, ANDRE, node("setting", json!({ "name": "a", "value": "1" }))));
        assert!(!net.step(&linux, ANDRE, node("setting", json!({ "name": "b", "value": "1" }))));
        assert_eq!(net.db.user(BOB).unwrap().name, "Bob");
        assert!(net.db.settings().is_empty());
        // Bob's own device renames him; it can't list a device for Andre, nor Andre's list a client for Bob.
        assert!(net.step(&bobs, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(!net.step(&bobs, BOB, device(&Dev::new(5).endpoint(), ANDRE, "x", &[Kind::Client], None)));
        assert!(!net.step(&mac, ANDRE, device(&Dev::new(6).endpoint(), BOB, "y", &[Kind::Client], None)));
        assert_eq!(net.db.user(BOB).unwrap().name, "Bobby");
        // The node runs Andre's scope and the network's, not Bob's.
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
    }

    #[test]
    fn a_new_epoch_counts_only_from_the_scopes_admins_and_names_its_key() {
        let (mac, mut net) = setup();
        let bobs = Dev::new(3);
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        assert_eq!((net.db.epoch(&Scope::Global), net.db.sealed_to(&Scope::Global)), (1, vec![mac.id()]), "global's first is its first device's");
        let to = json!([hex(&mac.id())]);
        let epoch = |scope: &str, n: u32, hash: Option<String>| node("epoch", json!({ "scope": scope, "n": n, "to": to, "hash": hash }));
        assert!(!net.step(&bobs, BOB, epoch("global", 2, Some(H.repeat(64)))), "not Bob's to start");
        assert!(!net.step(&bobs, BOB, epoch(&format!("user:{ANDRE}"), 1, Some(H.repeat(64)))));
        assert!(!net.step(&mac, ANDRE, epoch("global", 3, Some(H.repeat(64)))), "epochs go one at a time");
        assert!(!net.step(&mac, ANDRE, epoch("global", 2, None)), "each naming its key");
        assert!(net.step(&mac, ANDRE, epoch("global", 2, Some("f".repeat(64)))));
        let bob = hex(&bobs.id());
        assert!(net.step(&bobs, BOB, node("epoch", json!({ "scope": format!("user:{BOB}"), "n": 1, "to": [bob], "hash": H.repeat(64) }))), "a person's own scope is theirs");
        assert_eq!(net.db.epoch_hash(&Scope::Global, 2), Some("f".repeat(64)));
        assert_eq!(net.db.epoch_hash(&Scope::Global, 1), Some(H.repeat(64)), "each epoch's hash is kept");
        assert_eq!((net.db.epoch(&Scope::User(BOB.into())), net.db.epoch(&Scope::User(ANDRE.into()))), (1, 0));
    }

    /// A network whose recovery key is `recovery`'s, with Andre's mac and linux (a node), and Bob in with his phone.
    fn with_bob(recovery: &Dev) -> (Dev, Dev, Dev, Net) {
        let (mac, linux, bobs) = (Dev::new(1), Dev::new(4), Dev::new(3));
        let mut net = Net::new(&mac, BOTH, &recovery.id());
        net.step(&mac, ANDRE, device(&linux.endpoint(), ANDRE, "linux", &[Kind::Compute], None));
        net.step(&mac, ANDRE, device(&bobs.endpoint(), BOB, "bob's phone", &[Kind::Client], Some(("Bob", Role::Member))));
        (mac, linux, bobs, net)
    }

    #[test]
    fn only_a_persons_own_device_makes_their_scopes_keys_and_says_where_it_runs() {
        let (mac, linux, bobs, mut net) = with_bob(&Dev::new(9));
        let bob = Scope::User(BOB.into());
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        // Nothing runs Bob's own things until his device says: then the network's machine he chose does.
        assert!(!net.db.runs_for(&linux.id(), &bob));
        net.step(&mac, ANDRE, runs(&[linux.id()]));
        assert!(!net.step(&mac, BOB, runs(&[linux.id()])), "an admin's device can't say where Bob's things run");
        assert!(net.step(&bobs, BOB, runs(&[linux.id()])));
        assert!(net.db.runs_for(&linux.id(), &bob) && net.db.serves(&linux.id(), BOB));
        assert!(net.db.runs_for(&linux.id(), &Scope::User(ANDRE.into())), "Andre's says nothing of his own: his machines run it");
        // Only machines that run things: his phone isn't one.
        assert!(!net.step(&bobs, BOB, runs(&[linux.id(), bob_id])), "a list naming what can't run things doesn't count");
        assert!(net.step(&bobs, BOB, runs(&[])));
        assert!(!net.db.runs_for(&linux.id(), &bob), "nothing runs them now");
        // His scope's keys: an admin's device can't make them, his own can, sealed only to his devices.
        assert!(!net.step(&mac, ANDRE, epoch(&bob, 1, &[mac_id], &H.repeat(64))));
        assert!(!net.step(&mac, BOB, epoch(&bob, 1, &[mac_id], &H.repeat(64))));
        assert!(net.step(&bobs, BOB, epoch(&bob, 1, &[bob_id], &"b".repeat(64))));
        assert_eq!(net.db.epoch_hash(&bob, 1), Some("b".repeat(64)));
        assert_eq!(net.db.openers(&bob).iter().map(|d| d.key).collect::<Vec<_>>(), vec![bobs.id()], "only Bob's own devices open it");
    }

    #[test]
    fn a_removed_persons_devices_get_no_new_epoch_and_their_steps_stop() {
        let (mac, _, bobs, mut net) = with_bob(&Dev::new(9));
        let (mac_id, bob_id) = (mac.id(), bobs.id());
        assert!(net.step(&mac, ANDRE, remove_user(BOB)));
        // A new epoch of everyone's can't be sealed to Bob's phone, and what his phone writes after doesn't count.
        assert!(!net.step(&mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id, bob_id], &"c".repeat(64))), "an epoch sealed to a removed person's device doesn't count");
        assert!(!net.step(&bobs, BOB, node("rename", json!({ "user": BOB, "name": "Bobby" }))));
        assert!(net.db.client(&bobs.id()).is_none() && net.db.clients().iter().all(|c| c.user != BOB));
        assert_eq!(net.db.user_ever(BOB).unwrap().name, "Bob");
        assert!(net.step(&mac, ANDRE, epoch(&Scope::Global, 2, &[mac_id], &"c".repeat(64))), "sealed to those left, it does");
        assert!(net.db.scopes().contains(&Scope::User(BOB.into())), "their own scope is kept");
    }

    /// The network's last admin can't be taken out or step down, nor be made a member by another; once there's
    /// another, they can.
    #[test]
    fn the_last_admin_stays() {
        let (mac, _, bobs, mut net) = with_bob(&Dev::new(9));
        assert!(!net.step(&mac, ANDRE, role(ANDRE, Role::Member)), "the last admin doesn't step down");
        assert!(!net.step(&mac, ANDRE, remove_user(ANDRE)), "nor take themselves out");
        assert!(!net.step(&bobs, BOB, remove_user(ANDRE)) && !net.step(&bobs, BOB, role(ANDRE, Role::Member)), "nor does a member");
        assert!(!net.step(&bobs, BOB, role(BOB, Role::Admin)), "and no one gives themselves more");
        assert!(net.step(&mac, ANDRE, role(ANDRE, Role::Admin)), "though they may say again they're admin");
        assert!(net.step(&mac, ANDRE, role(BOB, Role::Admin)));
        assert!(net.step(&bobs, BOB, role(ANDRE, Role::Member)), "once there's another, they can");
        assert!(!net.step(&bobs, BOB, remove_user(BOB)), "and the last again stays");
        assert_eq!(net.db.user(ANDRE).unwrap().role, Role::Member);
    }

    #[test]
    fn a_scope_owes_a_new_epoch_once_someone_holding_its_keys_loses_the_right_until_one_counts() {
        let (mac, linux, bobs, mut net) = with_bob(&Dev::new(9));
        let (mac_id, bob_id, linux_id) = (mac.id(), bobs.id(), linux.id());
        let (bob, work) = (Scope::User(BOB.into()), Scope::Group("c000000000000003".into()));
        net.step(&bobs, BOB, epoch(&bob, 1, &[bob_id], &H.repeat(64)));
        net.step(&mac, ANDRE, group("c000000000000003", "Work"));
        net.step(&mac, ANDRE, epoch(&work, 1, &[mac_id], &H.repeat(64)));
        net.step(&mac, ANDRE, invite("c000000000000003", BOB, Role::Member));
        net.step(&bobs, BOB, accept("c000000000000003"));
        // Apps of the group's, the admins' and Bob's, and a Cloudflare node, which change nothing owed.
        net.step(&mac, ANDRE, node("app", json!({ "name": "notes", "owner": "group:c000000000000003" })));
        net.step(&mac, ANDRE, node("app", json!({ "name": "tally", "owner": "admins" })));
        net.step(&bobs, BOB, node("app", json!({ "name": "bobs", "owner": format!("user:{BOB}") })));
        net.step(&mac, ANDRE, setting(crate::database::scopes::CLOUDFLARE_NODE, Some(&hex(&Dev::new(14).id()))));
        let (laptop, tablet) = (Dev::new(5), Dev::new(6));
        net.step(&bobs, BOB, device(&laptop.endpoint(), BOB, "laptop", &[Kind::Client], None));
        net.step(&bobs, BOB, device(&tablet.endpoint(), BOB, "tablet", &[Kind::Client], None));
        assert!(net.owed().is_empty());
        // Signing itself out, a device deletes its own keys: nothing's owed.
        assert!(net.step(&laptop, BOB, remove_device(&laptop.id())));
        assert!(net.owed().is_empty() && net.db.machine(&laptop.id()).is_none(), "signing out owes nothing");
        // Nor one a node of its person's wrote for it, saying it signed itself out; an admin's saying so for another's
        // device is taken as removing it.
        let (asked, said) = (Dev::new(12), Dev::new(13));
        net.step(&bobs, BOB, device(&asked.endpoint(), BOB, "app", &[Kind::Client], None));
        net.step(&bobs, BOB, device(&said.endpoint(), BOB, "app", &[Kind::Client], None));
        let gone = |k: &Dev| node("remove", json!({ "key": hex(&k.id()), "itself": true }));
        assert!(net.step(&bobs, BOB, gone(&asked)));
        assert!(net.owed().is_empty() && net.db.machine(&asked.id()).is_none(), "its own node signing it out owes nothing");
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
    }

    /// Who may write each kind of step, as the rules say it: each step below, alone on one network, signed by each of
    /// its people's devices and its nodes in turn, counts (Y) or doesn't (.). The network: Andre and Carol its admins,
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
            (&mac, ANDRE, setting(crate::database::scopes::CLOUDFLARE_NODE, Some(&hex(&cloudflare.id())))),
            (&phone, BOB, group(G, "G")),
            (&phone, BOB, invite(G, DAN, Role::Member)),
            (&dans, DAN, accept(G)),
            (&phone, BOB, invite(G, CAROL, Role::Member)),
            (&phone, BOB, epoch(&bob, 1, &[phone.id()], &H.repeat(64))),
            (&mac, ANDRE, epoch(&andre, 1, &[mac.id()], &H.repeat(64))),
            (&phone, BOB, epoch(&g, 1, &[phone.id()], &H.repeat(64))),
            (&phone, BOB, node("app", json!({ "name": "notes", "owner": format!("user:{BOB}") }))),
            (&mac, ANDRE, node("app", json!({ "name": "tally", "owner": "admins" }))),
            (&mac, ANDRE, node("app", json!({ "name": "old", "owner": "admins" }))),
            (&mac, ANDRE, retire("old", "admins", "deleted").unwrap()),
            (&phone, BOB, node("app", json!({ "name": "band", "owner": format!("group:{G}") }))),
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
        let deploy = |app: &str| Content { blobs: vec![bundle.clone()], ..node("deploy", json!({ "app": app, "bundle": bundle })) };
        let request = node("vault-request", json!({ "id": "e000000000000008", "app": "notes", "title": "Token", "fields": { "token": { "secret": true } }, "hosts": ["api.example.com"] }));
        let global = [
            ("someone new, a member", device(&gus.endpoint(), GUS, "gus's phone", &[Kind::Client], Some(("Gus", Role::Member)))),
            ("someone new, an admin", device(&gus.endpoint(), GUS, "gus's phone", &[Kind::Client], Some(("Gus", Role::Admin)))),
            ("bob's new client", device(&gus.endpoint(), BOB, "bob's laptop", &[Kind::Client], None)),
            ("bob's new compute", device(&gus.endpoint(), BOB, "bob's server", &[Kind::Compute], None)),
            ("bob's new service", service_endpoint(&part.endpoint(), BOB, "box", "whatsapp", false, &box_.id())),
            ("andre's new client", device(&gus.endpoint(), ANDRE, "andre's laptop", &[Kind::Client], None)),
            ("fay's new client", device(&gus.endpoint(), FAY, "fay's laptop", &[Kind::Client], None)),
            ("bob's box listed again", device(&box_.endpoint(), BOB, "bob's box", &[Kind::Compute], None)),
            ("bob's phone listed again", device(&phone.endpoint(), BOB, "bob's phone", &[Kind::Client], None)),
            ("refuse a stranger", refuse(&stranger.endpoint(), "a stranger")),
            ("forget bob's phone's sign-in", forget_sign_in(&phone.id())),
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
            ("rename the cloudflare node", rename("key", &k(&cloudflare), "edge")),
            ("rename bob", rename("user", BOB, "Robert")),
            ("rename andre", rename("user", ANDRE, "Andrew")),
            ("rename g", rename("group", G, "Band")),
            ("bob's handle", handle("user", BOB, "robert")),
            ("andre's handle", handle("user", ANDRE, "andrew")),
            ("g's handle", handle("group", G, "band")),
            ("a setting", setting("name", Some("Ours"))),
            ("a new recovery key", node("recovery", json!({ "key": k(&stranger) }))),
            ("an app of bob's", node("app", json!({ "name": "bobs", "owner": format!("user:{BOB}") }))),
            ("an app of andre's", node("app", json!({ "name": "andres", "owner": format!("user:{ANDRE}") }))),
            ("an app of the admins'", node("app", json!({ "name": "admins", "owner": "admins" }))),
            ("an app of g's", node("app", json!({ "name": "g", "owner": format!("group:{G}") }))),
            ("delete bob's notes", retire("notes", &format!("user:{BOB}"), "deleted").unwrap()),
            ("delete the admins' tally", retire("tally", "admins", "deleted").unwrap()),
            ("delete g's band", retire("band", &format!("group:{G}"), "deleted").unwrap()),
            ("restore the admins' old", retire("old", "admins", "restored").unwrap()),
            ("erase the admins' old", retire("old", "admins", "erased").unwrap()),
            ("erase the admins' tally", retire("tally", "admins", "erased").unwrap()),
            ("give tally to bob", node("app", json!({ "name": "tally", "owner": format!("user:{BOB}") }))),
            ("give notes to the admins", node("app", json!({ "name": "notes", "owner": "admins" }))),
            ("tally's source", node("app", json!({ "name": "tally", "owner": "admins", "source": "https://example.com/tally.git" }))),
            ("deploy notes", deploy("notes")),
            ("deploy tally", deploy("tally")),
            ("deploy band", deploy("band")),
            ("deploy old (deleted)", deploy("old")),
            ("linux's whatsapp off", service(&linux.id(), "whatsapp", false, None)),
            ("linux's whatsapp on, global", service(&linux.id(), "whatsapp", true, Some(&Scope::Global))),
            ("linux's whatsapp on, bob's", service(&linux.id(), "whatsapp", true, Some(&bob))),
            ("linux's apps off", service(&linux.id(), crate::database::scopes::APPS_PART, false, None)),
            ("bob's box's whatsapp off", service(&box_.id(), "whatsapp", false, None)),
            ("bob's box's whatsapp on, global", service(&box_.id(), "whatsapp", true, Some(&Scope::Global))),
            ("bob's box's notes on, bob's", service(&box_.id(), "notes", true, Some(&bob))),
            ("the cloudflare node's notes on", service(&cloudflare.id(), "notes", true, Some(&Scope::Global))),
            ("the cloudflare node's notes off", service(&cloudflare.id(), "notes", false, None)),
            ("a node release", node("node-release", json!({ "version": "9.9.9" }))),
            ("an unknown kind", node("frobnicate", json!({}))),
            ("an app's own", Content::new("notes", "note", json!({ "text": "hi" }))),
        ];
        let own = |page: &str| {
            [
                ("install notes", install("notes", true)),
                ("uninstall notes", install("notes", false)),
                ("schedule", node("schedule", json!({ "app": "notes", "schedule": "daily", "on": true }))),
                ("vault request", request.clone()),
                ("vault request declined", node("vault-request", json!({ "id": "e000000000000008", "declined": true }))),
                ("keep", node("keep", json!({}))),
                ("forget", node("forget", json!({ "blob": "c".repeat(64) }))),
                ("arrange", node("arrange", json!({ "page": page, "items": [{ "app": "notes" }] }))),
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
    fn the_recovery_key_lets_someones_device_in_as_an_admins_approval_and_is_listed() {
        let recovery = Dev::new(9);
        let (mac, _, _, mut net) = with_bob(&recovery);
        let laptop = Dev::new(5);
        assert!(net.db.recovery_used().is_none());
        assert!(net.step(&recovery, BOB, device(&laptop.endpoint(), BOB, "bob's laptop", &[Kind::Client], None)));
        assert_eq!(net.db.client(&laptop.id()).map(|c| c.user), Some(BOB.to_string()), "it counts as an admin's approval");
        assert_eq!(net.db.recovery_used(), Some((net.at, BOB.to_string())), "and it's listed");
        // Only a device people use, of someone already in; and nothing else.
        assert!(!net.step(&recovery, BOB, device(&Dev::new(6).endpoint(), BOB, "box", &[Kind::Compute], None)));
        assert!(!net.step(&recovery, "c000000000000003", device(&Dev::new(7).endpoint(), "c000000000000003", "new", &[Kind::Client], Some(("Cy", Role::Admin)))));
        assert!(!net.step(&recovery, ANDRE, node("setting", json!({ "name": "a", "value": "1" }))));
        assert!(!net.step(&recovery, ANDRE, role(BOB, Role::Admin)));
        assert!(net.db.user("c000000000000003").is_none() && net.db.user(BOB).unwrap().role == Role::Member);
        // A key that isn't the network's recovery key lets nobody in.
        let other = Dev::new(8);
        assert!(!net.step(&other, BOB, device(&Dev::new(10).endpoint(), BOB, "x", &[Kind::Client], None)));
        // Only the recovery key hands over to a new one: an admin's device can't, and the old key counts no more.
        assert!(!net.step(&mac, ANDRE, node("recovery", json!({ "key": hex(&other.id()) }))), "not by an admin's device");
        assert!(net.step(&recovery, ANDRE, node("recovery", json!({ "key": hex(&other.id()) }))));
        assert_eq!((net.db.recovery(), net.db.recovery_set(), net.db.recovery_was()), (Some(other.id()), net.at, vec![recovery.id()]));
        assert!(!net.step(&recovery, BOB, device(&Dev::new(11).endpoint(), BOB, "y", &[Kind::Client], None)), "the old recovery key lets nobody in");
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
        global   global's epoch 2                   YY..........
        global   bob's epoch 2                      ..Y.........
        global   andre's epoch 2                    Y...........
        global   g's epoch 2                        YYY.........
        global   runs on linux                      YYYY........
        global   app schedules, bob's               ..Y.........
        global   app schedules, global's            YY..........
        global   app schedules, g's                 ..Y.........
        global   vault, global's                    YY..........
        global   vault, bob's                       ..Y.........
        global   vault, g's                         ..Y.........
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
        global   rename the cloudflare node         ............
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
        global   the cloudflare node's notes on     YY..........
        global   the cloudflare node's notes off    YY..........
        global   a node release                     YYYY........
        global   an unknown kind                    ............
        global   an app's own                       YYYYYYYY....
        global   install notes                      YY..........
        global   uninstall notes                    YY..........
        global   schedule                           YY..Y.......
        global   vault request                      YY..Y.......
        global   vault request declined             ............
        global   keep                               YYYYYYYY....
        global   forget                             YYYYYYYY....
        global   arrange                            ............
        global   an app's own                       YYYYYYYY....
        global   a setting                          YY..........
        andre's  install notes                      Y...Y.......
        andre's  uninstall notes                    Y...Y.......
        andre's  schedule                           Y...Y.......
        andre's  vault request                      Y...Y.......
        andre's  vault request declined             ............
        andre's  keep                               Y...Y.......
        andre's  forget                             Y...Y.......
        andre's  arrange                            Y...Y.......
        andre's  an app's own                       Y...Y.......
        andre's  a setting                          ............
        bob's    install notes                      ..Y..Y......
        bob's    uninstall notes                    ..Y..Y......
        bob's    schedule                           ..Y..Y......
        bob's    vault request                      ..Y..Y......
        bob's    vault request declined             ............
        bob's    keep                               ..Y..Y......
        bob's    forget                             ..Y..Y......
        bob's    arrange                            ..Y..Y......
        bob's    an app's own                       ..Y..Y......
        bob's    a setting                          ............
        g's      install notes                      ..Y..Y.Y....
        g's      uninstall notes                    ..Y..Y.Y....
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
