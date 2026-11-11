//! What serves the Vault's tools (vault/src/tools.rs).

use anyhow::{Context, Result, bail};
use api::Caller;
use api::feature::text;
use api::runtime::prelude::*;
use api::serves;
use custom_build::config::CLI;
use scope::scopes::{AGENTS, Scope};
use serde_json::{Map, Value, json};

use crate::items::{list as items_list, set_item};
use crate::{ASK_FOR_AGENTS, fetch, items, requests};

/// What serves each of the Vault's tools (vault/src/tools.rs).
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![
        ("list", serves!(list)),
        ("set", serves!(set)),
        ("use", serves!(switch)),
        ("grant", serves!(grant)),
        ("remove", serves!(remove)),
        ("request", serves!(request)),
        ("fetch", serves!(fetch)),
        ("status", serves!(status)),
    ]
}

/// The scope its input names: user, the default, global, or a group's name or id.
fn scope(rt: &Rt, me: &Caller, input: &Map<String, Value>) -> Result<Scope> {
    rt.read(|r| r.scope_for(&me.user, input.get("scope").and_then(Value::as_str).unwrap_or("user")))
}

async fn list(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let scope = scope(&rt, &me, &input)?;
    Ok(json!(items_list(&rt, &me, Some(&scope)).await?.iter().map(item_said).collect::<Vec<_>>()))
}

/// The apps switched on for an item, as a user says them: Agents for agents.
fn used_by(apps: &Value) -> Vec<&str> {
    apps.as_array().into_iter().flatten().filter_map(Value::as_str).map(|a| if a == AGENTS { "Agents" } else { a }).collect()
}

/// An item as an agent reads it: names, not scope ids or pictures, and when it last changed, in ISO 8601.
fn item_said(it: &Value) -> Value {
    let mut out = json!({
        "id": it["id"], "title": it["title"], "page": it["scope_name"], "fields": it["fields"], "hosts": it["hosts"], "apps": used_by(&it["apps"]), "wanted_by": it["wanted_by"],
        "changed": it["time"].as_i64().and_then(|ms| jiff::Timestamp::from_second(ms / 1000).ok()).map(|t| t.to_string()), "by": it["user"],
    });
    for k in ["type", "domain", "app"] {
        if let Some(v) = it[k].as_str().filter(|v| !v.is_empty()) {
            out[k] = json!(v);
        }
    }
    if it["account"] == true {
        out["account"] = json!(true);
    }
    out
}

/// The item `name` (its id or title) names in `scope`, if there's one.
async fn find_item(rt: &Rt, me: &Caller, scope: &Scope, name: &str) -> Result<Option<Value>> {
    let found: Vec<Value> = items_list(rt, me, Some(scope)).await?.into_iter().filter(|i| i["id"] == name || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(name))).collect();
    anyhow::ensure!(found.len() < 2, "several items are called {name}: give its id");
    Ok(found.into_iter().next())
}

/// Sets `field` of `item` (a new item by that title if there's none), secret if asked or if it was.
async fn set(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let (scope, item, field) = (scope(&rt, &me, &input)?, text(&input, "item")?, text(&input, "field")?);
    let app = input.get("app").and_then(Value::as_str);
    let found = find_item(&rt, &me, &scope, item).await?;
    let was = found.as_ref().and_then(|it| it["fields"].as_array()?.iter().find(|f| f["name"] == field).map(|f| f["secret"] == true));
    let secret = input.get("secret").and_then(Value::as_bool).or(was).unwrap_or(false);
    let mut data = json!({ "fields": { field: { "value": text(&input, "value")?, "secret": secret } } });
    if let Some(hosts) = input.get("hosts") {
        data["hosts"] = hosts.clone();
    }
    match &found {
        Some(it) => {
            data["id"] = it["id"].clone();
            if let Some(app) = app {
                let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default();
                if !apps.contains(&json!(app)) {
                    apps.push(json!(app));
                }
                data["apps"] = json!(apps);
            }
        }
        None => {
            data["title"] = json!(item);
            data["apps"] = json!(app.into_iter().collect::<Vec<_>>());
        }
    }
    let id = set_item(&rt, &me, &scope, data).await?;
    let done = match found {
        None => format!("Made {item} ({id}) in {}, with {field}.", vault_name(&input)),
        Some(_) => format!("Set {item}'s {field}."),
    };
    // Whether an agent may use it now, since only the user lets agents use an item.
    let agents = found.as_ref().is_some_and(|it| it["apps"].as_array().is_some_and(|a| a.iter().any(|a| a == AGENTS))) || app == Some(AGENTS);
    let said = match (agents, app) {
        (true, _) => format!("{done} Agents use it with vault_fetch, as {{{{{id}:{field}}}}}."),
        (false, None) => format!("{done} Agents may not use it yet: {} asks the user to let you, and they pick it.", ASK_FOR_AGENTS),
        (false, Some(_)) => done,
    };
    Ok(json!({ "said": said }))
}

async fn switch(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let (scope, item, app, on) = (scope(&rt, &me, &input)?, text(&input, "item")?, text(&input, "app")?, input.get("on") != Some(&json!(false)));
    let it = find_item(&rt, &me, &scope, item).await?.with_context(|| format!("no item {item} in {}", vault_name(&input)))?;
    let mut apps: Vec<Value> = it["apps"].as_array().cloned().unwrap_or_default().into_iter().filter(|a| a != app).collect();
    if on {
        apps.push(json!(app));
    }
    set_item(&rt, &me, &scope, json!({ "id": it["id"], "apps": apps })).await?;
    Ok(json!({ "said": format!("{} is switched {} for {}.", used_by(&json!([app]))[0], if on { "on" } else { "off" }, it["title"].as_str().unwrap_or(item)) }))
}

/// The Vault its input's scope names, as a user calls it: your Vault, Everyone's Vault, <name>'s Vault.
fn vault_name(input: &Map<String, Value>) -> String {
    named(input.get("scope").and_then(Value::as_str).unwrap_or("user"))
}

fn named(scope: &str) -> String {
    match scope {
        "user" => "your Vault".to_string(),
        "global" => "Everyone's Vault".to_string(),
        group => format!("{group}'s Vault"),
    }
}

/// Asked of this machine's runtime by its own programs, which answers it as its own device (`settings`), then seals
/// what's in that Vault to the new key if it opens it.
async fn grant(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let said = rt.answer_as_device("POST", "/api/vault/grant", &serde_json::to_vec(&input)?).await?;
    let scope = scope(&rt, &me, &input)?;
    if let Err(err) = items::reseal(&rt, &scope).await {
        tracing::warn!("sealing {scope}'s Vault to its new key: {err:#}");
    }
    Ok(said)
}

/// Deletes `item`, or with `field`, one of its fields. When it's not there but is in another Vault you may change,
/// says which and gives the command that removes it.
async fn remove(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let (scope, item, field) = (scope(&rt, &me, &input)?, text(&input, "item")?, input.get("field").and_then(Value::as_str));
    let Some(it) = find_item(&rt, &me, &scope, item).await? else {
        let where_it: Vec<String> = items_list(&rt, &me, None)
            .await?
            .iter()
            .filter(|i| i["id"] == item || i["title"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(item)))
            .map(|it| {
                let s = it["scope"].as_str().unwrap_or("");
                let (flag, vault) = match s {
                    s if s.starts_with("user:") => ("user".to_string(), named("user")),
                    s if s.starts_with("group:") => (s.to_string(), named(it["scope_name"].as_str().unwrap_or(s))),
                    _ => (s.to_string(), named(s)),
                };
                format!("in {vault}: {CLI} vault remove {}{} --scope {flag}", it["id"].as_str().unwrap_or(item), field.map(|f| format!(" {f}")).unwrap_or_default())
            })
            .collect();
        let there = match where_it.is_empty() {
            true => String::new(),
            false => format!("; it's {}", where_it.join(", and ")),
        };
        bail!("no item {item} in {}{there}", vault_name(&input));
    };
    let title = it["title"].as_str().unwrap_or(item);
    let said = match field {
        Some(field) => {
            set_item(&rt, &me, &scope, json!({ "id": it["id"], "fields": { field: null } })).await?;
            format!("Removed {title}'s {field}.")
        }
        None => {
            set_item(&rt, &me, &scope, json!({ "id": it["id"], "deleted": true })).await?;
            format!("Deleted {title}.")
        }
    };
    Ok(json!({ "said": said }))
}

/// Asks the user for an item for an app, or with none, for agents.
async fn request(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let scope = scope(&rt, &me, &input)?;
    let app = input.get("app").and_then(Value::as_str).unwrap_or(AGENTS);
    let ask = json!({ "title": input.get("title"), "fields": input.get("fields"), "hosts": input.get("hosts") });
    let why = input.get("why").and_then(Value::as_str).unwrap_or("");
    let mut asked = requests::request_item(&rt, &me, &scope, app, input.get("need").and_then(Value::as_str), &ask, why).await?;
    asked["said"] = json!(format!(
        "Asked. Give the user this link: {}\nIt's also waiting in their Settings, under Requests. Tell them; check vault_status with id {} when they say they've done it.",
        asked["link"].as_str().unwrap_or(""),
        asked["id"].as_str().unwrap_or("")
    ));
    Ok(asked)
}

async fn fetch(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    fetch::fetch(&rt, &me, &Value::Object(input)).await
}

async fn status(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let id = text(&input, "id")?;
    let asked = requests::item_requests(&rt, &me, None, Some(id)).await?;
    let r = asked.first().with_context(|| format!("no request {id}"))?;
    Ok(json!({ "state": r["state"], "app": r["app"], "title": r["title"], "answered": r["answered"] }))
}
