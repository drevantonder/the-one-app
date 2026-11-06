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

use crate::entry::{Entry, Key, Scope, hex, is_id, unhex};
use crate::members::{self, AGENTS, APP, APP_SCHEDULES, Kind, Machine, Principal, Role, handle_from, source_link, valid_app_name, valid_handle, valid_host, valid_page, valid_secret_name};

use super::scope::{group, int, owner, person, role};
use super::scopes::{NETWORK, ScopeDb, object, owner_object};

/// Kinds of step that change only their own scope's state, wherever they're written.
const OWN: &[&str] = &["schedule", "vault-request", "keep", "forget", "arrange"];

/// Whether a step is one of `global`'s own: who's who, written only by its person's own device (or the recovery key).
fn of_global(step: &Entry, scope: &Scope) -> bool {
    *scope == Scope::Global && step.content.app == APP && !OWN.contains(&step.content.kind.as_str())
}

/// Whether `step`'s signer may write it as its person, in `scope`, by `who`, `global`'s database before it.
pub fn signed(who: &ScopeDb, scope: &Scope, step: &Entry) -> Result<()> {
    let (w, author) = (step.user(), step.author());
    if who.recovery() == Some(author) {
        // The network's recovery key, kept offline by an admin, only lets a device of someone's in, as an admin's would,
        // and hands over to the next recovery key.
        ensure!(of_global(step, scope) && matches!(step.content.kind.as_str(), "device" | "recovery") && who.user(w).is_some(), "the recovery key only lets a device in");
        return Ok(());
    }
    ensure!(who.may(w, "use", &object(scope)), "{w} can't write in {scope}");
    let theirs = who.client(&author).is_some_and(|c| c.user == w);
    ensure!(theirs || (!of_global(step, scope) && who.runs_for(&author, scope)), "{} can't write in {scope} for that person", crate::entry::short(&author));
    Ok(())
}

/// Applies `step`, which counts by `signed` and `steps::allowed`, to `out`, `scope`'s database before it, by its kind's
/// rule, `who` being `global`'s database before it: → why it doesn't count, if it doesn't.
pub fn apply(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Entry) -> Result<()> {
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
        k if members::RELEASES.contains(&k) => Ok(()),
        other => bail!("unknown kind {other}"),
    }
}

/// The network's first person, device, epoch and recovery key, as its genesis says them, into `out`, empty.
pub fn genesis(out: &Connection, genesis: &Entry) -> Result<()> {
    let data = &genesis.content.data;
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let user = text("user")?;
    ensure!(is_id(&user) && user == genesis.user(), "genesis is written as its first user");
    let machine = listed(&data["device"], &user, genesis.time())?;
    ensure!(machine.key == genesis.author() && machine.is(Kind::Client), "genesis is signed by the first device it lists, a client");
    let hash = epoch_hash_of(&data["epoch"])?;
    let time = int(genesis.time());
    out.execute("UPDATE network SET root = ?, name = ?, devices_at = ?", params![genesis.hash().to_hex(), text("name")?, time])?;
    out.execute("INSERT INTO recovery VALUES (0, ?, ?)", params![hex(&unhex(&text("recovery")?)?), time])?;
    add_person(out, &user, &text("user_name")?, Role::Admin)?;
    put(out, &machine)?;
    // `global`'s first epoch, made with the network, before anything is sealed under it.
    out.execute("INSERT INTO epochs VALUES ('global', 1, ?)", [&hash])?;
    out.execute("INSERT INTO sealed VALUES ('global', 1, ?)", [hex(&genesis.author())])?;
    Ok(())
}

/// One of `global`'s own steps, being applied.
struct Change<'a> {
    who: &'a ScopeDb,
    out: &'a Connection,
    step: &'a Entry,
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
                    ensure!(user == self.w || self.admin(), "not allowed to change that person's handle");
                    self.who.user(&user).context("no such user")?;
                    Principal::User(user)
                }
                (_, Ok(g)) => {
                    ensure!(self.manages(&g), "not allowed to change that group's handle");
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
            ensure!(self.who.may(self.w, "remove", &super::scopes::device(&key)), "not allowed to rename that");
            self.out.execute("UPDATE devices SET name = ? WHERE key = ?", params![name, hex(&key)])?;
        } else if let Ok(user) = self.id("user") {
            ensure!(user == self.w || self.admin(), "not allowed to rename that user");
            self.who.user(&user).context("no such user")?;
            self.out.execute("UPDATE principals SET name = ? WHERE id = ?", params![name, person(&user)])?;
        } else {
            let g = self.id("group")?;
            ensure!(self.manages(&g), "not allowed to rename that group");
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
fn own(who: &ScopeDb, out: &Connection, scope: &Scope, step: &Entry) -> Result<()> {
    let w = step.user();
    let data = &step.content.data;
    let time = int(step.time());
    let manages = || who.may(w, "add-apps", &object(scope));
    match step.content.kind.as_str() {
        "install" | "uninstall" => {
            ensure!(manages(), "not allowed to install there");
            let app = data["app"].as_str().context("no app")?;
            if step.content.kind == "install" {
                who.app(app).context("no such app")?;
                out.execute("INSERT OR REPLACE INTO installs VALUES (?, 1, ?, ?)", params![app, person(w), time])?;
            } else {
                out.execute("UPDATE installs SET installed = 0 WHERE app = ?", [app])?;
            }
        }
        "schedule" => {
            ensure!(manages(), "not allowed to switch schedules there");
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
            ensure!(manages(), "not allowed to change the Vault there");
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

/// The `hash` an `epoch` step (or `epoch`, the genesis) names its key by: `keys::epoch_hash`, 64 hex characters.
fn epoch_hash_of(hash: &Value) -> Result<String> {
    let hash = hash.as_str().context("an epoch names its key's hash")?;
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "an epoch's hash is 64 hex characters");
    Ok(hash.to_string())
}

/// A device as a `device` step (or the genesis's `device`) lists it, for `user`, by them.
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
