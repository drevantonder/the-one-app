//! What a device asks of a scope database (`scope`): who's who and who may do what, from `global`'s, and what a
//! scope's own steps add up to, from its own, in the shapes it's read in (`User`, `Machine`, `Group`, `App`, …).
//! Every question of who may do what is `permissions::may` over its rows; the rest are its tables as they are.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::keys::{Hash, Key, hex, unhex};

use crate::permissions;

pub use super::scope::{app, device, group, object, user_principal, service};

pub const NETWORK: &str = permissions::NETWORK;

/// One scope database, and the time it was built at, which `may` asks as of. Nothing in it lasts until a time yet but
/// app schedules, which are asked with the time they're asked at (`app_schedules_at`).
pub struct ScopeDb {
    db: Connection,
    now: u64,
}

/// A row of an app's own relations: subject, relation, object, until (ms; None for good), as the engine's.
pub type Relation = (String, String, String, Option<i64>);

/// An app's own relation rows: the engine's columns of its database's `relations`; none if it has no such table.
pub fn app_relations(app: &Connection) -> Vec<Relation> {
    let read = || -> rusqlite::Result<Vec<Relation>> {
        app.prepare("SELECT subject, relation, object, until FROM relations")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect()
    };
    read().unwrap_or_default()
}

fn id(of: &str) -> String {
    of.strip_prefix("user:").or_else(|| of.strip_prefix("group:")).unwrap_or(of).to_string()
}

fn role_of(name: &str) -> Role {
    if name == "admin" { Role::Admin } else { Role::Member }
}

fn key(text: &str) -> Key {
    unhex(text).unwrap_or([0; 32])
}

fn owner_of(text: &str) -> Principal {
    match text.split_once(':') {
        Some(("user", id)) => Principal::User(id.into()),
        Some(("group", id)) => Principal::Group(id.into()),
        _ => Principal::Admins,
    }
}

/// A principal as an object of permissions: the scope it acts for.
pub fn owner_object(p: &Principal) -> String {
    match p {
        Principal::User(u) => user_principal(u),
        Principal::Group(g) => group(g),
        Principal::Admins => NETWORK.into(),
    }
}

const MACHINE: &str = "SELECT d.key, d.seal, d.principal, d.name, d.client, d.compute, d.service, d.shared, d.passkey, d.device, d.listed_by, d.listed_at,
    d.removed OR coalesce(p.removed, 1), d.sign_in, d.place, d.platform FROM devices d LEFT JOIN principals p ON p.id = d.principal";

fn machine(r: &rusqlite::Row) -> rusqlite::Result<Machine> {
    let mut roles = BTreeSet::new();
    let service: Option<String> = r.get(6)?;
    for (on, kind) in [(r.get::<_, bool>(4)?, Kind::Client), (r.get(5)?, Kind::Compute), (service.is_some(), Kind::Service)] {
        if on {
            roles.insert(kind);
        }
    }
    Ok(Machine {
        key: key(&r.get::<_, String>(0)?),
        seal: key(&r.get::<_, String>(1)?),
        user: id(&r.get::<_, String>(2)?),
        name: r.get(3)?,
        roles,
        service,
        shared: r.get(7)?,
        passkey: r.get(8)?,
        device: r.get::<_, Option<String>>(9)?.map(|k| key(&k)),
        by: id(&r.get::<_, String>(10)?),
        since: r.get::<_, i64>(11)? as u64,
        removed: r.get(12)?,
        sign_in: r.get::<_, Option<String>>(13)?.and_then(|a| serde_json::from_str(&a).ok()),
        place: r.get(14)?,
        platform: r.get(15)?,
    })
}

impl ScopeDb {
    pub fn empty(scope: &Scope, now: u64) -> Result<ScopeDb> {
        Self::open(super::scope::empty(scope)?, now)
    }

    /// A scope database in this release's tables (`steps::normalize` does that for a copy): who holds what worked out again.
    pub fn open(db: Connection, now: u64) -> Result<ScopeDb> {
        permissions::close(&db, None)?;
        Ok(ScopeDb { db, now })
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        Ok(self.db.serialize("main")?.to_vec())
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn conn(&self) -> &Connection {
        &self.db
    }

    fn rows<T>(&self, sql: &str, p: impl rusqlite::Params, f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>) -> Result<Vec<T>> {
        Ok(self.db.prepare_cached(sql).and_then(|mut q| q.query_map(p, f)?.collect::<rusqlite::Result<Vec<T>>>()).context("reading the scope database")?)
    }

    fn row<T>(&self, sql: &str, p: impl rusqlite::Params, f: impl FnOnce(&rusqlite::Row) -> rusqlite::Result<T>) -> Result<Option<T>> {
        Ok(self.db.prepare_cached(sql).and_then(|mut q| q.query_row(p, f).optional()).context("reading the scope database")?)
    }

    /// Whether `user` may do `verb` to `object` (`object(scope)`, `group(id)`, `device(key)`, …). Unknown means no, and
    /// so does a database that can't answer.
    pub fn may(&self, user: &str, verb: &str, object: &str) -> bool {
        self.asks(&user_principal(user), verb, object)
    }

    /// Whether the device `key` may do `verb` to `object`: `run` a scope's calls.
    pub fn device_may(&self, key: &Key, verb: &str, object: &str) -> bool {
        self.asks(&device(key), verb, object)
    }

    fn asks(&self, principal: &str, verb: &str, object: &str) -> bool {
        permissions::may(&self.db, principal, verb, object, self.now).unwrap_or_else(|err| {
            tracing::warn!("asking who may {verb} {object}: {err:#}");
            false
        })
    }

    /// Whether `giver` may give `user` the role `relation` on `object`, for good.
    pub fn may_give(&self, giver: &str, user: &str, relation: &str, object: &str) -> bool {
        permissions::may_give(&self.db, &user_principal(giver), &user_principal(user), relation, object, None, self.now).unwrap_or(false)
    }

    /// Whether `taker` may take the role `relation` on `object` from `user`.
    pub fn may_take(&self, taker: &str, user: &str, relation: &str, object: &str) -> bool {
        permissions::may_take(&self.db, &user_principal(taker), &user_principal(user), relation, object, self.now).unwrap_or(false)
    }

    /// Whether `user` may do each verb to each object of `asks`, in order (`ctx.<scope>.may` in an app), by the core's
    /// rows and an app's own, worked out once for them all:
    /// `types` as its code declares them (`{ <type>: { permissions: [verbs], roles: { <role>: [verbs] } } }`), and
    /// `rows` its relations (`app_relations`). An app's types follow the core's rules but never touch the core's: a
    /// type the core has is the core's, so an app's of that name counts for nothing; only its rows on its own types
    /// count, and never a rule (`#…`, `@`, `*`). Asked in a transaction that's rolled back, so the database stays as
    /// its steps left it.
    pub fn app_may(&self, user: &str, asks: &[(String, String)], types: &Value, rows: &[Relation], now: u64) -> Vec<bool> {
        let ask = || -> Result<Vec<bool>> {
            let tx = self.db.unchecked_transaction()?;
            let mut own = BTreeSet::new();
            for (name, t) in types.as_object().into_iter().flatten() {
                let core: bool = tx.query_row("SELECT exists (SELECT 1 FROM types WHERE name = ?1)", [name], |r| r.get(0))?;
                if core || name.is_empty() || name.contains([':', '#', '*', '@']) {
                    continue;
                }
                tx.execute("INSERT INTO types (name, permissions) VALUES (?1, ?2)", params![name, t["permissions"].to_string()])?;
                for (role, verbs) in t["roles"].as_object().into_iter().flatten() {
                    tx.execute("INSERT INTO roles (type, name, permissions, builtin) VALUES (?1, ?2, ?3, 1)", params![name, role, verbs.to_string()])?;
                }
                own.insert(name.as_str());
            }
            for (subject, relation, on, until) in rows {
                let rule = subject.starts_with(['#', '@']) || subject.contains('*') || on.contains('*');
                if !rule && on.split_once(':').is_some_and(|(ty, _)| own.contains(ty)) {
                    tx.execute("INSERT OR IGNORE INTO relations VALUES (?1, ?2, ?3, ?4)", params![subject, relation, on, until])?;
                }
            }
            permissions::close(&tx, Some(&user_principal(user)))?;
            asks.iter().map(|(verb, object)| permissions::may(&tx, &user_principal(user), verb, object, now)).collect()
        };
        ask().unwrap_or_else(|err| {
            tracing::warn!("asking what {user} may do in an app: {err:#}");
            vec![false; asks.len()]
        })
    }

    pub fn runs_for(&self, key: &Key, scope: &Scope) -> bool {
        self.device_may(key, "run", &object(scope))
    }

    /// What `principal` may do `verb` to (`permissions::objects`), to ask of many at once.
    fn objects(&self, principal: &str, verb: &str) -> Result<Vec<String>> {
        permissions::objects(&self.db, principal, verb, self.now).with_context(|| format!("asking what may be {verb}"))
    }

    pub fn may_scopes(&self, user: &str, verb: &str) -> Result<Vec<Scope>> {
        let may = self.objects(&user_principal(user), verb)?;
        Ok(self.scopes()?.into_iter().filter(|s| permissions::allows(&may, &object(s))).collect())
    }

    pub fn run_by(&self, key: &Key) -> Result<Vec<Scope>> {
        let runs = self.objects(&device(key), "run")?;
        Ok(self.scopes()?.into_iter().filter(|s| permissions::allows(&runs, &object(s))).collect())
    }

    /// Whether `key` runs any of `user`'s calls: some scope they may use.
    pub fn serves(&self, key: &Key, user: &str) -> Result<bool> {
        let uses = self.objects(&user_principal(user), "use")?;
        Ok(self.user(user)?.is_some() && self.run_by(key)?.iter().any(|s| permissions::allows(&uses, &object(s))))
    }

    /// The devices a call of `user`'s reaching `scopes` may be passed on to: each runs every one of them, all of which
    /// `user` may use, so none is handed a call, or its keys, that isn't theirs to run.
    pub fn runners(&self, user: &str, scopes: &[Scope]) -> Result<Vec<Machine>> {
        let uses = self.objects(&user_principal(user), "use")?;
        if self.user(user)?.is_none() || !scopes.iter().all(|s| permissions::allows(&uses, &object(s))) {
            return Ok(Vec::new());
        }
        let all: Vec<String> = self.scopes()?.iter().map(object).collect();
        let mut out = Vec::new();
        for m in self.computes()? {
            let runs = self.objects(&device(&m.key), "run")?;
            if all.iter().any(|o| permissions::allows(&uses, o) && permissions::allows(&runs, o)) && scopes.iter().all(|s| permissions::allows(&runs, &object(s))) {
                out.push(m);
            }
        }
        Ok(out)
    }

    /// Whom a scope's epochs are sealed to, besides the recovery key: the client devices of everyone who may use it,
    /// never a machine that only runs calls.
    pub fn openers(&self, scope: &Scope) -> Result<Vec<Machine>> {
        Ok(self.clients()?.into_iter().filter(|c| self.may(&c.user, "use", &object(scope))).collect())
    }

    /// The nodes that run calls, each with the key what's handed it is sealed to: the devices listed as compute, and
    /// the Cloudflare node, whose seal key is its key's twin.
    pub fn nodes(&self) -> Result<Vec<(Key, Key)>> {
        let mut out: Vec<(Key, Key)> = self.computes()?.into_iter().map(|d| (d.key, d.seal)).collect();
        if let Some(k) = self.cloudflare_node()?
            && let Ok(seal) = crate::keys::twin_public(&k)
        {
            out.push((k, seal));
        }
        Ok(out)
    }

    /// Where `user`'s own things run besides their own machines until they choose: others' machines that run
    /// everyone's; nowhere new once they have.
    pub fn runs_until_chosen(&self, user: &str) -> Result<Vec<Key>> {
        if self.runs(user)?.is_some() {
            return Ok(Vec::new());
        }
        Ok(self.computes()?.into_iter().filter(|c| c.user != user && self.runs_for(&c.key, &Scope::Global)).map(|c| c.key).collect())
    }

    pub fn name(&self) -> Result<String> {
        Ok(self.row("SELECT name FROM network", [], |r| r.get(0))?.unwrap_or_default())
    }

    pub fn root(&self) -> Result<Option<Hash>> {
        Ok(self.row("SELECT root FROM network", [], |r| r.get::<_, Option<String>>(0))?.flatten().and_then(|h| Hash::from_hex(&h).ok()))
    }

    pub fn devices_at(&self) -> Result<u64> {
        Ok(self.row("SELECT devices_at FROM network", [], |r| r.get::<_, i64>(0))?.unwrap_or(0) as u64)
    }

    /// When the recovery key last let a device in, and whose.
    pub fn recovery_used(&self) -> Result<Option<(u64, String)>> {
        Ok(self
            .row("SELECT recovery_used_at, recovery_used_by FROM network", [], |r| Ok(r.get::<_, Option<i64>>(0)?.zip(r.get::<_, Option<String>>(1)?)))?
            .flatten()
            .map(|(t, u)| (t as u64, id(&u))))
    }

    pub fn recovery(&self) -> Result<Option<Key>> {
        Ok(self.row("SELECT key FROM recovery WHERE set_at IS NOT NULL ORDER BY n DESC LIMIT 1", [], |r| r.get::<_, String>(0))?.map(|k| key(&k)))
    }

    pub fn recovery_set(&self) -> Result<u64> {
        Ok(self.row("SELECT set_at FROM recovery WHERE set_at IS NOT NULL ORDER BY n DESC LIMIT 1", [], |r| r.get::<_, i64>(0))?.unwrap_or(0) as u64)
    }

    /// The recovery keys before the network's, which the relay lets in no more.
    pub fn recovery_was(&self) -> Result<Vec<Key>> {
        Ok(self.rows("SELECT key FROM recovery WHERE set_at IS NULL ORDER BY n", [], |r| r.get::<_, String>(0))?.iter().map(|k| key(k)).collect())
    }

    /// A device as someone gives it, by its name or the start of its key: the device, not the parts of it named for it.
    pub fn find_machine(&self, name: &str) -> Result<Key> {
        let listed: Vec<Machine> = self.machines()?.into_iter().filter(|m| !m.removed).collect();
        let named: Vec<&Machine> = listed.iter().filter(|m| !name.is_empty() && (m.name.eq_ignore_ascii_case(name) || hex(&m.key).starts_with(name))).collect();
        let part = |d: &Machine| d.device.is_some_and(|of| named.iter().any(|n| n.key == of));
        match named.iter().filter(|d| !part(d)).collect::<Vec<_>>().as_slice() {
            [one] => Ok(one.key),
            [] => bail!("no device called {name}: your devices are {}", listed.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")),
            _ => bail!("several devices match {name}"),
        }
    }

    /// A node on a machine, which runs calls, as someone gives it, by its name or the start of its key.
    pub fn find_node(&self, name: &str) -> Result<Key> {
        let nodes = self.computes()?.into_iter().map(|m| (m.key, m.name)).collect();
        Self::find_among(nodes, name)
    }

    /// A node that runs a call, as someone names it for one (`--device`): a machine's, or the Cloudflare node, whose
    /// name is `cloudflare` wherever a device is named.
    pub fn find_runner(&self, name: &str) -> Result<Key> {
        let mut nodes: Vec<(Key, String)> = self.computes()?.into_iter().map(|m| (m.key, m.name)).collect();
        nodes.extend(self.cloudflare_node()?.map(|k| (k, crate::rank::CLOUDFLARE.to_string())));
        Self::find_among(nodes, name)
    }

    fn find_among(nodes: Vec<(Key, String)>, name: &str) -> Result<Key> {
        match nodes.iter().filter(|(k, n)| n.eq_ignore_ascii_case(name) || hex(k).starts_with(name)).collect::<Vec<_>>().as_slice() {
            [(key, _)] => Ok(*key),
            [] => Err(crate::protocol::Coded::error(crate::protocol::code::NOT_LISTED, format!("no device called {name}: your devices are {}", nodes.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ")))),
            _ => bail!("several devices match {name}"),
        }
    }

    /// A user as someone gives them, by their id or name.
    pub fn find_user(&self, name: &str) -> Result<String> {
        match self.users()?.into_iter().filter(|u| !u.removed && (u.id == name || u.name.eq_ignore_ascii_case(name))).collect::<Vec<_>>().as_slice() {
            [one] => Ok(one.id.clone()),
            [] => bail!("no user called {name}"),
            _ => bail!("several users are called {name}; give their id"),
        }
    }

    /// A group as someone gives it, by its id or name.
    pub fn find_group(&self, name: &str) -> Result<String> {
        match self.groups()?.into_iter().filter(|g| g.id == name || g.name.eq_ignore_ascii_case(name)).collect::<Vec<_>>().as_slice() {
            [one] => Ok(one.id.clone()),
            [] => bail!("no group called {name}: give user (you), global (everyone) or a group's name"),
            _ => bail!("several groups are called {name}; give its id"),
        }
    }

    /// Everyone who's been in the network, removed users too, by id.
    pub fn users(&self) -> Result<Vec<User>> {
        self.rows(
            "SELECT p.id, p.name, p.handle, p.removed, coalesce((SELECT min(relation) FROM relations WHERE subject = p.id AND object = ?1), 'member')
             FROM principals p WHERE p.kind = 'user' ORDER BY p.id",
            [NETWORK],
            |r| Ok(User { id: id(&r.get::<_, String>(0)?), name: r.get(1)?, handle: r.get(2)?, removed: r.get(3)?, role: role_of(&r.get::<_, String>(4)?) }),
        )
    }

    /// A user of the network, not removed.
    pub fn user(&self, user: &str) -> Result<Option<User>> {
        self.row(
            "SELECT p.name, p.handle, coalesce((SELECT min(relation) FROM relations WHERE subject = p.id AND object = ?2), 'member')
             FROM principals p WHERE p.id = ?1 AND p.removed = 0",
            params![user_principal(user), NETWORK],
            |r| Ok(User { id: user.to_string(), name: r.get(0)?, handle: r.get(1)?, removed: false, role: role_of(&r.get::<_, String>(2)?) }),
        )
    }

    /// A user as they were last in the network, removed or not.
    pub fn user_ever(&self, user: &str) -> Result<Option<User>> {
        if let Some(u) = self.user(user)? {
            return Ok(Some(u));
        }
        self.row("SELECT name, handle FROM principals WHERE id = ?1 AND kind = 'user'", [user_principal(user)], |r| {
            Ok(User { id: user.to_string(), name: r.get(0)?, handle: r.get(1)?, removed: true, role: Role::Member })
        })
    }

    /// Every group, by id, with who's in it and who's invited.
    pub fn groups(&self) -> Result<Vec<Group>> {
        let ids = self.rows("SELECT id FROM principals WHERE kind = 'group' ORDER BY id", [], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for g in ids {
            out.extend(self.group(&id(&g))?);
        }
        Ok(out)
    }

    pub fn group(&self, g: &str) -> Result<Option<Group>> {
        let Some((name, handle)) = self.row("SELECT name, handle FROM principals WHERE id = ?1 AND kind = 'group'", [group(g)], |r| Ok((r.get(0)?, r.get(1)?)))? else {
            return Ok(None);
        };
        let members = self
            .rows(
                "SELECT subject, min(relation) FROM relations WHERE object = ?1 AND substr(subject, 1, 5) = 'user:' AND relation IN ('admin', 'member') GROUP BY subject",
                [group(g)],
                |r| Ok((id(&r.get::<_, String>(0)?), role_of(&r.get::<_, String>(1)?))),
            )?
            .into_iter()
            .collect();
        let invited = self.rows("SELECT principal, role FROM invites WHERE invited_to = ?1", [group(g)], |r| Ok((id(&r.get::<_, String>(0)?), role_of(&r.get::<_, String>(1)?))))?.into_iter().collect();
        Ok(Some(Group { id: g.to_string(), name, handle, members, invited }))
    }

    pub fn group_role(&self, user: &str, g: &str) -> Result<Option<Role>> {
        Ok(self
            .row(
                "SELECT min(relation) FROM relations WHERE subject = ?1 AND object = ?2 AND relation IN ('admin', 'member') HAVING count(*) > 0",
                params![user_principal(user), group(g)],
                |r| r.get::<_, String>(0),
            )?
            .map(|r| role_of(&r)))
    }

    pub fn groups_of(&self, user: &str) -> Result<BTreeMap<String, Role>> {
        Ok(self
            .rows(
                "SELECT object, min(relation) FROM relations WHERE subject = ?1 AND substr(object, 1, 6) = 'group:' AND relation IN ('admin', 'member') GROUP BY object",
                [user_principal(user)],
                |r| Ok((id(&r.get::<_, String>(0)?), role_of(&r.get::<_, String>(1)?))),
            )?
            .into_iter()
            .collect())
    }

    /// A user's role in a scope: in the network, in a group, and admin of their own.
    pub fn role_in(&self, user: &str, scope: &Scope) -> Result<Option<Role>> {
        Ok(match scope {
            Scope::Global => self.user(user)?.map(|u| u.role),
            Scope::Group(g) => self.group_role(user, g)?,
            Scope::User(u) => (u == user && self.user(user)?.is_some()).then_some(Role::Admin),
        })
    }

    /// Everyone's role in each scope they may use, and in each group they're invited to: (role, invited). `role_in` of
    /// each, at once.
    pub fn roles(&self) -> Result<BTreeMap<(String, Scope), (Role, bool)>> {
        let mut out = BTreeMap::new();
        for u in self.users()?.into_iter().filter(|u| !u.removed) {
            out.insert((u.id.clone(), Scope::User(u.id.clone())), (Role::Admin, false));
            out.insert((u.id.clone(), Scope::Global), (u.role, false));
        }
        let in_groups = self.rows(
            "SELECT r.subject, r.object, min(r.relation) FROM relations r JOIN principals g ON g.id = r.object AND g.kind = 'group'
             WHERE substr(r.subject, 1, 5) = 'user:' AND r.relation IN ('admin', 'member') GROUP BY r.subject, r.object",
            [],
            |r| Ok((id(&r.get::<_, String>(0)?), id(&r.get::<_, String>(1)?), role_of(&r.get::<_, String>(2)?))),
        )?;
        for (user, g, role) in in_groups {
            out.insert((user, Scope::Group(g)), (role, false));
        }
        let invited = self.rows("SELECT i.principal, i.invited_to, i.role FROM invites i JOIN principals g ON g.id = i.invited_to AND g.kind = 'group'", [], |r| {
            Ok((id(&r.get::<_, String>(0)?), id(&r.get::<_, String>(1)?), role_of(&r.get::<_, String>(2)?)))
        })?;
        for (user, g, role) in invited {
            out.entry((user, Scope::Group(g))).or_insert((role, true));
        }
        Ok(out)
    }

    /// Whose a handle is: a user's (removed users keep theirs) or a group's.
    pub fn by_handle(&self, handle: &str) -> Result<Option<Principal>> {
        Ok(self.row("SELECT id FROM principals WHERE handle = ?1 ORDER BY kind DESC LIMIT 1", [handle], |r| r.get::<_, String>(0))?.map(|p| owner_of(&p)))
    }

    /// Everyone's and every group's handle, as apps get them (`ctx.handles`): `{ handle, id, name }`, `id` being
    /// `user:<id>` or `group:<id>`; a removed user's with `removed`, and a group's with its `members`' roles by id.
    pub fn handles(&self) -> Result<Value> {
        let users = self.users()?.into_iter().map(|u| {
            let mut v = json!({ "handle": u.handle, "id": format!("user:{}", u.id), "name": u.name });
            if u.removed {
                v["removed"] = json!(true);
            }
            v
        });
        let groups = self.groups()?.into_iter().map(|g| json!({ "handle": g.handle, "id": format!("group:{}", g.id), "name": g.name, "members": g.members }));
        Ok(Value::Array(users.chain(groups).collect()))
    }

    /// Every scope there is: `global`, each group's and each user's (removed users' too: removing deletes nothing).
    pub fn scopes(&self) -> Result<Vec<Scope>> {
        let mut out = vec![Scope::Global];
        out.extend(self.rows("SELECT id FROM principals WHERE kind = 'group' ORDER BY id", [], |r| r.get::<_, String>(0))?.iter().map(|g| Scope::Group(id(g))));
        out.extend(self.rows("SELECT id FROM principals WHERE kind = 'user' ORDER BY id", [], |r| r.get::<_, String>(0))?.iter().map(|u| Scope::User(id(u))));
        Ok(out)
    }

    /// Every device ever listed, removed ones too, by key.
    pub fn machines(&self) -> Result<Vec<Machine>> {
        self.rows(&format!("{MACHINE} ORDER BY d.key"), [], machine)
    }

    /// A device of the network: listed, not removed, and its user not removed.
    pub fn machine(&self, k: &Key) -> Result<Option<Machine>> {
        Ok(self.machine_ever(k)?.filter(|m| !m.removed))
    }

    /// A device as it was last listed, removed or not.
    pub fn machine_ever(&self, k: &Key) -> Result<Option<Machine>> {
        self.row(&format!("{MACHINE} WHERE d.key = ?1"), [hex(k)], machine)
    }

    fn listed(&self, kind: Kind) -> Result<Vec<Machine>> {
        Ok(self.machines()?.into_iter().filter(|m| !m.removed && m.is(kind)).collect())
    }

    /// One of a user's own devices, which signs for them.
    pub fn client(&self, k: &Key) -> Result<Option<Machine>> {
        Ok(self.machine(k)?.filter(|m| m.is(Kind::Client)))
    }

    /// A device that runs apps' calls: a node.
    pub fn compute(&self, k: &Key) -> Result<Option<Machine>> {
        Ok(self.machine(k)?.filter(|m| m.is(Kind::Compute)))
    }

    pub fn service(&self, k: &Key) -> Result<Option<Machine>> {
        Ok(self.machine(k)?.filter(|m| m.is(Kind::Service)))
    }

    /// What `user` signs in with, as their own devices recorded it of their clients listed now: what a new device's
    /// sign-in is checked against. Not what an admin's device recorded, of a machine or of their first device.
    pub fn sign_ins(&self, user: &str) -> Result<Vec<crate::onboarding::sign_in_proof::Account>> {
        let mut out: Vec<_> = self.machines()?.into_iter().filter(|m| !m.removed && m.user == user && m.by == user && m.is(Kind::Client)).filter_map(|m| m.sign_in).collect();
        out.dedup();
        Ok(out)
    }

    pub fn clients(&self) -> Result<Vec<Machine>> {
        self.listed(Kind::Client)
    }

    pub fn computes(&self) -> Result<Vec<Machine>> {
        self.listed(Kind::Compute)
    }

    /// The service endpoints offering `name`.
    pub fn offering(&self, name: &str) -> Result<Vec<Machine>> {
        Ok(self.listed(Kind::Service)?.into_iter().filter(|m| m.service.as_deref() == Some(name)).collect())
    }

    pub fn seal_key(&self, k: &Key) -> Result<Option<Key>> {
        Ok(self.machine(k)?.map(|m| m.seal))
    }

    /// The Cloudflare node, as an admin's `CLOUDFLARE_NODE` setting names it.
    pub fn cloudflare_node(&self) -> Result<Option<Key>> {
        Ok(self.setting(CLOUDFLARE_NODE)?.and_then(|k| unhex(&k).ok()))
    }

    /// Where a user's own things run besides their own machines, as their own device last said, if it has.
    pub fn runs(&self, user: &str) -> Result<Option<BTreeSet<Key>>> {
        if self.row("SELECT 1 FROM runs_chosen WHERE principal = ?1", [user_principal(user)], |_| Ok(()))?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.rows("SELECT device FROM runs WHERE principal = ?1", [user_principal(user)], |r| r.get::<_, String>(0))?.iter().map(|k| key(k)).collect()))
    }

    /// The parts of each device its user switched on or off, by (device, part), each service's with the scope whose
    /// added app it runs for. A copy from before services named their scope reads as naming none.
    pub fn switches(&self) -> Result<BTreeMap<(Key, String), Switch>> {
        let scoped = self.row("SELECT 1 FROM pragma_table_info('services') WHERE name = 'scope'", [], |_| Ok(()))?.is_some();
        let sql = format!("SELECT device, name, switched_on, switched_by, switched_at, {} FROM services", if scoped { "scope" } else { "NULL" });
        Ok(self.rows(&sql, [], |r| {
            Ok(((key(&r.get::<_, String>(0)?), r.get::<_, String>(1)?), Switch { on: r.get(2)?, by: id(&r.get::<_, String>(3)?), time: r.get::<_, i64>(4)? as u64, scope: r.get(5)? }))
        })?
        .into_iter()
        .collect())
    }

    pub fn switch(&self, k: &Key, name: &str) -> Result<Option<Switch>> {
        Ok(self.switches()?.remove(&(*k, name.to_string())))
    }

    /// A scope's current epoch: 0 while it has none yet.
    pub fn epoch(&self, scope: &Scope) -> Result<u32> {
        Ok(self.row("SELECT max(epoch) FROM epochs WHERE scope = ?1", [scope.to_string()], |r| r.get::<_, Option<u32>>(0))?.flatten().unwrap_or(0))
    }

    /// Whether `user` may start a scope's new epoch, as its `epoch` step's rule says: they may `remove` from it, and it's
    /// a group that's there.
    pub fn starts_epochs(&self, user: &str, scope: &Scope) -> Result<bool> {
        let gone = match scope {
            Scope::Group(g) => self.group(g)?.is_none(),
            _ => false,
        };
        Ok(!gone && self.may(user, "remove", &object(scope)))
    }

    pub fn epoch_hashes(&self, scope: &Scope) -> Result<BTreeMap<u32, String>> {
        Ok(self.rows("SELECT epoch, key_hash FROM epochs WHERE scope = ?1", [scope.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
    }

    /// The hash of a scope's epoch `n`, if an entry that counts made it.
    pub fn epoch_hash(&self, scope: &Scope, n: u32) -> Result<Option<String>> {
        self.row("SELECT key_hash FROM epochs WHERE scope = ?1 AND epoch = ?2", params![scope.to_string(), n], |r| r.get(0))
    }

    pub fn epochs(&self) -> Result<BTreeMap<Scope, u32>> {
        Ok(self
            .rows("SELECT scope, max(epoch) FROM epochs GROUP BY scope", [], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
            .into_iter()
            .filter_map(|(s, n)| Some((Scope::parse(&s).ok()?, n)))
            .collect())
    }

    /// The devices a scope's current epoch was first sealed to.
    pub fn sealed_to(&self, scope: &Scope) -> Result<Vec<Key>> {
        Ok(self
            .rows("SELECT device FROM sealed WHERE scope = ?1 AND epoch = (SELECT max(epoch) FROM epochs WHERE scope = ?1)", [scope.to_string()], |r| r.get::<_, String>(0))?
            .iter()
            .map(|k| key(k))
            .collect())
    }

    pub fn owed(&self) -> Result<Vec<Scope>> {
        Ok(self.rows("SELECT scope FROM owed", [], |r| r.get::<_, String>(0))?.iter().filter_map(|s| Scope::parse(s).ok()).collect())
    }

    /// All app schedules, by (device, scope, app), as their latest `app-schedules` step said: whether in force is `app_schedules_at`'s.
    pub fn app_schedules(&self) -> Result<BTreeMap<(Key, Scope, String), AppSchedules>> {
        let rows = self.rows("SELECT device, scope, app, principal, until FROM app_schedules", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, id(&r.get::<_, String>(3)?), r.get::<_, i64>(4)? as u64))
        })?;
        let mut out = BTreeMap::new();
        for (k, s, a, by, until) in rows {
            let Ok(scope) = Scope::parse(&s) else { continue };
            let hashes = self.rows("SELECT epoch, key_hash FROM app_schedule_keys WHERE device = ?1 AND scope = ?2 AND app = ?3", params![k, s, a], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect();
            out.insert((key(&k), scope, a), AppSchedules { by, until, hashes });
        }
        Ok(out)
    }

    /// The app schedules in force at `now`: not over, on a device still listed, by someone who may still set them there.
    pub fn app_schedules_at(&self, now: u64) -> Result<Vec<((Key, Scope, String), AppSchedules)>> {
        let mut out = Vec::new();
        for (at, g) in self.app_schedules()? {
            if g.until > now && self.machine(&at.0)?.is_some() && self.may(&g.by, "choose-devices", &object(&at.1)) {
                out.push((at, g));
            }
        }
        Ok(out)
    }

    pub fn vault(&self, scope: &Scope) -> Result<Option<VaultKey>> {
        let Some((n, public)) = self.row("SELECT n, public FROM vault WHERE scope = ?1 ORDER BY n DESC LIMIT 1", [scope.to_string()], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? else {
            return Ok(None);
        };
        let to = self.rows("SELECT device FROM vault_opens WHERE scope = ?1", [scope.to_string()], |r| r.get::<_, String>(0))?.iter().map(|k| key(k)).collect();
        Ok(Some(VaultKey { n, public: key(&public), to }))
    }

    pub fn vaults(&self) -> Result<BTreeMap<Scope, VaultKey>> {
        let scopes = self.rows("SELECT DISTINCT scope FROM vault", [], |r| r.get::<_, String>(0))?;
        let mut out = BTreeMap::new();
        for s in scopes.iter().filter_map(|s| Scope::parse(s).ok()) {
            if let Some(v) = self.vault(&s)? {
                out.insert(s, v);
            }
        }
        Ok(out)
    }

    pub fn vault_publics(&self, scope: &Scope) -> Result<BTreeMap<u32, Key>> {
        Ok(self.rows("SELECT n, public FROM vault WHERE scope = ?1", [scope.to_string()], |r| Ok((r.get(0)?, key(&r.get::<_, String>(1)?))))?.into_iter().collect())
    }

    /// Whether `k` opens `scope`'s Vault now: its newest key lists it, and it's still listed.
    pub fn opens_vault(&self, k: &Key, scope: &Scope) -> Result<bool> {
        Ok(self.vault(scope)?.is_some_and(|v| v.to.contains(k)) && self.machine(k)?.is_some())
    }

    /// Every app ever made, erased ones too, by name.
    pub fn apps(&self) -> Result<Vec<App>> {
        let names = self.rows("SELECT name FROM apps ORDER BY name", [], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for n in names {
            out.extend(self.app(&n)?);
        }
        Ok(out)
    }

    pub fn app(&self, name: &str) -> Result<Option<App>> {
        let Some(mut app) = self.row("SELECT name, owner, forked_from, bundle, deploys, deleted_at, deleted_by, erased, source FROM apps WHERE name = ?1", [name], |r| {
            let deleted = r.get::<_, Option<i64>>(5)?.zip(r.get::<_, Option<String>>(6)?).map(|(t, u)| (t as u64, id(&u)));
            Ok(App {
                name: r.get(0)?,
                owner: owner_of(&r.get::<_, String>(1)?),
                from: r.get(2)?,
                bundle: r.get(3)?,
                deploys: r.get(4)?,
                history: Vec::new(),
                deleted,
                erased: r.get(7)?,
                source: r.get(8)?,
            })
        })?
        else {
            return Ok(None);
        };
        app.history = self.rows("SELECT bundle, deployed_at, deployed_by FROM deploys WHERE app = ?1 ORDER BY n", [name], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64, id(&r.get::<_, String>(2)?))))?;
        Ok(Some(app))
    }

    pub fn settings(&self) -> Result<BTreeMap<String, String>> {
        Ok(self.rows("SELECT name, value FROM settings", [], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect())
    }

    pub fn setting(&self, name: &str) -> Result<Option<String>> {
        self.row("SELECT value FROM settings WHERE name = ?1", [name], |r| r.get(0))
    }

    pub fn added(&self) -> Result<BTreeSet<String>> {
        Ok(self.rows("SELECT app FROM added WHERE added = 1", [], |r| r.get(0))?.into_iter().collect())
    }

    /// The releases steps said (`steps`), oldest first, as entries of their author's (`release::latest` reads them as
    /// it reads the log's): their content, author and time.
    pub fn releases(&self) -> Result<Vec<(crate::database::content::Content, Key, u64)>> {
        Ok(self
            .rows("SELECT content, author, time FROM releases ORDER BY at", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?
            .into_iter()
            .filter_map(|(c, a, t)| Some((serde_json::from_str(&c).ok()?, key(&a), t as u64)))
            .collect())
    }

    /// Whether a step named `blob` and the scope still needs it: forgotten, only if a node entry other than a keep
    /// named it; or it's a deploy's bundle, which is never forgotten.
    pub fn names_blob(&self, blob: &str) -> Result<bool> {
        let named = self.row("SELECT pinned OR NOT EXISTS (SELECT 1 FROM forgotten WHERE blob = ?1) FROM blobs WHERE blob = ?1", [blob], |r| r.get::<_, bool>(0))?.unwrap_or(false);
        Ok(named || self.row("SELECT 1 FROM deploys WHERE bundle = ?1", [blob], |_| Ok(()))?.is_some())
    }

    pub fn blobs(&self) -> Result<BTreeSet<String>> {
        Ok(self.rows("SELECT blob FROM blobs WHERE pinned OR blob NOT IN (SELECT blob FROM forgotten) UNION SELECT bundle FROM deploys", [], |r| r.get(0))?.into_iter().collect())
    }

    pub fn state(&self) -> Result<ScopeState> {
        let mut s = ScopeState::default();
        for (app, added, by, at) in self.rows("SELECT app, added, added_by, added_at FROM added", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, id(&r.get::<_, String>(2)?), r.get::<_, i64>(3)? as u64))
        })? {
            if added {
                s.added.insert(app.clone());
            }
            s.added_by.insert(app, (by, at));
        }
        for (app, schedule, on, by, at) in self.rows("SELECT app, schedule, switched_on, switched_by, switched_at FROM schedules", [], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, id(&r.get::<_, String>(3)?), r.get::<_, i64>(4)? as u64))
        })? {
            s.switches.insert((app, schedule), (on, by, at));
        }
        let requests = self.rows("SELECT id, app, title, fields, hosts, why, need, asked_by, agent, asked_at, answer, answered_by, answered_at FROM vault_requests", [], |r| {
            let fields: Value = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
            let hosts: Value = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default();
            let answer = match (r.get::<_, Option<String>>(10)?, r.get::<_, Option<String>>(11)?, r.get::<_, Option<i64>>(12)?) {
                (Some(how), Some(by), Some(at)) => Some((how, id(&by), at as u64)),
                _ => None,
            };
            Ok((
                r.get::<_, String>(0)?,
                crate::database::scopes::VaultRequest {
                    app: r.get(1)?,
                    title: r.get(2)?,
                    fields: fields.as_array().into_iter().flatten().map(|f| (f["name"].as_str().unwrap_or("").to_string(), f["secret"] == true)).collect(),
                    hosts: hosts.as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect(),
                    why: r.get(5)?,
                    need: r.get(6)?,
                    user: id(&r.get::<_, String>(7)?),
                    agent: r.get(8)?,
                    time: r.get::<_, i64>(9)? as u64,
                    answer,
                },
            ))
        })?;
        s.requests = requests.into_iter().collect();
        for (page, arrangement) in self.rows("SELECT page, arrangement FROM pages", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            s.arrangements.insert(page, serde_json::from_str(&arrangement).unwrap_or_default());
        }
        s.forgotten = self.rows("SELECT blob FROM forgotten", [], |r| r.get(0))?.into_iter().collect();
        Ok(s)
    }
}

/// The core's own steps and databases (`CoreDb`), beside apps' in each scope's folder, where no app can be called it.
/// Apps never see them. The core's keys are derived from it too (`keys::app_key`), so it stays.
pub const APP: &str = "node";

/// The core's own databases of a scope, under `APP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreDb {
    /// Its scope database, whose steps are who's who, permissions and keys (`steps`).
    Scope,
    /// Its frozen log: history, kept for backups and read by nothing else. The scope database's steps are sealed with
    /// its key, which they took over.
    Log,
    /// Its Vault's items (`vault`).
    Vault,
    /// The blobs its steps name (`bucket::blob_name`).
    Blobs,
}

impl CoreDb {
    const ALL: [CoreDb; 4] = [CoreDb::Scope, CoreDb::Log, CoreDb::Vault, CoreDb::Blobs];

    pub const fn name(self) -> &'static str {
        match self {
            CoreDb::Scope => "scope",
            CoreDb::Log => "log",
            CoreDb::Vault => "vault",
            CoreDb::Blobs => "blobs",
        }
    }

    /// Which it is, if `app`'s `database` is one.
    pub fn of(app: &str, database: &str) -> Option<CoreDb> {
        CoreDb::ALL.into_iter().find(|c| app == APP && database == c.name())
    }

    /// Where it is in `scope`'s folder (`bucket::database_prefix`).
    pub fn prefix(self, scope: &str) -> String {
        crate::bucket::database_prefix(scope, APP, self.name())
    }

    /// What a user calls it.
    pub fn said(self) -> &'static str {
        match self {
            CoreDb::Scope => "who's who",
            CoreDb::Log => "history",
            CoreDb::Vault => "Vault",
            CoreDb::Blobs => "files",
        }
    }
}
/// What an app's own database is called: SQLite's name for a connection's first.
pub const APP_DATABASE: &str = "main";
/// The setting that names the Cloudflare node's endpoint (its key, hex).
pub const CLOUDFLARE_NODE: &str = "cloudflare_node";
/// The app service, as a part of a device's oned its `service` entries switch: running apps' calls.
pub const APPS_PART: &str = "apps";
/// Who agents act as when the Vault fills their requests (`vault_fetch`), in an item's apps: no app's name.
pub const AGENTS: &str = "ai-agents";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
}

/// Who owns an app.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    User(String),
    Group(String),
    Admins,
}

impl Principal {
    pub fn parse(s: &str) -> Result<Principal> {
        Ok(match s.split_once(':') {
            None if s == "admins" => Principal::Admins,
            Some(("user", id)) if is_id(id) => Principal::User(id.into()),
            Some(("group", id)) if is_id(id) => Principal::Group(id.into()),
            _ => bail!("{s} isn't user:<id>, group:<id> or admins"),
        })
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Principal::User(id) => write!(f, "user:{id}"),
            Principal::Group(id) => write!(f, "group:{id}"),
            Principal::Admins => write!(f, "admins"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: String,
    pub name: String,
    /// What every app calls them by in addresses and lists (`valid_handle`), unique among users and groups.
    pub handle: String,
    pub role: Role,
    pub removed: bool,
}

/// What an endpoint does: a client is what a user and their agents use, and signs for them; compute runs apps'
/// calls; a service offers a device's abilities.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Client,
    Compute,
    Service,
}

impl Kind {
    /// Every role, as the relay lists them (`protocol::javascript`'s `ROLES`).
    pub const ALL: [Kind; 3] = [Kind::Client, Kind::Compute, Kind::Service];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Client => "client",
            Kind::Compute => "compute",
            Kind::Service => "service",
        }
    }
}

/// An endpoint of the network, listed by a `device` step (or a new network's first, `content::first`): its key, the key things are sealed to,
/// whose it is, and what it does.
#[derive(Clone, Debug)]
pub struct Machine {
    pub key: Key,
    pub seal: Key,
    pub user: String,
    pub name: String,
    pub roles: BTreeSet<Kind>,
    /// A passkey listed as a device: its credential id.
    pub passkey: Option<String>,
    /// What a client said it's built for (GLOSSARY's platform), once it has: `rules::PLATFORMS`.
    pub platform: Option<String>,
    /// For a service endpoint, the service it offers (`whatsapp`): what calls naming it reach.
    pub service: Option<String>,
    /// For a service endpoint, whether it serves everyone its user's listing lets it (a push sender, say), not only
    /// its own user.
    pub shared: bool,
    /// The device it's part of, when it's one's: for a service, its device's oned; for a native app's sign-in, the
    /// device it's on (that device's oned, or an id of the device's own).
    pub device: Option<Key>,
    /// Who listed it: its user, from one of their own devices, or an admin.
    pub by: String,
    /// When it was listed, as its entry says (ms).
    pub since: u64,
    /// What it signed in with, when that was checked as it was let in (`sign_in_proof`): one of its user's accounts,
    /// which their next device's sign-in is checked against. None for a shape this release doesn't know.
    pub sign_in: Option<crate::onboarding::sign_in_proof::Account>,
    /// The city it asked to join from, as its request said.
    pub place: Option<String>,
    pub removed: bool,
}

impl Machine {
    pub fn is(&self, kind: Kind) -> bool {
        self.roles.contains(&kind)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// As a user's: unique among users and groups.
    pub handle: String,
    pub members: BTreeMap<String, Role>,
    pub invited: BTreeMap<String, Role>,
}

#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    pub owner: Principal,
    pub from: Option<String>,
    /// Its latest deploy's bundle, a blob.
    pub bundle: Option<String>,
    pub deploys: u32,
    /// Every deploy: (bundle, time, user), oldest first.
    pub history: Vec<(String, u64, String)>,
    /// When it was deleted, and by whom: it's in Recently Deleted until it's restored or erased.
    pub deleted: Option<(u64, String)>,
    /// Erased for good: gone from every list, and its name never used again.
    pub erased: bool,
    /// Where its code is kept, as its owners last said: a web address or `<cli>://<repo>`; empty once they cleared it.
    /// None if they never did, so a built-in app links to its folder in the release's repo.
    pub source: Option<String>,
}

/// The latest `service` entry for a part of a device's oned (a service, or `apps`, the app service): on or off there,
/// as the device's user switched it.
#[derive(Clone, Debug)]
pub struct Switch {
    pub on: bool,
    pub by: String,
    pub time: u64,
    /// The scope whose added app it runs for (a service's), if its writer said: removing the app there, or moving its
    /// service, switches it off, and that scope's admins may.
    pub scope: Option<String>,
}

/// App schedules: who set them, until when, and the hash (`keys::epoch_hash`) of the app's key in each epoch
/// it sealed to the endpoint (`keys::app_schedules_key_name`), so the endpoint takes from the bucket only those.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSchedules {
    pub by: String,
    pub until: u64,
    pub hashes: BTreeMap<u32, String>,
}

/// A scope's Vault key: its number, its X25519 public half, and the machines its secret half is sealed to
/// (`keys::vault_key_name`), besides the recovery key. A new one whenever who's granted changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultKey {
    pub n: u32,
    pub public: [u8; 32],
    pub to: Vec<Key>,
}

/// An app's source link as an `app` entry gives it: a web address, or empty (or null) for none.
pub fn source_link(v: &Value) -> Result<String> {
    let link = match v {
        Value::Null => "",
        v => v.as_str().context("a source link is text")?.trim(),
    };
    let scheme = link.split_once("://").map(|(s, _)| s);
    ensure!(link.is_empty() || (link.len() <= 500 && matches!(scheme, Some("https" | "http"))), "a source link is a web address (https://…)");
    ensure!(!link.chars().any(char::is_whitespace), "a source link has no spaces");
    Ok(link.to_string())
}

/// A user's or group's handle: lowercase letters, digits and single hyphens, neither first nor last, up to 39
/// characters, like GitHub's logins.
pub fn valid_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 39 && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !h.starts_with('-') && !h.ends_with('-') && !h.contains("--")
}

/// A handle made from a name: its ASCII letters and digits, lowercased, words joined by single hyphens, cut to 39;
/// `fallback` for a name with none.
pub fn handle_from(name: &str, fallback: &str) -> String {
    let words: Vec<String> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    let handle = words.join("-");
    let handle = handle[..handle.len().min(39)].trim_end_matches('-');
    if handle.is_empty() { fallback.to_string() } else { handle.to_string() }
}

pub fn valid_app_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && name != APP && name != AGENTS
}

/// What a scope's own node entries add up to: the apps added to it, every `open` of it, requests for Vault items
/// (the items themselves are in its `vault` database, `src/vault.rs`), and its schedules' switches.
#[derive(Clone, Debug, Default)]
pub struct ScopeState {
    pub added: BTreeSet<String>,
    /// Who last added each app here, and when: its schedules run as them, unless someone switched one on since.
    pub added_by: BTreeMap<String, (String, u64)>,
    /// Each schedule switched on or off here, by (app, schedule): (on, by whom, when).
    pub switches: BTreeMap<(String, String), (bool, String, u64)>,
    /// Apps' and agents' requests for a user's item here, by id.
    pub requests: BTreeMap<String, VaultRequest>,
    /// The blobs this scope's apps said they no longer need, though their entries name them.
    pub forgotten: BTreeSet<String>,
    /// In a user's own scope: how they arranged each page, by page (`user`, `group:<id>` or `global`):
    /// `{ items, hidden }`, as their latest `arrange` of it said.
    pub arrangements: BTreeMap<String, Value>,
}

/// An app's or agent's request for a user's item that an app may use: the fields it wants (name, and whether
/// secret), what for, who asked, and how it was answered: (`set` or `declined`, by whom, when).
#[derive(Clone, Debug)]
pub struct VaultRequest {
    pub app: String,
    pub title: String,
    pub fields: Vec<(String, bool)>,
    pub hosts: Vec<String>,
    pub why: String,
    pub need: Option<String>,
    pub user: String,
    pub agent: Option<String>,
    pub time: u64,
    pub answer: Option<(String, String, u64)>,
}

/// A page a user arranges: their own (`user`), a group's (`group:<id>`) or everyone's (`global`).
pub fn valid_page(page: &str) -> bool {
    page == "user" || page == "global" || page.strip_prefix("group:").is_some_and(is_id)
}

/// A Vault field's name (once a secret's): letters, digits, `_`, `-` and `.`.
pub fn valid_secret_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// A host an item's secret fields are sent to: a lowercase host name or IP address, without scheme or port.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An app's own types and rows answer only for its own objects: Alice is group g's admin, Bob its member, Dave a
    /// collaborator, Carol owns repo 2, and Erin is out of the network. Rows that would give a core role, make a rule,
    /// or that a type of the core's name declares, count for nothing, and nothing stays in the database.
    #[test]
    fn app_may() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(permissions::SCHEMA).unwrap();
        db.execute_batch(permissions::DEFAULTS).unwrap();
        for (s, r, o) in [
            ("user:alice", "member", NETWORK),
            ("user:bob", "member", NETWORK),
            ("user:carol", "member", NETWORK),
            ("user:dave", "member", NETWORK),
            ("user:alice", "admin", "group:g"),
            ("user:bob", "member", "group:g"),
        ] {
            db.execute("INSERT INTO relations VALUES (?1, ?2, ?3, NULL)", [s, r, o]).unwrap();
        }
        let db = ScopeDb { db, now: 0 };
        let types = json!({
            "repo": { "permissions": ["push", "manage"], "roles": { "owner": ["push", "manage"], "collaborator": ["push"] } },
            "group": { "permissions": ["use"], "roles": { "fan": ["use"] } },
        });
        let rows: Vec<Relation> = [
            ("group:g#admin", "owner", "repo:1"),
            ("group:g#member", "collaborator", "repo:1"),
            ("user:dave", "collaborator", "repo:1"),
            ("user:carol", "owner", "repo:2"),
            ("user:erin", "owner", "repo:1"),
            ("user:carol", "admin", NETWORK),
            ("user:carol", "fan", "group:g"),
            ("#collaborator", "owner", "repo:1"),
            ("@", "owner", "repo:*"),
        ]
        .into_iter()
        .map(|(s, r, o)| (s.to_string(), r.to_string(), o.to_string(), None))
        .collect();
        let count = |table: &str| db.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get::<_, i64>(0)).unwrap();
        let before = (count("types"), count("roles"), count("relations"));
        let may = |who: &str, verb: &str, object: &str| db.app_may(who, &[(verb.into(), object.into())], &types, &rows, 0) == [true];
        assert!(may("alice", "push", "repo:1") && may("alice", "manage", "repo:1"));
        assert!(may("bob", "push", "repo:1") && !may("bob", "manage", "repo:1"));
        assert!(may("dave", "push", "repo:1") && !may("dave", "manage", "repo:1"));
        assert!(may("carol", "manage", "repo:2") && !may("carol", "push", "repo:1"));
        assert!(!may("erin", "push", "repo:1"));
        assert!(!may("carol", "change-roles", NETWORK) && !may("carol", "use", "group:g"));
        assert!(may("alice", "use", "group:g") && !may("dave", "push", "repo:2"));
        let asks = [("push", "repo:1"), ("manage", "repo:1"), ("push", "repo:2")].map(|(v, o)| (v.to_string(), o.to_string()));
        assert_eq!(db.app_may("bob", &asks, &types, &rows, 0), [true, false, false]);
        assert_eq!((count("types"), count("roles"), count("relations")), before);
    }
}

/// Where everything lives.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Global,
    Group(String),
    User(String),
}

impl Scope {
    /// A scope as `user` names it: `user` is their own.
    pub fn parse_for(s: &str, user: &str) -> Result<Scope> {
        match s {
            "user" => Ok(Scope::User(user.to_string())),
            s => Scope::parse(s),
        }
    }

    pub fn parse(s: &str) -> Result<Scope> {
        Ok(match s.split_once(':') {
            None if s == "global" => Scope::Global,
            Some(("group", id)) if is_id(id) => Scope::Group(id.into()),
            Some(("user", id)) if is_id(id) => Scope::User(id.into()),
            _ => anyhow::bail!("{s} isn't a scope: global, group:<id> or user:<id>"),
        })
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Global => write!(f, "global"),
            Scope::Group(id) => write!(f, "group:{id}"),
            Scope::User(id) => write!(f, "user:{id}"),
        }
    }
}

pub fn is_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn new_id() -> String {
    data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>())
}
