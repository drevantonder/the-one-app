//! Permissions: who may do what, as rows of three tables in a SQLite database it's given (the scope database), and
//! queries over them. No I/O but that connection, and no clock: each check is told the time.
//!
//! - **Types.** Each object type names its permissions, its verbs (`types`). An object is `<type>:<id>`
//!   (`group:0123456789abcdef`); `<type>:*` is every object of that type, for rules.
//! - **Roles.** A role is a name and a set of its type's permissions (`roles`). `builtin` marks the ones the release
//!   ships; a network adds its own. A role of the release's may also give others of its type that its holders don't
//!   hold (`gives`): the network's admins add users to every group without opening any.
//! - **Relations.** A row of `relations` says its subject holds a role (its `relation`) on its object, until a time
//!   (milliseconds; NULL for good). A subject is a principal (`user:<id>`, `device:<key in hex>`), everyone who
//!   holds a role on an object (`group:<id>#member`), or a rule: `#<role>`, everyone who holds that role on the row's
//!   own object (`('#admin', 'member', 'group:*')`: a group's admins are its members), or `@`, each object of the
//!   type itself (`('@', 'owner', 'user:*')`: everyone owns their own scope). A role held on `<type>:*` is held on
//!   each object of that type, for usersets and rules too. A role reached through others lasts only as long as each.
//! - **Rules are the release's**: a row whose subject is `#…` or `@`, or that names `*`, is written by `DEFAULTS` and
//!   by nothing else, and nobody gives or takes one.
//! - **Principals are users in the network, and their devices.** A user holds nothing while they hold no role on
//!   the network, so removing them is taking that; a device holds nothing unless a user in the network, or the
//!   network (`network:global#…`), holds a role on it by a row of its own, so removing it is taking its owner's row.
//!
//! Unknown means no: a type, role or permission this release doesn't know grants nothing, nor anything reached
//! through it, so a row written by a newer one is ignored here, never misread.
//!
//! The core's types, their default roles and the rules between them are `DEFAULTS`, rows like any others. Apps add
//! their own types (`repo`) the same way. Who holds what through them is worked out once the rows are written
//! (`close`), so each check is a lookup. It works out holdings only for principals some row names as its subject,
//! which is enough while the release's `@` rules are only `user:*`'s: an `@` rule of another type needs it to work
//! them out for that type's objects too. Four checks read them:
//! - `may`: whether a principal holds a permission on an object, now.
//! - `may_give`: no escalation. You give a role only where you may `invite`, only one you hold there yourself (or
//!   that a role of yours gives), and for no longer than you hold both.
//! - `may_take`: taking a role takes `remove`, or `leave` for your own; and the network's last admin stays.
//! - `may_make`: the first row of a new object (a device, a Vault item, a service, an app), which nobody holds a role
//!   on yet to give: by whoever the verb that makes it lets, to themselves, its container, or a role on the container.
//!
//! Which object a call or a step is about, and who signed it, is the caller's to say: this only answers. So is moving
//! an app to another owner, which a step's rule allows only someone who may `delete` it and may `add-apps` where it goes.

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

pub const NETWORK: &str = "network:global";
/// The network's role that must always have someone.
const ADMIN: &str = "admin";
const FOREVER: i64 = i64::MAX;

/// The core's types and roles, and the rules between them: run on a database `SCHEMA` made, after it. Each release
/// writes its own; a network's roles and rows are left as they are.
pub const DEFAULTS: &str = r#"
-- Verbs, each for what a user does:
--   use: open its data and its apps
--   invite: give its roles to others (only those you hold, or a role of yours gives)
--   remove: take its roles from others, and start its next key
--   leave: give up your own role on it
--   rename: change its name and handle
--   add-apps: make, add and remove its apps, switch their schedules, and ask for Vault items for them
--   choose-devices: choose which devices keep its apps running, run its calls and open its Vault
--   run: run its calls (a machine whose user holds it, or a device that does)
--   make-groups, change-settings, change-roles: what they say, in the network
--   add-devices: list their phones, computers and services; add-machines: list machines that run things for them
--   switch: turn a device's services on and off
--   change: set or clear a Vault item
--   deploy: send an app a new version, or its source; delete: delete, restore or erase an app
-- A device's and an app's roles come only with making them (may_make): nobody gives one.
insert or replace into types (name, permissions) values
    ('network', '["use","invite","remove","add-apps","choose-devices","run","make-groups","change-settings","change-roles"]'),
    ('user', '["use","remove","rename","add-apps","choose-devices","run","add-devices","add-machines"]'),
    ('group', '["use","invite","remove","leave","rename","add-apps","choose-devices","run"]'),
    ('device', '["rename","remove","switch"]'),
    ('service', '["use","invite","remove","leave"]'),
    ('vault', '["use","change","invite","remove","leave"]'),
    ('app', '["deploy","delete"]');

delete from roles where builtin = 1;
insert or replace into roles (type, name, permissions, builtin) values
    ('network', 'admin', '["use","invite","remove","add-apps","choose-devices","run","make-groups","change-settings","change-roles"]', 1),
    ('network', 'member', '["use","make-groups"]', 1),
    ('network', 'runner', '["run"]', 1),
    -- a user's own scope is theirs; the network's admins only rename them and add machines for them
    ('user', 'owner', '["use","remove","rename","add-apps","choose-devices","run","add-devices","add-machines"]', 1),
    ('user', 'admin', '["rename","add-machines"]', 1),
    ('user', 'runner', '["run"]', 1),
    ('group', 'admin', '["use","invite","remove","leave","rename","add-apps","choose-devices","run"]', 1),
    ('group', 'member', '["use","leave"]', 1),
    ('group', 'runner', '["run"]', 1),
    ('device', 'owner', '["rename","remove","switch"]', 1),
    ('device', 'admin', '["rename","remove"]', 1),
    ('service', 'owner', '["use","invite","remove"]', 1),
    ('service', 'user', '["use","leave"]', 1),
    ('vault', 'owner', '["use","change","invite","remove"]', 1),
    ('vault', 'user', '["use","leave"]', 1),
    ('app', 'owner', '["deploy","delete"]', 1);
-- the network's admins, who manage every group and add users to it, or make someone its admin
-- (so a group whose admins have all gone gets a new one), but open none
insert or replace into roles (type, name, permissions, gives, builtin) values
    ('group', 'manager', '["invite","remove","rename","run"]', '["member","admin"]', 1);

delete from relations where substr(subject, 1, 1) in ('#', '@') or instr(subject, '*') > 0 or instr(object, '*') > 0;
insert into relations (subject, relation, object, until) values
    ('@', 'owner', 'user:*', null),
    ('#admin', 'member', 'network:*', null),
    ('#admin', 'member', 'group:*', null),
    ('#owner', 'user', 'service:*', null),
    ('#owner', 'user', 'vault:*', null),
    ('network:global#admin', 'manager', 'group:*', null),
    ('network:global#admin', 'admin', 'user:*', null),
    ('network:global#admin', 'admin', 'device:*', null);
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

/// Every role each principal holds on which object, through usersets and rules, at any time: `held(principal, object,
/// role, until)`, only roles of known types, `until` the soonest of the rows it came through. A principal is anyone a
/// row names as itself (`user:…`, `device:…`), or as the holders of a role on it; or only `?1`, if it's given. A
/// role reached through others lasts only as long as each, so one is held at a time only by a way whose soonest until
/// is after it.
const HELD: &str = "
with recursive known(type, name, permissions, gives, builtin) as materialized (
    select ro.type, ro.name,
        (select json_group_array(p.value) from json_each(ro.permissions) p
         where p.value in (select value from json_each(t.permissions))),
        ro.gives, ro.builtin
    from roles ro join types t on t.name = ro.type
),
rows(subject, relation, object, until) as materialized (
    select subject, relation, object, coalesce(until, 9223372036854775807) from relations r
    where exists (select 1 from known where type = substr(r.object, 1, instr(r.object, ':') - 1) and name = r.relation)
),
principals(p) as (
    select p from (select iif(instr(subject, '#') > 0, substr(subject, 1, instr(subject, '#') - 1), subject) p from relations)
    where instr(p, ':') > 1 and instr(p, ':') < length(p) and instr(p, '*') = 0 and (?1 is null or p = ?1)
),
-- the rows for everyone who holds a role on an object, and the release's rules, which are few
usersets(holders, every, role, relation, object, until) as materialized (
    select substr(subject, 1, instr(subject, '#') - 1), substr(subject, 1, instr(subject, ':')) || '*',
        substr(subject, instr(subject, '#') + 1), relation, object, until
    from rows where instr(subject, '#') > 1
),
rules(role, relation, object, until) as materialized (
    select substr(subject, 2), relation, object, until from rows where substr(subject, 1, 1) = '#'
),
held(principal, object, role, until) as (
    select subject, object, relation, until from rows where subject in (select p from principals)
    union
    select p, p, relation, until from principals join rows on subject = '@' and object = substr(p, 1, instr(p, ':')) || '*'
    union
    select h.principal, u.object, u.relation, min(h.until, u.until)
    from held h join usersets u on u.role = h.role and (u.holders = h.object or u.every = h.object)
    union
    select h.principal, iif(substr(h.object, -2) = ':*', r.object, h.object), r.relation, min(h.until, r.until)
    from held h join rules r on r.role = h.role
        and (r.object in (h.object, substr(h.object, 1, instr(h.object, ':')) || '*')
            or h.object = substr(r.object, 1, instr(r.object, ':')) || '*')
)";

/// Works out who holds what, once, for every check to look up: each principal's roles on each object with the latest
/// they last to (`held`), until when each counts (`counting`), and so what each may do to each object until when
/// (`allowed`), in tables of the connection's own (`temp`, so no state hash or changeset sees them). Run it once the
/// rows are written, and again whenever they change: until it's run, every check fails. `only` one user, for asking
/// of rows added for a moment (an app's own), leaves everyone else out. Kept in memory, as no device need have a place
/// for temporary files.
pub fn close(conn: &Connection, only: Option<&str>) -> Result<()> {
    conn.execute_batch(
        "pragma temp_store = memory;
        drop table if exists temp.held;
        drop table if exists temp.counting;
        drop table if exists temp.allowed;
        create temp table held (
            principal text not null,
            object text not null,
            role text not null,
            until integer not null,
            -- the role's permissions its type knows, and the roles it gives, both JSON
            permissions text not null,
            gives text not null,
            primary key (principal, object, role)
        ) without rowid;
        create temp table counting (principal text primary key, until integer not null) without rowid;
        create temp table allowed (
            principal text not null,
            permission text not null,
            object text not null,
            until integer not null,
            primary key (principal, permission, object)
        ) without rowid;",
    )?;
    conn.execute(
        &format!(
            "insert into temp.held {HELD}
            select h.principal, h.object, h.role, max(h.until), k.permissions, iif(k.builtin, k.gives, '[]') from held h
                join known k on k.type = substr(h.object, 1, instr(h.object, ':') - 1) and k.name = h.role
            group by h.principal, h.object, h.role"
        ),
        [only],
    )?;
    conn.execute_batch(&format!(
        "-- a user counts while they hold a role on the network
        insert into temp.counting select principal, max(until) from temp.held
        where substr(principal, 1, 5) = 'user:' and object in ('{NETWORK}', 'network:*') group by principal;
        -- a device, while the network's holders of a role, or a user who counts, hold one on it by a row of its own
        insert into temp.counting
        select r.object, max(min(coalesce(r.until, {FOREVER}), iif(substr(r.subject, 1, {n}) = '{NETWORK}#', {FOREVER}, c.until)))
        from relations r join roles ro on ro.type = 'device' and ro.name = r.relation join types t on t.name = ro.type
            left join temp.counting c on c.principal = r.subject
        where substr(r.object, 1, 7) = 'device:' and length(r.object) > 7 and instr(r.object, '#') = 0 and instr(r.object, '*') = 0
            and (substr(r.subject, 1, {n}) = '{NETWORK}#' or c.until is not null)
        group by r.object;
        insert into temp.allowed
        select h.principal, p.value, h.object, min(max(h.until), c.until)
        from temp.held h join temp.counting c on c.principal = h.principal, json_each(h.permissions) p
        group by h.principal, p.value, h.object;",
        n = NETWORK.len() + 1
    ))?;
    Ok(())
}

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
    let mut q = conn.prepare_cached(
        "select role, max(until), permissions, gives from temp.held
        where principal = ?1 and object in (?2, ?3 || ':*') and until > ?4 group by role",
    )?;
    let rows = q.query_map(params![who, object, ty, ms(now)], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
    rows.map(|row| {
        let (role, until, permissions, gives) = row?;
        Ok(Held { role, until, permissions: serde_json::from_str(&permissions)?, gives: serde_json::from_str(&gives)? })
    })
    .collect()
}

/// Whether `principal` counts now: a user who holds a role on the network, or a device that such a user, or the
/// network, holds a role on by a row of its own.
fn counts(conn: &Connection, principal: &str, now: u64) -> Result<bool> {
    let mut q = conn.prepare_cached("select exists (select 1 from temp.counting where principal = ?1 and until > ?2)")?;
    Ok(q.query_row(params![principal, ms(now)], |r| r.get(0))?)
}

/// Whether `principal` (`user:…`, `device:…`) may do `permission` to `object` at `now` (ms): it counts, and holds a
/// role on the object, or on every object of its type, that has the permission, and the type has it too.
pub fn may(conn: &Connection, principal: &str, permission: &str, object: &str, now: u64) -> Result<bool> {
    let Some(ty) = type_of(object) else {
        return Ok(false);
    };
    let mut q = conn.prepare_cached("select exists (select 1 from temp.allowed where principal = ?1 and permission = ?2 and object in (?3, ?4 || ':*') and until > ?5)")?;
    Ok(q.query_row(params![principal, permission, object, ty, ms(now)], |r| r.get(0))?)
}

/// The objects `principal` may do `permission` to now, `<type>:*` for every one of a type: what `may` says yes to, for
/// asking of many objects at once (`allows`).
pub fn objects(conn: &Connection, principal: &str, permission: &str, now: u64) -> Result<Vec<String>> {
    let mut q = conn.prepare_cached("select object from temp.allowed where principal = ?1 and permission = ?2 and until > ?3")?;
    Ok(q.query_map(params![principal, permission, ms(now)], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

/// Whether what `objects` gave has `object`, or every object of its type: what `may` would say of it.
pub fn allows(objects: &[String], object: &str) -> bool {
    type_of(object).is_some_and(|ty| objects.iter().any(|o| o == object || o.strip_suffix(":*") == Some(ty)))
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

/// Whether `subject` is the network's last lasting admin: no other user is one for good.
pub fn last_admin(conn: &Connection, subject: &str) -> Result<bool> {
    Ok(conn.query_row(
        "select exists (select 1 from relations where subject = ?1 and relation = ?2 and object = ?3 and until is null)
            and not exists (select 1 from relations where subject != ?1 and substr(subject, 1, 5) = 'user:'
                and relation = ?2 and object = ?3 and until is null)",
        params![subject, ADMIN, NETWORK],
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
    Ok(until.map_or(FOREVER, ms) <= invite.min(from) && !(until.is_some() && relation == ADMIN && object == NETWORK && last_admin(conn, subject)?))
}

/// Whether `taker` may take the role `relation` on `object` from `subject`: it isn't a rule, they may `remove`
/// there, or `leave` if it's their own; and it isn't the network's last admin.
pub fn may_take(conn: &Connection, taker: &str, subject: &str, relation: &str, object: &str, now: u64) -> Result<bool> {
    if rule(subject, object) {
        return Ok(false);
    }
    let allowed = may(conn, taker, "remove", object, now)? || (subject == taker && may(conn, taker, "leave", object, now)?);
    Ok(allowed && !(relation == ADMIN && object == NETWORK && last_admin(conn, subject)?))
}

/// Whether `maker` may make `object`, new (no row of `relations` names it; whether it was once is the tables' to say),
/// with its first row `(subject, relation, object)`, for good: they may `verb` on `container` (`add-devices` or
/// `add-machines` on a user for a device, `add-apps` on a scope for an app or Vault item, `switch` on a device for
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

#[cfg(test)]
mod tests {
    use crate::permissions::*;

    /// Who may do what in a small network: Alice its admin, Bob, Carol, Gina and Hana members, Carol the admin of
    /// group g and Bob in it, Gina its host (a network's own role), Hana group h's admin until 2000, Erin a member
    /// until 1000 and Frank until 2000, Dave holding a role nobody knows, Ivan out of the network; asked at 1500.
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
            ("user:alice", "admin", "network:global", None),
            ("user:bob", "member", "network:global", None),
            ("user:carol", "member", "network:global", None),
            ("user:gina", "member", "network:global", None),
            ("user:hana", "member", "network:global", None),
            ("user:erin", "member", "network:global", Some(1000)),
            ("user:frank", "member", "network:global", Some(2000)),
            ("user:dave", "superuser", "network:global", None),
            ("user:dave", "admin", "rocket:1", None),
            ("user:bob", "admin", "rocket:1", None),
            ("user:bob", "owner", "user:bob", None),
            ("user:carol", "admin", "group:g", None),
            ("user:bob", "member", "group:g", None),
            ("user:gina", "host", "group:g", None),
            ("user:hana", "admin", "group:h", Some(2000)),
            ("group:g#member", "user", "vault:group:g/token", None),
            ("group:g#admin", "owner", "vault:group:g/token", None),
            ("user:frank", "user", "vault:group:g/token", None),
            ("group:g#manager", "user", "vault:group:g/wifi", None),
            ("rocket:1#admin", "user", "vault:group:g/wifi", None),
            ("user:bob", "owner", "device:bb", None),
            ("network:global#member", "user", "service:ss", None),
            ("user:bob", "owner", "service:sb", None),
            ("group:g#admin", "owner", "app:notes", None),
            ("user:bob", "owner", "device:cc", None),
            ("device:cc", "runner", "user:bob", None),
            ("network:global#admin", "admin", "device:sl", None),
            ("device:sl", "runner", "network:global", None),
            ("device:zz", "runner", "network:global", None),
            ("user:ivan", "owner", "user:ivan", None),
            ("user:ivan", "owner", "device:ii", None),
            ("device:ii", "runner", "group:g", None),
            ("user:bob", "barista", "café:1", None),
        ];
        for (s, r, o, u) in rows {
            conn.execute("insert into relations values (?1, ?2, ?3, ?4)", params![s, r, o, u]).unwrap();
        }
        close(&conn, None).unwrap();
        let now = 1500;
        let may_cases: &[(&str, &str, &str, bool)] = &[
            ("user:alice", "change-settings", "network:global", true),
            ("user:bob", "change-settings", "network:global", false),
            ("user:bob", "use", "network:global", true),
            ("user:bob", "make-groups", "network:global", true),
            ("user:bob", "invite", "network:global", false),
            ("user:erin", "use", "network:global", false),
            ("user:frank", "use", "network:global", true),
            ("user:nobody", "use", "network:global", false),
            // a group's members, and its admins through the rule that they're members too
            ("user:bob", "use", "group:g", true),
            ("user:bob", "invite", "group:g", false),
            ("user:carol", "use", "vault:group:g/token", true),
            ("user:carol", "change", "vault:group:g/token", true),
            ("user:bob", "use", "vault:group:g/token", true),
            ("user:bob", "change", "vault:group:g/token", false),
            ("user:carol", "deploy", "app:notes", true),
            ("user:bob", "deploy", "app:notes", false),
            // a network's own role reaches only what it says, not what members get
            ("user:gina", "use", "group:g", true),
            ("user:gina", "use", "vault:group:g/token", false),
            // the network's admins manage every group, rename every user and look after every device, and open none
            ("user:alice", "invite", "group:g", true),
            ("user:alice", "use", "group:g", false),
            ("user:alice", "use", "vault:group:g/token", false),
            ("user:alice", "rename", "user:bob", true),
            ("user:alice", "use", "user:bob", false),
            ("user:bob", "use", "user:bob", true),
            ("user:carol", "use", "user:carol", true),
            ("user:carol", "use", "user:bob", false),
            ("user:alice", "remove", "device:bb", true),
            ("user:alice", "switch", "device:bb", false),
            ("user:bob", "switch", "device:bb", true),
            ("user:carol", "use", "service:ss", true),
            ("user:erin", "use", "service:ss", false),
            ("device:cc", "run", "user:bob", true),
            ("device:cc", "use", "user:bob", false),
            // a role held on every group reaches what's written for one group's holders of it
            ("user:alice", "use", "vault:group:g/wifi", true),
            ("user:carol", "use", "vault:group:g/wifi", false),
            // someone out of the network holds nothing, nor do their devices; a device nobody in it holds holds nothing
            ("user:ivan", "use", "user:ivan", false),
            ("device:ii", "run", "group:g", false),
            ("device:sl", "run", "network:global", true),
            ("device:zz", "run", "network:global", false),
            // unknown means no, through usersets too
            ("user:bob", "fly", "network:global", false),
            ("user:dave", "change-settings", "network:global", false),
            ("user:dave", "use", "rocket:1", false),
            ("user:bob", "use", "vault:group:g/wifi", false),
            ("#admin", "use", "network:global", false),
            ("network:global#admin", "change-settings", "network:global", false),
            ("user:alice", "use", "network", false),
        ];
        for (who, what, on, expected) in may_cases {
            assert_eq!(may(&conn, who, what, on, now).unwrap(), *expected, "may({who}, {what}, {on})");
            assert_eq!(allows(&objects(&conn, who, what, now).unwrap(), on), *expected, "objects({who}, {what}) has {on}");
        }

        // No escalation: only a role you hold (or one of yours gives), for no longer than you hold it; nothing on a
        // device or an app, and no rule; and the network's last admin stays.
        let give_cases: &[(&str, &str, &str, &str, Option<u64>, bool)] = &[
            ("user:carol", "user:erin", "member", "group:g", None, true),
            ("user:carol", "user:erin", "admin", "group:g", None, true),
            ("user:bob", "user:erin", "member", "group:g", None, false),
            ("user:gina", "user:erin", "host", "group:g", None, true),
            ("user:gina", "user:gina", "member", "group:g", None, false),
            ("user:gina", "user:erin", "member", "group:g", None, false),
            // the network's admins add users to every group, and make someone its admin only as the release says
            ("user:alice", "user:erin", "member", "group:g", None, true),
            ("user:alice", "user:erin", "admin", "group:g", None, true),
            ("user:alice", "user:erin", "member", "group:h", None, true),
            ("user:hana", "user:erin", "member", "group:h", None, false),
            ("user:hana", "user:erin", "member", "group:h", Some(2000), true),
            ("user:hana", "user:erin", "admin", "group:h", Some(2500), false),
            ("user:alice", "user:bob", "admin", "network:global", None, true),
            ("user:alice", "user:alice", "admin", "network:global", Some(3000), false),
            ("user:bob", "user:erin", "member", "network:global", None, false),
            ("user:carol", "user:erin", "superuser", "group:g", None, false),
            ("user:carol", "user:erin", "user", "vault:group:g/token", None, true),
            ("user:carol", "network:global#member", "user", "vault:group:g/token", None, true),
            ("user:bob", "user:erin", "user", "vault:group:g/token", None, false),
            ("user:bob", "user:carol", "user", "service:sb", None, true),
            ("user:bob", "user:carol", "owner", "device:bb", None, false),
            ("user:carol", "user:erin", "owner", "app:notes", None, false),
            ("user:alice", "user:bob", "manager", "group:*", None, false),
            ("user:alice", "#admin", "admin", "network:global", None, false),
            ("user:ivan", "user:erin", "owner", "user:ivan", None, false),
        ];
        for (giver, to, role, on, until, expected) in give_cases {
            assert_eq!(may_give(&conn, giver, to, role, on, *until, now).unwrap(), *expected, "may_give({giver}, {to}, {role}, {on}, {until:?})");
        }
        let take_cases: &[(&str, &str, &str, &str, bool)] = &[
            ("user:alice", "user:alice", "admin", "network:global", false),
            ("user:alice", "user:bob", "member", "network:global", true),
            ("user:bob", "user:bob", "member", "group:g", true),
            ("user:bob", "user:carol", "admin", "group:g", false),
            ("user:alice", "user:carol", "admin", "group:g", true),
            ("user:bob", "user:bob", "member", "network:global", false),
            ("user:carol", "user:frank", "user", "vault:group:g/token", true),
            ("user:frank", "user:frank", "user", "vault:group:g/token", true),
            ("user:bob", "user:frank", "user", "vault:group:g/token", false),
            ("user:bob", "user:carol", "user", "service:sb", true),
            ("user:alice", "user:bob", "owner", "device:bb", true),
            ("user:carol", "group:g#admin", "owner", "app:notes", false),
            // the release's rules
            ("user:alice", "#admin", "member", "group:*", false),
            ("user:alice", "network:global#admin", "manager", "group:*", false),
            ("user:bob", "@", "owner", "user:*", false),
        ];
        for (taker, from, role, on, expected) in take_cases {
            assert_eq!(may_take(&conn, taker, from, role, on, now).unwrap(), *expected, "may_take({taker}, {from}, {role}, {on})");
        }
        conn.execute("insert into relations values ('user:bob', 'admin', 'network:global', null)", []).unwrap();
        close(&conn, None).unwrap();
        assert!(may_take(&conn, "user:alice", "user:alice", "admin", "network:global", now).unwrap());
        // A user's own scope is theirs by the release's rule, whatever happens to a row saying so.
        conn.execute("delete from relations where subject = 'user:bob' and object = 'user:bob'", []).unwrap();
        close(&conn, None).unwrap();
        assert!(may(&conn, "user:bob", "use", "user:bob", now).unwrap());
        // A rule taken anyway comes back with the release.
        conn.execute("delete from relations where subject = '#admin' and object = 'group:*'", []).unwrap();
        conn.execute_batch(DEFAULTS).unwrap();
        close(&conn, None).unwrap();
        assert!(may(&conn, "user:carol", "leave", "group:g", now).unwrap());

        // The first row of a new device, Vault item, service or app: by whoever may make it there, to its container.
        let make_cases: &[(&str, &str, &str, &str, &str, &str, bool)] = &[
            ("user:carol", "add-apps", "group:g", "group:g#admin", "owner", "vault:group:g/new", true),
            ("user:bob", "add-apps", "group:g", "group:g#admin", "owner", "vault:group:g/new", false),
            ("user:carol", "add-apps", "group:g", "group:g#admin", "owner", "app:notes", false),
            ("user:carol", "add-apps", "group:g", "network:global#admin", "owner", "app:new", false),
            ("user:carol", "add-apps", "group:g", "group:g#admin", "owner", "app:new", true),
            ("user:alice", "add-machines", "user:bob", "user:bob", "owner", "device:new", true),
            ("user:carol", "add-machines", "user:bob", "user:bob", "owner", "device:new", false),
            ("user:bob", "add-devices", "user:bob", "user:bob", "owner", "device:new", true),
            ("user:bob", "switch", "device:bb", "device:bb#owner", "owner", "service:new", true),
            ("user:bob", "switch", "device:bb", "device:bb#owner", "owner", "service:*", false),
        ];
        for (maker, verb, container, subject, role, object, expected) in make_cases {
            assert_eq!(may_make(&conn, maker, verb, container, subject, role, object, now).unwrap(), *expected, "may_make({maker}, {verb}, {container}, {subject}, {object})");
        }
    }
}
