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
