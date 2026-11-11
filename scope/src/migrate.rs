//! Old names, read as this release names them: what a release from before the network and app schedules had their
//! names wrote, in the scope database's copies and in signed steps and entries, which can't be written again. A reader
//! turns them into this release's names as it reads them, here and nowhere else; nothing writes the old ones. The network's table and type were `install`; app schedules' tables `keeps` and `keep_keys`, and their
//! step `grant`; the apps added to a scope, its table `installs` (`installed`, `installed_by`, `installed_at`) and
//! steps `install` and `uninstall`; the permission to choose which devices keep a scope's apps running, run its calls and open its Vault
//! was `keep`. The setting that names the Cloudflare node was `serverless`. The Vault's agents, in an item's apps and a
//! request's app, were `agents`, the AGENTS.md app's name, which never used the Vault. A user, as a principal and a
//! type of object, was a `person`.

use rusqlite::Connection;
use serde_json::Value;

use crate::content::{Change, Content};

/// A scope database as an earlier release made it, renamed in place: its tables, and its types', roles' and rules'
/// names. Nothing changes in one this release made.
pub fn database(db: &Connection) -> rusqlite::Result<()> {
    let has = |t: &str| db.query_row("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1", [t], |_| Ok(())).is_ok();
    for (old, new) in [("install", "network"), ("keeps", "app_schedules"), ("keep_keys", "app_schedule_keys")] {
        if has(old) && !has(new) {
            db.execute_batch(&format!("ALTER TABLE {old} RENAME TO {new}"))?;
        }
    }
    if has("installs") && !has("added") {
        db.execute_batch(
            "ALTER TABLE installs RENAME TO added;
             ALTER TABLE added RENAME COLUMN installed TO added;
             ALTER TABLE added RENAME COLUMN installed_by TO added_by;
             ALTER TABLE added RENAME COLUMN installed_at TO added_at;",
        )?;
    }
    if has("vault_requests") {
        db.execute("UPDATE vault_requests SET app = ?1 WHERE app = 'agents'", [crate::scopes::AGENTS])?;
    }
    if has("settings") {
        db.execute_batch("UPDATE OR REPLACE settings SET name = 'cloudflare_node' WHERE name = 'serverless'")?;
    }
    if has("types") && has("roles") && has("relations") {
        db.execute_batch(
            "UPDATE types SET name = 'network' WHERE name = 'install';
             UPDATE types SET permissions = replace(permissions, '\"keep\"', '\"choose-devices\"');
             UPDATE roles SET type = 'network' WHERE type = 'install';
             UPDATE roles SET permissions = replace(permissions, '\"keep\"', '\"choose-devices\"');
             UPDATE relations SET object = 'network:' || substr(object, 9) WHERE object LIKE 'install:%';
             UPDATE relations SET subject = 'network:' || substr(subject, 9) WHERE subject LIKE 'install:%';
             UPDATE types SET name = 'user' WHERE name = 'person';
             UPDATE roles SET type = 'user' WHERE type = 'person';",
        )?;
    }
    if has("principals") {
        // The copy's table still has its old CHECK; `scope::fresh` makes the new one from these rows.
        db.pragma_update(None, "ignore_check_constraints", true)?;
        db.execute_batch("UPDATE principals SET kind = 'user' WHERE kind = 'person'")?;
        db.pragma_update(None, "ignore_check_constraints", false)?;
    }
    for (table, column) in NAMES_A_USER {
        let there: bool = db.query_row("SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2", [table, column], |r| r.get::<_, i64>(0))? > 0;
        if there {
            db.execute_batch(&format!("UPDATE {table} SET {column} = 'user:' || substr({column}, 8) WHERE {column} LIKE 'person:%'"))?;
        }
    }
    Ok(())
}

/// Every column of the scope database that holds a user as `user:<id>`, once `person:<id>`.
const NAMES_A_USER: &[(&str, &str)] = &[
    ("network", "recovery_used_by"),
    ("principals", "id"),
    ("invites", "principal"),
    ("devices", "principal"),
    ("devices", "listed_by"),
    ("runs", "principal"),
    ("runs_chosen", "principal"),
    ("app_schedules", "principal"),
    ("apps", "owner"),
    ("apps", "deleted_by"),
    ("deploys", "deployed_by"),
    ("services", "switched_by"),
    ("added", "added_by"),
    ("schedules", "switched_by"),
    ("vault_requests", "asked_by"),
    ("vault_requests", "answered_by"),
    ("relations", "subject"),
    ("relations", "object"),
];

/// A signed step's or entry's content as an earlier release wrote it, in this release's names: app schedules' step,
/// adding and removing an app, and the types, roles and rules a release's `defaults` step says.
pub fn content(c: &mut Content) {
    if c.app != key::APP {
        return;
    }
    let was = match c.kind.as_str() {
        "grant" => Some(Change::AppSchedules),
        "install" => Some(Change::AddApp),
        "uninstall" => Some(Change::RemoveApp),
        _ => None,
    };
    if let Some(now) = was {
        c.kind = now.name().to_string();
    }
    match c.change() {
        Some(Change::VaultRequest) if c.data["app"] == "agents" => c.data["app"] = crate::scopes::AGENTS.into(),
        Some(Change::Setting) if c.data["name"] == "serverless" => c.data["name"] = crate::scopes::CLOUDFLARE_NODE.into(),
        Some(Change::Defaults) => renamed(&mut c.data),
        _ => {}
    }
}

/// A Vault item's apps, from its row's or its box's `used_by`, or as one from before kept them, in `apps`.
pub fn vault_apps(apps: &mut std::collections::BTreeSet<String>, before: Option<std::collections::BTreeSet<String>>) {
    if let Some(before) = before {
        *apps = before.into_iter().map(|a| if a == "agents" { crate::scopes::AGENTS.to_string() } else { a }).collect();
    }
}

/// Every name in a release's types, roles and rules: the network's type and a user's, their objects and usersets, and the verb.
fn renamed(v: &mut Value) {
    match v {
        Value::String(s) if s == "install" => *s = "network".into(),
        Value::String(s) if s.starts_with("install:") => *s = format!("network:{}", &s["install:".len()..]),
        Value::String(s) if s == "person" => *s = "user".into(),
        Value::String(s) if s.starts_with("person:") => *s = format!("user:{}", &s["person:".len()..]),
        // A role's or type's verbs, as the JSON array of text a row holds.
        Value::String(s) if s.contains("\"keep\"") => *s = s.replace("\"keep\"", "\"choose-devices\""),
        Value::Array(a) => a.iter_mut().for_each(renamed),
        Value::Object(o) => o.values_mut().for_each(renamed),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::migrate::*;

    #[test]
    fn a_database_from_before_reads_as_one_made_now() {
        let old = Connection::open_in_memory().unwrap();
        let new = Connection::open_in_memory().unwrap();
        let now = crate::permissions::DEFAULTS;
        let then = now.replace("'network'", "'install'").replace("network:", "install:").replace("\"choose-devices\"", "\"keep\"").replace("('user',", "('person',").replace("user:*", "person:*");
        let schema = crate::scope::SCHEMA;
        let schema_then = schema.replace("TABLE network ", "TABLE install ").replace("TABLE app_schedules ", "TABLE keeps ").replace("TABLE app_schedule_keys ", "TABLE keep_keys ")
            .replace("TABLE added (\n  app TEXT PRIMARY KEY,\n  added INTEGER NOT NULL,\n  added_by TEXT NOT NULL,\n  added_at", "TABLE installs (\n  app TEXT PRIMARY KEY,\n  installed INTEGER NOT NULL,\n  installed_by TEXT NOT NULL,\n  installed_at")
            .replace("CHECK (kind IN ('user', 'group'))", "CHECK (kind IN ('person', 'group'))");
        assert_ne!(schema, schema_then, "the old tables' names");
        for (db, schema, defaults) in [(&old, &schema_then, &then), (&new, &schema.to_string(), &now.to_string())] {
            db.execute_batch(crate::permissions::SCHEMA).unwrap();
            db.execute_batch(schema).unwrap();
            db.execute_batch(defaults).unwrap();
            let (user, kind) = if std::ptr::eq(db, &old) { ("person:a", "person") } else { ("user:a", "user") };
            db.execute("INSERT INTO principals VALUES (?1, ?2, 'A', 'a', 0)", [user, kind]).unwrap();
            db.execute("INSERT INTO relations VALUES (?1, 'admin', ?2, NULL)", [user.to_string(), crate::permissions::NETWORK.replace("network:", if std::ptr::eq(db, &old) { "install:" } else { "network:" })]).unwrap();
        }
        old.execute("INSERT INTO installs VALUES ('notes', 1, 'person:a', 0)", []).unwrap();
        new.execute("INSERT INTO added VALUES ('notes', 1, 'user:a', 0)", []).unwrap();
        database(&old).unwrap();
        database(&new).unwrap();
        assert_eq!(database::stored::state_hash(&old).unwrap(), database::stored::state_hash(&new).unwrap());
        old.execute("INSERT INTO settings VALUES ('serverless', 'abc')", []).unwrap();
        database(&old).unwrap();
        assert_eq!(old.query_row("SELECT value FROM settings WHERE name = 'cloudflare_node'", [], |r| r.get::<_, String>(0)).unwrap(), "abc");
        old.execute("INSERT INTO vault_requests VALUES ('r', 'agents', 'Key', '{}', '[]', '', NULL, 'a', NULL, 0, NULL, NULL, NULL)", []).unwrap();
        database(&old).unwrap();
        assert_eq!(old.query_row("SELECT app FROM vault_requests", [], |r| r.get::<_, String>(0)).unwrap(), crate::scopes::AGENTS);
        crate::permissions::close(&old, None).unwrap();
        assert!(crate::permissions::may(&old, "user:a", "choose-devices", crate::permissions::NETWORK, 0).unwrap());
    }

    #[test]
    fn a_step_from_before_reads_in_this_releases_names() {
        let mut grant = Content::new(key::APP, "grant", serde_json::json!({ "app": "notes" }));
        content(&mut grant);
        assert!(grant.is(Change::AppSchedules));
        for (old, new) in [("install", Change::AddApp), ("uninstall", Change::RemoveApp)] {
            let mut step = Content::new(key::APP, old, serde_json::json!({ "app": "notes" }));
            content(&mut step);
            assert!(step.is(new));
        }
        let mut setting = crate::content::setting("serverless", Some("abc"));
        content(&mut setting);
        assert_eq!(setting.data["name"], crate::scopes::CLOUDFLARE_NODE);
        let mut request = Content::new(key::APP, "vault-request", serde_json::json!({ "id": "r", "app": "agents" }));
        content(&mut request);
        assert_eq!(request.data["app"], crate::scopes::AGENTS);
        let mut defaults = Content::new(key::APP, "defaults", serde_json::json!({
            "types": [["install", "[\"use\",\"keep\"]"], ["person", "[]"]], "roles": [["install", "admin", "[\"keep\"]", "[]"], ["person", "owner", "[]", "[]"]], "rules": [["install:global#admin", "admin", "install:*"], ["@", "owner", "person:*"]],
        }));
        content(&mut defaults);
        assert_eq!(defaults.data, serde_json::json!({
            "types": [["network", "[\"use\",\"choose-devices\"]"], ["user", "[]"]], "roles": [["network", "admin", "[\"choose-devices\"]", "[]"], ["user", "owner", "[]", "[]"]], "rules": [["network:global#admin", "admin", "network:*"], ["@", "owner", "user:*"]],
        }));
    }
}
