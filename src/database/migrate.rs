//! Old names, read as this release names them: what a release from before the network and app schedules had their
//! names wrote, in the scope database's copies and in signed steps and entries, which can't be written again. A reader
//! turns them into this release's names as it reads them, here and nowhere else; nothing writes the old ones. The network's table and type were `install`; app schedules' tables `keeps` and `keep_keys`, and their
//! step `grant`; the apps added to a scope, its table `installs` (`installed`, `installed_by`, `installed_at`) and
//! steps `install` and `uninstall`; the permission to choose which devices keep a scope's apps running, run its calls and open its Vault
//! was `keep`. The setting that names the Cloudflare node was `serverless`. The Vault's agents, in an item's apps and a
//! request's app, were `agents`, the AGENTS.md app's name, which never used the Vault.

use rusqlite::Connection;
use serde_json::Value;

use crate::database::content::{Change, Content};

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
        db.execute("UPDATE vault_requests SET app = ?1 WHERE app = 'agents'", [crate::database::scopes::AGENTS])?;
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
             UPDATE relations SET subject = 'network:' || substr(subject, 9) WHERE subject LIKE 'install:%';",
        )?;
    }
    Ok(())
}

/// A signed step's or entry's content as an earlier release wrote it, in this release's names: app schedules' step,
/// adding and removing an app, and the types, roles and rules a release's `defaults` step says.
pub fn content(c: &mut Content) {
    if c.app != crate::database::scopes::APP {
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
        Some(Change::VaultRequest) if c.data["app"] == "agents" => c.data["app"] = crate::database::scopes::AGENTS.into(),
        Some(Change::Setting) if c.data["name"] == "serverless" => c.data["name"] = crate::database::scopes::CLOUDFLARE_NODE.into(),
        Some(Change::Defaults) => renamed(&mut c.data),
        _ => {}
    }
}

/// A Vault item's apps, from its row's or its box's `used_by`, or as one from before kept them, in `apps`.
pub fn vault_apps(apps: &mut std::collections::BTreeSet<String>, before: Option<std::collections::BTreeSet<String>>) {
    if let Some(before) = before {
        *apps = before.into_iter().map(|a| if a == "agents" { crate::database::scopes::AGENTS.to_string() } else { a }).collect();
    }
}

/// Every name in a release's types, roles and rules: the network's type, its objects and usersets, and the verb.
fn renamed(v: &mut Value) {
    match v {
        Value::String(s) if s == "install" => *s = "network".into(),
        Value::String(s) if s.starts_with("install:") => *s = format!("network:{}", &s["install:".len()..]),
        // A role's or type's verbs, as the JSON array of text a row holds.
        Value::String(s) if s.contains("\"keep\"") => *s = s.replace("\"keep\"", "\"choose-devices\""),
        Value::Array(a) => a.iter_mut().for_each(renamed),
        Value::Object(o) => o.values_mut().for_each(renamed),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_from_before_reads_as_one_made_now() {
        let old = Connection::open_in_memory().unwrap();
        let new = Connection::open_in_memory().unwrap();
        let now = crate::permissions::DEFAULTS;
        let then = now.replace("'network'", "'install'").replace("network:", "install:").replace("\"choose-devices\"", "\"keep\"");
        let schema = crate::database::scope::SCHEMA;
        let schema_then = schema.replace("TABLE network ", "TABLE install ").replace("TABLE app_schedules ", "TABLE keeps ").replace("TABLE app_schedule_keys ", "TABLE keep_keys ")
            .replace("TABLE added (\n  app TEXT PRIMARY KEY,\n  added INTEGER NOT NULL,\n  added_by TEXT NOT NULL,\n  added_at", "TABLE installs (\n  app TEXT PRIMARY KEY,\n  installed INTEGER NOT NULL,\n  installed_by TEXT NOT NULL,\n  installed_at");
        assert_ne!(schema, schema_then, "the old tables' names");
        for (db, schema, defaults) in [(&old, &schema_then, &then), (&new, &schema.to_string(), &now.to_string())] {
            db.execute_batch(crate::permissions::SCHEMA).unwrap();
            db.execute_batch(schema).unwrap();
            db.execute_batch(defaults).unwrap();
            db.execute("INSERT INTO relations VALUES ('person:a', 'admin', ?1, NULL)", [crate::permissions::NETWORK.replace("network:", if std::ptr::eq(db, &old) { "install:" } else { "network:" })]).unwrap();
        }
        old.execute("INSERT INTO installs VALUES ('notes', 1, 'person:a', 0)", []).unwrap();
        new.execute("INSERT INTO added VALUES ('notes', 1, 'person:a', 0)", []).unwrap();
        database(&old).unwrap();
        database(&new).unwrap();
        assert_eq!(crate::database::stored::state_hash(&old).unwrap(), crate::database::stored::state_hash(&new).unwrap());
        old.execute("INSERT INTO settings VALUES ('serverless', 'abc')", []).unwrap();
        database(&old).unwrap();
        assert_eq!(old.query_row("SELECT value FROM settings WHERE name = 'cloudflare_node'", [], |r| r.get::<_, String>(0)).unwrap(), "abc");
        old.execute("INSERT INTO vault_requests VALUES ('r', 'agents', 'Key', '{}', '[]', '', NULL, 'a', NULL, 0, NULL, NULL, NULL)", []).unwrap();
        database(&old).unwrap();
        assert_eq!(old.query_row("SELECT app FROM vault_requests", [], |r| r.get::<_, String>(0)).unwrap(), crate::database::scopes::AGENTS);
        crate::permissions::close(&old, None).unwrap();
        assert!(crate::permissions::may(&old, "person:a", "choose-devices", crate::permissions::NETWORK, 0).unwrap());
    }

    #[test]
    fn a_step_from_before_reads_in_this_releases_names() {
        let mut grant = Content::new(crate::database::scopes::APP, "grant", serde_json::json!({ "app": "notes" }));
        content(&mut grant);
        assert!(grant.is(Change::AppSchedules));
        for (old, new) in [("install", Change::AddApp), ("uninstall", Change::RemoveApp)] {
            let mut step = Content::new(crate::database::scopes::APP, old, serde_json::json!({ "app": "notes" }));
            content(&mut step);
            assert!(step.is(new));
        }
        let mut setting = crate::database::content::setting("serverless", Some("abc"));
        content(&mut setting);
        assert_eq!(setting.data["name"], crate::database::scopes::CLOUDFLARE_NODE);
        let mut request = Content::new(crate::database::scopes::APP, "vault-request", serde_json::json!({ "id": "r", "app": "agents" }));
        content(&mut request);
        assert_eq!(request.data["app"], crate::database::scopes::AGENTS);
        let mut defaults = Content::new(crate::database::scopes::APP, "defaults", serde_json::json!({
            "types": [["install", "[\"use\",\"keep\"]"]], "roles": [["install", "admin", "[\"keep\"]", "[]"]], "rules": [["install:global#admin", "admin", "install:*"]],
        }));
        content(&mut defaults);
        assert_eq!(defaults.data, serde_json::json!({
            "types": [["network", "[\"use\",\"choose-devices\"]"]], "roles": [["network", "admin", "[\"choose-devices\"]", "[]"]], "rules": [["network:global#admin", "admin", "network:*"]],
        }));
    }
}
