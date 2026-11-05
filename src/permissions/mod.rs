//! Permissions: who may do what, as rows of three tables in a SQLite database it's given (the scope database), and
//! queries over them. No I/O but that connection, and no clock: each check is told the time.
//!
//! - **Types.** Each object type names its permissions, its verbs (`types`). An object is `<type>:<id>`
//!   (`group:0123456789abcdef`); `<type>:*` is every object of that type, for rules.
//! - **Roles.** A role is a name and a set of its type's permissions (`roles`). `builtin` marks the ones the release
//!   ships; an install adds its own. A role of the release's may also give others of its type that its holders don't
//!   hold (`gives`): the install's admins add people to every group without opening any.
//! - **Relations.** A row of `relations` says its subject holds a role (its `relation`) on its object, until a time
//!   (milliseconds; NULL for good). A subject is a principal (`person:<id>`, `device:<key in hex>`), everyone who
//!   holds a role on an object (`group:<id>#member`), or a rule: `#<role>`, everyone who holds that role on the row's
//!   own object (`('#admin', 'member', 'group:*')`: a group's admins are its members), or `@`, each object of the
//!   type itself (`('@', 'owner', 'person:*')`: everyone owns their own scope). A role held on `<type>:*` is held on
//!   each object of that type, for usersets and rules too. A role reached through others lasts only as long as each.
//! - **Rules are the release's**: a row whose subject is `#…` or `@`, or that names `*`, is written by `DEFAULTS` and
//!   by nothing else, and nobody gives or takes one.
//! - **Principals are people in the install, and their devices.** A person holds nothing while they hold no role on
//!   the install, so removing them is taking that; a device holds nothing unless a person in the install, or the
//!   install (`install:global#…`), holds a role on it by a row of its own, so removing it is taking its owner's row.
//!
//! Unknown means no: a type, role or permission this release doesn't know grants nothing, nor anything reached
//! through it, so a row written by a newer one is ignored here, never misread.
//!
//! The core's types, their default roles and the rules between them are `DEFAULTS`, rows like any others. Apps add
//! their own types (`repo`) the same way. Five checks read them:
//! - `may`: whether a principal holds a permission on an object, now.
//! - `may_give`: no escalation. You give a role only where you may `invite`, only one you hold there yourself (or
//!   that a role of yours gives), and for no longer than you hold both.
//! - `may_take`: taking a role takes `remove`, or `leave` for your own; and the install's last admin stays.
//! - `may_make`: the first row of a new object (a device, a Vault item, a service, an app), which nobody holds a role
//!   on yet to give: by whoever the verb that makes it lets, to themselves, its container, or a role on the container.
//! - `may_change_role`: an install's own role is changed by whoever has `change-roles`, and widened only by someone
//!   who already holds what it adds wherever it's held. The release's own roles change only with the release.
//!
//! Which object a call or a step is about, and who signed it, is the caller's to say: this only answers. So is moving
//! an app to another owner, which replay allows only someone who may `delete` it and may `add-apps` where it goes.

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
    -- a JSON array of its type's other roles its holders give, though they don't hold them; only the release's count
    gives text not null default '[]',
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
/// For good, as an until.
const FOREVER: i64 = i64::MAX;

/// The core's types and roles, and the rules between them: run on a database `SCHEMA` made, after it. Each release
/// writes its own; an install's roles and rows are left as they are.
pub const DEFAULTS: &str = r#"
-- Verbs, each for what a person does:
--   use: open its data and its apps
--   invite: give its roles to others (only those you hold, or a role of yours gives)
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
-- A device's and an app's roles come only with making them (may_make): nobody gives one.
insert or replace into types (name, permissions) values
    ('install', '["use","invite","remove","add-apps","keep","run","make-groups","change-settings","change-roles"]'),
    ('person', '["use","remove","rename","add-apps","keep","run","add-devices","add-machines"]'),
    ('group', '["use","invite","remove","leave","rename","add-apps","keep","run"]'),
    ('device', '["rename","remove","switch"]'),
    ('service', '["use","invite","remove","leave"]'),
    ('vault', '["use","change","invite","remove","leave"]'),
    ('app', '["deploy","delete"]');

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
    ('group', 'runner', '["run"]', 1),
    ('device', 'owner', '["rename","remove","switch"]', 1),
    ('device', 'admin', '["rename","remove"]', 1),
    ('service', 'owner', '["use","invite","remove"]', 1),
    ('service', 'user', '["use","leave"]', 1),
    ('vault', 'owner', '["use","change","invite","remove"]', 1),
    ('vault', 'user', '["use","leave"]', 1),
    ('app', 'owner', '["deploy","delete"]', 1);
-- the install's admins, who manage every group as replay lets them (members.rs `manages`) and add people to it, but
-- open none
insert or replace into roles (type, name, permissions, gives, builtin) values
    ('group', 'manager', '["invite","remove","rename","run"]', '["member"]', 1);

delete from relations where substr(subject, 1, 1) in ('#', '@') or instr(subject, '*') > 0 or instr(object, '*') > 0;
insert into relations (subject, relation, object, until) values
    ('@', 'owner', 'person:*', null),
    ('#admin', 'member', 'install:*', null),
    ('#admin', 'member', 'group:*', null),
    ('#owner', 'user', 'service:*', null),
    ('#owner', 'user', 'vault:*', null),
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

/// Whether a row is a rule, the release's: its subject `#…` or `@`, or `*` in it.
fn rule(subject: &str, object: &str) -> bool {
    subject.starts_with(['#', '@']) || subject.contains('*') || object.contains('*')
}

/// Whether `subject` is one a role can be given to: one object (a principal), or everyone holding a role on one.
fn givable(subject: &str) -> bool {
    let (who, role) = subject.split_once('#').unwrap_or((subject, "-"));
    type_of(who).is_some() && !role.is_empty() && !role.contains('#') && !rule(subject, "")
}

fn ms(t: u64) -> i64 {
    i64::try_from(t).unwrap_or(FOREVER)
}

/// Every role `?1` holds while `?2` is before its until, on which object, through usersets and rules:
/// `held(object, role, until)`, only roles of known types, `until` the soonest of the rows it came through.
const HELD: &str = "
with recursive known(type, name, permissions, gives, builtin) as (
    select ro.type, ro.name,
        (select json_group_array(p.value) from json_each(ro.permissions) p
         where p.value in (select value from json_each(t.permissions))),
        ro.gives, ro.builtin
    from roles ro join types t on t.name = ro.type
),
held(object, role, until) as (
    select object, relation, coalesce(until, 9223372036854775807) from relations
    where subject = ?1 and coalesce(until, 9223372036854775807) > ?2
        and exists (select 1 from known where type = substr(object, 1, instr(object, ':') - 1) and name = relation)
    union
    select ?1, relation, coalesce(until, 9223372036854775807) from relations
    where subject = '@' and object = substr(?1, 1, instr(?1, ':')) || '*' and coalesce(until, 9223372036854775807) > ?2
        and exists (select 1 from known where type = substr(object, 1, instr(object, ':') - 1) and name = relation)
    union
    select r.object, r.relation, min(h.until, coalesce(r.until, 9223372036854775807))
    from held h join relations r on instr(r.subject, '#') > 1
        and substr(r.subject, instr(r.subject, '#') + 1) = h.role
        and (substr(r.subject, 1, instr(r.subject, '#') - 1) = h.object
            or h.object = substr(r.subject, 1, instr(r.subject, ':')) || '*')
    where coalesce(r.until, 9223372036854775807) > ?2
        and exists (select 1 from known where type = substr(r.object, 1, instr(r.object, ':') - 1) and name = r.relation)
    union
    select iif(substr(h.object, -2) = ':*', r.object, h.object), r.relation, min(h.until, coalesce(r.until, 9223372036854775807))
    from held h join relations r on r.subject = '#' || h.role
        and (r.object in (h.object, substr(h.object, 1, instr(h.object, ':')) || '*')
            or h.object = substr(r.object, 1, instr(r.object, ':')) || '*')
    where coalesce(r.until, 9223372036854775807) > ?2
        and exists (select 1 from known where type = substr(r.object, 1, instr(r.object, ':') - 1) and name = r.relation)
)";

/// A role someone holds on an object: until when, and the permissions it has and the roles it gives that their type
/// knows.
struct Held {
    role: String,
    until: i64,
    permissions: Vec<String>,
    gives: Vec<String>,
}

/// The roles `who` holds on `object` now, on it or on every object of its type, each with the latest it lasts to.
fn holds(conn: &Connection, who: &str, object: &str, now: u64) -> Result<Vec<Held>> {
    let Some(ty) = type_of(object) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "{HELD}
        select h.role, max(h.until), k.permissions, iif(k.builtin, k.gives, '[]') from held h
            join known k on k.type = ?3 and k.name = h.role
        where h.object in (?4, ?3 || ':*') group by h.role"
    );
    let mut q = conn.prepare(&sql)?;
    let rows = q.query_map(params![who, ms(now), ty, object], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
    rows.map(|row| {
        let (role, until, permissions, gives) = row?;
        Ok(Held { role, until, permissions: serde_json::from_str(&permissions)?, gives: serde_json::from_str(&gives)? })
    })
    .collect()
}

/// Whether `principal` counts now: a person who holds a role on the install, or a device that such a person, or the
/// install, holds a role on by a row of its own.
fn counts(conn: &Connection, principal: &str, now: u64) -> Result<bool> {
    if principal.contains('*') {
        return Ok(false);
    }
    match type_of(principal) {
        Some("person") => Ok(!holds(conn, principal, INSTALL, now)?.is_empty()),
        Some("device") => {
            let mut q = conn.prepare(
                "select r.subject from relations r join roles ro on ro.type = 'device' and ro.name = r.relation
                 join types t on t.name = ro.type
                 where r.object = ?1 and coalesce(r.until, 9223372036854775807) > ?2",
            )?;
            let holders = q.query_map(params![principal, ms(now)], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for h in holders {
                if h.strip_prefix(INSTALL).is_some_and(|rest| rest.starts_with('#')) || (type_of(&h) == Some("person") && counts(conn, &h, now)?) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Whether `principal` (`person:…`, `device:…`) may do `permission` to `object` at `now` (ms): it counts, and holds a
/// role on the object, or on every object of its type, that has the permission, and the type has it too.
pub fn may(conn: &Connection, principal: &str, permission: &str, object: &str, now: u64) -> Result<bool> {
    Ok(counts(conn, principal, now)? && holds(conn, principal, object, now)?.iter().any(|h| h.permissions.iter().any(|p| p == permission)))
}

/// A role's permissions that its type has, and whether the release ships it, or None if the type or role is unknown.
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

/// Whether `giver` may give `subject` (a principal, or everyone holding a role on an object) the role `relation` on
/// `object` until `until` (None: for good): they count, may `invite` there, hold the role there themselves or one of
/// theirs gives it, for at least as long as `until`; and it doesn't give the last admin an end.
pub fn may_give(conn: &Connection, giver: &str, subject: &str, relation: &str, object: &str, until: Option<u64>, now: u64) -> Result<bool> {
    let Some(ty) = type_of(object).filter(|_| !rule(subject, object) && givable(subject)) else {
        return Ok(false);
    };
    if role(conn, ty, relation)?.is_none() || !counts(conn, giver, now)? {
        return Ok(false);
    }
    let held = holds(conn, giver, object, now)?;
    let longest = |f: &dyn Fn(&Held) -> bool| held.iter().filter(|h| f(h)).map(|h| h.until).max();
    let invite = longest(&|h| h.permissions.iter().any(|p| p == "invite"));
    let from = longest(&|h| h.role == relation || h.gives.iter().any(|g| g == relation));
    let (Some(invite), Some(from)) = (invite, from) else {
        return Ok(false);
    };
    Ok(until.map_or(FOREVER, ms) <= invite.min(from) && !(until.is_some() && relation == ADMIN && object == INSTALL && last_admin(conn, subject)?))
}

/// Whether `taker` may take the role `relation` on `object` from `subject`: it isn't a rule, they may `remove`
/// there, or `leave` if it's their own; and it isn't the install's last admin.
pub fn may_take(conn: &Connection, taker: &str, subject: &str, relation: &str, object: &str, now: u64) -> Result<bool> {
    if rule(subject, object) {
        return Ok(false);
    }
    let allowed = may(conn, taker, "remove", object, now)? || (subject == taker && may(conn, taker, "leave", object, now)?);
    Ok(allowed && !(relation == ADMIN && object == INSTALL && last_admin(conn, subject)?))
}

/// Whether `maker` may make `object`, new (no row of `relations` names it; whether it was once is the tables' to say),
/// with its first row `(subject, relation, object)`, for good: they may `verb` on `container` (`add-devices` or
/// `add-machines` on a person for a device, `add-apps` on a scope for an app or Vault item, `switch` on a device for
/// a service: the caller says which), and the subject is themselves, the container, or a role on it
/// (`group:<id>#admin`).
#[allow(clippy::too_many_arguments)]
pub fn may_make(conn: &Connection, maker: &str, verb: &str, container: &str, subject: &str, relation: &str, object: &str, now: u64) -> Result<bool> {
    let Some(ty) = type_of(object).filter(|_| !rule(subject, object) && !container.contains('*') && givable(subject)) else {
        return Ok(false);
    };
    let ours = subject == maker || subject.split_once('#').map_or(subject, |(o, _)| o) == container;
    let named: bool = conn.query_row(
        "select exists (select 1 from relations where object = ?1 or substr(subject, 1, length(?1) + 1) = ?1 || '#')",
        [object],
        |r| r.get(0),
    )?;
    Ok(ours && !named && role(conn, ty, relation)?.is_some() && may(conn, maker, verb, container, now)?)
}

/// Whether `who` may make the install's own role `name` on `ty` have `permissions`: they may `change-roles`, each is
/// one of the type's, it isn't one the release ships, and they hold whatever it adds on every object it's held on.
/// An install's own role gives no other (`gives` counts only on the release's).
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
    let mut held = conn.prepare("select distinct object from relations where relation = ?1 and substr(object, 1, instr(object, ':') - 1) = ?2")?;
    let objects = held.query_map(params![name, ty], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
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

    /// Who may do what in a small install: Alice its admin, Bob, Carol, Gina and Hana members, Carol the admin of
    /// group g and Bob in it, Gina its host (an install's own role), Hana group h's admin until 2000, Erin a member
    /// until 1000 and Frank until 2000, Dave holding a role nobody knows, Ivan out of the install; asked at 1500.
    #[test]
    #[allow(clippy::type_complexity)]
    fn who_may_do_what() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        conn.execute_batch(
            r#"insert into roles values ('group', 'host', '["use","invite","leave"]', '[]', 0);
            insert into roles values ('rocket', 'admin', '["use"]', '[]', 0);
            insert into types values ('café', '["use","run"]');
            insert into roles values ('café', 'barista', '["use"]', '[]', 0);"#,
        )
        .unwrap();
        let rows: &[(&str, &str, &str, Option<i64>)] = &[
            ("person:alice", "admin", "install:global", None),
            ("person:bob", "member", "install:global", None),
            ("person:carol", "member", "install:global", None),
            ("person:gina", "member", "install:global", None),
            ("person:hana", "member", "install:global", None),
            ("person:erin", "member", "install:global", Some(1000)),
            ("person:frank", "member", "install:global", Some(2000)),
            ("person:dave", "superuser", "install:global", None),
            ("person:dave", "admin", "rocket:1", None),
            ("person:bob", "admin", "rocket:1", None),
            ("person:bob", "owner", "person:bob", None),
            ("person:carol", "admin", "group:g", None),
            ("person:bob", "member", "group:g", None),
            ("person:gina", "host", "group:g", None),
            ("person:hana", "admin", "group:h", Some(2000)),
            ("group:g#member", "user", "vault:group:g/token", None),
            ("group:g#admin", "owner", "vault:group:g/token", None),
            ("person:frank", "user", "vault:group:g/token", None),
            ("group:g#manager", "user", "vault:group:g/wifi", None),
            ("rocket:1#admin", "user", "vault:group:g/wifi", None),
            ("person:bob", "owner", "device:bb", None),
            ("install:global#member", "user", "service:ss", None),
            ("person:bob", "owner", "service:sb", None),
            ("group:g#admin", "owner", "app:notes", None),
            ("person:bob", "owner", "device:cc", None),
            ("device:cc", "runner", "person:bob", None),
            ("install:global#admin", "admin", "device:sl", None),
            ("device:sl", "runner", "install:global", None),
            ("device:zz", "runner", "install:global", None),
            ("person:ivan", "owner", "person:ivan", None),
            ("person:ivan", "owner", "device:ii", None),
            ("device:ii", "runner", "group:g", None),
            ("person:bob", "barista", "café:1", None),
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
            // an install's own role reaches only what it says, not what members get
            ("person:gina", "use", "group:g", true),
            ("person:gina", "use", "vault:group:g/token", false),
            // the install's admins manage every group, rename every person and look after every device, and open none
            ("person:alice", "invite", "group:g", true),
            ("person:alice", "use", "group:g", false),
            ("person:alice", "use", "vault:group:g/token", false),
            ("person:alice", "rename", "person:bob", true),
            ("person:alice", "use", "person:bob", false),
            ("person:bob", "use", "person:bob", true),
            ("person:carol", "use", "person:carol", true),
            ("person:carol", "use", "person:bob", false),
            ("person:alice", "remove", "device:bb", true),
            ("person:alice", "switch", "device:bb", false),
            ("person:bob", "switch", "device:bb", true),
            ("person:carol", "use", "service:ss", true),
            ("person:erin", "use", "service:ss", false),
            ("device:cc", "run", "person:bob", true),
            ("device:cc", "use", "person:bob", false),
            // a role held on every group reaches what's written for one group's holders of it
            ("person:alice", "use", "vault:group:g/wifi", true),
            ("person:carol", "use", "vault:group:g/wifi", false),
            // someone out of the install holds nothing, nor do their devices; a device nobody in it holds holds nothing
            ("person:ivan", "use", "person:ivan", false),
            ("device:ii", "run", "group:g", false),
            ("device:sl", "run", "install:global", true),
            ("device:zz", "run", "install:global", false),
            // unknown means no, through usersets too
            ("person:bob", "fly", "install:global", false),
            ("person:dave", "change-settings", "install:global", false),
            ("person:dave", "use", "rocket:1", false),
            ("person:bob", "use", "vault:group:g/wifi", false),
            ("#admin", "use", "install:global", false),
            ("install:global#admin", "change-settings", "install:global", false),
            ("person:alice", "use", "install", false),
        ];
        for (who, what, on, expected) in may_cases {
            assert_eq!(may(&conn, who, what, on, now).unwrap(), *expected, "may({who}, {what}, {on})");
        }

        // No escalation: only a role you hold (or one of yours gives), for no longer than you hold it; nothing on a
        // device or an app, and no rule; and the install's last admin stays.
        let give_cases: &[(&str, &str, &str, &str, Option<u64>, bool)] = &[
            ("person:carol", "person:erin", "member", "group:g", None, true),
            ("person:carol", "person:erin", "admin", "group:g", None, true),
            ("person:bob", "person:erin", "member", "group:g", None, false),
            ("person:gina", "person:erin", "host", "group:g", None, true),
            ("person:gina", "person:gina", "member", "group:g", None, false),
            ("person:gina", "person:erin", "member", "group:g", None, false),
            // the install's admins add people to every group, and make no one its admin
            ("person:alice", "person:erin", "member", "group:g", None, true),
            ("person:alice", "person:erin", "admin", "group:g", None, false),
            ("person:alice", "person:erin", "member", "group:h", None, true),
            ("person:hana", "person:erin", "member", "group:h", None, false),
            ("person:hana", "person:erin", "member", "group:h", Some(2000), true),
            ("person:hana", "person:erin", "admin", "group:h", Some(2500), false),
            ("person:alice", "person:bob", "admin", "install:global", None, true),
            ("person:alice", "person:alice", "admin", "install:global", Some(3000), false),
            ("person:bob", "person:erin", "member", "install:global", None, false),
            ("person:carol", "person:erin", "superuser", "group:g", None, false),
            ("person:carol", "person:erin", "user", "vault:group:g/token", None, true),
            ("person:carol", "install:global#member", "user", "vault:group:g/token", None, true),
            ("person:bob", "person:erin", "user", "vault:group:g/token", None, false),
            ("person:bob", "person:carol", "user", "service:sb", None, true),
            ("person:bob", "person:carol", "owner", "device:bb", None, false),
            ("person:carol", "person:erin", "owner", "app:notes", None, false),
            ("person:alice", "person:bob", "manager", "group:*", None, false),
            ("person:alice", "#admin", "admin", "install:global", None, false),
            ("person:ivan", "person:erin", "owner", "person:ivan", None, false),
        ];
        for (giver, to, role, on, until, expected) in give_cases {
            assert_eq!(may_give(&conn, giver, to, role, on, *until, now).unwrap(), *expected, "may_give({giver}, {to}, {role}, {on}, {until:?})");
        }
        let take_cases: &[(&str, &str, &str, &str, bool)] = &[
            ("person:alice", "person:alice", "admin", "install:global", false),
            ("person:alice", "person:bob", "member", "install:global", true),
            ("person:bob", "person:bob", "member", "group:g", true),
            ("person:bob", "person:carol", "admin", "group:g", false),
            ("person:alice", "person:carol", "admin", "group:g", true),
            ("person:bob", "person:bob", "member", "install:global", false),
            ("person:carol", "person:frank", "user", "vault:group:g/token", true),
            ("person:frank", "person:frank", "user", "vault:group:g/token", true),
            ("person:bob", "person:frank", "user", "vault:group:g/token", false),
            ("person:bob", "person:carol", "user", "service:sb", true),
            ("person:alice", "person:bob", "owner", "device:bb", true),
            ("person:carol", "group:g#admin", "owner", "app:notes", false),
            // the release's rules
            ("person:alice", "#admin", "member", "group:*", false),
            ("person:alice", "install:global#admin", "manager", "group:*", false),
            ("person:bob", "@", "owner", "person:*", false),
        ];
        for (taker, from, role, on, expected) in take_cases {
            assert_eq!(may_take(&conn, taker, from, role, on, now).unwrap(), *expected, "may_take({taker}, {from}, {role}, {on})");
        }
        conn.execute("insert into relations values ('person:bob', 'admin', 'install:global', null)", []).unwrap();
        assert!(may_take(&conn, "person:alice", "person:alice", "admin", "install:global", now).unwrap());
        // A person's own scope is theirs by the release's rule, whatever happens to a row saying so.
        conn.execute("delete from relations where subject = 'person:bob' and object = 'person:bob'", []).unwrap();
        assert!(may(&conn, "person:bob", "use", "person:bob", now).unwrap());
        // A rule taken anyway comes back with the release.
        conn.execute("delete from relations where subject = '#admin' and object = 'group:*'", []).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        assert!(may(&conn, "person:carol", "leave", "group:g", now).unwrap());

        // The first row of a new device, Vault item, service or app: by whoever may make it there, to its container.
        let make_cases: &[(&str, &str, &str, &str, &str, &str, bool)] = &[
            ("person:carol", "add-apps", "group:g", "group:g#admin", "owner", "vault:group:g/new", true),
            ("person:bob", "add-apps", "group:g", "group:g#admin", "owner", "vault:group:g/new", false),
            ("person:carol", "add-apps", "group:g", "group:g#admin", "owner", "app:notes", false),
            ("person:carol", "add-apps", "group:g", "install:global#admin", "owner", "app:new", false),
            ("person:carol", "add-apps", "group:g", "group:g#admin", "owner", "app:new", true),
            ("person:alice", "add-machines", "person:bob", "person:bob", "owner", "device:new", true),
            ("person:carol", "add-machines", "person:bob", "person:bob", "owner", "device:new", false),
            ("person:bob", "add-devices", "person:bob", "person:bob", "owner", "device:new", true),
            ("person:bob", "switch", "device:bb", "device:bb#owner", "owner", "service:new", true),
            ("person:bob", "switch", "device:bb", "device:bb#owner", "owner", "service:*", false),
        ];
        for (maker, verb, container, subject, role, object, expected) in make_cases {
            assert_eq!(may_make(&conn, maker, verb, container, subject, role, object, now).unwrap(), *expected, "may_make({maker}, {verb}, {container}, {subject}, {object})");
        }

        // An install's own roles: made by its admins, widened only by someone who holds what it adds where it's held.
        assert!(may_change_role(&conn, "person:alice", "group", "editor", &["use"], now).unwrap());
        assert!(!may_change_role(&conn, "person:carol", "group", "editor", &["use"], now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "editor", &["fly"], now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "member", &["use", "invite"], now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "café", "barista", &["use", "run"], now).unwrap());
        conn.execute("insert into roles values ('group', 'editor', '[\"use\"]', '[]', 0)", []).unwrap();
        conn.execute("insert into relations values ('person:frank', 'editor', 'group:g', null)", []).unwrap();
        assert!(may(&conn, "person:frank", "use", "group:g", now).unwrap());
        assert!(!may_change_role(&conn, "person:alice", "group", "editor", &["use", "add-apps"], now).unwrap());
        conn.execute("insert into relations values ('person:alice', 'admin', 'group:g', null)", []).unwrap();
        assert!(may_change_role(&conn, "person:alice", "group", "editor", &["use", "add-apps"], now).unwrap());
    }
}
