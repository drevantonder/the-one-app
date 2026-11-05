//! Permissions: who may do what, as rows of three tables in a SQLite database it's given (the scope database), and
//! queries over them. No I/O but that connection, and no clock: each check is told the time.
//!
//! - **Types.** Each object type names its permissions, its verbs (`types`). An object is `<type>:<id>`
//!   (`group:0123456789abcdef`); `<type>:*` is every object of that type, for rules.
//! - **Roles.** A role is a name and a set of its type's permissions (`roles`). `builtin` marks the ones the release
//!   ships; an install adds its own.
//! - **Relations.** A row of `relations` says its subject holds a role (its `relation`) on its object, until a time
//!   (milliseconds; NULL for good). A subject is a principal (`person:<id>`, `device:<key in hex>`), everyone who
//!   holds a role on an object (`group:<id>#member`), or `#<role>`: everyone who holds that role on the row's own
//!   object, which is how a rule such as "a group's admins are its members" is one row,
//!   `('#admin', 'member', 'group:*')`.
//!
//! Unknown means no: a type, role or permission this release doesn't know grants nothing, so a row written by a newer
//! one is ignored here, never misread.
//!
//! The core's types, their default roles and the rules between them are `DEFAULTS`, rows like any others. Apps add
//! their own types (`repo`) the same way. Four checks read them:
//! - `may`: whether a principal holds a permission on an object, now. One query, through roles held on other objects
//!   (a group's members) and rules, each row only until its `until`.
//! - `may_give`: no escalation. Giving a role takes `invite` on the object and every permission of the role there.
//! - `may_take`: taking a role takes `remove`, or `leave` for your own; and the install's last admin stays.
//! - `may_change_role`: an install's own role is changed by whoever has `change-roles`, and widened only by someone
//!   who already holds what it adds wherever it's held. The release's own roles change only with the release.
//!
//! Which object a call or a step is about, and who signed it, is the caller's to say: this only answers.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// The tables, as every database that holds permissions makes them: run it on a new one.
pub const SCHEMA: &str = "
create table if not exists types (
    name text primary key,
    -- its verbs, as a JSON array of text
    permissions text not null
) strict;
create table if not exists roles (
    type text not null,
    name text not null,
    -- a JSON array of its type's verbs
    permissions text not null,
    builtin integer not null default 0,
    primary key (type, name)
) strict;
create table if not exists relations (
    subject text not null,
    relation text not null,
    object text not null,
    -- milliseconds; null for good
    until integer,
    primary key (subject, relation, object)
) strict;
create index if not exists relations_by_object on relations (object, relation);
";

/// The install, as an object.
pub const INSTALL: &str = "install:global";
/// The install's role that must always have someone.
const ADMIN: &str = "admin";

/// The core's types and roles, and the rules between them: run on a database `SCHEMA` made, after it. Each release
/// writes its own; an install's roles and rows are left as they are.
pub const DEFAULTS: &str = r#"
-- Verbs, each for what a person does:
--   use: open its data and its apps
--   invite: give its roles to others (only those you hold all of)
--   remove: take its roles from others, and start its next key
--   leave: give up your own role on it
--   rename: change its name and handle
--   add-apps: make, add and remove its apps, switch their schedules, and ask for Vault items for them
--   keep: choose which devices keep its apps running, run its calls and open its Vault
--   run: run its calls (a machine whose person holds it, or a device that does)
--   make-groups, change-settings, change-roles: what they say, in the install
--   add-devices: list their phones, computers and services; add-machines: list machines that run things for them
--   switch: turn a device's services on and off
--   change: set or clear a Vault item
--   deploy: send an app a new version, or its source; delete: delete, restore or erase an app
insert or replace into types (name, permissions) values
    ('install', '["use","invite","remove","add-apps","keep","run","make-groups","change-settings","change-roles"]'),
    ('person', '["use","remove","rename","add-apps","keep","run","add-devices","add-machines"]'),
    ('group', '["use","invite","remove","leave","rename","add-apps","keep","run"]'),
    ('device', '["rename","remove","switch"]'),
    ('service', '["use"]'),
    ('vault', '["use","change"]'),
    ('app', '["deploy","delete","invite","remove"]');

delete from roles where builtin = 1;
insert or replace into roles (type, name, permissions, builtin) values
    ('install', 'admin', '["use","invite","remove","add-apps","keep","run","make-groups","change-settings","change-roles"]', 1),
    ('install', 'member', '["use","make-groups"]', 1),
    ('install', 'runner', '["run"]', 1),
    -- a person's own scope is theirs; the install's admins only rename them and add machines for them
    ('person', 'owner', '["use","remove","rename","add-apps","keep","run","add-devices","add-machines"]', 1),
    ('person', 'admin', '["rename","add-machines"]', 1),
    ('person', 'runner', '["run"]', 1),
    ('group', 'admin', '["use","invite","remove","leave","rename","add-apps","keep","run"]', 1),
    ('group', 'member', '["use","leave"]', 1),
    -- the install's admins, who manage every group as replay lets them (members.rs `manages`), giving only what they hold
    ('group', 'manager', '["invite","remove","rename","run"]', 1),
    ('group', 'runner', '["run"]', 1),
    ('device', 'owner', '["rename","remove","switch"]', 1),
    ('device', 'admin', '["rename","remove"]', 1),
    ('service', 'user', '["use"]', 1),
    ('vault', 'owner', '["use","change"]', 1),
    ('vault', 'user', '["use"]', 1),
    ('app', 'owner', '["deploy","delete","invite","remove"]', 1);

insert or ignore into relations (subject, relation, object, until) values
    ('#admin', 'member', 'install:*', null),
    ('#admin', 'member', 'group:*', null),
    ('install:global#admin', 'manager', 'group:*', null),
    ('install:global#admin', 'admin', 'person:*', null),
    ('install:global#admin', 'admin', 'device:*', null);
"#;

/// The type of an object (`group` of `group:…`), if it's one: a principal or object is `<type>:<id>`, and never a
/// userset (`#`).
fn type_of(object: &str) -> Option<&str> {
    let (ty, id) = object.split_once(':')?;
    (!ty.is_empty() && !id.is_empty() && !object.contains('#')).then_some(ty)
}

fn ms(t: u64) -> i64 {
    i64::try_from(t).unwrap_or(i64::MAX)
}

/// Every role `principal` holds now, on which object, through usersets and rules: `held(object, role)`.
const HELD: &str = "
with recursive held(object, role) as (
    select object, relation from relations
    where subject = ?1 and (until is null or until > ?2)
        and exists (select 1 from roles where type = substr(object, 1, instr(object, ':') - 1) and name = relation)
    union
    select r.object, r.relation from held h join relations r on r.subject = h.object || '#' || h.role
    where (r.until is null or r.until > ?2)
        and exists (select 1 from roles where type = substr(r.object, 1, instr(r.object, ':') - 1) and name = r.relation)
    union
    select h.object, r.relation from held h join relations r on r.subject = '#' || h.role
        and r.object in (h.object, substr(h.object, 1, instr(h.object, ':')) || '*')
    where (r.until is null or r.until > ?2)
        and exists (select 1 from roles where type = substr(r.object, 1, instr(r.object, ':') - 1) and name = r.relation)
)";

/// Whether `principal` (`person:…`, `device:…`) may do `permission` to `object` at `now` (ms): it holds a role on
/// the object, or on every object of its type, that has the permission, and the type has it too.
pub fn may(conn: &Connection, principal: &str, permission: &str, object: &str, now: u64) -> Result<bool> {
    let (Some(_), Some(ty)) = (type_of(principal).filter(|_| !principal.contains('*')), type_of(object)) else {
        return Ok(false);
    };
    let sql = format!(
        "{HELD}
        select exists (
            select 1 from held h join roles ro on ro.type = ?3 and ro.name = h.role join types t on t.name = ?3
            where h.object in (?4, ?3 || ':*')
                and exists (select 1 from json_each(ro.permissions) where value = ?5)
                and exists (select 1 from json_each(t.permissions) where value = ?5)
        )"
    );
    Ok(conn.query_row(&sql, params![principal, ms(now), ty, object, permission], |r| r.get(0))?)
}

/// A role's permissions that its type has, or None if the type or role is unknown.
fn role(conn: &Connection, ty: &str, name: &str) -> Result<Option<(Vec<String>, bool)>> {
    let found: Option<(String, bool)> = conn
        .query_row(
            "select (select json_group_array(p.value) from json_each(ro.permissions) p
                     where p.value in (select value from json_each(t.permissions))), ro.builtin
             from roles ro join types t on t.name = ro.type where ro.type = ?1 and ro.name = ?2",
            params![ty, name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        Some((perms, builtin)) => Some((serde_json::from_str(&perms)?, builtin)),
        None => None,
    })
}

/// Whether `subject` is the install's last lasting admin: no other person is one for good.
pub fn last_admin(conn: &Connection, subject: &str) -> Result<bool> {
    Ok(conn.query_row(
        "select exists (select 1 from relations where subject = ?1 and relation = ?2 and object = ?3 and until is null)
            and not exists (select 1 from relations where subject != ?1 and substr(subject, 1, 7) = 'person:'
                and relation = ?2 and object = ?3 and until is null)",
        params![subject, ADMIN, INSTALL],
        |r| r.get(0),
    )?)
}

/// Whether `giver` may give `subject` the role `relation` on `object` until `until` (None: for good): they may
/// `invite` there and hold every permission of the role there, and it doesn't give the last admin an end.
pub fn may_give(conn: &Connection, giver: &str, subject: &str, relation: &str, object: &str, until: Option<u64>, now: u64) -> Result<bool> {
    let Some((perms, _)) = type_of(object).map(|ty| role(conn, ty, relation)).transpose()?.flatten() else {
        return Ok(false);
    };
    if !may(conn, giver, "invite", object, now)? {
        return Ok(false);
    }
    for p in &perms {
        if !may(conn, giver, p, object, now)? {
            return Ok(false);
        }
    }
    Ok(!(until.is_some() && relation == ADMIN && object == INSTALL && last_admin(conn, subject)?))
}

/// Whether `taker` may take the role `relation` on `object` from `subject`: they may `remove` there, or `leave` if
/// it's their own; and it isn't the install's last admin.
pub fn may_take(conn: &Connection, taker: &str, subject: &str, relation: &str, object: &str, now: u64) -> Result<bool> {
    let allowed = may(conn, taker, "remove", object, now)? || (subject == taker && may(conn, taker, "leave", object, now)?);
    Ok(allowed && !(relation == ADMIN && object == INSTALL && last_admin(conn, subject)?))
}

/// Whether `who` may make the install's own role `name` on `ty` have `permissions`: they may `change-roles`, each is
/// one of the type's, it isn't one the release ships, and they hold whatever it adds on every object it's held on.
pub fn may_change_role(conn: &Connection, who: &str, ty: &str, name: &str, permissions: &[&str], now: u64) -> Result<bool> {
    let known: Option<String> = conn.query_row("select permissions from types where name = ?1", [ty], |r| r.get(0)).optional()?;
    let Some(known) = known.map(|k| serde_json::from_str::<Vec<String>>(&k)).transpose()? else {
        return Ok(false);
    };
    let was = match role(conn, ty, name)? {
        Some((_, true)) => return Ok(false),
        Some((perms, false)) => perms,
        None => Vec::new(),
    };
    if !permissions.iter().all(|p| known.iter().any(|k| k == p)) || !may(conn, who, "change-roles", INSTALL, now)? {
        return Ok(false);
    }
    let mut held = conn.prepare("select distinct object from relations where relation = ?1 and substr(object, 1, ?3) = ?2 || ':'")?;
    let objects = held.query_map(params![name, ty, ty.len() as i64 + 1], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for object in &objects {
        for p in permissions.iter().filter(|p| !was.iter().any(|w| w == *p)) {
            if !may(conn, who, p, object, now)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Who may do what in a small install: Alice its admin, Bob and Carol members, Carol the admin of group g and Bob
    /// in it, Erin a member until 1000 and Frank until 2000, Dave holding a role nobody knows; asked at 1500.
    #[test]
    #[allow(clippy::type_complexity)]
    fn who_may_do_what() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        let rows: &[(&str, &str, &str, Option<i64>)] = &[
            ("person:alice", "admin", "install:global", None),
            ("person:bob", "member", "install:global", None),
            ("person:carol", "member", "install:global", None),
            ("person:erin", "member", "install:global", Some(1000)),
            ("person:frank", "member", "install:global", Some(2000)),
            ("person:dave", "superuser", "install:global", None),
            ("person:dave", "admin", "rocket:1", None),
            ("person:bob", "owner", "person:bob", None),
            ("person:carol", "admin", "group:g", None),
            ("person:bob", "member", "group:g", None),
            ("group:g#member", "user", "vault:group:g/token", None),
            ("group:g#admin", "owner", "vault:group:g/token", None),
            ("person:bob", "owner", "device:bb", None),
            ("install:global#member", "user", "service:ss", None),
            ("group:g#admin", "owner", "app:notes", None),
            ("device:cc", "runner", "person:bob", None),
        ];
        for (s, r, o, u) in rows {
            conn.execute("insert into relations values (?1, ?2, ?3, ?4)", params![s, r, o, u]).unwrap();
        }
        let now = 1500;
        let may_cases: &[(&str, &str, &str, bool)] = &[
            ("person:alice", "change-settings", "install:global", true),
            ("person:bob", "change-settings", "install:global", false),
            ("person:bob", "use", "install:global", true),
            ("person:bob", "make-groups", "install:global", true),
            ("person:bob", "invite", "install:global", false),
            ("person:erin", "use", "install:global", false),
            ("person:frank", "use", "install:global", true),
            ("person:nobody", "use", "install:global", false),
            // a group's members, and its admins through the rule that they're members too
            ("person:bob", "use", "group:g", true),
            ("person:bob", "invite", "group:g", false),
            ("person:carol", "use", "vault:group:g/token", true),
            ("person:carol", "change", "vault:group:g/token", true),
            ("person:bob", "use", "vault:group:g/token", true),
            ("person:bob", "change", "vault:group:g/token", false),
            ("person:carol", "deploy", "app:notes", true),
            ("person:bob", "deploy", "app:notes", false),
            // the install's admins manage every group, rename every person and look after every device, and open none
            ("person:alice", "invite", "group:g", true),
            ("person:alice", "use", "group:g", false),
            ("person:alice", "use", "vault:group:g/token", false),
            ("person:alice", "rename", "person:bob", true),
            ("person:alice", "use", "person:bob", false),
            ("person:bob", "use", "person:bob", true),
            ("person:carol", "use", "person:bob", false),
            ("person:alice", "remove", "device:bb", true),
            ("person:alice", "switch", "device:bb", false),
            ("person:bob", "switch", "device:bb", true),
            ("person:carol", "use", "service:ss", true),
            ("person:erin", "use", "service:ss", false),
            ("device:cc", "run", "person:bob", true),
            ("device:cc", "use", "person:bob", false),
            // unknown means no
            ("person:bob", "fly", "install:global", false),
            ("person:dave", "change-settings", "install:global", false),
            ("person:dave", "use", "rocket:1", false),
            ("#admin", "use", "install:global", false),
            ("install:global#admin", "change-settings", "install:global", false),
            ("person:alice", "use", "install", false),
        ];
        for (who, what, on, expected) in may_cases {
            assert_eq!(may(&conn, who, what, on, now).unwrap(), *expected, "may({who}, {what}, {on})");
        }

        // No escalation, and the install's last admin stays.
        let give_cases: &[(&str, &str, &str, &str, Option<u64>, bool)] = &[
            ("person:carol", "person:erin", "member", "group:g", None, true),
            ("person:carol", "person:erin", "admin", "group:g", None, true),
            ("person:bob", "person:erin", "member", "group:g", None, false),
            ("person:alice", "person:erin", "member", "group:g", None, false),
            ("person:alice", "person:bob", "admin", "install:global", None, true),
            ("person:alice", "person:alice", "admin", "install:global", Some(3000), false),
            ("person:bob", "person:erin", "member", "install:global", None, false),
            ("person:carol", "person:erin", "superuser", "group:g", None, false),
        ];
        for (giver, to, role, on, until, expected) in give_cases {
            assert_eq!(may_give(&conn, giver, to, role, on, *until, now).unwrap(), *expected, "may_give({giver}, {to}, {role}, {on})");
        }
        let take_cases: &[(&str, &str, &str, &str, bool)] = &[
            ("person:alice", "person:alice", "admin", "install:global", false),
            ("person:alice", "person:bob", "member", "install:global", true),
            ("person:bob", "person:bob", "member", "group:g", true),
            ("person:bob", "person:carol", "admin", "group:g", false),
            ("person:alice", "person:carol", "admin", "group:g", true),
            ("person:bob", "person:bob", "member", "install:global", false),
        ];
        for (taker, from, role, on, expected) in take_cases {
            assert_eq!(may_take(&conn, taker, from, role, on, now).unwrap(), *expected, "may_take({taker}, {from}, {role}, {on})");
        }
        conn.execute("insert into relations values ('person:bob', 'admin', 'install:global', null)", []).unwrap();
        assert!(may_take(&conn, "person:alice", "person:alice", "admin", "install:global", now).unwrap());

        // An install's own roles: made by its admins, widened only by someone who holds what it adds where it's held.
        assert!(may_change_role(&conn, "person:alice", "group", "editor", &["use"], now).unwrap());
        assert!(!may_change_role(&conn, "person:carol", "group", "editor", &["use"], now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "editor", &["fly"], now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "member", &["use", "invite"], now).unwrap());
        conn.execute("insert into roles values ('group', 'editor', '[\"use\"]', 0)", []).unwrap();
        conn.execute("insert into relations values ('person:frank', 'editor', 'group:g', null)", []).unwrap();
        assert!(may(&conn, "person:frank", "use", "group:g", now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "editor", &["use", "add-apps"], now).unwrap());
        conn.execute("insert into relations values ('person:alice', 'admin', 'group:g', null)", []).unwrap();
        assert!(may_change_role(&conn, "person:alice", "group", "editor", &["use", "add-apps"], now).unwrap());
    }
}
